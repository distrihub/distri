use crate::{FileType, Part};

#[test]
fn part_file_serializes_with_part_type_file() {
    let part = Part::File(FileType::Bytes {
        bytes: "JVBERi0xLjQKJ...".to_string(),
        mime_type: "application/pdf".to_string(),
        name: Some("report.pdf".to_string()),
    });

    let json = serde_json::to_value(&part).unwrap();
    assert_eq!(json["part_type"], "file");
    assert_eq!(json["data"]["type"], "bytes");
    assert_eq!(json["data"]["mime_type"], "application/pdf");
    assert_eq!(json["data"]["name"], "report.pdf");
}

#[test]
fn part_file_round_trips_through_json() {
    let original = Part::File(FileType::Url {
        url: "https://example.com/doc.pdf".to_string(),
        mime_type: "application/pdf".to_string(),
        name: None,
    });

    let json = serde_json::to_string(&original).unwrap();
    let decoded: Part = serde_json::from_str(&json).unwrap();

    match decoded {
        Part::File(FileType::Url {
            url,
            mime_type,
            name,
        }) => {
            assert_eq!(url, "https://example.com/doc.pdf");
            assert_eq!(mime_type, "application/pdf");
            assert_eq!(name, None);
        }
        _ => panic!("expected Part::File(Url)"),
    }
}

#[test]
fn part_file_type_name_is_file() {
    let part = Part::File(FileType::Bytes {
        bytes: String::new(),
        mime_type: "text/plain".to_string(),
        name: None,
    });
    assert_eq!(part.type_name(), "file");
}

#[test]
fn filetype_mime_type_accessor_works_for_both_variants() {
    let b = FileType::Bytes {
        bytes: String::new(),
        mime_type: "application/pdf".to_string(),
        name: None,
    };
    assert_eq!(b.mime_type(), "application/pdf");

    let u = FileType::Url {
        url: String::new(),
        mime_type: "text/csv".to_string(),
        name: None,
    };
    assert_eq!(u.mime_type(), "text/csv");
}

use distri_a2a::{Message as A2AMessage, Part as A2APart, PartContent};
use serde_json::json;

fn inbound(part: A2APart) -> crate::Message {
    A2AMessage {
        parts: vec![part],
        ..Default::default()
    }
    .try_into()
    .unwrap()
}

#[test]
fn a2a_pdf_inbound_becomes_part_file() {
    let msg = inbound(
        A2APart::raw(b"%PDF-1.4\n".to_vec())
            .with_media_type("application/pdf")
            .with_filename("doc.pdf"),
    );
    match &msg.parts[0] {
        Part::File(FileType::Bytes {
            bytes,
            mime_type,
            name,
        }) => {
            assert_eq!(bytes, "JVBERi0xLjQK");
            assert_eq!(mime_type, "application/pdf");
            assert_eq!(name.as_deref(), Some("doc.pdf"));
        }
        other => panic!("expected Part::File(Bytes), got {:?}", other),
    }
}

#[test]
fn a2a_image_inbound_still_becomes_part_image() {
    let msg = inbound(A2APart::url("https://example.com/cat.png").with_media_type("image/png"));
    match &msg.parts[0] {
        Part::Image(FileType::Url { url, mime_type, .. }) => {
            assert_eq!(url, "https://example.com/cat.png");
            assert_eq!(mime_type, "image/png");
        }
        other => panic!("expected Part::Image(Url), got {:?}", other),
    }
}

#[test]
fn a2a_unknown_mime_inbound_becomes_part_file_no_error() {
    let msg = inbound(A2APart::raw(vec![0, 0, 0]));
    assert!(matches!(&msg.parts[0], Part::File(_)));
}

#[test]
fn legacy_0x_file_part_inbound_still_reads() {
    let part: A2APart = serde_json::from_value(json!({
        "kind": "file",
        "file": {"bytes": "JVBERi0xLjQK", "mimeType": "application/pdf", "name": "doc.pdf"}
    }))
    .unwrap();
    assert!(matches!(
        &inbound(part).parts[0],
        Part::File(FileType::Bytes { .. })
    ));
}

#[test]
fn distri_part_file_outbound_becomes_a2a_url_part() {
    let a2a: A2APart = Part::File(FileType::Url {
        url: "https://example.com/r.pdf".to_string(),
        mime_type: "application/pdf".to_string(),
        name: Some("r.pdf".to_string()),
    })
    .into();
    assert_eq!(
        a2a.content,
        PartContent::Url("https://example.com/r.pdf".into())
    );
    assert_eq!(a2a.media_type.as_deref(), Some("application/pdf"));
    assert_eq!(a2a.filename.as_deref(), Some("r.pdf"));
}

#[test]
fn plain_data_part_is_accepted_as_data() {
    let msg = inbound(A2APart::data(json!({"city": "Paris"})));
    assert_eq!(msg.parts[0], Part::Data(json!({"city": "Paris"})));
}

#[test]
fn distri_parts_ride_as_data_with_part_type_metadata() {
    let call = Part::ToolCall(crate::ToolCall {
        tool_call_id: "c1".into(),
        tool_name: "search".into(),
        input: json!({"q": "x"}),
    });
    let a2a: A2APart = call.clone().into();
    let wire = serde_json::to_value(&a2a).unwrap();
    assert_eq!(wire["metadata"]["distri/part_type"], "tool_call");
    assert_eq!(wire["data"]["tool_call_id"], "c1");
    assert!(wire["data"].get("part_type").is_none(), "no 0.x wrapper");
    // And back.
    assert_eq!(inbound(a2a).parts[0], call);
}

#[test]
fn legacy_wrapped_distri_part_is_still_accepted() {
    let msg = inbound(A2APart::data(json!({
        "part_type": "tool_result",
        "data": {"tool_call_id": "c1", "tool_name": "search", "parts": [{"part_type": "text", "data": "ok"}]}
    })));
    assert!(matches!(&msg.parts[0], Part::ToolResult(r) if r.tool_call_id == "c1"));
}
