//! A [`Provider`] backed by the local filesystem: a local-folder source, the
//! counterpart to the remote backends for content on this machine.
//!
//! Like the remote providers, it holds no local copy of the folder's contents:
//! every call re-reads the filesystem, so the source always reflects what's on
//! disk. The structural rules it applies:
//!
//! - The source has a single collection, its root, which is either a **folder**
//!   or a **single file**. A file root is one series holding one standalone
//!   volume.
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
    collection::{Collection, CollectionKind},
    collection_item::CollectionItem,
    series_entry::{EntryKind, SeriesEntry},
};
use crate::providers::{PageRef, Provider, paginate};

/// File extensions treated as page images inside an archive.
const IMAGE_EXTENSIONS: &[&str] = &["png", "jpg", "jpeg", "gif", "webp", "bmp", "avif"];

/// Archive extensions whose image entries can be extracted for reading.
const READABLE_ARCHIVE_EXTENSIONS: &[&str] = &["cbz", "zip"];

pub struct FileSystemProvider {
    /// The source's root path: a folder, or a single file.
    root: PathBuf,
    /// The source's name, which its one collection takes.
    name: String,
}

impl FileSystemProvider {
    pub fn new(root: impl Into<PathBuf>, name: impl Into<String>) -> Self {
        Self {
            root: root.into(),
            name: name.into(),
        }
    }

    /// Reads from the on-disk cover cache for `key`, falling back to `produce`
    /// (run on a blocking thread via the caller's runtime) and writing the
    /// result back. Mirrors the Kavita provider's cover caching so both backends
    /// behave identically from the screens' point of view.
    fn cached_cover(
        &self,
        key: String,
        produce: impl FnOnce() -> Option<Vec<u8>>,
    ) -> Result<Vec<u8>, String> {
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
    /// The root folder is the source's only collection. It takes imported files
    /// when it's a folder (a single-file root has nowhere to put them).
    async fn list_collections(&self) -> Result<Vec<Collection>, String> {
        Ok(vec![Collection {
            importable: self.root.is_dir(),
            ..Collection::new(encode_id(&self.root), &self.name, CollectionKind::Folder)
        }])
    }

    async fn list_items(
        &self,
        _collection_id: &str,
        page: Option<u32>,
        page_size: Option<u32>,
    ) -> Result<Vec<CollectionItem>, String> {
        let series = scan_series(&self.root);

        let items: Vec<CollectionItem> = series
            .into_iter()
            .map(|path| CollectionItem {
                external_id: encode_id(&path),
                name: display_name(&path),
                cover: String::new(),
                pages: None,
            })
            .collect();

        Ok(paginate(items, page, page_size))
    }

    /// Copies each file into the root folder, so the next scan surfaces it as a
    /// new series. Existing files are left untouched (never overwritten).
    async fn import_files(&self, _collection_id: &str, files: &[PathBuf]) -> Result<usize, String> {
        import_into(&self.root, files)
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
                read_state: Default::default(),
            })
            .collect())
    }

    /// The entry after `current` in the series folder, by file name, which is the
    /// reading order.
    async fn next_entry(
        &self,
        series_id: &str,
        current: &SeriesEntry,
    ) -> Result<Option<SeriesEntry>, String> {
        let entries = Provider::list_series_entries(self, series_id).await?;
        Ok(crate::providers::entry_after(entries, current))
    }

    async fn previous_entry(
        &self,
        series_id: &str,
        current: &SeriesEntry,
    ) -> Result<Option<SeriesEntry>, String> {
        let entries = Provider::list_series_entries(self, series_id).await?;
        Ok(crate::providers::entry_before(entries, current))
    }

    async fn cover(&self, integration_id: &str, series_id: &str) -> Result<Vec<u8>, String> {
        let path = decode_id(series_id)?;
        self.cached_cover(format!("{integration_id}-{series_id}"), || {
            first_image_under(&path)
        })
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
/// Copies `files` into the folder `root`, skipping any whose name is already
/// there. Returns how many were copied in.
fn import_into(root: &Path, files: &[PathBuf]) -> Result<usize, String> {
    if !root.is_dir() {
        return Err("cannot import items into a single-file source".to_string());
    }

    let mut imported = 0;
    for source in files {
        let Some(name) = source.file_name() else {
            continue;
        };
        let destination = root.join(name);

        if destination.exists() {
            tracing::debug!(
                "skipping import of {}: already present",
                destination.display()
            );
            continue;
        }

        std::fs::copy(source, &destination)
            .map_err(|error| format!("could not copy {}: {error}", source.display()))?;
        imported += 1;
    }

    Ok(imported)
}

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
        let hex =
            std::str::from_utf8(pair).map_err(|_| format!("malformed filesystem id: {id}"))?;
        let byte =
            u8::from_str_radix(hex, 16).map_err(|_| format!("malformed filesystem id: {id}"))?;
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

/// The series of a source root: the root itself if it's a single file,
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
        assert_eq!(
            display_name(Path::new("/x/One Piece Vol 1.cbz")),
            "One Piece Vol 1"
        );
    }

    #[test]
    fn is_image_name_matches_known_extensions_case_insensitively() {
        assert!(is_image_name("page01.JPG"));
        assert!(is_image_name("cover.png"));
        assert!(!is_image_name("info.txt"));
        assert!(!is_image_name("chapter.cbz"));
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

        let provider = FileSystemProvider::new(&root, "Comics");

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
        // A single-file source: the file itself is the root.
        let file = root.join("Oneshot.cbz");
        write_cbz(&file, &["01.jpg", "02.jpg"]);

        let provider = FileSystemProvider::new(&file, "Comics");

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
    fn a_folder_source_is_one_importable_folder_collection() {
        let root = temp_dir("collections-folder");
        let provider = FileSystemProvider::new(&root, "My Comics");

        let collections = block_on(provider.list_collections()).unwrap();

        assert_eq!(collections.len(), 1);
        assert_eq!(collections[0].name, "My Comics");
        assert_eq!(collections[0].kind, CollectionKind::Folder);
        assert!(collections[0].importable);

        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn a_single_file_source_is_not_importable() {
        let root = temp_dir("collections-file");
        let file = root.join("Oneshot.cbz");
        write_cbz(&file, &["01.jpg"]);
        let provider = FileSystemProvider::new(&file, "Oneshot");

        let collections = block_on(provider.list_collections()).unwrap();

        assert!(!collections[0].importable);
        assert!(block_on(provider.import_files("", &[root.join("x.cbz")])).is_err());

        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn import_files_copies_new_files_and_skips_existing_ones() {
        let root = temp_dir("import-target");
        let outside = temp_dir("import-source");
        write_cbz(&root.join("Berserk.cbz"), &["01.jpg"]);
        write_cbz(&outside.join("Berserk.cbz"), &["01.jpg", "02.jpg"]);
        write_cbz(&outside.join("Akira.cbz"), &["01.jpg"]);
        let provider = FileSystemProvider::new(&root, "Comics");

        let imported = block_on(provider.import_files(
            "",
            &[outside.join("Berserk.cbz"), outside.join("Akira.cbz")],
        ))
        .unwrap();

        assert_eq!(imported, 1);
        let items = block_on(provider.list_items("", None, None)).unwrap();
        let names: Vec<&str> = items.iter().map(|item| item.name.as_str()).collect();
        assert_eq!(names, vec!["Akira", "Berserk"]);
        // The existing file wasn't overwritten.
        assert_eq!(archive_image_count(&root.join("Berserk.cbz")), 1);

        std::fs::remove_dir_all(&root).ok();
        std::fs::remove_dir_all(&outside).ok();
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
