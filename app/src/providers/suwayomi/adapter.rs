use suwayomi_client::models::{
    category::Category as SuwayomiCategory, chapter::Chapter as SuwayomiChapter,
    manga::Manga as SuwayomiManga,
};

use crate::{
    adapter::{CollectionAdapter, ItemAdapter},
    models::{
        collection::{Collection, CollectionKind},
        collection_item::CollectionItem,
        series_entry::{EntryKind, ReadState, SeriesEntry},
    },
};

/// A category's collection id (`category:{id}`).
pub fn collection_id(category_id: i32) -> String {
    format!("category:{category_id}")
}

/// The category id back out of a [`collection_id`].
pub fn category_id(collection_id: &str) -> Result<i32, String> {
    collection_id
        .strip_prefix("category:")
        .and_then(|id| id.parse().ok())
        .ok_or_else(|| format!("{collection_id} is not a Suwayomi category id"))
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

impl ItemAdapter<SuwayomiManga> for SuwayomiItemAdapter {
    fn adapt_item(source: SuwayomiManga) -> CollectionItem {
        CollectionItem {
            external_id: source.id.to_string(),
            name: source.title,
            cover: String::new(),
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

    // `lastPageRead` is a 0-based index, so it's one behind the pages read.
    let read_state = if chapter.is_read {
        ReadState::Read
    } else if chapter.last_page_read > 0 {
        let pages_read = u32::try_from(chapter.last_page_read + 1).unwrap_or(0);
        let pages = u32::try_from(chapter.page_count)
            .ok()
            .filter(|pages| *pages > 0);
        match ReadState::from_pages_read(pages_read, pages) {
            // Not flagged read, so the last page hasn't been finished.
            ReadState::Read => ReadState::InProgress(Some(0.99)),
            state => state,
        }
    } else {
        ReadState::Unread
    };

    SeriesEntry {
        external_id: chapter_external_id(chapter.manga_id, chapter.id),
        kind: EntryKind::Chapter,
        label,
        pages: u16::try_from(chapter.page_count)
            .ok()
            .filter(|pages| *pages > 0),
        read_state,
    }
}

pub struct SuwayomiCollectionAdapter;

impl CollectionAdapter<SuwayomiCategory> for SuwayomiCollectionAdapter {
    fn adapt_collection(source: SuwayomiCategory) -> Collection {
        Collection::new(
            collection_id(source.id),
            source.name,
            CollectionKind::Category,
        )
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
    fn adapt_collection_makes_a_category() {
        let source = SuwayomiCategory {
            id: 0,
            name: "Default".to_string(),
            default: true,
            ..SuwayomiCategory::default()
        };

        let collection = SuwayomiCollectionAdapter::adapt_collection(source);

        assert_eq!(collection.name, "Default");
        assert_eq!(collection.external_id, "category:0");
        assert_eq!(collection.kind, CollectionKind::Category);
    }

    #[test]
    fn category_id_round_trips_and_rejects_foreign_ids() {
        assert_eq!(category_id(&collection_id(7)), Ok(7));
        assert!(category_id("7").is_err());
        assert!(category_id("category:abc").is_err());
        assert!(category_id("library:7").is_err());
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

        let item = SuwayomiItemAdapter::adapt_item(source);

        assert_eq!(item.external_id, "9");
        assert_eq!(item.name, "Voyeurs, Inc.");
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

    #[test]
    fn chapter_entry_maps_the_read_state() {
        let unread = chapter("Ch. 1", 1.0, 20);
        let started = SuwayomiChapter {
            last_page_read: 4,
            ..chapter("Ch. 1", 1.0, 20)
        };
        let read = SuwayomiChapter {
            is_read: true,
            ..chapter("Ch. 1", 1.0, 20)
        };

        assert_eq!(chapter_entry(unread).read_state, ReadState::Unread);
        // `lastPageRead` is 0-based: page index 4 means 5 of 20 pages read.
        assert_eq!(
            chapter_entry(started).read_state,
            ReadState::InProgress(Some(0.25))
        );
        assert_eq!(chapter_entry(read).read_state, ReadState::Read);
    }
}
