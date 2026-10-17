use diesel::{ExpressionMethods, QueryDsl, SelectableHelper};
use diesel_async::RunQueryDsl;
use uuid::Uuid;

use crate::{
    database,
    models::library::{Library, LibraryType},
};

pub async fn all() -> Result<Vec<Library>, String> {
    use crate::schema::libraries::dsl::*;

    let mut conn = database::connect()
        .await
        .map_err(crate::error::log_and_stringify)?;
    libraries
        .select(Library::as_select())
        .load(&mut conn)
        .await
        .map_err(crate::error::log_and_stringify)
}

pub async fn find_by_external_id(remote_id: &str) -> Result<Library, String> {
    use crate::schema::libraries::dsl::*;

    let mut conn = database::connect()
        .await
        .map_err(crate::error::log_and_stringify)?;
    libraries
        .filter(external_id.eq(remote_id))
        .select(Library::as_select())
        .first(&mut conn)
        .await
        .map_err(crate::error::log_and_stringify)
}

/// Creates a local, filesystem-backed library over `path` (a folder or a single
/// imported file). It gets its own id as its `external_id` so it resolves
/// through [`provider_for_library`](crate::providers::provider_for_library) the
/// same way a remote library does, and no integration.
pub async fn create_local(
    name: String,
    kind: LibraryType,
    path: String,
) -> Result<Library, String> {
    use crate::schema::libraries::dsl::libraries;

    let id = Uuid::new_v4().to_string();
    let new_library = Library {
        id: id.clone(),
        kind,
        name,
        external_id: Some(id),
        folder: Some(path),
        cover: None,
        integration_id: None,
    };

    let mut conn = database::connect()
        .await
        .map_err(crate::error::log_and_stringify)?;
    diesel::insert_into(libraries)
        .values(&new_library)
        .execute(&mut conn)
        .await
        .map_err(crate::error::log_and_stringify)?;

    Ok(new_library)
}

pub async fn create(library: Library) -> Result<Library, String> {
    use crate::schema::libraries::dsl::libraries;

    let mut conn = database::connect()
        .await
        .map_err(crate::error::log_and_stringify)?;
    diesel::insert_into(libraries)
        .values(&library)
        .execute(&mut conn)
        .await
        .map_err(crate::error::log_and_stringify)?;

    Ok(library)
}
