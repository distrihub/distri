#![allow(clippy::all)]

use diesel::allow_tables_to_appear_in_same_query;
use diesel::joinable;

pub mod types {
    pub type Jsonb = diesel::sql_types::Text;
}

diesel::table! {
    use diesel::sql_types::*;
    use crate::schema::types::Jsonb;

    agent_configs (name) {
        name -> Text,
        config -> Jsonb,
        created_at -> Timestamp,
        updated_at -> Timestamp,
    }
}

diesel::table! {
    use diesel::sql_types::*;
    use crate::schema::types::Jsonb;

    threads (id) {
        id -> Text,
        agent_id -> Text,
        title -> Text,
        created_at -> Timestamp,
        updated_at -> Timestamp,
        message_count -> Integer,
        last_message -> Nullable<Text>,
        metadata -> Jsonb,
        attributes -> Jsonb,
    }
}

diesel::table! {
    use diesel::sql_types::*;

    tasks (id) {
        id -> Text,
        thread_id -> Text,
        parent_task_id -> Nullable<Text>,
        status -> Text,
        created_at -> BigInt,
        updated_at -> BigInt,
    }
}

diesel::table! {
    use diesel::sql_types::*;
    use crate::schema::types::Jsonb;

    task_messages (id) {
        id -> Integer,
        task_id -> Text,
        kind -> Text,
        payload -> Jsonb,
        created_at -> BigInt,
    }
}

diesel::table! {
    use diesel::sql_types::*;
    use crate::schema::types::Jsonb;

    session_entries (thread_id, key) {
        thread_id -> Text,
        key -> Text,
        value -> Jsonb,
        expiry -> Nullable<Timestamp>,
        created_at -> Timestamp,
        updated_at -> Timestamp,
    }
}

diesel::table! {
    use diesel::sql_types::*;
    use crate::schema::types::Jsonb;

    memory_entries (id) {
        id -> Integer,
        user_id -> Text,
        content -> Jsonb,
        created_at -> Timestamp,
    }
}

diesel::table! {
    use diesel::sql_types::*;
    use crate::schema::types::Jsonb;

    scratchpad_entries (id) {
        id -> Integer,
        thread_id -> Text,
        task_id -> Text,
        parent_task_id -> Nullable<Text>,
        entry -> Jsonb,
        entry_type -> Nullable<Text>,
        timestamp -> BigInt,
        created_at -> Timestamp,
    }
}

diesel::table! {
    use diesel::sql_types::*;

    browser_sessions (user_id) {
        user_id -> Text,
        state -> Text,
        created_at -> Timestamp,
        updated_at -> Timestamp,
    }
}

diesel::table! {
    use diesel::sql_types::*;
    use crate::schema::types::Jsonb;

    integrations (id) {
        id -> Text,
        user_id -> Text,
        provider -> Text,
        session_data -> Nullable<Jsonb>,
        secrets_data -> Nullable<Jsonb>,
        oauth_state -> Nullable<Text>,
        oauth_state_data -> Nullable<Jsonb>,
        created_at -> BigInt,
        updated_at -> BigInt,
    }
}

diesel::table! {
    use diesel::sql_types::*;
    use crate::schema::types::Jsonb;

    external_tool_calls (id) {
        id -> Text,
        status -> Text,
        request -> Nullable<Jsonb>,
        response -> Nullable<Jsonb>,
        created_at -> Timestamp,
        updated_at -> Timestamp,
        locked_at -> Nullable<Timestamp>,
    }
}

diesel::table! {
    use diesel::sql_types::*;
    use crate::schema::types::Jsonb;

    external_tool_call_events (id) {
        id -> Integer,
        tool_call_id -> Text,
        payload -> Jsonb,
        created_at -> Timestamp,
    }
}

diesel::table! {
    use diesel::sql_types::*;

    plugin_catalog (package_name) {
        package_name -> Text,
        version -> Nullable<Text>,
        object_prefix -> Text,
        entrypoint -> Nullable<Text>,
        artifact_json -> Text,
        updated_at -> Timestamp,
    }
}

diesel::table! {
    use diesel::sql_types::*;

    prompt_templates (id) {
        id -> Text,
        name -> Text,
        template -> Text,
        description -> Nullable<Text>,
        version -> Nullable<Text>,
        source -> Text,
        is_system -> Integer,
        created_at -> Timestamp,
        updated_at -> Timestamp,
    }
}

diesel::table! {
    use diesel::sql_types::*;
    use crate::schema::types::Jsonb;

    server_settings (id) {
        id -> Text,
        config_json -> Jsonb,
        created_at -> Timestamp,
        updated_at -> Timestamp,
    }
}

diesel::table! {
    use diesel::sql_types::*;

    secrets (id) {
        id -> Text,
        key -> Text,
        value -> Text,
        created_at -> Timestamp,
        updated_at -> Timestamp,
    }
}

diesel::joinable!(task_messages -> tasks (task_id));
diesel::joinable!(tasks -> threads (thread_id));
diesel::joinable!(external_tool_call_events -> external_tool_calls (tool_call_id));
diesel::joinable!(session_entries -> threads (thread_id));
diesel::joinable!(scratchpad_entries -> threads (thread_id));

diesel::allow_tables_to_appear_in_same_query!(
    agent_configs,
    threads,
    tasks,
    task_messages,
    session_entries,
    memory_entries,
    scratchpad_entries,
    integrations,
    external_tool_calls,
    external_tool_call_events,
    plugin_catalog,
    browser_sessions,
    prompt_templates,
    server_settings,
    secrets,
);

diesel::table! {
    use diesel::sql_types::*;
    use crate::schema::types::Jsonb;

    browser_sequences (id) {
        id -> Text,
        goal -> Nullable<Text>,
        task_id -> Nullable<Text>,
        thread_id -> Nullable<Text>,
        created_at -> Timestamp,
        updated_at -> Timestamp,
    }
}

diesel::table! {
    use diesel::sql_types::*;
    use crate::schema::types::Jsonb;

    browser_steps (id) {
        id -> Text,
        browser_sequence_id -> Text,
        commands -> Jsonb,
        reason -> Nullable<Text>,
        thread_id -> Nullable<Text>,
        task_id -> Nullable<Text>,
        run_id -> Nullable<Text>,
        thinking -> Nullable<Text>,
        evaluation_previous_goal -> Nullable<Text>,
        memory -> Nullable<Text>,
        next_goal -> Nullable<Text>,
        success -> Bool,
        action_result -> Nullable<Jsonb>,
        created_at -> Timestamp,
        updated_at -> Timestamp,
    }
}

diesel::table! {
    use diesel::sql_types::*;
    use crate::schema::types::Jsonb;

    browser_step_observations (id) {
        id -> Integer,
        thread_id -> Text,
        task_id -> Text,
        run_id -> Text,
        sequence_id -> Text,
        observation -> Jsonb,
        created_at -> Timestamp,
    }
}

diesel::table! {
    use diesel::sql_types::*;
    use crate::schema::types::Jsonb;

    browser_step_screenshots (id) {
        id -> Integer,
        thread_id -> Text,
        task_id -> Text,
        run_id -> Text,
        sequence_id -> Text,
        screenshot -> Jsonb,
        created_at -> Timestamp,
    }
}

joinable!(browser_steps -> browser_sequences (browser_sequence_id));
joinable!(browser_step_observations -> browser_sequences (sequence_id));
joinable!(browser_step_screenshots -> browser_sequences (sequence_id));

allow_tables_to_appear_in_same_query!(
    browser_sequences,
    browser_steps,
    browser_step_observations,
    browser_step_screenshots,
);
