use std::path::PathBuf;

use crate::models::collection::Collection;
use crate::models::collection_item::CollectionItem;
use crate::models::integration::{Integration, IntegrationType};
use crate::models::series_entry::SeriesEntry;
use crate::repository::integration as integration_repository;

pub mod file_system;
#[cfg(feature = "kavita")]
pub mod kavita;
#[cfg(feature = "komga")]
pub mod komga;
#[cfg(feature = "opds")]
pub mod opds;
#[cfg(feature = "suwayomi")]
pub mod suwayomi;

use file_system::FileSystemProvider;
#[cfg(feature = "kavita")]
use kavita::provider::KavitaProvider;
#[cfg(feature = "komga")]
use komga::provider::KomgaProvider;
#[cfg(feature = "opds")]
use opds::provider::OpdsProvider;
#[cfg(feature = "suwayomi")]
use suwayomi::provider::SuwayomiProvider;

/// One page to fetch from a provider: an opaque, provider-defined `chapter_id`
/// and the page's 0-based index within that chapter. A provider flattens a
/// [`SeriesEntry`] (a volume or a chapter) into an ordered list of these; the
/// reader launcher then fetches each one's bytes. The `chapter_id` is only ever
/// meaningful to the provider that produced it (a Kavita chapter id, a
/// filesystem archive path, …), so the launcher stays backend-agnostic.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PageRef {
    pub chapter_id: String,
    pub page: u16,
}

/// A page the user has just read, as reported to a provider's
/// [`Provider::update_reading_progress`]. `page` is one of the [`PageRef`]s the
/// provider produced in [`Provider::reader_pages`], and `chapter_pages` is how
/// many pages that chapter has (the reader may be showing a volume made of
/// several chapters, and each chapter's progress is tracked on its own).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReadingProgress {
    pub page: PageRef,
    pub chapter_pages: u16,
}

impl ReadingProgress {
    /// Whether this is the chapter's last page, i.e. reading it finishes the
    /// chapter.
    pub fn finishes_chapter(&self) -> bool {
        self.page.page + 1 >= self.chapter_pages
    }
}

/// The backend-agnostic operations the app performs against a source (an
/// integration). Each backend plugs in by adding a [`ResolvedProvider`]
/// variant. Uses native `async fn`, so it is not
/// `dyn`-compatible — callers hold a [`ResolvedProvider`] (an `impl Provider`),
/// not a trait object.
pub trait Provider {
    /// Lists everything in the source that gathers series (libraries,
    /// collections, reading lists, categories, feeds, …), live from the backend.
    async fn list_collections(&self) -> Result<Vec<Collection>, String>;
    /// Lists a collection's series as display items, one page at a time
    /// (1-based `page`), mirroring the browse screen's pagination.
    /// `collection_id` is a [`Collection::external_id`] this provider listed.
    async fn list_items(
        &self,
        collection_id: &str,
        page: Option<u32>,
        page_size: Option<u32>,
    ) -> Result<Vec<CollectionItem>, String>;
    /// Copies `files` into the collection, for collections that are
    /// [`Collection::importable`], returning how many were added. The default
    /// refuses, for backends whose collections can't take files.
    async fn import_files(
        &self,
        _collection_id: &str,
        _files: &[PathBuf],
    ) -> Result<usize, String> {
        Err("this collection doesn't accept imported files".to_string())
    }
    async fn list_series_entries(&self, series_id: &str) -> Result<Vec<SeriesEntry>, String>;
    async fn cover(&self, integration_id: &str, series_id: &str) -> Result<Vec<u8>, String>;
    async fn volume_cover(&self, integration_id: &str, volume_id: &str) -> Result<Vec<u8>, String>;
    async fn chapter_cover(
        &self,
        integration_id: &str,
        chapter_id: &str,
    ) -> Result<Vec<u8>, String>;
    /// Resolves a volume/chapter entry into the ordered pages to read.
    async fn reader_pages(&self, entry: &SeriesEntry) -> Result<Vec<PageRef>, String>;
    /// Fetches the bytes of a single page.
    async fn reader_image(&self, chapter_id: &str, page: u16) -> Result<Vec<u8>, String>;
    /// Records that the user read `progress.page`, so the backend's reading
    /// history follows what's read in Komasabi. Called on every page the reader
    /// shows (see [`crate::reading_progress`]). Backends that keep reading
    /// history override this. The default does nothing, for those that don't
    /// (e.g. local folders and OPDS catalogs).
    async fn update_reading_progress(&self, _progress: &ReadingProgress) -> Result<(), String> {
        Ok(())
    }
    /// The entry to read after `current` in the series `series_id`, offered to
    /// the user when the reader reaches `current`'s last page. `None` when
    /// `current` is the last one the backend has. Each backend decides what
    /// "next" means: its own next-book lookup where it has one, otherwise the
    /// entry after `current` in its reading order (see [`entry_after`]).
    async fn next_entry(
        &self,
        series_id: &str,
        current: &SeriesEntry,
    ) -> Result<Option<SeriesEntry>, String>;
    /// The entry to read before `current` in the series `series_id`, offered
    /// when the reader turns back past `current`'s first page. `None` when
    /// `current` is the first one. The mirror image of [`Self::next_entry`]
    /// (see [`entry_before`]).
    async fn previous_entry(
        &self,
        series_id: &str,
        current: &SeriesEntry,
    ) -> Result<Option<SeriesEntry>, String>;
}

/// Cuts the 1-based `page` of `page_size` items out of `items`, for backends
/// that hand over a whole collection at once. Without both (or with either
/// zero), every item is returned.
pub fn paginate<T>(items: Vec<T>, page: Option<u32>, page_size: Option<u32>) -> Vec<T> {
    let (Some(page), Some(page_size)) = (page, page_size) else {
        return items;
    };
    if page == 0 || page_size == 0 {
        return items;
    }

    let start = ((page - 1) * page_size) as usize;
    items
        .into_iter()
        .skip(start)
        .take(page_size as usize)
        .collect()
}

/// The entry that follows `current` in `entries`, a series' entries in reading
/// order (as [`Provider::list_series_entries`] lists them). `None` when
/// `current` is the last one, or isn't among them at all.
pub fn entry_after(entries: Vec<SeriesEntry>, current: &SeriesEntry) -> Option<SeriesEntry> {
    let position = entries
        .iter()
        .position(|entry| entry.kind == current.kind && entry.external_id == current.external_id)?;
    entries.into_iter().nth(position + 1)
}

/// The entry that precedes `current` in `entries`, a series' entries in reading
/// order. `None` when `current` is the first one, or isn't among them at all.
pub fn entry_before(entries: Vec<SeriesEntry>, current: &SeriesEntry) -> Option<SeriesEntry> {
    let position = entries
        .iter()
        .position(|entry| entry.kind == current.kind && entry.external_id == current.external_id)?;
    entries.into_iter().nth(position.checked_sub(1)?)
}

/// A provider resolved for a particular source, erasing the concrete backend
/// behind the [`Provider`] trait. One variant per supported backend.
pub enum ResolvedProvider {
    #[cfg(feature = "kavita")]
    Kavita(KavitaProvider),
    #[cfg(feature = "komga")]
    Komga(KomgaProvider),
    FileSystem(FileSystemProvider),
    #[cfg(feature = "opds")]
    Opds(OpdsProvider),
    #[cfg(feature = "suwayomi")]
    Suwayomi(SuwayomiProvider),
}

/// Forwards a [`Provider`] method call to whichever backend `$provider` holds.
/// Calls use fully-qualified trait-method syntax (`Provider::foo(p, …)`) rather
/// than `p.foo(…)`, because `KavitaProvider` has inherent methods of the same
/// names taking numeric (`u16`) ids that would otherwise shadow the string-id
/// trait methods and fail to type-check.
macro_rules! dispatch {
    ($provider:expr, $method:ident($($arg:expr),*)) => {
        match $provider {
            #[cfg(feature = "kavita")]
            ResolvedProvider::Kavita(provider) => Provider::$method(provider, $($arg),*).await,
            #[cfg(feature = "komga")]
            ResolvedProvider::Komga(provider) => Provider::$method(provider, $($arg),*).await,
            ResolvedProvider::FileSystem(provider) => Provider::$method(provider, $($arg),*).await,
            #[cfg(feature = "opds")]
            ResolvedProvider::Opds(provider) => Provider::$method(provider, $($arg),*).await,
            #[cfg(feature = "suwayomi")]
            ResolvedProvider::Suwayomi(provider) => Provider::$method(provider, $($arg),*).await,
        }
    };
}

impl Provider for ResolvedProvider {
    async fn list_collections(&self) -> Result<Vec<Collection>, String> {
        dispatch!(self, list_collections())
    }

    async fn list_items(
        &self,
        collection_id: &str,
        page: Option<u32>,
        page_size: Option<u32>,
    ) -> Result<Vec<CollectionItem>, String> {
        dispatch!(self, list_items(collection_id, page, page_size))
    }

    async fn import_files(&self, collection_id: &str, files: &[PathBuf]) -> Result<usize, String> {
        dispatch!(self, import_files(collection_id, files))
    }

    async fn list_series_entries(&self, series_id: &str) -> Result<Vec<SeriesEntry>, String> {
        dispatch!(self, list_series_entries(series_id))
    }

    async fn cover(&self, integration_id: &str, series_id: &str) -> Result<Vec<u8>, String> {
        dispatch!(self, cover(integration_id, series_id))
    }

    async fn volume_cover(&self, integration_id: &str, volume_id: &str) -> Result<Vec<u8>, String> {
        dispatch!(self, volume_cover(integration_id, volume_id))
    }

    async fn chapter_cover(
        &self,
        integration_id: &str,
        chapter_id: &str,
    ) -> Result<Vec<u8>, String> {
        dispatch!(self, chapter_cover(integration_id, chapter_id))
    }

    async fn reader_pages(&self, entry: &SeriesEntry) -> Result<Vec<PageRef>, String> {
        dispatch!(self, reader_pages(entry))
    }

    async fn reader_image(&self, chapter_id: &str, page: u16) -> Result<Vec<u8>, String> {
        dispatch!(self, reader_image(chapter_id, page))
    }

    async fn update_reading_progress(&self, progress: &ReadingProgress) -> Result<(), String> {
        dispatch!(self, update_reading_progress(progress))
    }

    async fn next_entry(
        &self,
        series_id: &str,
        current: &SeriesEntry,
    ) -> Result<Option<SeriesEntry>, String> {
        dispatch!(self, next_entry(series_id, current))
    }

    async fn previous_entry(
        &self,
        series_id: &str,
        current: &SeriesEntry,
    ) -> Result<Option<SeriesEntry>, String> {
        dispatch!(self, previous_entry(series_id, current))
    }
}

/// Resolves the provider for the source (integration) `integration_id`, plus
/// the cache namespace to key its covers/pages under, which is the
/// integration's id (a UUID, so always a safe cache-key segment).
pub async fn provider_for_integration(
    integration_id: &str,
) -> Result<(ResolvedProvider, String), String> {
    let integration = integration_repository::find(integration_id).await?;
    let provider = resolve(integration).await?;
    Ok((provider, integration_id.to_string()))
}

/// Connects (authenticating where the backend needs it) the provider for
/// `integration`. The `match` on the integration type is the single place new
/// backends are wired.
pub async fn resolve(integration: Integration) -> Result<ResolvedProvider, String> {
    let missing_url = || "integration is missing a url".to_string();

    match integration.integration_type {
        IntegrationType::LocalFolder => {
            let folder = integration
                .folder
                .ok_or_else(|| "local folder source has no folder to read from".to_string())?;
            Ok(ResolvedProvider::FileSystem(FileSystemProvider::new(
                folder,
                integration.name,
            )))
        }
        #[cfg(feature = "kavita")]
        IntegrationType::Kavita => {
            let url = integration.url.ok_or_else(missing_url)?;
            let api_key = integration
                .api_key
                .ok_or_else(|| "integration is missing an api key".to_string())?;

            let provider = KavitaProvider::authenticate(url, api_key).await?;
            Ok(ResolvedProvider::Kavita(provider))
        }
        #[cfg(feature = "opds")]
        IntegrationType::Opds => {
            let url = integration.url.ok_or_else(missing_url)?;
            let provider = OpdsProvider::connect(url, integration.username, integration.password);
            Ok(ResolvedProvider::Opds(provider))
        }
        #[cfg(feature = "komga")]
        IntegrationType::Komga => {
            let url = integration.url.ok_or_else(missing_url)?;
            let provider = KomgaProvider::connect(
                url,
                integration.api_key,
                integration.username,
                integration.password,
            )?;
            Ok(ResolvedProvider::Komga(provider))
        }
        #[cfg(feature = "suwayomi")]
        IntegrationType::Suwayomi => {
            let url = integration.url.ok_or_else(missing_url)?;
            let provider =
                SuwayomiProvider::connect(url, integration.username, integration.password)?;
            Ok(ResolvedProvider::Suwayomi(provider))
        }
        #[allow(unreachable_patterns)]
        integration_type => Err(not_built_in(integration_type)),
    }
}

/// The error for an integration whose provider was left out of this build by
/// its cargo feature (e.g. a database shared with a build that had it).
pub fn not_built_in(integration_type: IntegrationType) -> String {
    format!(
        "this build of Komasabi doesn't include {} support",
        integration_type.label()
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::series_entry::EntryKind;

    fn entry(kind: EntryKind, id: &str) -> SeriesEntry {
        SeriesEntry {
            external_id: id.to_string(),
            kind,
            label: format!("{kind:?} {id}"),
            pages: None,
            read_state: Default::default(),
        }
    }

    #[test]
    fn paginate_slices_by_page_and_size() {
        let items: Vec<u32> = (0..5).collect();

        assert_eq!(paginate(items.clone(), Some(2), Some(2)), vec![2, 3]);
        assert_eq!(paginate(items.clone(), Some(3), Some(2)), vec![4]);
        assert!(paginate(items.clone(), Some(9), Some(2)).is_empty());
        assert_eq!(paginate(items, None, Some(2)).len(), 5);
    }

    #[test]
    fn entry_after_is_the_following_entry_of_the_same_kind_and_id() {
        let entries = vec![
            entry(EntryKind::Volume, "1"),
            entry(EntryKind::Chapter, "1"),
            entry(EntryKind::Chapter, "2"),
        ];

        // A volume and a chapter can share an id; the kind tells them apart.
        assert_eq!(
            entry_after(entries.clone(), &entry(EntryKind::Volume, "1")),
            Some(entry(EntryKind::Chapter, "1"))
        );
        assert_eq!(
            entry_after(entries.clone(), &entry(EntryKind::Chapter, "1")),
            Some(entry(EntryKind::Chapter, "2"))
        );
        assert_eq!(
            entry_after(entries.clone(), &entry(EntryKind::Chapter, "2")),
            None
        );
        assert_eq!(entry_after(entries, &entry(EntryKind::Chapter, "9")), None);
    }

    #[test]
    fn entry_before_is_the_preceding_entry_of_the_same_kind_and_id() {
        let entries = vec![
            entry(EntryKind::Volume, "1"),
            entry(EntryKind::Chapter, "1"),
            entry(EntryKind::Chapter, "2"),
        ];

        assert_eq!(
            entry_before(entries.clone(), &entry(EntryKind::Chapter, "1")),
            Some(entry(EntryKind::Volume, "1"))
        );
        assert_eq!(
            entry_before(entries.clone(), &entry(EntryKind::Chapter, "2")),
            Some(entry(EntryKind::Chapter, "1"))
        );
        assert_eq!(
            entry_before(entries.clone(), &entry(EntryKind::Volume, "1")),
            None
        );
        assert_eq!(entry_before(entries, &entry(EntryKind::Chapter, "9")), None);
    }
}
