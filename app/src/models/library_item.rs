//! A volume or chapter in the user's Library: added from a source's series
//! (see [`crate::library`]) and still read through that source. The Library is
//! a single one, made of these items; its series are the distinct series they
//! come from.

use diesel::{Insertable, Selectable, deserialize::Queryable};

use crate::models::series_entry::{EntryKind, ReadState, SeriesEntry};
use crate::schema::library_items;

#[derive(Clone, Debug, Queryable, Selectable, Insertable, PartialEq)]
#[diesel(table_name = library_items)]
pub struct LibraryItem {
    pub id: String,
    /// The source the item was added from.
    pub integration_id: String,
    /// The item's id in that source (a [`SeriesEntry::external_id`]).
    pub external_id: String,
    /// `volume` or `chapter`; see [`LibraryItem::entry_kind`].
    pub kind: String,
    pub name: String,
    pub series_external_id: String,
    pub series_name: String,
    /// How many pages the item has, once known (from the source when it was
    /// added, or from the reader).
    pub pages: Option<i32>,
    pub pages_read: i32,
    pub added_at: String,
}

impl LibraryItem {
    pub fn entry_kind(&self) -> EntryKind {
        match self.kind.as_str() {
            "volume" => EntryKind::Volume,
            _ => EntryKind::Chapter,
        }
    }

    pub fn read_state(&self) -> ReadState {
        ReadState::from_pages_read(
            self.pages_read.max(0) as u32,
            self.pages.map(|pages| pages.max(0) as u32),
        )
    }

    /// The item as a series entry, for the same cards a source's entries get.
    pub fn as_entry(&self) -> SeriesEntry {
        SeriesEntry {
            external_id: self.external_id.clone(),
            kind: self.entry_kind(),
            label: self.name.clone(),
            pages: self.pages.and_then(|pages| u16::try_from(pages).ok()),
            read_state: self.read_state(),
        }
    }
}

/// How an [`EntryKind`] is stored in `library_items.kind`.
pub fn kind_name(kind: EntryKind) -> &'static str {
    match kind {
        EntryKind::Volume => "volume",
        EntryKind::Chapter => "chapter",
    }
}

/// One series in the Library, gathered from its items.
#[derive(Clone, Debug, PartialEq)]
pub struct LibrarySeries {
    pub integration_id: String,
    pub series_external_id: String,
    pub name: String,
    pub items: usize,
    /// How many of its items are read through.
    pub items_read: usize,
}

/// Groups `items` into their series, sorted by name.
pub fn group_into_series(items: &[LibraryItem]) -> Vec<LibrarySeries> {
    let mut series: Vec<LibrarySeries> = Vec::new();

    for item in items {
        let read = item.read_state() == ReadState::Read;
        match series.iter_mut().find(|series| {
            series.integration_id == item.integration_id
                && series.series_external_id == item.series_external_id
        }) {
            Some(series) => {
                series.items += 1;
                series.items_read += usize::from(read);
            }
            None => series.push(LibrarySeries {
                integration_id: item.integration_id.clone(),
                series_external_id: item.series_external_id.clone(),
                name: item.series_name.clone(),
                items: 1,
                items_read: usize::from(read),
            }),
        }
    }

    series.sort_by_key(|series| series.name.to_lowercase());
    series
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    pub fn item(integration: &str, series: &str, id: &str, pages: i32, read: i32) -> LibraryItem {
        LibraryItem {
            id: format!("{integration}-{id}"),
            integration_id: integration.to_string(),
            external_id: id.to_string(),
            kind: "chapter".to_string(),
            name: format!("#{id}"),
            series_external_id: series.to_string(),
            series_name: format!("Series {series}"),
            pages: Some(pages),
            pages_read: read,
            added_at: "2026-11-12T00:00:00Z".to_string(),
        }
    }

    #[test]
    fn items_map_onto_entries_with_their_read_state() {
        assert_eq!(
            item("s", "1", "1", 20, 0).as_entry().read_state,
            ReadState::Unread
        );
        assert_eq!(
            item("s", "1", "1", 20, 5).as_entry().read_state,
            ReadState::InProgress(Some(0.25))
        );
        assert_eq!(
            item("s", "1", "1", 20, 20).as_entry().read_state,
            ReadState::Read
        );
        assert_eq!(
            item("s", "1", "1", 20, 0).as_entry().kind,
            EntryKind::Chapter
        );
    }

    #[test]
    fn items_group_into_series_per_source_sorted_by_name() {
        let items = vec![
            item("s", "b", "1", 10, 10),
            item("s", "a", "2", 10, 0),
            item("s", "b", "3", 10, 0),
            // Same series id, other source: another series.
            item("t", "b", "1", 10, 0),
        ];

        let series = group_into_series(&items);

        assert_eq!(series.len(), 3);
        assert_eq!(series[0].series_external_id, "a");
        let b = series
            .iter()
            .find(|s| s.integration_id == "s" && s.series_external_id == "b")
            .unwrap();
        assert_eq!((b.items, b.items_read), (2, 1));
    }
}
