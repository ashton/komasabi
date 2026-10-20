use komga_client::models::{
    book::Book as KomgaBook, library::Library as KomgaLibrary, series::Series as KomgaSeries,
};
use uuid::Uuid;

use crate::{
    adapter::{ItemAdapter, LibraryAdapter},
    models::{
        library::{Library, LibraryType},
        library_item::LibraryItem,
        series_entry::{EntryKind, SeriesEntry},
    },
};

pub struct KomgaItemAdapter;

impl ItemAdapter<KomgaSeries> for KomgaItemAdapter {
    fn adapt_item(source: KomgaSeries) -> LibraryItem {
        let title = source.metadata.title.trim();
        let name = if title.is_empty() {
            source.name
        } else {
            title.to_string()
        };

        LibraryItem {
            external_id: source.id,
            name,
            cover: String::new(),
            library_id: source.library_id,
            // Komga counts a series' books, not its pages.
            pages: None,
        }
    }
}

/// Turns a Komga book into a series-detail card. Komga doesn't say whether a
/// book is a volume or an issue (it's just a file), so every book is shown as
/// a chapter card, labelled with its title — which Komga derives from the file
/// name or ComicInfo, so a "v01" file reads as such.
pub fn book_entry(book: KomgaBook) -> SeriesEntry {
    let label = [&book.metadata.title, &book.name]
        .into_iter()
        .map(|value| value.trim())
        .find(|value| !value.is_empty())
        .map(str::to_string)
        .unwrap_or_else(|| format!("#{}", book.metadata.number.trim()));

    SeriesEntry {
        external_id: book.id,
        kind: EntryKind::Chapter,
        label,
        pages: u16::try_from(book.media.pages_count)
            .ok()
            .filter(|pages| *pages > 0),
    }
}

pub struct KomgaLibraryAdapter;

impl LibraryAdapter<KomgaLibrary> for KomgaLibraryAdapter {
    /// Komga libraries carry no content type, so they're imported as comics.
    fn adapt_library(source: KomgaLibrary, integration_id: &str) -> Library {
        Library {
            id: Uuid::new_v4().to_string(),
            kind: LibraryType::Comic,
            name: source.name,
            external_id: Some(source.id),
            folder: None,
            cover: None,
            integration_id: Some(integration_id.to_string()),
        }
    }
}

#[cfg(test)]
mod tests {
    use komga_client::models::book::{BookMetadata, Media};
    use komga_client::models::series::SeriesMetadata;

    use super::*;

    fn book(title: &str, name: &str, number: &str, pages_count: u32) -> KomgaBook {
        KomgaBook {
            id: "B1".to_string(),
            name: name.to_string(),
            media: Media {
                pages_count,
                ..Media::default()
            },
            metadata: BookMetadata {
                title: title.to_string(),
                number: number.to_string(),
                ..BookMetadata::default()
            },
            ..KomgaBook::default()
        }
    }

    #[test]
    fn adapt_library_maps_fields() {
        let source = KomgaLibrary {
            id: "0B8RAK3Q2Y1VX".to_string(),
            name: "Manga".to_string(),
            ..KomgaLibrary::default()
        };

        let library = KomgaLibraryAdapter::adapt_library(source, "integration-id");

        assert_eq!(library.name, "Manga");
        assert_eq!(library.external_id, Some("0B8RAK3Q2Y1VX".to_string()));
        assert_eq!(library.kind, LibraryType::Comic);
        assert_eq!(library.integration_id, Some("integration-id".to_string()));
        assert_eq!(library.folder, None);
    }

    #[test]
    fn adapt_item_prefers_the_metadata_title_over_the_folder_name() {
        let source = KomgaSeries {
            id: "S1".to_string(),
            library_id: "L1".to_string(),
            name: "alpha_saga".to_string(),
            metadata: SeriesMetadata {
                title: "Alpha Saga".to_string(),
                ..SeriesMetadata::default()
            },
            ..KomgaSeries::default()
        };

        let item = KomgaItemAdapter::adapt_item(source);

        assert_eq!(item.external_id, "S1");
        assert_eq!(item.library_id, "L1");
        assert_eq!(item.name, "Alpha Saga");
        assert_eq!(item.pages, None);
    }

    #[test]
    fn adapt_item_falls_back_to_the_folder_name_without_a_title() {
        let source = KomgaSeries {
            name: "alpha_saga".to_string(),
            ..KomgaSeries::default()
        };

        assert_eq!(KomgaItemAdapter::adapt_item(source).name, "alpha_saga");
    }

    #[test]
    fn book_entry_is_a_chapter_labelled_with_its_title() {
        let entry = book_entry(book("Alpha Saga v01", "file", "1", 3));

        assert_eq!(entry.external_id, "B1");
        assert_eq!(entry.kind, EntryKind::Chapter);
        assert_eq!(entry.label, "Alpha Saga v01");
        assert_eq!(entry.pages, Some(3));
    }

    #[test]
    fn book_entry_falls_back_to_the_file_name_then_the_number() {
        assert_eq!(book_entry(book(" ", "file", "1", 3)).label, "file");
        assert_eq!(book_entry(book("", "", "7", 3)).label, "#7");
    }

    #[test]
    fn book_entry_maps_zero_pages_to_none() {
        assert_eq!(book_entry(book("t", "n", "1", 0)).pages, None);
    }
}
