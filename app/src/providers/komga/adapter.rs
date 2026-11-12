use komga_client::models::{
    book::Book as KomgaBook, collection::Collection as KomgaCollection,
    library::Library as KomgaLibrary, read_list::ReadList, series::Series as KomgaSeries,
};

use crate::{
    adapter::{CollectionAdapter, ItemAdapter},
    models::{
        collection::{Collection, CollectionKind},
        collection_item::CollectionItem,
        series_entry::{EntryKind, ReadState, SeriesEntry},
    },
};

pub struct KomgaItemAdapter;

impl ItemAdapter<KomgaSeries> for KomgaItemAdapter {
    fn adapt_item(source: KomgaSeries) -> CollectionItem {
        let title = source.metadata.title.trim();
        let name = if title.is_empty() {
            source.name
        } else {
            title.to_string()
        };

        CollectionItem {
            external_id: source.id,
            name,
            cover: String::new(),
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

    let read_state = match &book.read_progress {
        Some(progress) if progress.completed => ReadState::Read,
        Some(progress) => ReadState::from_pages_read(
            progress.page,
            (book.media.pages_count > 0).then_some(book.media.pages_count),
        ),
        None => ReadState::Unread,
    };

    SeriesEntry {
        external_id: book.id,
        kind: EntryKind::Chapter,
        label,
        pages: u16::try_from(book.media.pages_count)
            .ok()
            .filter(|pages| *pages > 0),
        read_state,
    }
}

/// Which of Komga's groupings of series a [`Collection`] is, encoded into its
/// `external_id` (e.g. `library:0B8RAK3Q2Y1VX`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum KomgaCollectionId {
    Library(String),
    Collection(String),
    ReadList(String),
}

impl KomgaCollectionId {
    pub fn encode(&self) -> String {
        match self {
            KomgaCollectionId::Library(id) => format!("library:{id}"),
            KomgaCollectionId::Collection(id) => format!("collection:{id}"),
            KomgaCollectionId::ReadList(id) => format!("read-list:{id}"),
        }
    }

    pub fn decode(external_id: &str) -> Result<Self, String> {
        let invalid = || format!("{external_id} is not a Komga collection id");
        let (kind, id) = external_id.split_once(':').ok_or_else(invalid)?;
        if id.is_empty() {
            return Err(invalid());
        }
        let id = id.to_string();

        match kind {
            "library" => Ok(Self::Library(id)),
            "collection" => Ok(Self::Collection(id)),
            "read-list" => Ok(Self::ReadList(id)),
            _ => Err(invalid()),
        }
    }
}

pub struct KomgaCollectionAdapter;

impl CollectionAdapter<KomgaLibrary> for KomgaCollectionAdapter {
    fn adapt_collection(source: KomgaLibrary) -> Collection {
        Collection::new(
            KomgaCollectionId::Library(source.id).encode(),
            source.name,
            CollectionKind::Library,
        )
    }
}

impl CollectionAdapter<KomgaCollection> for KomgaCollectionAdapter {
    fn adapt_collection(source: KomgaCollection) -> Collection {
        Collection::new(
            KomgaCollectionId::Collection(source.id).encode(),
            source.name,
            CollectionKind::Collection,
        )
    }
}

impl CollectionAdapter<ReadList> for KomgaCollectionAdapter {
    fn adapt_collection(source: ReadList) -> Collection {
        Collection::new(
            KomgaCollectionId::ReadList(source.id).encode(),
            source.name,
            CollectionKind::ReadingList,
        )
    }
}

/// The series a read list's books come from, once each, in the order the list
/// first reaches them. Deleted books are skipped.
pub fn read_list_series(books: Vec<KomgaBook>) -> Vec<CollectionItem> {
    let mut seen = std::collections::HashSet::new();

    books
        .into_iter()
        .filter(|book| !book.deleted)
        .filter(|book| seen.insert(book.series_id.clone()))
        .map(|book| CollectionItem {
            external_id: book.series_id,
            name: book.series_title,
            cover: String::new(),
            pages: None,
        })
        .collect()
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
    fn collection_ids_round_trip_and_reject_garbage() {
        for id in [
            KomgaCollectionId::Library("0B8RAK3Q2Y1VX".to_string()),
            KomgaCollectionId::Collection("C1".to_string()),
            KomgaCollectionId::ReadList("R1".to_string()),
        ] {
            assert_eq!(KomgaCollectionId::decode(&id.encode()), Ok(id));
        }

        assert!(KomgaCollectionId::decode("0B8RAK3Q2Y1VX").is_err());
        assert!(KomgaCollectionId::decode("library:").is_err());
        assert!(KomgaCollectionId::decode("shelf:C1").is_err());
    }

    #[test]
    fn adapt_collection_encodes_each_kind_into_the_id() {
        let library = KomgaCollectionAdapter::adapt_collection(KomgaLibrary {
            id: "L1".to_string(),
            name: "Manga".to_string(),
            ..KomgaLibrary::default()
        });
        assert_eq!(library.name, "Manga");
        assert_eq!(library.external_id, "library:L1");
        assert_eq!(library.kind, CollectionKind::Library);

        let collection = KomgaCollectionAdapter::adapt_collection(KomgaCollection {
            id: "C1".to_string(),
            name: "Classics".to_string(),
            ..KomgaCollection::default()
        });
        assert_eq!(collection.external_id, "collection:C1");
        assert_eq!(collection.kind, CollectionKind::Collection);

        let read_list = KomgaCollectionAdapter::adapt_collection(ReadList {
            id: "R1".to_string(),
            name: "Event".to_string(),
            ..ReadList::default()
        });
        assert_eq!(read_list.external_id, "read-list:R1");
        assert_eq!(read_list.kind, CollectionKind::ReadingList);
    }

    #[test]
    fn read_list_series_keeps_each_live_series_once_in_list_order() {
        let book = |series_id: &str, title: &str, deleted: bool| KomgaBook {
            series_id: series_id.to_string(),
            series_title: title.to_string(),
            deleted,
            ..KomgaBook::default()
        };

        let series = read_list_series(vec![
            book("S2", "Beta", false),
            book("S1", "Alpha", false),
            book("S2", "Beta", false),
            book("S3", "Gone", true),
        ]);

        let names: Vec<&str> = series.iter().map(|item| item.name.as_str()).collect();
        assert_eq!(names, vec!["Beta", "Alpha"]);
        assert_eq!(series[0].external_id, "S2");
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

    #[test]
    fn book_entry_maps_the_read_progress() {
        use komga_client::models::book::ReadProgress;

        let with_progress = |page, completed| KomgaBook {
            read_progress: Some(ReadProgress { page, completed }),
            ..book("t", "n", "1", 20)
        };

        assert_eq!(
            book_entry(book("t", "n", "1", 20)).read_state,
            ReadState::Unread
        );
        assert_eq!(
            book_entry(with_progress(5, false)).read_state,
            ReadState::InProgress(Some(0.25))
        );
        assert_eq!(
            book_entry(with_progress(20, true)).read_state,
            ReadState::Read
        );
    }
}
