//! Adding a source's volume/chapter, or a whole series of them (see
//! [`add_series_to_library`]), to the user's Library. Adding only saves
//! the item in the `library_items` table: nothing is downloaded, and the item
//! is still read, and its covers fetched, through its source. Downloading an
//! item for offline reading is meant to be a separate option.
//!
//! Items added from a source that keeps the user's reading history (see
//! [`Provider::tracks_reading_progress`]) follow the progress kept there:
//! [`sync_progress`] copies it over whenever the Library's items are loaded.

use uuid::Uuid;

use crate::models::library_item::{LibraryItem, kind_name};
use crate::models::series_entry::{ReadState, SeriesEntry};
use crate::providers::{Provider, provider_for_integration};
use crate::repository::library as library_repository;

/// Saves `entry`, of the series `series_id` in the source `integration_id`,
/// in the Library. Adding an entry that's already in the Library keeps the
/// progress made on it.
pub async fn add_to_library(
    integration_id: String,
    series_id: String,
    series_name: String,
    entry: SeriesEntry,
) -> Result<LibraryItem, String> {
    let previous = library_repository::series_items(integration_id.clone(), series_id.clone())
        .await?
        .into_iter()
        .find(|item| item.entry_kind() == entry.kind && item.external_id == entry.external_id);

    let item = new_item(integration_id, series_id, series_name, &entry, previous);
    library_repository::save(item).await
}

/// Saves every volume/chapter of the series `series_id`, in the source
/// `integration_id`, in the Library, as listed by the source now. Entries
/// already in the Library keep their progress. Returns how many entries were
/// newly added.
pub async fn add_series_to_library(
    integration_id: String,
    series_id: String,
    series_name: String,
) -> Result<usize, String> {
    let (provider, _namespace) = provider_for_integration(&integration_id).await?;
    let entries = provider.list_series_entries(&series_id).await?;
    if entries.is_empty() {
        return Err(format!("{series_name} has no volumes or chapters"));
    }

    let saved = library_repository::series_items(integration_id.clone(), series_id.clone()).await?;

    let mut added = 0;
    for entry in &entries {
        let previous = saved
            .iter()
            .find(|item| item.entry_kind() == entry.kind && item.external_id == entry.external_id)
            .cloned();
        if previous.is_none() {
            added += 1;
        }

        let item = new_item(
            integration_id.clone(),
            series_id.clone(),
            series_name.clone(),
            entry,
            previous,
        );
        library_repository::save(item).await?;
    }

    Ok(added)
}

/// The Library item for `entry`, carrying over the id, page count and
/// progress of the `previous` item saved for it, if any.
fn new_item(
    integration_id: String,
    series_id: String,
    series_name: String,
    entry: &SeriesEntry,
    previous: Option<LibraryItem>,
) -> LibraryItem {
    let pages = entry
        .pages
        .map(i32::from)
        .or_else(|| previous.as_ref().and_then(|item| item.pages));
    let pages_read = previous
        .as_ref()
        .map(|item| item.pages_read)
        .unwrap_or(0)
        .max(pages_read_from(entry.read_state, pages));

    LibraryItem {
        id: previous
            .map(|item| item.id)
            .unwrap_or_else(|| Uuid::new_v4().to_string()),
        integration_id,
        external_id: entry.external_id.clone(),
        kind: kind_name(entry.kind).to_string(),
        name: entry.label.clone(),
        series_external_id: series_id,
        series_name,
        pages,
        pages_read,
        added_at: chrono::Utc::now().to_rfc3339(),
    }
}

/// Brings `items`' progress in line with their sources', for the sources that
/// keep reading history, saving every item that changed. Returns the items,
/// in the same order, with the progress they now have. A source or series
/// that can't be reached leaves its items as they were (it's only logged), so
/// the Library still shows what it has.
pub async fn sync_progress(mut items: Vec<LibraryItem>) -> Vec<LibraryItem> {
    // Each source is resolved once, and each of its series listed once.
    let mut series: Vec<(String, String)> = Vec::new();
    for item in &items {
        let key = (item.integration_id.clone(), item.series_external_id.clone());
        if !series.contains(&key) {
            series.push(key);
        }
    }

    let mut sources: Vec<String> = Vec::new();
    for (source, _) in &series {
        if !sources.contains(source) {
            sources.push(source.clone());
        }
    }
    for source in sources {
        let provider = match provider_for_integration(&source).await {
            Ok((provider, _namespace)) if provider.tracks_reading_progress() => provider,
            Ok(_) => continue,
            Err(error) => {
                tracing::debug!("could not sync library progress from {source}: {error}");
                continue;
            }
        };

        for (_, series_id) in series.iter().filter(|(id, _)| *id == source) {
            let entries = match provider.list_series_entries(series_id).await {
                Ok(entries) => entries,
                Err(error) => {
                    tracing::debug!("could not sync library progress of {series_id}: {error}");
                    continue;
                }
            };

            for item in items.iter_mut().filter(|item| {
                item.integration_id == source && item.series_external_id == *series_id
            }) {
                let Some(entry) = entries.iter().find(|entry| {
                    entry.kind == item.entry_kind() && entry.external_id == item.external_id
                }) else {
                    continue;
                };
                let Some((pages, pages_read)) = synced_progress(item, entry) else {
                    continue;
                };
                match library_repository::set_progress(&item.id, pages, pages_read).await {
                    Ok(()) => {
                        item.pages = pages;
                        item.pages_read = pages_read;
                    }
                    Err(error) => tracing::debug!("could not save synced progress: {error}"),
                }
            }
        }
    }

    items
}

/// The page count and pages read `item` should have to match the source's
/// `entry`, or `None` when it already does, or when the source's state can't
/// be told in pages (started, with no page count to place it).
fn synced_progress(item: &LibraryItem, entry: &SeriesEntry) -> Option<(Option<i32>, i32)> {
    let pages = entry.pages.map(i32::from).or(item.pages);
    let pages_read = match (entry.read_state, pages) {
        (ReadState::Unread, _) => 0,
        (ReadState::Read, Some(pages)) => pages,
        // Started but not finished: somewhere past the first page and short
        // of the last, however the source rounds its fraction.
        (ReadState::InProgress(Some(fraction)), Some(pages)) if pages > 1 => {
            ((fraction * pages as f32).round() as i32).clamp(1, pages - 1)
        }
        (ReadState::InProgress(Some(_)), Some(pages)) => pages.min(1),
        (ReadState::InProgress(None), _) if item.pages_read > 0 => item.pages_read,
        (ReadState::Read | ReadState::InProgress(_), _) => return None,
    };

    (pages != item.pages || pages_read != item.pages_read).then_some((pages, pages_read))
}

/// How many pages the source had marked as read, carried over to the Library.
/// Without a page count, only an entry read through can't be told apart from
/// an unread one, so nothing is carried over.
fn pages_read_from(state: ReadState, pages: Option<i32>) -> i32 {
    let Some(pages) = pages else {
        return 0;
    };
    match state {
        ReadState::Unread | ReadState::InProgress(None) => 0,
        ReadState::InProgress(Some(fraction)) => {
            ((fraction * pages as f32).round() as i32).clamp(0, pages)
        }
        ReadState::Read => pages,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::series_entry::EntryKind;

    fn entry(pages: Option<u16>, read_state: ReadState) -> SeriesEntry {
        SeriesEntry {
            external_id: "7".to_string(),
            kind: EntryKind::Volume,
            label: "Volume 1".to_string(),
            pages,
            read_state,
        }
    }

    #[test]
    fn read_state_carries_over_as_pages_read() {
        assert_eq!(pages_read_from(ReadState::Unread, Some(20)), 0);
        assert_eq!(pages_read_from(ReadState::InProgress(None), Some(20)), 0);
        assert_eq!(
            pages_read_from(ReadState::InProgress(Some(0.25)), Some(20)),
            5
        );
        assert_eq!(pages_read_from(ReadState::Read, Some(20)), 20);
        assert_eq!(pages_read_from(ReadState::Read, None), 0);
    }

    #[test]
    fn a_new_item_records_the_entry_without_downloading_it() {
        let item = new_item(
            "src".to_string(),
            "42".to_string(),
            "Series".to_string(),
            &entry(Some(20), ReadState::InProgress(Some(0.5))),
            None,
        );

        assert_eq!(item.integration_id, "src");
        assert_eq!(item.series_external_id, "42");
        assert_eq!(item.external_id, "7");
        assert_eq!(item.kind, "volume");
        assert_eq!(item.name, "Volume 1");
        assert_eq!(item.pages, Some(20));
        assert_eq!(item.pages_read, 10);
    }

    fn synced(
        item_pages: i32,
        item_read: i32,
        pages: Option<u16>,
        state: ReadState,
    ) -> Option<(Option<i32>, i32)> {
        let item =
            crate::models::library_item::tests::item("src", "42", "7", item_pages, item_read);
        let entry = SeriesEntry {
            kind: EntryKind::Chapter,
            ..entry(pages, state)
        };
        synced_progress(&item, &entry)
    }

    #[test]
    fn the_sources_progress_wins_both_ways() {
        // Read further on the source.
        assert_eq!(
            synced(20, 5, Some(20), ReadState::InProgress(Some(0.5))),
            Some((Some(20), 10))
        );
        assert_eq!(
            synced(20, 5, Some(20), ReadState::Read),
            Some((Some(20), 20))
        );
        // Marked unread on the source.
        assert_eq!(
            synced(20, 20, Some(20), ReadState::Unread),
            Some((Some(20), 0))
        );
    }

    #[test]
    fn matching_progress_leaves_the_item_alone() {
        assert_eq!(
            synced(20, 10, Some(20), ReadState::InProgress(Some(0.5))),
            None
        );
        assert_eq!(synced(20, 0, Some(20), ReadState::Unread), None);
        assert_eq!(synced(20, 20, None, ReadState::Read), None);
    }

    #[test]
    fn a_started_entry_never_syncs_as_unread_or_read() {
        assert_eq!(
            synced(20, 0, Some(20), ReadState::InProgress(Some(0.99))),
            Some((Some(20), 19))
        );
        assert_eq!(
            synced(20, 0, Some(20), ReadState::InProgress(Some(0.01))),
            Some((Some(20), 1))
        );
    }

    #[test]
    fn progress_that_cant_be_placed_in_pages_is_left_alone() {
        let item = LibraryItem {
            pages: None,
            ..crate::models::library_item::tests::item("src", "42", "7", 0, 0)
        };
        let started = SeriesEntry {
            kind: EntryKind::Chapter,
            ..entry(None, ReadState::InProgress(None))
        };
        let read = SeriesEntry {
            kind: EntryKind::Chapter,
            ..entry(None, ReadState::Read)
        };
        assert_eq!(synced_progress(&item, &started), None);
        assert_eq!(synced_progress(&item, &read), None);
        // A started item stays where it is when the source has no fraction.
        assert_eq!(synced(20, 7, Some(20), ReadState::InProgress(None)), None);
    }

    #[test]
    fn adding_again_keeps_the_id_page_count_and_progress() {
        let previous = LibraryItem {
            id: "kept".to_string(),
            ..crate::models::library_item::tests::item("src", "42", "7", 30, 12)
        };

        let item = new_item(
            "src".to_string(),
            "42".to_string(),
            "Series".to_string(),
            &entry(None, ReadState::Unread),
            Some(previous),
        );

        assert_eq!(item.id, "kept");
        assert_eq!(item.pages, Some(30));
        assert_eq!(item.pages_read, 12);
    }
}
