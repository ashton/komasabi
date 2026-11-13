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
        folder -> Nullable<Text>,
    }
}

diesel::table! {
    library_items (id) {
        id -> Text,
        integration_id -> Text,
        external_id -> Text,
        kind -> Text,
        name -> Text,
        series_external_id -> Text,
        series_name -> Text,
        pages -> Nullable<Integer>,
        pages_read -> Integer,
        added_at -> Text,
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

diesel::allow_tables_to_appear_in_same_query!(integrations, library_items, servers,);
