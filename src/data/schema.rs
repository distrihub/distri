// @generated automatically by Diesel CLI.

diesel::table! {
    agents (id) {
        id -> Int4,
        #[max_length = 255]
        name -> Varchar,
        description -> Nullable<Text>,
        tools -> Nullable<Jsonb>,
        #[max_length = 255]
        model -> Varchar,
        model_settings -> Nullable<Jsonb>,
        #[max_length = 255]
        provider_name -> Varchar,
        prompt -> Nullable<Text>,
        avatar -> Nullable<Text>,
        created_at -> Timestamp,
        updated_at -> Timestamp,
        user_id -> Nullable<Int4>,
        tags -> Nullable<Array<Nullable<Text>>>,
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
