diesel::table! {
    agents (id) {
        id -> Int4,
        name -> Varchar,
        description -> Nullable<Text>,
        tools -> Nullable<Jsonb>,
        model -> Varchar,
        model_settings -> Nullable<Jsonb>,
        provider_name -> Varchar,
        prompt -> Nullable<Text>,
        avatar -> Nullable<Text>,
        created_at -> Timestamp,
        updated_at -> Timestamp,
    }
}
