use diesel::{ExpressionMethods, QueryDsl, SelectableHelper};
use diesel_async::RunQueryDsl;
use uuid::Uuid;

use crate::{
    database,
    models::integration::{Integration, IntegrationType},
};

pub async fn all() -> Result<Vec<Integration>, String> {
    use crate::schema::integrations::dsl::*;

    let mut conn = database::connect()
        .await
        .map_err(crate::error::log_and_stringify)?;
    integrations
        .select(Integration::as_select())
        .load(&mut conn)
        .await
        .map_err(crate::error::log_and_stringify)
}

pub async fn find(integration_id: &str) -> Result<Integration, String> {
    use crate::schema::integrations::dsl::*;

    let mut conn = database::connect()
        .await
        .map_err(crate::error::log_and_stringify)?;
    integrations
        .filter(id.eq(integration_id))
        .select(Integration::as_select())
        .first(&mut conn)
        .await
        .map_err(crate::error::log_and_stringify)
}

pub async fn create(
    name: String,
    integration_type: IntegrationType,
    url: Option<String>,
    api_key: Option<String>,
) -> Result<Integration, String> {
    insert(Integration {
        id: Uuid::new_v4().to_string(),
        name,
        integration_type,
        url,
        api_key,
        username: None,
        password: None,
    })
    .await
}

/// Creates an integration authenticated with basic-auth credentials rather
/// than an API key (an OPDS catalog). Blank credentials are stored as absent.
pub async fn create_with_credentials(
    name: String,
    integration_type: IntegrationType,
    url: String,
    username: Option<String>,
    password: Option<String>,
) -> Result<Integration, String> {
    insert(Integration {
        id: Uuid::new_v4().to_string(),
        name,
        integration_type,
        url: Some(url),
        api_key: None,
        username: username.filter(|value| !value.is_empty()),
        password: password.filter(|value| !value.is_empty()),
    })
    .await
}

async fn insert(new_integration: Integration) -> Result<Integration, String> {
    use crate::schema::integrations::dsl::integrations;

    let mut conn = database::connect()
        .await
        .map_err(crate::error::log_and_stringify)?;
    diesel::insert_into(integrations)
        .values(&new_integration)
        .execute(&mut conn)
        .await
        .map_err(crate::error::log_and_stringify)?;

    Ok(new_integration)
}
