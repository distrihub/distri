// @generated automatically by Diesel CLI.

diesel::table! {
    agents (id) {
        id -> Int4,
        #[max_length = 255]
        name -> Varchar,
        description -> Text,
        definition -> Jsonb,
        created_at -> Timestamp,
        updated_at -> Timestamp,
        user_id -> Nullable<Int4>,
    }
}

diesel::table! {
    sessions (id) {
        id -> Int4,
        user_id -> Int4,
        cookie_string -> Text,
        session_token -> Text,
        created_at -> Timestamp,
        expires_at -> Timestamp,
    }
}

diesel::table! {
    user_memory (id) {
        id -> Int4,
        user_id -> Int4,
        memory -> Text,
        created_at -> Timestamp,
        valid_until -> Nullable<Timestamp>,
    }
}

diesel::table! {
    users (id) {
        id -> Int4,
        #[max_length = 128]
        twitter_id -> Varchar,
        #[max_length = 255]
        name -> Varchar,
        description -> Nullable<Text>,
        #[max_length = 255]
        location -> Nullable<Varchar>,
        #[max_length = 255]
        twitter_url -> Nullable<Varchar>,
        #[max_length = 255]
        profile_image_url -> Nullable<Varchar>,
        created_at -> Timestamp,
        updated_at -> Timestamp,
    }
}

diesel::joinable!(agents -> users (user_id));
diesel::joinable!(sessions -> users (user_id));
diesel::joinable!(user_memory -> users (user_id));

diesel::allow_tables_to_appear_in_same_query!(
    agents,
    sessions,
    user_memory,
    users,
);
