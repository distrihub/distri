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
    }
}
