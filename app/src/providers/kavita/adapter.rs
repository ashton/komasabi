use kavita_client::models::{
    chapter::Chapter as KavitaChapter,
    collection::KavitaCollection,
    library::KavitaLibrary,
    reading_list::{ReadingList, ReadingListItem},
    series::Series as KavitaSeries,
    volume::{LOOSE_LEAF_VOLUME_NUMBER, SPECIAL_VOLUME_NUMBER, Volume as KavitaVolume},
};

use crate::{
    adapter::{CollectionAdapter, ItemAdapter},
    models::{
        collection::{Collection, CollectionKind},
        collection_item::CollectionItem,
        series_entry::{EntryKind, ReadState, SeriesEntry},
    },
};

pub struct KavitaItemAdapter;

impl ItemAdapter<KavitaSeries> for KavitaItemAdapter {
    fn adapt_item(source: KavitaSeries) -> CollectionItem {
        CollectionItem {
            external_id: source.id.to_string(),
            name: source.name,
            cover: source.cover_image,
            pages: (source.pages > 0).then_some(source.pages),
        }
    }
}

/// Flattens a series' volumes into the entries its detail grid should show,
/// mirroring how Kavita itself models series contents:
///
/// - A **bound volume** — a numbered volume holding just its single "default"
///   chapter (the whole-volume stand-in, marked with the loose-leaf sentinel
///   number) — is read as a unit, so it becomes a single volume card and isn't
///   descended into.
/// - Any other volume is descended into: each of its chapters becomes its own
///   chapter card. This covers the **loose-leaf** and **specials** buckets
///   (individual issues/specials) *and* a numbered volume that groups several
///   real issues (e.g. a year/arc like "1991" holding issues #1–#5).
///
/// A series with both (an ongoing title whose early issues are collected into
/// bound volumes while recent ones are still loose) therefore shows volume cards
/// for the collected part and chapter cards for the loose part — Kavita's
/// "Storyline" ordering, preserved by iterating volumes in the order returned.
pub fn series_entries(volumes: Vec<KavitaVolume>) -> Vec<SeriesEntry> {
    let mut entries = Vec::new();

    for volume in volumes {
        if is_single_bound_volume(&volume) {
            entries.push(volume_entry(volume));
        } else {
            entries.extend(volume.chapters.into_iter().map(chapter_entry));
        }
    }

    entries
}

/// Whether a volume is a real, numbered volume rather than one of Kavita's
/// sentinel buckets (loose-leaf / specials), both of which are negative.
fn is_numbered_volume(volume: &KavitaVolume) -> bool {
    volume.min_number != LOOSE_LEAF_VOLUME_NUMBER && volume.min_number != SPECIAL_VOLUME_NUMBER
}

/// Whether a volume is a single bound unit: a numbered volume whose only chapter
/// is its "default" chapter (the whole-volume stand-in Kavita marks with the
/// loose-leaf sentinel number). Such a volume collapses to one volume card. A
/// numbered volume that instead holds multiple real, separately-numbered
/// chapters is *not* bound — its chapters are shown individually.
fn is_single_bound_volume(volume: &KavitaVolume) -> bool {
    is_numbered_volume(volume)
        && volume.chapters.len() == 1
        && volume.chapters[0].min_number == LOOSE_LEAF_VOLUME_NUMBER
}

fn volume_entry(volume: KavitaVolume) -> SeriesEntry {
    let name = volume.name.trim();
    let label = if name.is_empty() {
        "Volume".to_string()
    } else if is_number_like(name) {
        format!("Volume {name}")
    } else {
        name.to_string()
    };

    SeriesEntry {
        external_id: volume.id.to_string(),
        kind: EntryKind::Volume,
        label,
        pages: (volume.pages > 0).then_some(volume.pages),
        read_state: kavita_read_state(volume.pages_read, volume.pages),
    }
}

fn chapter_entry(chapter: KavitaChapter) -> SeriesEntry {
    SeriesEntry {
        external_id: chapter.id.to_string(),
        kind: EntryKind::Chapter,
        label: chapter_label(&chapter),
        pages: (chapter.pages > 0).then_some(chapter.pages),
        read_state: kavita_read_state(chapter.pages_read, chapter.pages),
    }
}

/// Kavita reports how many of a volume's or chapter's pages have been read.
fn kavita_read_state(pages_read: u16, pages: u16) -> ReadState {
    ReadState::from_pages_read(pages_read.into(), (pages > 0).then_some(pages.into()))
}

/// Builds a chapter card's label. A genuinely distinct title wins; Kavita fills
/// `title_name` with the bare number when there's no real title, so a title
/// that just repeats the range is ignored. Specials (and the loose-leaf
/// sentinel range) fall back to "Special"; everything else is shown as "#<n>".
fn chapter_label(chapter: &KavitaChapter) -> String {
    let range = chapter.range.trim();

    if let Some(title) = chapter
        .title_name
        .as_deref()
        .map(str::trim)
        .filter(|title| !title.is_empty() && *title != range)
    {
        return title.to_string();
    }

    if chapter.is_special
        || range.is_empty()
        || chapter.min_number == LOOSE_LEAF_VOLUME_NUMBER
        || chapter.min_number == SPECIAL_VOLUME_NUMBER
    {
        "Special".to_string()
    } else {
        format!("#{range}")
    }
}

fn is_number_like(value: &str) -> bool {
    !value.is_empty() && value.chars().all(|c| c.is_ascii_digit() || c == '.')
}

/// Which of Kavita's groupings of series a [`Collection`] is, encoded into its
/// `external_id` (e.g. `library:5`), since libraries, collections and reading
/// lists each have their own id space.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum KavitaCollectionId {
    Library(u16),
    Collection(u32),
    ReadingList(u32),
}

impl KavitaCollectionId {
    pub fn encode(&self) -> String {
        match self {
            KavitaCollectionId::Library(id) => format!("library:{id}"),
            KavitaCollectionId::Collection(id) => format!("collection:{id}"),
            KavitaCollectionId::ReadingList(id) => format!("reading-list:{id}"),
        }
    }

    pub fn decode(external_id: &str) -> Result<Self, String> {
        let invalid = || format!("{external_id} is not a Kavita collection id");
        let (kind, id) = external_id.split_once(':').ok_or_else(invalid)?;

        match kind {
            "library" => id.parse().map(Self::Library).map_err(|_| invalid()),
            "collection" => id.parse().map(Self::Collection).map_err(|_| invalid()),
            "reading-list" => id.parse().map(Self::ReadingList).map_err(|_| invalid()),
            _ => Err(invalid()),
        }
    }
}

pub struct KavitaCollectionAdapter;

impl CollectionAdapter<KavitaLibrary> for KavitaCollectionAdapter {
    fn adapt_collection(source: KavitaLibrary) -> Collection {
        Collection::new(
            KavitaCollectionId::Library(source.id).encode(),
            source.name,
            CollectionKind::Library,
        )
    }
}

impl CollectionAdapter<KavitaCollection> for KavitaCollectionAdapter {
    fn adapt_collection(source: KavitaCollection) -> Collection {
        Collection::new(
            KavitaCollectionId::Collection(source.id).encode(),
            source.title,
            CollectionKind::Collection,
        )
    }
}

impl CollectionAdapter<ReadingList> for KavitaCollectionAdapter {
    fn adapt_collection(source: ReadingList) -> Collection {
        Collection::new(
            KavitaCollectionId::ReadingList(source.id).encode(),
            source.title,
            CollectionKind::ReadingList,
        )
    }
}

/// The series a reading list's chapters come from, once each, in the order
/// the list first reaches them.
pub fn reading_list_series(items: Vec<ReadingListItem>) -> Vec<CollectionItem> {
    let mut seen = std::collections::HashSet::new();

    items
        .into_iter()
        .filter(|item| seen.insert(item.series_id))
        .map(|item| CollectionItem {
            external_id: item.series_id.to_string(),
            name: item.series_name,
            cover: String::new(),
            pages: None,
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn collection_ids_round_trip_and_reject_garbage() {
        for id in [
            KavitaCollectionId::Library(5),
            KavitaCollectionId::Collection(7),
            KavitaCollectionId::ReadingList(9),
        ] {
            assert_eq!(KavitaCollectionId::decode(&id.encode()), Ok(id));
        }

        assert!(KavitaCollectionId::decode("5").is_err());
        assert!(KavitaCollectionId::decode("shelf:5").is_err());
        assert!(KavitaCollectionId::decode("library:x").is_err());
    }

    #[test]
    fn adapt_collection_encodes_each_kind_into_the_id() {
        let library = KavitaCollectionAdapter::adapt_collection(KavitaLibrary {
            id: 7,
            name: "Comics".to_string(),
            ..KavitaLibrary::default()
        });
        assert_eq!(library.name, "Comics");
        assert_eq!(library.external_id, "library:7");
        assert_eq!(library.kind, CollectionKind::Library);
        assert!(!library.importable);

        let collection = KavitaCollectionAdapter::adapt_collection(KavitaCollection {
            id: 3,
            title: "Favorites".to_string(),
            ..KavitaCollection::default()
        });
        assert_eq!(collection.external_id, "collection:3");
        assert_eq!(collection.kind, CollectionKind::Collection);

        let list = KavitaCollectionAdapter::adapt_collection(ReadingList {
            id: 2,
            title: "Crossover".to_string(),
            ..ReadingList::default()
        });
        assert_eq!(list.external_id, "reading-list:2");
        assert_eq!(list.kind, CollectionKind::ReadingList);
    }

    #[test]
    fn reading_list_series_keeps_each_series_once_in_list_order() {
        let item = |series_id: u16, name: &str| ReadingListItem {
            series_id,
            series_name: name.to_string(),
            ..ReadingListItem::default()
        };

        let series = reading_list_series(vec![item(2, "Saga"), item(1, "Batman"), item(2, "Saga")]);

        let names: Vec<&str> = series.iter().map(|item| item.name.as_str()).collect();
        assert_eq!(names, vec!["Saga", "Batman"]);
        assert_eq!(series[0].external_id, "2");
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

    /// A numbered volume holding a single "default" chapter whose number is the
    /// loose-leaf sentinel — the shape Kavita returns for a bound volume (as in
    /// series 193).
    fn numbered_volume(id: u16, name: &str, min_number: f32, pages: u16) -> KavitaVolume {
        KavitaVolume {
            id,
            name: name.to_string(),
            min_number,
            max_number: min_number,
            pages,
            chapters: vec![KavitaChapter {
                id: id + 1000,
                range: LOOSE_LEAF_VOLUME_NUMBER.to_string(),
                min_number: LOOSE_LEAF_VOLUME_NUMBER,
                pages,
                ..KavitaChapter::default()
            }],
            ..KavitaVolume::default()
        }
    }

    fn chapter(id: u16, range: &str, min_number: f32, pages: u16) -> KavitaChapter {
        KavitaChapter {
            id,
            range: range.to_string(),
            min_number,
            pages,
            ..KavitaChapter::default()
        }
    }

    /// The loose-leaf volume holding real chapters (as in series 345).
    fn loose_leaf_volume(chapters: Vec<KavitaChapter>) -> KavitaVolume {
        KavitaVolume {
            id: 1605,
            name: LOOSE_LEAF_VOLUME_NUMBER.to_string(),
            min_number: LOOSE_LEAF_VOLUME_NUMBER,
            max_number: LOOSE_LEAF_VOLUME_NUMBER,
            chapters,
            ..KavitaVolume::default()
        }
    }

    #[test]
    fn series_entries_shows_numbered_volumes_as_volume_cards() {
        // Series 193: three numbered volumes, each with one sentinel chapter.
        let volumes = vec![
            numbered_volume(1144, "1", 1.0, 219),
            numbered_volume(1143, "2", 2.0, 227),
            numbered_volume(1142, "3", 3.0, 204),
        ];

        let entries = series_entries(volumes);

        assert_eq!(entries.len(), 3);
        assert!(entries.iter().all(|e| e.kind == EntryKind::Volume));
        assert_eq!(entries[0].external_id, "1144");
        assert_eq!(entries[0].label, "Volume 1");
        assert_eq!(entries[0].pages, Some(219));
        assert_eq!(entries[2].label, "Volume 3");
    }

    #[test]
    fn series_entries_descends_a_numbered_volume_that_groups_multiple_issues() {
        // Series 288: a single numbered volume "1991" grouping five separately-
        // numbered issues. Each issue must be its own chapter card, not collapsed
        // into one "1991" volume card.
        let volumes = vec![KavitaVolume {
            id: 1485,
            name: "1991".to_string(),
            min_number: 1991.0,
            max_number: 1991.0,
            pages: 131,
            chapters: (1..=5)
                .map(|n| chapter(2108 + n, &n.to_string(), n as f32, 26))
                .collect(),
            ..KavitaVolume::default()
        }];

        let entries = series_entries(volumes);

        assert_eq!(entries.len(), 5);
        assert!(entries.iter().all(|e| e.kind == EntryKind::Chapter));
        assert_eq!(entries[0].label, "#1");
        assert_eq!(entries[4].label, "#5");
    }

    #[test]
    fn series_entries_shows_loose_leaf_chapters_as_chapter_cards() {
        // Series 345: one loose-leaf volume holding six real chapters.
        let chapters = (1..=6)
            .map(|n| chapter(2270 + n, &n.to_string(), n as f32, 30))
            .collect();
        let volumes = vec![loose_leaf_volume(chapters)];

        let entries = series_entries(volumes);

        assert_eq!(entries.len(), 6);
        assert!(entries.iter().all(|e| e.kind == EntryKind::Chapter));
        assert_eq!(entries[0].external_id, "2271");
        assert_eq!(entries[0].label, "#1");
        assert_eq!(entries[5].label, "#6");
    }

    #[test]
    fn series_entries_mixes_volumes_and_loose_chapters_in_order() {
        // An ongoing series: volume 1 collected, issues 2-3 still loose.
        let volumes = vec![
            numbered_volume(10, "1", 1.0, 200),
            loose_leaf_volume(vec![chapter(20, "2", 2.0, 30), chapter(21, "3", 3.0, 30)]),
        ];

        let entries = series_entries(volumes);

        assert_eq!(entries.len(), 3);
        assert_eq!(entries[0].kind, EntryKind::Volume);
        assert_eq!(entries[0].label, "Volume 1");
        assert_eq!(entries[1].kind, EntryKind::Chapter);
        assert_eq!(entries[1].label, "#2");
        assert_eq!(entries[2].label, "#3");
    }

    #[test]
    fn series_entries_labels_specials_as_special() {
        let special = KavitaChapter {
            id: 99,
            range: LOOSE_LEAF_VOLUME_NUMBER.to_string(),
            is_special: true,
            ..KavitaChapter::default()
        };
        let volumes = vec![KavitaVolume {
            id: 500,
            min_number: SPECIAL_VOLUME_NUMBER,
            max_number: SPECIAL_VOLUME_NUMBER,
            chapters: vec![special],
            ..KavitaVolume::default()
        }];

        let entries = series_entries(volumes);

        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].kind, EntryKind::Chapter);
        assert_eq!(entries[0].label, "Special");
    }

    #[test]
    fn series_entries_prefers_a_distinct_chapter_title_over_the_number() {
        let titled = KavitaChapter {
            id: 7,
            range: "3".to_string(),
            min_number: 3.0,
            title_name: Some("The Reckoning".to_string()),
            ..KavitaChapter::default()
        };

        let entries = series_entries(vec![loose_leaf_volume(vec![titled])]);

        assert_eq!(entries[0].label, "The Reckoning");
    }

    #[test]
    fn series_entries_ignores_a_chapter_title_that_just_repeats_the_range() {
        // Kavita sets title_name to the bare number when there's no real title.
        let numbered = KavitaChapter {
            id: 7,
            range: "3".to_string(),
            min_number: 3.0,
            title_name: Some("3".to_string()),
            ..KavitaChapter::default()
        };

        let entries = series_entries(vec![loose_leaf_volume(vec![numbered])]);

        assert_eq!(entries[0].label, "#3");
    }

    #[test]
    fn series_entries_maps_zero_pages_to_none() {
        let entries = series_entries(vec![loose_leaf_volume(vec![chapter(1, "1", 1.0, 0)])]);

        assert_eq!(entries[0].pages, None);
    }

    #[test]
    fn series_entries_carry_each_cards_read_state() {
        let mut read = numbered_volume(1, "1", 1.0, 20);
        read.pages_read = 20;
        let mut started = numbered_volume(2, "2", 2.0, 20);
        started.pages_read = 5;
        let unread = numbered_volume(3, "3", 3.0, 20);

        let states: Vec<_> = series_entries(vec![read, started, unread])
            .into_iter()
            .map(|entry| entry.read_state)
            .collect();

        assert_eq!(
            states,
            [
                ReadState::Read,
                ReadState::InProgress(Some(0.25)),
                ReadState::Unread
            ]
        );
    }

    #[test]
    fn series_entries_carry_a_chapter_cards_read_state() {
        let mut started = chapter(1, "1", 1.0, 10);
        started.pages_read = 4;

        let entries = series_entries(vec![loose_leaf_volume(vec![started])]);

        assert_eq!(entries[0].read_state, ReadState::InProgress(Some(0.4)));
    }
}
