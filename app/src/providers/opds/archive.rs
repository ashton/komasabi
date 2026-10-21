//! Turning a publication file downloaded from an OPDS catalog into page images
//! the reader can be fed one at a time.
//!
//! The whole archive is extracted up front rather than page by page: RAR
//! archives (`.cbr`, common in comic catalogs) are often *solid*, so reaching
//! page N means decompressing every page before it anyway. Pages are written as
//! `{index:05}.page` in name order, the order comic archives encode reading
//! order in.

use std::io::Read;
use std::path::{Path, PathBuf};

/// Entry extensions treated as page images.
const IMAGE_EXTENSIONS: &[&str] = &["png", "jpg", "jpeg", "gif", "webp", "bmp", "avif"];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ArchiveFormat {
    /// `.cbz` / `.zip`.
    Zip,
    /// `.cbr` / `.rar` (RAR 4 and 5).
    Rar,
}

/// Identifies an archive by its magic bytes. Catalogs label files loosely
/// (or not at all), so the content decides, not the advertised media type.
pub fn sniff(path: &Path) -> Option<ArchiveFormat> {
    let mut magic = [0u8; 7];
    let mut file = std::fs::File::open(path).ok()?;
    let read = file.read(&mut magic).ok()?;

    match &magic[..read] {
        [b'P', b'K', 0x03, 0x04, ..] => Some(ArchiveFormat::Zip),
        [b'R', b'a', b'r', b'!', 0x1A, 0x07, ..] => Some(ArchiveFormat::Rar),
        _ => None,
    }
}

/// The file page `index` was extracted to.
pub fn page_path(pages_dir: &Path, index: usize) -> PathBuf {
    pages_dir.join(format!("{index:05}.page"))
}

/// How many pages a prepared pages folder holds.
pub fn page_count(pages_dir: &Path) -> usize {
    std::fs::read_dir(pages_dir)
        .map(|entries| {
            entries
                .filter_map(Result::ok)
                .filter(|entry| entry.path().extension().is_some_and(|ext| ext == "page"))
                .count()
        })
        .unwrap_or(0)
}

/// Extracts every page image of `archive` into `pages_dir` (which must exist),
/// returning the page count.
pub fn extract_pages(archive: &Path, pages_dir: &Path) -> Result<usize, String> {
    let mut pages = match sniff(archive) {
        Some(ArchiveFormat::Zip) => zip_pages(archive)?,
        Some(ArchiveFormat::Rar) => rar_pages(archive)?,
        None => return Err("the downloaded file is not a comic archive (cbz/cbr)".to_string()),
    };

    pages.sort_by(|(a, _), (b, _)| a.cmp(b));

    for (index, (_, bytes)) in pages.iter().enumerate() {
        std::fs::write(page_path(pages_dir, index), bytes)
            .map_err(crate::error::log_and_stringify)?;
    }

    Ok(pages.len())
}

fn is_page(name: &str) -> bool {
    let path = Path::new(name);
    let hidden = path.components().any(|component| {
        let part = component.as_os_str().to_string_lossy();
        part.starts_with('.') || part == "__MACOSX"
    });

    !hidden
        && path
            .extension()
            .and_then(|ext| ext.to_str())
            .is_some_and(|ext| IMAGE_EXTENSIONS.contains(&ext.to_ascii_lowercase().as_str()))
}

fn zip_pages(path: &Path) -> Result<Vec<(String, Vec<u8>)>, String> {
    let file = std::fs::File::open(path).map_err(crate::error::log_and_stringify)?;
    let mut archive = zip::ZipArchive::new(file).map_err(crate::error::log_and_stringify)?;

    let mut pages = Vec::new();
    for index in 0..archive.len() {
        let mut entry = archive
            .by_index(index)
            .map_err(crate::error::log_and_stringify)?;
        if !entry.is_file() || !is_page(entry.name()) {
            continue;
        }

        let name = entry.name().to_string();
        let mut bytes = Vec::new();
        entry
            .read_to_end(&mut bytes)
            .map_err(crate::error::log_and_stringify)?;
        pages.push((name, bytes));
    }

    Ok(pages)
}

fn rar_pages(path: &Path) -> Result<Vec<(String, Vec<u8>)>, String> {
    let mut archive = unrar::Archive::new(path)
        .open_for_processing()
        .map_err(crate::error::log_and_stringify)?;

    let mut pages = Vec::new();
    while let Some(header) = archive
        .read_header()
        .map_err(crate::error::log_and_stringify)?
    {
        let name = header.entry().filename.to_string_lossy().into_owned();

        archive = if header.entry().is_file() && is_page(&name) {
            let (bytes, rest) = header.read().map_err(crate::error::log_and_stringify)?;
            pages.push((name, bytes));
            rest
        } else {
            header.skip().map_err(crate::error::log_and_stringify)?
        };
    }

    Ok(pages)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn scratch_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("komasabi-opds-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn write_zip(path: &Path, entries: &[(&str, &[u8])]) {
        let mut archive = zip::ZipWriter::new(std::fs::File::create(path).unwrap());
        let options = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Stored);
        for (name, bytes) in entries {
            archive.start_file(*name, options).unwrap();
            archive.write_all(bytes).unwrap();
        }
        archive.finish().unwrap();
    }

    #[test]
    fn sniff_recognises_zip_and_rar_by_content() {
        let dir = scratch_dir("sniff");
        let zip = dir.join("mislabelled.cbr");
        write_zip(&zip, &[("a.jpg", b"a")]);
        let rar = dir.join("book");
        std::fs::write(&rar, b"Rar!\x1a\x07\x01\x00rest").unwrap();
        let other = dir.join("book.cbz");
        std::fs::write(&other, b"%PDF-1.7").unwrap();

        assert_eq!(sniff(&zip), Some(ArchiveFormat::Zip));
        assert_eq!(sniff(&rar), Some(ArchiveFormat::Rar));
        assert_eq!(sniff(&other), None);

        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn extract_pages_writes_images_in_name_order_and_skips_everything_else() {
        let dir = scratch_dir("extract");
        let archive = dir.join("book.cbz");
        write_zip(
            &archive,
            &[
                ("002.png", b"two"),
                ("ComicInfo.xml", b"<xml/>"),
                ("001.JPG", b"one"),
                ("__MACOSX/._001.jpg", b"junk"),
                (".hidden.jpg", b"junk"),
            ],
        );
        let pages = dir.join("pages");
        std::fs::create_dir_all(&pages).unwrap();

        let count = extract_pages(&archive, &pages).unwrap();

        assert_eq!(count, 2);
        assert_eq!(page_count(&pages), 2);
        assert_eq!(std::fs::read(page_path(&pages, 0)).unwrap(), b"one");
        assert_eq!(std::fs::read(page_path(&pages, 1)).unwrap(), b"two");

        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn extract_pages_rejects_files_that_are_not_comic_archives() {
        let dir = scratch_dir("reject");
        let epub_like = dir.join("book.epub");
        std::fs::write(&epub_like, b"not an archive").unwrap();

        assert!(extract_pages(&epub_like, &dir).is_err());

        let _ = std::fs::remove_dir_all(dir);
    }
}
