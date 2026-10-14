//! A small on-disk byte cache for cover images, surviving app restarts.
//!
//! It's intentionally generic: callers pass an opaque string `key` (used
//! directly as the filename) and raw bytes. The key must not contain path
//! separators — callers compose it from safe identifiers (e.g. an integration
//! UUID and a numeric series id).

use std::path::PathBuf;

use directories::ProjectDirs;

fn covers_dir() -> Option<PathBuf> {
    Some(
        ProjectDirs::from("", "", "sabita")?
            .cache_dir()
            .join("covers"),
    )
}

/// The on-disk path a cover with `key` would live at, or `None` if no cache
/// directory could be resolved for this platform/environment.
pub fn cover_path(key: &str) -> Option<PathBuf> {
    Some(covers_dir()?.join(key))
}

/// Reads the cached bytes for `key`, or `None` on a miss (or any read error —
/// a cache miss and an unreadable file are treated the same).
pub fn read_cover(key: &str) -> Option<Vec<u8>> {
    std::fs::read(cover_path(key)?).ok()
}

/// Writes `bytes` to the cache under `key`, creating the cache directory if
/// needed.
pub fn write_cover(key: &str, bytes: &[u8]) -> std::io::Result<()> {
    let path = cover_path(key).ok_or_else(|| {
        std::io::Error::new(
            std::io::ErrorKind::NotFound,
            "no cache directory available for covers",
        )
    })?;

    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }

    std::fs::write(path, bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn read_cover_returns_none_for_a_missing_key() {
        // A key that cannot have been written; a miss must be None, never a
        // panic, even if the cache directory doesn't exist.
        assert!(read_cover("does-not-exist-9c51-6fa51bff9730").is_none());
    }

    #[test]
    fn write_then_read_round_trips_when_a_cache_dir_is_available() {
        // Skip silently if the environment has no resolvable cache dir
        // (`ProjectDirs::from` returning None) — there's nothing to assert.
        let Some(_) = cover_path("probe") else {
            return;
        };

        let key = format!("sabita-cache-test-{}", std::process::id());
        let bytes: &[u8] = &[1, 2, 3, 4];

        write_cover(&key, bytes).expect("write should succeed with a cache dir");
        assert_eq!(read_cover(&key).as_deref(), Some(bytes));

        if let Some(path) = cover_path(&key) {
            let _ = std::fs::remove_file(path);
        }
    }
}
