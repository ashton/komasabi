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

#[cfg(feature = "kavita")]
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
        folder: None,
    })
    .await
}

#[cfg(any(feature = "opds", feature = "suwayomi"))]
/// Creates an integration authenticated with basic-auth credentials rather
/// than an API key (an OPDS catalog or a Suwayomi server). Blank credentials
/// are stored as absent.
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
        folder: None,
    })
    .await
}

#[cfg(feature = "komga")]
/// Creates an integration that authenticates with either an API key or
/// basic-auth credentials (a Komga server). Blank values are stored as absent.
pub async fn create_with_auth(
    name: String,
    integration_type: IntegrationType,
    url: String,
    api_key: Option<String>,
    username: Option<String>,
    password: Option<String>,
) -> Result<Integration, String> {
    insert(Integration {
        id: Uuid::new_v4().to_string(),
        name,
        integration_type,
        url: Some(url),
        api_key: api_key.filter(|value| !value.is_empty()),
        username: username.filter(|value| !value.is_empty()),
        password: password.filter(|value| !value.is_empty()),
        folder: None,
    })
    .await
}

/// Creates a local-folder source reading the folder at `path`.
pub async fn create_local_folder(name: String, path: String) -> Result<Integration, String> {
    insert(Integration {
        id: Uuid::new_v4().to_string(),
        name,
        integration_type: IntegrationType::LocalFolder,
        url: None,
        api_key: None,
        username: None,
        password: None,
        folder: Some(path),
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

/// Removes the source `source_id`, along with the items added from it to
/// the Library, which can't be read without it. Both go in one transaction.
pub async fn remove(source_id: String) -> Result<(), String> {
    use crate::schema::{integrations, library_items};
    use diesel_async::AsyncConnection;
    use diesel_async::scoped_futures::ScopedFutureExt;

    let mut conn = database::connect()
        .await
        .map_err(crate::error::log_and_stringify)?;
    conn.transaction::<_, diesel::result::Error, _>(|conn| {
        async move {
            diesel::delete(
                library_items::table.filter(library_items::integration_id.eq(&source_id)),
            )
            .execute(conn)
            .await?;
            diesel::delete(integrations::table.filter(integrations::id.eq(&source_id)))
                .execute(conn)
                .await?;
            Ok(())
        }
        .scope_boxed()
    })
    .await
    .map_err(crate::error::log_and_stringify)
}
