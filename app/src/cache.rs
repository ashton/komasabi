//! A small on-disk byte cache for cover images, surviving app restarts.
//!
//! It's intentionally generic: callers pass an opaque string `key` and raw
//! bytes. Keys become file names through [`file_name`], which escapes anything
//! a filesystem might reject (e.g. the `:` in `vol:42`, invalid on Windows).

use std::path::PathBuf;

use directories::ProjectDirs;

/// The app's cache root for this platform, or `None` if one can't be resolved.
fn cache_root() -> Option<PathBuf> {
    Some(
        ProjectDirs::from("", "", "komasabi")?
            .cache_dir()
            .to_path_buf(),
    )
}

fn covers_dir() -> Option<PathBuf> {
    Some(cache_root()?.join("covers"))
}

/// The on-disk path a cover with `key` would live at, or `None` if no cache
/// directory could be resolved for this platform/environment.
pub fn cover_path(key: &str) -> Option<PathBuf> {
    Some(covers_dir()?.join(file_name(key)))
}

/// The directory a chapter/volume's downloaded reader pages live in, keyed by an
/// opaque `key` the caller composes from safe identifiers (as with covers). The
/// reader reads the page images straight from this folder, so they survive
/// across runs and are reused when the same entry is opened again. Returns
/// `None` if no cache directory could be resolved.
pub fn pages_dir(key: &str) -> Option<PathBuf> {
    Some(cache_root()?.join("pages").join(file_name(key)))
}

/// Where a whole downloaded file (e.g. a remote comic archive read page by
/// page) is kept, keyed like covers.
#[cfg(feature = "opds")]
pub fn download_path(key: &str) -> Option<PathBuf> {
    Some(cache_root()?.join("downloads").join(file_name(key)))
}

/// Longest key used verbatim as a file name; comfortably under the 255-byte
/// limit of common filesystems.
const MAX_KEY_LEN: usize = 160;

/// The file name for `key`: the escaped key when short enough, otherwise a
/// stable hash of it. Provider ids can be long (an OPDS id encodes a URL), and
/// a name past the filesystem's limit would make the cache silently miss.
pub(crate) fn file_name(key: &str) -> String {
    let escaped = escape(key);
    if escaped.len() <= MAX_KEY_LEN {
        escaped
    } else {
        format!("h-{:016x}", fnv1a(key.as_bytes()))
    }
}

/// `key` with every byte outside `[a-z0-9_-]` written as `%XX`. That keeps the
/// name valid everywhere (Windows rejects `:`, `\`, `*`, `?`, … and trailing
/// dots), never `.`/`..`, and distinct on case-insensitive filesystems
/// (Windows, macOS), since uppercase letters are escaped too. The mapping is
/// injective, so two keys never share a file.
fn escape(key: &str) -> String {
    use std::fmt::Write;

    key.bytes()
        .fold(String::with_capacity(key.len()), |mut out, byte| {
            if byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-' || byte == b'_' {
                out.push(char::from(byte));
            } else {
                let _ = write!(out, "%{byte:02X}");
            }
            out
        })
}

/// 64-bit FNV-1a: tiny, and stable across Rust versions and runs (unlike
/// `DefaultHasher`), which an on-disk cache name needs.
fn fnv1a(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0xcbf2_9ce4_8422_2325, |hash, byte| {
        (hash ^ u64::from(*byte)).wrapping_mul(0x0000_0100_0000_01b3)
    })
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
    fn file_name_keeps_short_keys_and_hashes_long_ones_stably() {
        assert_eq!(file_name("integration-42"), "integration-42");

        let long = "x".repeat(MAX_KEY_LEN + 1);
        let name = file_name(&long);
        assert!(name.starts_with("h-") && name.len() == 18);
        assert_eq!(name, file_name(&long));
        assert_ne!(name, file_name(&"y".repeat(MAX_KEY_LEN + 1)));
    }

    #[test]
    fn file_name_escapes_characters_windows_rejects_and_uppercase() {
        assert_eq!(file_name("ns-vol:42"), "ns-vol%3A42");
        assert_eq!(file_name("a/b\\c*?\"<>|"), "a%2Fb%5Cc%2A%3F%22%3C%3E%7C");
        assert_eq!(file_name(".."), "%2E%2E");
        assert_ne!(file_name("Ab"), file_name("ab"));
        assert_ne!(file_name("%3A"), file_name(":"));
    }

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

        let key = format!("komasabi-cache-test-{}", std::process::id());
        let bytes: &[u8] = &[1, 2, 3, 4];

        write_cover(&key, bytes).expect("write should succeed with a cache dir");
        assert_eq!(read_cover(&key).as_deref(), Some(bytes));

        if let Some(path) = cover_path(&key) {
            let _ = std::fs::remove_file(path);
        }
    }
}
