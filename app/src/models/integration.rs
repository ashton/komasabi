use diesel::{Insertable, Selectable, deserialize::Queryable};
use diesel_derive_enum::DbEnum;

use crate::schema::integrations;

#[derive(Clone, Copy, Debug, DbEnum, PartialEq, Eq)]
pub enum IntegrationType {
    Kavita,
    Komga,
    Opds,
    Suwayomi,
}

impl IntegrationType {
    pub fn label(&self) -> &'static str {
        match self {
            IntegrationType::Kavita => "Kavita",
            IntegrationType::Komga => "Komga",
            IntegrationType::Suwayomi => "Suwayomi",
            IntegrationType::Opds => "OPDS Server",
        }
    }
}

#[derive(Clone, Debug, Queryable, Selectable, Insertable, PartialEq)]
#[diesel(table_name = integrations)]
pub struct Integration {
    pub id: String,
    pub name: String,
    pub integration_type: IntegrationType,
    pub url: Option<String>,
    pub api_key: Option<String>,
    /// Basic-auth credentials, for integrations that authenticate that way
    /// (OPDS catalogs).
    pub username: Option<String>,
    pub password: Option<String>,
}
