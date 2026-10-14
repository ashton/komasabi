use kavita_client::models::{
    chapter::Chapter as KavitaChapter,
    library::{KavitaLibrary, KavitaLibraryType},
    series::Series as KavitaSeries,
    volume::{LOOSE_LEAF_VOLUME_NUMBER, SPECIAL_VOLUME_NUMBER, Volume as KavitaVolume},
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

/// Flattens a series' volumes into the entries its detail grid should show,
/// mirroring how Kavita itself models series contents:
///
/// - A **numbered volume** (`min_number > 0`) is a bound book read as a unit,
///   so it becomes a single volume card; its internal "default" chapter (whose
///   number is the loose-leaf sentinel) is deliberately not descended into.
/// - The **loose-leaf** volume (and the **specials** volume) hold chapters that
///   aren't collected into a numbered volume — the individual issues/specials —
///   so each of their chapters becomes its own chapter card.
///
/// A series with both (an ongoing title whose early issues are collected into
/// volumes while recent ones are still loose) therefore shows volume cards for
/// the collected part and chapter cards for the loose part — Kavita's
/// "Storyline" ordering, preserved by iterating volumes in the order returned.
pub fn series_entries(volumes: Vec<KavitaVolume>) -> Vec<SeriesEntry> {
    let mut entries = Vec::new();

    for volume in volumes {
        if is_numbered_volume(&volume) {
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
    }
}

fn chapter_entry(chapter: KavitaChapter) -> SeriesEntry {
    SeriesEntry {
        external_id: chapter.id.to_string(),
        kind: EntryKind::Chapter,
        label: chapter_label(&chapter),
        pages: (chapter.pages > 0).then_some(chapter.pages),
    }
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
            loose_leaf_volume(vec![
                chapter(20, "2", 2.0, 30),
                chapter(21, "3", 3.0, 30),
            ]),
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
}
