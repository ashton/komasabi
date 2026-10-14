use kavita_client::models::{
    library::{KavitaLibrary, KavitaLibraryType},
    series::Series as KavitaSeries,
};
use uuid::Uuid;

use crate::{
    adapter::{ItemAdapter, LibraryAdapter},
    models::{
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
}
