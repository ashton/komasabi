use std::{env, fs, path::PathBuf, sync::OnceLock};

use diesel::{Connection, ConnectionError, ConnectionResult, RunQueryDsl, SqliteConnection};
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
    // Diesel turns foreign keys on for every connection, but migrations that
    // rebuild a table (sqlite can't alter a CHECK in place) drop the old one,
    // whose implicit delete then fails for any row still referencing it. As
    // sqlite's own table-rebuild recipe says, they're off while migrating. The
    // pragma is ignored inside a transaction, so it can't go in a migration.
    diesel::sql_query("PRAGMA foreign_keys = OFF")
        .execute(&mut conn)
        .map_err(|e| e.to_string())?;
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
    Some(
        ProjectDirs::from("", "", "komasabi")?
            .data_dir()
            .to_path_buf(),
    )
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

    /// Migrations that rebuild `integrations` drop the old table, which would
    /// trip the `libraries.integration_id` foreign key of a database that
    /// synced a source's libraries.
    #[test]
    fn migrations_rebuild_tables_that_rows_reference() {
        use diesel::migration::MigrationSource;
        use diesel::sql_query;

        let path = env::temp_dir().join(format!(
            "komasabi-migrate-fk-{}.sqlite3",
            std::process::id()
        ));
        let _ = fs::remove_file(&path);
        let url = path.to_string_lossy().into_owned();

        let mut conn = SqliteConnection::establish(&url).unwrap();
        diesel::migration::MigrationConnection::setup(&mut conn).unwrap();
        let mut migrations =
            MigrationSource::<diesel::sqlite::Sqlite>::migrations(&MIGRATIONS).unwrap();
        migrations.sort_by_key(|m| m.name().to_string());
        for migration in &migrations {
            if migration.name().to_string().starts_with("2026-10-20") {
                break;
            }
            conn.run_migration(migration).unwrap();
        }
        sql_query("INSERT INTO integrations (id, name, integration_type, url) VALUES ('k', 'Kavita', 'kavita', 'http://k')")
            .execute(&mut conn)
            .unwrap();
        sql_query("INSERT INTO libraries (id, name, external_id, integration_id) VALUES ('l', 'Manga', '1', 'k')")
            .execute(&mut conn)
            .unwrap();
        drop(conn);

        assert_eq!(migrate(&url), Ok(()));
        let _ = fs::remove_file(&path);
    }
}
