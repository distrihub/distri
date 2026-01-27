#![allow(clippy::all)]

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
        external_id -> Nullable<Text>,
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

diesel::table! {
    use diesel::sql_types::*;

    message_reads (id) {
        id -> Text,
        thread_id -> Text,
        message_id -> Text,
        user_id -> Text,
        read_at -> Timestamp,
        created_at -> Timestamp,
    }
}

diesel::table! {
    use diesel::sql_types::*;

    message_votes (id) {
        id -> Text,
        thread_id -> Text,
        message_id -> Text,
        user_id -> Text,
        vote_type -> Text,
        comment -> Nullable<Text>,
        created_at -> Timestamp,
        updated_at -> Timestamp,
    }
}

diesel::joinable!(task_messages -> tasks (task_id));
diesel::joinable!(tasks -> threads (thread_id));
diesel::joinable!(external_tool_call_events -> external_tool_calls (tool_call_id));
diesel::joinable!(session_entries -> threads (thread_id));
diesel::joinable!(scratchpad_entries -> threads (thread_id));
diesel::joinable!(message_reads -> threads (thread_id));
diesel::joinable!(message_votes -> threads (thread_id));

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
    message_reads,
    message_votes,
);
