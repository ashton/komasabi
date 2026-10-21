//! A [`Provider`] backed by the local filesystem, the counterpart to
//! [`KavitaProvider`](crate::providers::kavita::provider::KavitaProvider) for
//! libraries imported from a folder (or a single file) rather than a remote
//! service.
//!
//! Like the Kavita provider, it holds no local copy of the library's contents:
//! every call re-reads the filesystem, so the library always reflects what's on
//! disk. The structural rules it applies:
//!
//! - The library root is either a **single file** (imported on its own) or a
//!   **folder**. A file root is one series holding one standalone volume.
//! - In a folder root, each child (sub-folder or loose file) is a **series**.
//! - Within a series folder, if it holds sub-folders each is a **volume** whose
//!   files are its **chapters**; if it holds only files, each file is a
//!   standalone volume.
//!
//! Reading is supported for `.cbz`/`.zip` archives (the images inside, in name
//! order). Other formats (`.cbr`, `.pdf`, `.epub`, `.doc`) are listed and
//! browsable but have no extractable pages yet, so opening them in the reader
//! yields nothing rather than failing.

use std::path::{Path, PathBuf};

use crate::models::{
    library_item::LibraryItem,
    series_entry::{EntryKind, SeriesEntry},
};
use crate::providers::{PageRef, Provider};

/// File extensions treated as page images inside an archive.
const IMAGE_EXTENSIONS: &[&str] = &["png", "jpg", "jpeg", "gif", "webp", "bmp", "avif"];

/// Archive extensions whose image entries can be extracted for reading.
const READABLE_ARCHIVE_EXTENSIONS: &[&str] = &["cbz", "zip"];

pub struct FileSystemProvider {
    /// The library's root path: either a single imported file or a folder.
    root: PathBuf,
}

impl FileSystemProvider {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    /// Reads from the on-disk cover cache for `key`, falling back to `produce`
    /// (run on a blocking thread via the caller's runtime) and writing the
    /// result back. Mirrors the Kavita provider's cover caching so both backends
    /// behave identically from the screens' point of view.
    fn cached_cover(&self, key: String, produce: impl FnOnce() -> Option<Vec<u8>>) -> Result<Vec<u8>, String> {
        if let Some(bytes) = crate::cache::read_cover(&key) {
            return Ok(bytes);
        }

        let bytes = produce().ok_or_else(|| "no cover image available".to_string())?;

        if let Err(error) = crate::cache::write_cover(&key, &bytes) {
            tracing::debug!("failed to cache cover for {key}: {error}");
        }

        Ok(bytes)
    }
}

impl Provider for FileSystemProvider {
    async fn list_items(
        &self,
        _library_external_id: &str,
        page: Option<u32>,
        page_size: Option<u32>,
    ) -> Result<Vec<LibraryItem>, String> {
        let series = scan_series(&self.root);

        let items: Vec<LibraryItem> = series
            .into_iter()
            .map(|path| LibraryItem {
                external_id: encode_id(&path),
                name: display_name(&path),
                cover: String::new(),
                library_id: encode_id(&self.root),
                pages: None,
            })
            .collect();

        Ok(paginate(items, page, page_size))
    }

    async fn list_series_entries(&self, series_id: &str) -> Result<Vec<SeriesEntry>, String> {
        let series_path = decode_id(series_id)?;

        Ok(scan_volumes(&series_path)
            .into_iter()
            .map(|path| SeriesEntry {
                external_id: encode_id(&path),
                kind: EntryKind::Volume,
                label: display_name(&path),
                pages: None,
            })
            .collect())
    }

    async fn cover(&self, integration_id: &str, series_id: &str) -> Result<Vec<u8>, String> {
        let path = decode_id(series_id)?;
        self.cached_cover(format!("{integration_id}-{series_id}"), || first_image_under(&path))
    }

    async fn volume_cover(&self, integration_id: &str, volume_id: &str) -> Result<Vec<u8>, String> {
        let path = decode_id(volume_id)?;
        self.cached_cover(format!("{integration_id}-vol-{volume_id}"), || {
            first_image_under(&path)
        })
    }

    async fn chapter_cover(
        &self,
        integration_id: &str,
        chapter_id: &str,
    ) -> Result<Vec<u8>, String> {
        let path = decode_id(chapter_id)?;
        self.cached_cover(format!("{integration_id}-ch-{chapter_id}"), || {
            first_image_under(&path)
        })
    }

    async fn reader_pages(&self, entry: &SeriesEntry) -> Result<Vec<PageRef>, String> {
        let path = decode_id(&entry.external_id)?;

        let mut pages = Vec::new();
        for archive in chapter_files(&path) {
            let count = archive_image_count(&archive);
            let chapter_id = encode_id(&archive);
            for page in 0..count {
                pages.push(PageRef {
                    chapter_id: chapter_id.clone(),
                    page: page as u16,
                });
            }
        }

        Ok(pages)
    }

    async fn reader_image(&self, chapter_id: &str, page: u16) -> Result<Vec<u8>, String> {
        let path = decode_id(chapter_id)?;
        archive_image_bytes(&path, page as usize)
            .ok_or_else(|| format!("no page {page} in {}", path.display()))
    }
}

/// Encodes a path as a filesystem- and cache-safe, reversible id: the hex of its
/// UTF-8 bytes. Hex contains only `[0-9a-f]`, so an id never carries a path
/// separator into a cache key, yet [`decode_id`] recovers the exact path without
/// a rescan.
fn encode_id(path: &Path) -> String {
    let bytes = path.to_string_lossy();
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes.as_bytes() {
        out.push_str(&format!("{byte:02x}"));
    }
    out
}

fn decode_id(id: &str) -> Result<PathBuf, String> {
    if id.len() % 2 != 0 {
        return Err(format!("malformed filesystem id: {id}"));
    }

    let mut bytes = Vec::with_capacity(id.len() / 2);
    for pair in id.as_bytes().chunks(2) {
        let hex = std::str::from_utf8(pair).map_err(|_| format!("malformed filesystem id: {id}"))?;
        let byte = u8::from_str_radix(hex, 16).map_err(|_| format!("malformed filesystem id: {id}"))?;
        bytes.push(byte);
    }

    let string = String::from_utf8(bytes).map_err(|_| format!("malformed filesystem id: {id}"))?;
    Ok(PathBuf::from(string))
}

/// A human-readable name for a path: the file stem for files, the directory name
/// for folders, falling back to the full path when neither is available.
fn display_name(path: &Path) -> String {
    if path.is_dir() {
        path.file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_else(|| path.to_string_lossy().into_owned())
    } else {
        path.file_stem()
            .map(|stem| stem.to_string_lossy().into_owned())
            .unwrap_or_else(|| path.to_string_lossy().into_owned())
    }
}

/// The series of a library root: the root itself if it's a single file,
/// otherwise each of its children (sub-folders and loose files), name-sorted.
fn scan_series(root: &Path) -> Vec<PathBuf> {
    if root.is_file() {
        return vec![root.to_path_buf()];
    }
    sorted_children(root)
}

/// The volumes of a series. A file series is a single standalone volume. A
/// folder series whose children include sub-folders exposes every child (folders
/// as multi-chapter volumes, loose files as standalone volumes); a folder with
/// only files exposes each file as a standalone volume.
fn scan_volumes(series: &Path) -> Vec<PathBuf> {
    if series.is_file() {
        return vec![series.to_path_buf()];
    }
    sorted_children(series)
}

/// The chapter archive files that make up a volume, in reading order: the file
/// itself for a standalone (file) volume, otherwise the volume folder's files
/// (name-sorted), descending into sub-folders if it holds no direct files.
fn chapter_files(volume: &Path) -> Vec<PathBuf> {
    if volume.is_file() {
        return vec![volume.to_path_buf()];
    }

    let children = sorted_children(volume);
    let files: Vec<PathBuf> = children.iter().filter(|p| p.is_file()).cloned().collect();
    if !files.is_empty() {
        return files;
    }

    children
        .iter()
        .filter(|p| p.is_dir())
        .flat_map(|dir| chapter_files(dir))
        .collect()
}

/// The directory's entries as absolute paths, sorted by file name so reading
/// order is stable and matches what the user sees in a file browser.
fn sorted_children(dir: &Path) -> Vec<PathBuf> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };

    let mut paths: Vec<PathBuf> = entries
        .filter_map(|entry| entry.ok().map(|entry| entry.path()))
        .filter(|path| !is_hidden(path))
        .collect();
    paths.sort();
    paths
}

/// Whether a path is a hidden/dotfile entry, skipped during scanning.
fn is_hidden(path: &Path) -> bool {
    path.file_name()
        .map(|name| name.to_string_lossy().starts_with('.'))
        .unwrap_or(false)
}

/// The first page image found under `path` (its own first image if it's a
/// readable archive, otherwise the first image of its first chapter), used as a
/// cover. `None` when nothing readable is found.
fn first_image_under(path: &Path) -> Option<Vec<u8>> {
    for archive in chapter_files(path) {
        if let Some(bytes) = archive_image_bytes(&archive, 0) {
            return Some(bytes);
        }
    }
    None
}

/// Whether a file name has a page-image extension.
fn is_image_name(name: &str) -> bool {
    extension_matches(Path::new(name), IMAGE_EXTENSIONS)
}

/// Whether `path`'s extension matches one of `candidates`, case-insensitively.
fn extension_matches(path: &Path, candidates: &[&str]) -> bool {
    path.extension()
        .and_then(|ext| ext.to_str())
        .map(|ext| ext.to_ascii_lowercase())
        .is_some_and(|ext| candidates.contains(&ext.as_str()))
}

/// The number of page images in a readable archive, or 0 for an unreadable or
/// unsupported file.
fn archive_image_count(path: &Path) -> usize {
    zip_image_names(path).map(|names| names.len()).unwrap_or(0)
}

/// The bytes of the `index`-th page image in an archive, name-sorted, or `None`
/// if the archive is unsupported/unreadable or has no such page.
fn archive_image_bytes(path: &Path, index: usize) -> Option<Vec<u8>> {
    use std::io::Read;

    let names = zip_image_names(path)?;
    let name = names.get(index)?;

    let file = std::fs::File::open(path).ok()?;
    let mut archive = zip::ZipArchive::new(file).ok()?;
    let mut entry = archive.by_name(name).ok()?;

    let mut bytes = Vec::new();
    entry.read_to_end(&mut bytes).ok()?;
    Some(bytes)
}

/// The names of the image entries in a zip-based archive (`.cbz`/`.zip`),
/// name-sorted. `None` for any other extension or an unreadable file.
fn zip_image_names(path: &Path) -> Option<Vec<String>> {
    if !extension_matches(path, READABLE_ARCHIVE_EXTENSIONS) {
        return None;
    }

    let file = std::fs::File::open(path).ok()?;
    let mut archive = zip::ZipArchive::new(file).ok()?;

    // A plain loop rather than an iterator chain: `by_index` borrows `archive`
    // mutably, so an entry can't escape a closure into a lazy adaptor.
    let mut names = Vec::new();
    for i in 0..archive.len() {
        if let Ok(entry) = archive.by_index(i) {
            if entry.is_file() && is_image_name(entry.name()) {
                names.push(entry.name().to_string());
            }
        }
    }
    names.sort();
    Some(names)
}

/// Keeps only the requested page of `items`, matching the Kavita provider's
/// pagination contract (1-based page number). An out-of-range page yields an
/// empty slice, which the browse screen reads as "no more items".
fn paginate(items: Vec<LibraryItem>, page: Option<u32>, page_size: Option<u32>) -> Vec<LibraryItem> {
    let (Some(page), Some(page_size)) = (page, page_size) else {
        return items;
    };
    if page == 0 || page_size == 0 {
        return items;
    }

    let start = ((page - 1) * page_size) as usize;
    items.into_iter().skip(start).take(page_size as usize).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encode_id_round_trips_through_decode() {
        let path = PathBuf::from("/home/john/comics/One Piece/Vol 1.cbz");
        let encoded = encode_id(&path);

        assert!(encoded.chars().all(|c| c.is_ascii_hexdigit()));
        assert_eq!(decode_id(&encoded).unwrap(), path);
    }

    #[test]
    fn decode_id_rejects_malformed_ids() {
        assert!(decode_id("zz").is_err());
        assert!(decode_id("abc").is_err());
    }

    #[test]
    fn display_name_uses_the_file_stem_for_files() {
        // A non-existent path is treated as a file (not a dir), so the stem wins.
        assert_eq!(display_name(Path::new("/x/One Piece Vol 1.cbz")), "One Piece Vol 1");
    }

    #[test]
    fn is_image_name_matches_known_extensions_case_insensitively() {
        assert!(is_image_name("page01.JPG"));
        assert!(is_image_name("cover.png"));
        assert!(!is_image_name("info.txt"));
        assert!(!is_image_name("chapter.cbz"));
    }

    #[test]
    fn paginate_slices_by_page_and_size() {
        let items: Vec<LibraryItem> = (0..5)
            .map(|i| LibraryItem {
                external_id: i.to_string(),
                name: i.to_string(),
                cover: String::new(),
                library_id: "lib".to_string(),
                pages: None,
            })
            .collect();

        let page2 = paginate(items.clone(), Some(2), Some(2));
        assert_eq!(page2.len(), 2);
        assert_eq!(page2[0].name, "2");

        let past_end = paginate(items, Some(9), Some(2));
        assert!(past_end.is_empty());
    }

    /// Builds a temp directory unique to this test run, cleaned up by the caller.
    fn temp_dir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("komasabi-fs-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// Writes a `.cbz` at `path` containing `images` named image entries (plus a
    /// non-image entry that must be ignored), each holding trivial bytes.
    fn write_cbz(path: &Path, images: &[&str]) {
        use std::io::Write;
        use zip::write::SimpleFileOptions;

        let file = std::fs::File::create(path).unwrap();
        let mut zip = zip::ZipWriter::new(file);
        let options = SimpleFileOptions::default();

        zip.start_file("ComicInfo.xml", options).unwrap();
        zip.write_all(b"<xml/>").unwrap();

        for (i, name) in images.iter().enumerate() {
            zip.start_file(*name, options).unwrap();
            zip.write_all(format!("image-{i}").as_bytes()).unwrap();
        }
        zip.finish().unwrap();
    }

    #[test]
    fn zip_image_names_lists_only_images_sorted() {
        let dir = temp_dir("zip-names");
        let cbz = dir.join("chapter.cbz");
        write_cbz(&cbz, &["02.jpg", "01.jpg", "03.png"]);

        let names = zip_image_names(&cbz).unwrap();
        assert_eq!(names, vec!["01.jpg", "02.jpg", "03.png"]);
        assert_eq!(archive_image_count(&cbz), 3);

        let bytes = archive_image_bytes(&cbz, 0).unwrap();
        assert_eq!(bytes, b"image-1"); // "01.jpg" was written second (index 1)

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn zip_image_names_is_none_for_unsupported_extensions() {
        assert!(zip_image_names(Path::new("/x/book.pdf")).is_none());
        assert!(zip_image_names(Path::new("/x/book.cbr")).is_none());
    }

    #[test]
    fn scan_series_on_a_file_root_is_a_single_series() {
        let dir = temp_dir("file-root");
        let cbz = dir.join("Oneshot.cbz");
        write_cbz(&cbz, &["01.jpg"]);

        let series = scan_series(&cbz);
        assert_eq!(series, vec![cbz]);

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn scan_volumes_of_a_files_only_series_are_the_files() {
        let dir = temp_dir("files-series");
        let series = dir.join("One Piece");
        std::fs::create_dir_all(&series).unwrap();
        write_cbz(&series.join("Vol 1.cbz"), &["01.jpg"]);
        write_cbz(&series.join("Vol 2.cbz"), &["01.jpg"]);

        let volumes = scan_volumes(&series);
        assert_eq!(volumes.len(), 2);
        assert_eq!(display_name(&volumes[0]), "Vol 1");
        assert_eq!(display_name(&volumes[1]), "Vol 2");

        std::fs::remove_dir_all(&dir).ok();
    }

    /// Drives a current-thread runtime to completion for one future, so the
    /// async [`Provider`] methods can be exercised in a plain `#[test]`.
    fn block_on<F: std::future::Future>(future: F) -> F::Output {
        tokio::runtime::Builder::new_current_thread()
            .build()
            .unwrap()
            .block_on(future)
    }

    #[test]
    fn provider_pipeline_lists_series_entries_and_reads_pages() {
        // root/
        //   Berserk.cbz                 (file series -> standalone volume)
        //   One Piece/                  (folder series)
        //     Volume 1/                 (folder volume -> chapter files)
        //       ch1.cbz (2 images)
        //       ch2.cbz (1 image)
        let root = temp_dir("provider-pipeline");
        write_cbz(&root.join("Berserk.cbz"), &["01.jpg"]);
        let volume = root.join("One Piece").join("Volume 1");
        std::fs::create_dir_all(&volume).unwrap();
        write_cbz(&volume.join("ch1.cbz"), &["01.jpg", "02.jpg"]);
        write_cbz(&volume.join("ch2.cbz"), &["01.jpg"]);

        let provider = FileSystemProvider::new(&root);

        let items = block_on(provider.list_items("lib", Some(1), Some(10))).unwrap();
        let names: Vec<&str> = items.iter().map(|item| item.name.as_str()).collect();
        assert_eq!(names, vec!["Berserk", "One Piece"]);

        // Descend "One Piece" into its single volume.
        let one_piece = items.iter().find(|item| item.name == "One Piece").unwrap();
        let entries = block_on(provider.list_series_entries(&one_piece.external_id)).unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].kind, EntryKind::Volume);
        assert_eq!(entries[0].label, "Volume 1");

        // The volume reads as its two chapters' images concatenated (2 + 1).
        let pages = block_on(provider.reader_pages(&entries[0])).unwrap();
        assert_eq!(pages.len(), 3);

        let bytes = block_on(provider.reader_image(&pages[0].chapter_id, pages[0].page)).unwrap();
        assert!(!bytes.is_empty());

        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn provider_reads_a_single_file_series_as_a_standalone_volume() {
        let root = temp_dir("provider-file-series");
        // A single-file "library": the file itself is the library root.
        let file = root.join("Oneshot.cbz");
        write_cbz(&file, &["01.jpg", "02.jpg"]);

        let provider = FileSystemProvider::new(&file);

        let items = block_on(provider.list_items("lib", None, None)).unwrap();
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].name, "Oneshot");

        let entries = block_on(provider.list_series_entries(&items[0].external_id)).unwrap();
        assert_eq!(entries.len(), 1);

        let pages = block_on(provider.reader_pages(&entries[0])).unwrap();
        assert_eq!(pages.len(), 2);

        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn chapter_files_of_a_volume_folder_are_its_files() {
        let dir = temp_dir("vol-folder");
        let volume = dir.join("Volume 1");
        std::fs::create_dir_all(&volume).unwrap();
        write_cbz(&volume.join("ch1.cbz"), &["01.jpg", "02.jpg"]);
        write_cbz(&volume.join("ch2.cbz"), &["01.jpg"]);

        let files = chapter_files(&volume);
        assert_eq!(files.len(), 2);

        std::fs::remove_dir_all(&dir).ok();
    }
}
