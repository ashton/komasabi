use kavita_client::models::{
    chapter::Chapter as KavitaChapter,
    library::{KavitaLibrary, KavitaLibraryType},
    series::Series as KavitaSeries,
};
use uuid::Uuid;

use crate::{
    adapter::{ChapterAdapter, ItemAdapter, LibraryAdapter},
    models::{
        chapter::Chapter,
        library::{Library, LibraryType},
        library_item::LibraryItem,
    },
};

pub struct KavitaItemAdapter;

impl ItemAdapter<KavitaSeries> for KavitaItemAdapter {
    fn adapt_item(source: KavitaSeries) -> LibraryItem {
        LibraryItem {
            external_id: source.id.to_string(),
            name: source.name,
            library_id: source.library_id.to_string(),
            cover: source.cover_image,
            pages: (source.pages > 0).then_some(source.pages),
        }
    }
}

pub struct KavitaChapterAdapter;

impl ChapterAdapter<KavitaChapter> for KavitaChapterAdapter {
    fn adapt_chapter(source: KavitaChapter) -> Chapter {
        // Kavita fills `title`/`title_name` with the bare chapter number when
        // there's no real title, so a title is only meaningful when it differs
        // from the range we'd show anyway.
        let title = source
            .title_name
            .filter(|title| !title.trim().is_empty() && title.trim() != source.range.trim());

        Chapter {
            external_id: source.id.to_string(),
            range: source.range,
            title,
            is_special: source.is_special,
            pages: (source.pages > 0).then_some(source.pages),
        }
    }
}

pub struct KavitaLibraryAdapter;

impl LibraryAdapter<KavitaLibrary> for KavitaLibraryAdapter {
    fn adapt_library(source: KavitaLibrary, integration_id: &str) -> Library {
        Library {
            id: Uuid::new_v4().to_string(),
            kind: adapt_library_type(source.library_type),
            name: source.name,
            external_id: Some(source.id.to_string()),
            folder: None,
            cover: None,
            integration_id: Some(integration_id.to_string()),
        }
    }
}

fn adapt_library_type(source: KavitaLibraryType) -> LibraryType {
    match source {
        KavitaLibraryType::Manga => LibraryType::Manga,
        KavitaLibraryType::Comic | KavitaLibraryType::ComicVine | KavitaLibraryType::Image => {
            LibraryType::Comic
        }
        KavitaLibraryType::Book | KavitaLibraryType::LightNovel => LibraryType::Ebook,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn adapt_library_type_maps_manga() {
        assert_eq!(
            adapt_library_type(KavitaLibraryType::Manga),
            LibraryType::Manga
        );
    }

    #[test]
    fn adapt_library_type_maps_comic_variants_to_comic() {
        assert_eq!(
            adapt_library_type(KavitaLibraryType::Comic),
            LibraryType::Comic
        );
        assert_eq!(
            adapt_library_type(KavitaLibraryType::ComicVine),
            LibraryType::Comic
        );
        assert_eq!(
            adapt_library_type(KavitaLibraryType::Image),
            LibraryType::Comic
        );
    }

    #[test]
    fn adapt_library_type_maps_text_variants_to_ebook() {
        assert_eq!(
            adapt_library_type(KavitaLibraryType::Book),
            LibraryType::Ebook
        );
        assert_eq!(
            adapt_library_type(KavitaLibraryType::LightNovel),
            LibraryType::Ebook
        );
    }

    #[test]
    fn adapt_library_maps_fields_and_type() {
        let source = KavitaLibrary {
            id: 7,
            name: "Comics".to_string(),
            library_type: KavitaLibraryType::Manga,
            ..KavitaLibrary::default()
        };

        let library = KavitaLibraryAdapter::adapt_library(source, "integration-id");

        assert_eq!(library.name, "Comics");
        assert_eq!(library.external_id, Some("7".to_string()));
        assert_eq!(library.kind, LibraryType::Manga);
        assert_eq!(library.integration_id, Some("integration-id".to_string()));
    }

    #[test]
    fn adapt_item_maps_fields_and_pages() {
        let source = KavitaSeries {
            id: 1,
            name: "One Piece".to_string(),
            cover_image: "1.png".to_string(),
            library_id: 7,
            pages: 42,
            ..KavitaSeries::default()
        };

        let item = KavitaItemAdapter::adapt_item(source);

        assert_eq!(item.external_id, "1");
        assert_eq!(item.name, "One Piece");
        assert_eq!(item.cover, "1.png");
        assert_eq!(item.library_id, "7");
        assert_eq!(item.pages, Some(42));
    }

    #[test]
    fn adapt_item_maps_zero_pages_to_none() {
        let source = KavitaSeries {
            pages: 0,
            ..KavitaSeries::default()
        };

        let item = KavitaItemAdapter::adapt_item(source);

        assert_eq!(item.pages, None);
    }

    #[test]
    fn adapt_chapter_maps_fields_and_pages() {
        let source = KavitaChapter {
            id: 9,
            range: "1".to_string(),
            is_special: false,
            pages: 24,
            ..KavitaChapter::default()
        };

        let chapter = KavitaChapterAdapter::adapt_chapter(source);

        assert_eq!(chapter.external_id, "9");
        assert_eq!(chapter.range, "1");
        assert!(!chapter.is_special);
        assert_eq!(chapter.pages, Some(24));
    }

    #[test]
    fn adapt_chapter_maps_zero_pages_to_none() {
        let source = KavitaChapter {
            pages: 0,
            ..KavitaChapter::default()
        };

        let chapter = KavitaChapterAdapter::adapt_chapter(source);

        assert_eq!(chapter.pages, None);
    }

    #[test]
    fn adapt_chapter_ignores_a_title_that_just_repeats_the_range() {
        let source = KavitaChapter {
            range: "3".to_string(),
            title_name: Some("3".to_string()),
            ..KavitaChapter::default()
        };

        let chapter = KavitaChapterAdapter::adapt_chapter(source);

        assert_eq!(chapter.title, None);
    }

    #[test]
    fn adapt_chapter_keeps_a_distinct_title() {
        let source = KavitaChapter {
            range: "3".to_string(),
            title_name: Some("The Reckoning".to_string()),
            ..KavitaChapter::default()
        };

        let chapter = KavitaChapterAdapter::adapt_chapter(source);

        assert_eq!(chapter.title, Some("The Reckoning".to_string()));
    }
}
