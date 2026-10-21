use std::{env, fs, path::PathBuf, sync::OnceLock};

use diesel::{Connection, ConnectionError, ConnectionResult, SqliteConnection};
use diesel_async::{AsyncConnection, sync_connection_wrapper::SyncConnectionWrapper};
use diesel_migrations::{EmbeddedMigrations, MigrationHarness, embed_migrations};
use directories::ProjectDirs;

const DATABASE_FILE: &str = "komasabi.sqlite3";

const MIGRATIONS: EmbeddedMigrations = embed_migrations!("migrations");

/// The outcome of migrating the database, which happens once per process,
/// before the first connection is handed out.
static MIGRATED: OnceLock<Result<(), String>> = OnceLock::new();

pub async fn connect() -> ConnectionResult<SyncConnectionWrapper<SqliteConnection>> {
    let database_url = database_url()?;
    MIGRATED
        .get_or_init(|| migrate(&database_url))
        .clone()
        .map_err(ConnectionError::BadConnection)?;
    SyncConnectionWrapper::<SqliteConnection>::establish(&database_url).await
}

/// Runs the migrations compiled into the binary that the database hasn't seen
/// yet, so a fresh install gets its tables without `diesel` being around.
fn migrate(database_url: &str) -> Result<(), String> {
    let mut conn = SqliteConnection::establish(database_url).map_err(|e| e.to_string())?;
    conn.run_pending_migrations(MIGRATIONS)
        .map(|_| ())
        .map_err(|e| format!("failed to migrate the database: {e}"))
}

/// `DATABASE_URL` when set (mise sets it for development), otherwise the
/// database file in the app's data dir, which is created if it's missing since
/// sqlite only creates the file, not its parent folders.
fn database_url() -> ConnectionResult<String> {
    if let Ok(url) = env::var("DATABASE_URL") {
        return Ok(url);
    }
    let data_dir = data_dir().ok_or_else(|| {
        ConnectionError::BadConnection("no data directory for this platform".to_string())
    })?;
    fs::create_dir_all(&data_dir).map_err(|e| ConnectionError::BadConnection(e.to_string()))?;
    Ok(data_dir.join(DATABASE_FILE).to_string_lossy().into_owned())
}

fn data_dir() -> Option<PathBuf> {
    Some(ProjectDirs::from("", "", "komasabi")?.data_dir().to_path_buf())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn embedded_migrations_build_a_fresh_database() {
        let path = env::temp_dir().join(format!("komasabi-migrate-{}.sqlite3", std::process::id()));
        let _ = fs::remove_file(&path);
        let url = path.to_string_lossy().into_owned();

        assert_eq!(migrate(&url), Ok(()));
        // A second run finds nothing pending.
        assert_eq!(migrate(&url), Ok(()));

        let mut conn = SqliteConnection::establish(&url).unwrap();
        assert!(!conn.has_pending_migration(MIGRATIONS).unwrap());
        let _ = fs::remove_file(&path);
    }
}
