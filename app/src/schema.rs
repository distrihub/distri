diesel::table! {
    users (id) {
        id -> Int4,
        firebase_id -> Varchar,
        created_at -> Timestamp,
        updated_at -> Timestamp,
    }
}

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
        user_id -> Nullable<Int4>,
        tags -> Nullable<Array<Text>>,
    }
}

diesel::joinable!(agents -> users (user_id));
diesel::allow_tables_to_appear_in_same_query!(agents, users);
