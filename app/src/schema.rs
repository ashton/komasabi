// @generated automatically by Diesel CLI.

diesel::table! {
    use diesel::sql_types::{Nullable, Text};
    use crate::models::integration::IntegrationTypeMapping;

    integrations (id) {
        id -> Text,
        name -> Text,
        integration_type -> IntegrationTypeMapping,
        url -> Nullable<Text>,
        api_key -> Nullable<Text>,
        username -> Nullable<Text>,
        password -> Nullable<Text>,
    }
}

diesel::table! {
    use diesel::sql_types::{Nullable, Text, Timestamp};
    use crate::models::job::{JobStatusMapping, JobTypeMapping};

    jobs (id) {
        id -> Text,
        job_type -> JobTypeMapping,
        status -> JobStatusMapping,
        integration_id -> Nullable<Text>,
        error -> Nullable<Text>,
        created_at -> Timestamp,
        started_at -> Nullable<Timestamp>,
        finished_at -> Nullable<Timestamp>,
    }
}

diesel::table! {
    use diesel::sql_types::{Nullable, Text};
    use crate::models::library::LibraryTypeMapping;

    libraries (id) {
        id -> Text,
        name -> Text,
        external_id -> Nullable<Text>,
        folder -> Nullable<Text>,
        cover -> Nullable<Text>,
        kind -> LibraryTypeMapping,
        integration_id -> Nullable<Text>,
    }
}

diesel::table! {
    servers (id) {
        id -> Integer,
        name -> Text,
        server_type -> Text,
        url -> Text,
    }
}

diesel::joinable!(libraries -> integrations (integration_id));

diesel::allow_tables_to_appear_in_same_query!(integrations, jobs, libraries, servers,);
