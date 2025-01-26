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
    users (id) {
        id -> Int4,
        #[max_length = 128]
        firebase_id -> Varchar,
        created_at -> Timestamp,
        updated_at -> Timestamp,
    }
}

diesel::joinable!(agents -> users (user_id));

diesel::allow_tables_to_appear_in_same_query!(
    agents,
    users,
);
