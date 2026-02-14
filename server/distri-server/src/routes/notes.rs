use actix_web::{web, HttpMessage, HttpRequest, HttpResponse};
use distri_core::agent::AgentOrchestrator;
use distri_core::ToolAuthRequestContext;
use distri_types::stores::{CreateNoteRequest, NoteSearchFilter, UpdateNoteRequest};
use serde::Deserialize;
use serde_json::json;
use std::sync::Arc;

use crate::context::UserContext;

pub fn configure_note_routes(cfg: &mut web::ServiceConfig) {
    cfg.service(
        web::resource("/notes")
            .route(web::get().to(list_notes))
            .route(web::post().to(create_note)),
    )
    .service(web::resource("/notes/search").route(web::post().to(search_notes)))
    .service(
        web::resource("/notes/{id}")
            .route(web::get().to(get_note))
            .route(web::put().to(update_note))
            .route(web::delete().to(delete_note)),
    )
    .service(web::resource("/notes/{id}/summarise").route(web::post().to(summarise_note)));
}

fn get_user_id(req: &HttpRequest) -> String {
    req.extensions()
        .get::<UserContext>()
        .map(|ctx: &UserContext| ctx.user_id())
        .unwrap_or_else(|| "local_dev_user".to_string())
}

#[derive(Debug, Deserialize)]
struct ListNotesQuery {
    limit: Option<u32>,
    offset: Option<u32>,
}

async fn list_notes(
    query: web::Query<ListNotesQuery>,
    executor: web::Data<Arc<AgentOrchestrator>>,
    req: HttpRequest,
) -> HttpResponse {
    let store = match &executor.stores.note_store {
        Some(s) => s,
        None => {
            return HttpResponse::InternalServerError()
                .json(json!({"error": "Note store not initialized"}))
        }
    };

    let user_id = get_user_id(&req);

    match store
        .list_notes(&user_id, query.limit, query.offset)
        .await
    {
        Ok(response) => HttpResponse::Ok().json(response),
        Err(e) => HttpResponse::InternalServerError().json(json!({"error": e.to_string()})),
    }
}

async fn create_note(
    executor: web::Data<Arc<AgentOrchestrator>>,
    payload: web::Json<CreateNoteRequest>,
    req: HttpRequest,
) -> HttpResponse {
    let store = match &executor.stores.note_store {
        Some(s) => s,
        None => {
            return HttpResponse::InternalServerError()
                .json(json!({"error": "Note store not initialized"}))
        }
    };

    let user_id = get_user_id(&req);

    match store.create_note(&user_id, payload.into_inner()).await {
        Ok(note) => {
            // Trigger async summarisation via notes_summariser agent
            let note_id = note.id.clone();
            let orchestrator = executor.get_ref().clone();
            tokio::spawn(async move {
                if let Err(e) = trigger_note_summarisation(&orchestrator, &note_id).await {
                    tracing::warn!("Failed to trigger note summarisation for {}: {}", note_id, e);
                }
            });

            HttpResponse::Ok().json(note)
        }
        Err(e) => HttpResponse::InternalServerError().json(json!({"error": e.to_string()})),
    }
}

async fn get_note(
    id: web::Path<String>,
    executor: web::Data<Arc<AgentOrchestrator>>,
) -> HttpResponse {
    let store = match &executor.stores.note_store {
        Some(s) => s,
        None => {
            return HttpResponse::InternalServerError()
                .json(json!({"error": "Note store not initialized"}))
        }
    };

    match store.get_note(&id).await {
        Ok(Some(note)) => HttpResponse::Ok().json(note),
        Ok(None) => HttpResponse::NotFound().json(json!({"error": "Note not found"})),
        Err(e) => HttpResponse::InternalServerError().json(json!({"error": e.to_string()})),
    }
}

async fn update_note(
    id: web::Path<String>,
    executor: web::Data<Arc<AgentOrchestrator>>,
    payload: web::Json<UpdateNoteRequest>,
    _req: HttpRequest,
) -> HttpResponse {
    let store = match &executor.stores.note_store {
        Some(s) => s,
        None => {
            return HttpResponse::InternalServerError()
                .json(json!({"error": "Note store not initialized"}))
        }
    };

    let note_id = id.into_inner();
    let has_content_change = payload.content.is_some();

    match store.update_note(&note_id, payload.into_inner()).await {
        Ok(note) => {
            // Re-summarise if content changed
            if has_content_change {
                let orchestrator = executor.get_ref().clone();
                let nid = note.id.clone();
                tokio::spawn(async move {
                    if let Err(e) = trigger_note_summarisation(&orchestrator, &nid).await {
                        tracing::warn!(
                            "Failed to trigger note re-summarisation for {}: {}",
                            nid,
                            e
                        );
                    }
                });
            }
            HttpResponse::Ok().json(note)
        }
        Err(e) => HttpResponse::InternalServerError().json(json!({"error": e.to_string()})),
    }
}

async fn delete_note(
    id: web::Path<String>,
    executor: web::Data<Arc<AgentOrchestrator>>,
) -> HttpResponse {
    let store = match &executor.stores.note_store {
        Some(s) => s,
        None => {
            return HttpResponse::InternalServerError()
                .json(json!({"error": "Note store not initialized"}))
        }
    };

    match store.delete_note(&id).await {
        Ok(_) => HttpResponse::NoContent().finish(),
        Err(e) => HttpResponse::InternalServerError().json(json!({"error": e.to_string()})),
    }
}

#[derive(Debug, Deserialize)]
struct SearchNotesRequest {
    #[serde(flatten)]
    filter: NoteSearchFilter,
    limit: Option<u32>,
    offset: Option<u32>,
}

async fn search_notes(
    executor: web::Data<Arc<AgentOrchestrator>>,
    payload: web::Json<SearchNotesRequest>,
    req: HttpRequest,
) -> HttpResponse {
    let store = match &executor.stores.note_store {
        Some(s) => s,
        None => {
            return HttpResponse::InternalServerError()
                .json(json!({"error": "Note store not initialized"}))
        }
    };

    let user_id = get_user_id(&req);
    let search = payload.into_inner();

    match store
        .search_notes(&user_id, &search.filter, search.limit, search.offset)
        .await
    {
        Ok(response) => HttpResponse::Ok().json(response),
        Err(e) => HttpResponse::InternalServerError().json(json!({"error": e.to_string()})),
    }
}

async fn summarise_note(
    id: web::Path<String>,
    executor: web::Data<Arc<AgentOrchestrator>>,
    req: HttpRequest,
) -> HttpResponse {
    let store = match &executor.stores.note_store {
        Some(s) => s,
        None => {
            return HttpResponse::InternalServerError()
                .json(json!({"error": "Note store not initialized"}))
        }
    };

    let note_id = id.into_inner();
    let _user_id = get_user_id(&req);

    // Verify note exists
    match store.get_note(&note_id).await {
        Ok(Some(_)) => {}
        Ok(None) => return HttpResponse::NotFound().json(json!({"error": "Note not found"})),
        Err(e) => {
            return HttpResponse::InternalServerError().json(json!({"error": e.to_string()}))
        }
    }

    match trigger_note_summarisation(executor.get_ref(), &note_id).await {
        Ok(_) => {
            // Return the updated note
            match store.get_note(&note_id).await {
                Ok(Some(note)) => HttpResponse::Ok().json(note),
                Ok(None) => {
                    HttpResponse::NotFound().json(json!({"error": "Note not found after update"}))
                }
                Err(e) => {
                    HttpResponse::InternalServerError().json(json!({"error": e.to_string()}))
                }
            }
        }
        Err(e) => HttpResponse::InternalServerError()
            .json(json!({"error": format!("Summarisation failed: {}", e)})),
    }
}

/// Trigger summarisation of a note using the notes_summariser agent via LLM execute
async fn trigger_note_summarisation(
    orchestrator: &Arc<AgentOrchestrator>,
    note_id: &str,
) -> anyhow::Result<()> {
    let store = orchestrator
        .stores
        .note_store
        .as_ref()
        .ok_or_else(|| anyhow::anyhow!("Note store not initialized"))?;

    let note = store
        .get_note(note_id)
        .await?
        .ok_or_else(|| anyhow::anyhow!("Note not found: {}", note_id))?;

    // Use the LLM service to summarise
    let service = distri_core::llm_service::LlmExecuteService::new(orchestrator.clone());

    let prompt = format!(
        "Analyse the following markdown note and return a JSON object with these fields:\n\
         - \"summary\": A concise 1-3 sentence summary of the note\n\
         - \"tags\": An array of relevant topic tags (lowercase, no spaces, use hyphens)\n\
         - \"keywords\": An array of important keywords from the content\n\
         - \"headings\": An array of all headings found in the markdown\n\n\
         Return ONLY the JSON object, no markdown formatting or explanation.\n\n\
         Note title: {}\n\n\
         Note content:\n{}",
        note.title, note.content
    );

    let mut message = distri_types::Message::default();
    message.role = distri_types::MessageRole::User;
    message.parts = vec![distri_types::Part::Text(prompt)];

    let messages = vec![message];

    let model_settings = orchestrator.get_default_model_settings().await;

    let result = service
        .execute(
            "system".to_string(),
            None,
            "notes_summariser".to_string(),
            None,
            None,
            None,
            messages,
            vec![],
            model_settings,
            None,
            Some(format!("Summarise: {}", note.title)),
            None,
            true,
        )
        .await;

    match result {
        Ok(exec_result) => {
            // LLMResponse.content is a String
            let response_text = &exec_result.response.content;

            // Try to extract JSON from the response (handle markdown code blocks)
            let json_str = response_text
                .trim()
                .trim_start_matches("```json")
                .trim_start_matches("```")
                .trim_end_matches("```")
                .trim();

            if let Ok(parsed) = serde_json::from_str::<serde_json::Value>(json_str) {
                let summary = parsed
                    .get("summary")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                let tags: Vec<String> = parsed
                    .get("tags")
                    .and_then(|v| v.as_array())
                    .map(|arr| {
                        arr.iter()
                            .filter_map(|v| v.as_str().map(String::from))
                            .collect()
                    })
                    .unwrap_or_default();
                let keywords: Vec<String> = parsed
                    .get("keywords")
                    .and_then(|v| v.as_array())
                    .map(|arr| {
                        arr.iter()
                            .filter_map(|v| v.as_str().map(String::from))
                            .collect()
                    })
                    .unwrap_or_default();
                let headings: Vec<String> = parsed
                    .get("headings")
                    .and_then(|v| v.as_array())
                    .map(|arr| {
                        arr.iter()
                            .filter_map(|v| v.as_str().map(String::from))
                            .collect()
                    })
                    .unwrap_or_default();

                // Merge existing tags with generated tags
                let mut all_tags = note.tags.clone();
                for tag in &tags {
                    if !all_tags
                        .iter()
                        .any(|t| t.to_lowercase() == tag.to_lowercase())
                    {
                        all_tags.push(tag.clone());
                    }
                }

                store
                    .update_note_index(note_id, &summary, headings, keywords, all_tags)
                    .await?;
                tracing::info!("Note {} summarised successfully", note_id);
            } else {
                tracing::warn!(
                    "Failed to parse summarisation response as JSON for note {}: {}",
                    note_id,
                    json_str
                );
            }
        }
        Err(e) => {
            tracing::warn!("LLM summarisation call failed for note {}: {}", note_id, e);
        }
    }

    Ok(())
}
