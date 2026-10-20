use suwayomi_client::models::{
    category::Category as SuwayomiCategory, chapter::Chapter as SuwayomiChapter,
    manga::Manga as SuwayomiManga,
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

/// A library's external id: its category id, prefixed with the integration's
/// id. Category ids are small integers on every server (the default category
/// is always `0`), so on their own they'd collide with another server's, or
/// with Kavita's numeric library ids, in the app-wide external id lookup.
pub fn library_external_id(integration_id: &str, category_id: i32) -> String {
    format!("{integration_id}:{category_id}")
}

/// The category id back out of a [`library_external_id`].
pub fn category_id(library_external_id: &str) -> Result<i32, String> {
    library_external_id
        .rsplit_once(':')
        .and_then(|(_, id)| id.parse().ok())
        .ok_or_else(|| format!("{library_external_id} is not a Suwayomi library id"))
}

/// A chapter entry's external id: its manga's id and its own, so a chapter
/// card can show its manga's cover without another lookup.
pub fn chapter_external_id(manga_id: i32, chapter_id: i32) -> String {
    format!("{manga_id}-{chapter_id}")
}

/// The `(manga id, chapter id)` back out of a [`chapter_external_id`].
pub fn parse_chapter_external_id(external_id: &str) -> Result<(i32, i32), String> {
    external_id
        .split_once('-')
        .and_then(|(manga_id, chapter_id)| Some((manga_id.parse().ok()?, chapter_id.parse().ok()?)))
        .ok_or_else(|| format!("{external_id} is not a Suwayomi chapter id"))
}

/// What a [`crate::providers::PageRef`]'s `chapter_id` holds: the manga id and
/// the chapter's source order, which is how Suwayomi's page images are
/// addressed.
pub fn page_chapter_id(manga_id: i32, source_order: i32) -> String {
    format!("{manga_id}-{source_order}")
}

/// The `(manga id, source order)` back out of a [`page_chapter_id`].
pub fn parse_page_chapter_id(chapter_id: &str) -> Result<(i32, i32), String> {
    parse_chapter_external_id(chapter_id)
}

pub struct SuwayomiItemAdapter;

impl ItemAdapter<(SuwayomiManga, &str)> for SuwayomiItemAdapter {
    /// Adapts a manga, paired with the external id of the library it's listed
    /// under.
    fn adapt_item((source, library_external_id): (SuwayomiManga, &str)) -> LibraryItem {
        LibraryItem {
            external_id: source.id.to_string(),
            name: source.title,
            cover: String::new(),
            library_id: library_external_id.to_string(),
            // Suwayomi counts a manga's chapters, not its pages.
            pages: None,
        }
    }
}

/// Turns a chapter into a series-detail card, labelled with its name (e.g.
/// "Vol.1 Ch.1"), or its number when it has none. Suwayomi only knows
/// chapters, so there are no volume cards.
pub fn chapter_entry(chapter: SuwayomiChapter) -> SeriesEntry {
    let name = chapter.name.trim();
    let label = if name.is_empty() {
        format!("#{}", chapter.chapter_number)
    } else {
        name.to_string()
    };

    SeriesEntry {
        external_id: chapter_external_id(chapter.manga_id, chapter.id),
        kind: EntryKind::Chapter,
        label,
        pages: u16::try_from(chapter.page_count)
            .ok()
            .filter(|pages| *pages > 0),
    }
}

pub struct SuwayomiLibraryAdapter;

impl LibraryAdapter<SuwayomiCategory> for SuwayomiLibraryAdapter {
    /// Suwayomi is a manga server, so its categories are imported as manga
    /// libraries.
    fn adapt_library(source: SuwayomiCategory, integration_id: &str) -> Library {
        Library {
            id: Uuid::new_v4().to_string(),
            kind: LibraryType::Manga,
            name: source.name,
            external_id: Some(library_external_id(integration_id, source.id)),
            folder: None,
            cover: None,
            integration_id: Some(integration_id.to_string()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn chapter(name: &str, chapter_number: f64, page_count: i32) -> SuwayomiChapter {
        SuwayomiChapter {
            id: 45,
            manga_id: 25,
            name: name.to_string(),
            chapter_number,
            source_order: 1,
            page_count,
            ..SuwayomiChapter::default()
        }
    }

    #[test]
    fn adapt_library_namespaces_the_category_id_by_integration() {
        let source = SuwayomiCategory {
            id: 0,
            name: "Default".to_string(),
            default: true,
            ..SuwayomiCategory::default()
        };

        let library = SuwayomiLibraryAdapter::adapt_library(source, "integration-id");

        assert_eq!(library.name, "Default");
        assert_eq!(library.external_id, Some("integration-id:0".to_string()));
        assert_eq!(library.kind, LibraryType::Manga);
        assert_eq!(library.integration_id, Some("integration-id".to_string()));
        assert_eq!(library.folder, None);
    }

    #[test]
    fn category_id_round_trips_and_rejects_foreign_ids() {
        let external_id = library_external_id("3f2c-uuid", 7);

        assert_eq!(category_id(&external_id), Ok(7));
        assert!(category_id("7").is_err());
        assert!(category_id("uuid:abc").is_err());
    }

    #[test]
    fn chapter_external_id_round_trips() {
        assert_eq!(
            parse_chapter_external_id(&chapter_external_id(25, 45)),
            Ok((25, 45))
        );
        assert!(parse_chapter_external_id("45").is_err());
        assert!(parse_chapter_external_id("a-b").is_err());
    }

    #[test]
    fn adapt_item_maps_fields() {
        let source = SuwayomiManga {
            id: 9,
            title: "Voyeurs, Inc.".to_string(),
            ..SuwayomiManga::default()
        };

        let item = SuwayomiItemAdapter::adapt_item((source, "integration-id:0"));

        assert_eq!(item.external_id, "9");
        assert_eq!(item.name, "Voyeurs, Inc.");
        assert_eq!(item.library_id, "integration-id:0");
        assert_eq!(item.pages, None);
    }

    #[test]
    fn chapter_entry_is_a_chapter_labelled_with_its_name() {
        let entry = chapter_entry(chapter("Vol.1 Ch.1", 1.0, 31));

        assert_eq!(entry.external_id, "25-45");
        assert_eq!(entry.kind, EntryKind::Chapter);
        assert_eq!(entry.label, "Vol.1 Ch.1");
        assert_eq!(entry.pages, Some(31));
    }

    #[test]
    fn chapter_entry_falls_back_to_the_chapter_number() {
        assert_eq!(chapter_entry(chapter(" ", 12.5, 31)).label, "#12.5");
        assert_eq!(chapter_entry(chapter("", 3.0, 31)).label, "#3");
    }

    #[test]
    fn chapter_entry_maps_unfetched_page_counts_to_none() {
        assert_eq!(chapter_entry(chapter("Ch. 1", 1.0, -1)).pages, None);
        assert_eq!(chapter_entry(chapter("Ch. 1", 1.0, 0)).pages, None);
    }
}
