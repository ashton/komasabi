use crate::models::integration::IntegrationType;
use crate::models::library_item::LibraryItem;
use crate::models::series_entry::SeriesEntry;
use crate::repository::{integration as integration_repository, library as library_repository};

pub mod file_system;
pub mod kavita;
pub mod komga;
pub mod opds;

use file_system::FileSystemProvider;
use kavita::provider::KavitaProvider;
use komga::provider::KomgaProvider;
use opds::provider::OpdsProvider;

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

/// The backend-agnostic operations the app performs against a library's source.
/// Kavita is the first implementation; others plug in by adding a
/// [`ResolvedProvider`] variant. Uses native `async fn`, so it is not
/// `dyn`-compatible — callers hold a [`ResolvedProvider`] (an `impl Provider`),
/// not a trait object.
pub trait Provider {
    /// Lists a library's series as display items, one page at a time (1-based
    /// `page`), mirroring the browse screen's pagination.
    async fn list_items(
        &self,
        library_external_id: &str,
        page: Option<u32>,
        page_size: Option<u32>,
    ) -> Result<Vec<LibraryItem>, String>;
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
}

/// A provider resolved for a particular library, erasing the concrete backend
/// behind the [`Provider`] trait. One variant per supported backend.
pub enum ResolvedProvider {
    Kavita(KavitaProvider),
    Komga(KomgaProvider),
    FileSystem(FileSystemProvider),
    Opds(OpdsProvider),
}

impl Provider for ResolvedProvider {
    // Each arm uses fully-qualified trait-method syntax (`Provider::foo(p, …)`)
    // rather than `p.foo(…)`, because `KavitaProvider` has inherent methods of
    // the same names taking numeric (`u16`) ids that would otherwise shadow the
    // string-id trait methods and fail to type-check.
    async fn list_items(
        &self,
        library_external_id: &str,
        page: Option<u32>,
        page_size: Option<u32>,
    ) -> Result<Vec<LibraryItem>, String> {
        match self {
            ResolvedProvider::Kavita(provider) => {
                Provider::list_items(provider, library_external_id, page, page_size).await
            }
            ResolvedProvider::FileSystem(provider) => {
                Provider::list_items(provider, library_external_id, page, page_size).await
            }
            ResolvedProvider::Opds(provider) => {
                Provider::list_items(provider, library_external_id, page, page_size).await
            }
            ResolvedProvider::Komga(provider) => {
                Provider::list_items(provider, library_external_id, page, page_size).await
            }
        }
    }

    async fn list_series_entries(&self, series_id: &str) -> Result<Vec<SeriesEntry>, String> {
        match self {
            ResolvedProvider::Kavita(provider) => {
                Provider::list_series_entries(provider, series_id).await
            }
            ResolvedProvider::FileSystem(provider) => {
                Provider::list_series_entries(provider, series_id).await
            }
            ResolvedProvider::Opds(provider) => {
                Provider::list_series_entries(provider, series_id).await
            }
            ResolvedProvider::Komga(provider) => {
                Provider::list_series_entries(provider, series_id).await
            }
        }
    }

    async fn cover(&self, integration_id: &str, series_id: &str) -> Result<Vec<u8>, String> {
        match self {
            ResolvedProvider::Kavita(provider) => {
                Provider::cover(provider, integration_id, series_id).await
            }
            ResolvedProvider::FileSystem(provider) => {
                Provider::cover(provider, integration_id, series_id).await
            }
            ResolvedProvider::Opds(provider) => {
                Provider::cover(provider, integration_id, series_id).await
            }
            ResolvedProvider::Komga(provider) => {
                Provider::cover(provider, integration_id, series_id).await
            }
        }
    }

    async fn volume_cover(&self, integration_id: &str, volume_id: &str) -> Result<Vec<u8>, String> {
        match self {
            ResolvedProvider::Kavita(provider) => {
                Provider::volume_cover(provider, integration_id, volume_id).await
            }
            ResolvedProvider::FileSystem(provider) => {
                Provider::volume_cover(provider, integration_id, volume_id).await
            }
            ResolvedProvider::Opds(provider) => {
                Provider::volume_cover(provider, integration_id, volume_id).await
            }
            ResolvedProvider::Komga(provider) => {
                Provider::volume_cover(provider, integration_id, volume_id).await
            }
        }
    }

    async fn chapter_cover(
        &self,
        integration_id: &str,
        chapter_id: &str,
    ) -> Result<Vec<u8>, String> {
        match self {
            ResolvedProvider::Kavita(provider) => {
                Provider::chapter_cover(provider, integration_id, chapter_id).await
            }
            ResolvedProvider::FileSystem(provider) => {
                Provider::chapter_cover(provider, integration_id, chapter_id).await
            }
            ResolvedProvider::Opds(provider) => {
                Provider::chapter_cover(provider, integration_id, chapter_id).await
            }
            ResolvedProvider::Komga(provider) => {
                Provider::chapter_cover(provider, integration_id, chapter_id).await
            }
        }
    }

    async fn reader_pages(&self, entry: &SeriesEntry) -> Result<Vec<PageRef>, String> {
        match self {
            ResolvedProvider::Kavita(provider) => Provider::reader_pages(provider, entry).await,
            ResolvedProvider::FileSystem(provider) => Provider::reader_pages(provider, entry).await,
            ResolvedProvider::Opds(provider) => Provider::reader_pages(provider, entry).await,
            ResolvedProvider::Komga(provider) => Provider::reader_pages(provider, entry).await,
        }
    }

    async fn reader_image(&self, chapter_id: &str, page: u16) -> Result<Vec<u8>, String> {
        match self {
            ResolvedProvider::Kavita(provider) => {
                Provider::reader_image(provider, chapter_id, page).await
            }
            ResolvedProvider::FileSystem(provider) => {
                Provider::reader_image(provider, chapter_id, page).await
            }
            ResolvedProvider::Opds(provider) => {
                Provider::reader_image(provider, chapter_id, page).await
            }
            ResolvedProvider::Komga(provider) => {
                Provider::reader_image(provider, chapter_id, page).await
            }
        }
    }
}

/// Resolves the provider (and the cache namespace to key its covers/pages
/// under) for the library with `library_external_id`. A library tied to an
/// integration authenticates that backend; a local library (one with a `folder`
/// and no integration) resolves a [`FileSystemProvider`] over that path. The
/// `match` on the integration type is the single place new remote backends are
/// wired.
pub async fn provider_for_library(
    library_external_id: &str,
) -> Result<(ResolvedProvider, String), String> {
    let library = library_repository::find_by_external_id(library_external_id).await?;

    let Some(integration_id) = library.integration_id.clone() else {
        // No integration: a local, filesystem-backed library. Its own id is the
        // cache namespace (a UUID, so always a safe cache-key segment).
        let folder = library
            .folder
            .ok_or_else(|| "local library has no folder to read from".to_string())?;
        return Ok((
            ResolvedProvider::FileSystem(FileSystemProvider::new(folder)),
            library.id,
        ));
    };

    let integration = integration_repository::find(&integration_id).await?;

    match integration.integration_type {
        IntegrationType::Kavita => {
            let url = integration
                .url
                .ok_or_else(|| "integration is missing a url".to_string())?;
            let api_key = integration
                .api_key
                .ok_or_else(|| "integration is missing an api key".to_string())?;

            let provider = KavitaProvider::authenticate(url, api_key).await?;
            Ok((ResolvedProvider::Kavita(provider), integration_id))
        }
        IntegrationType::Opds => {
            let url = integration
                .url
                .ok_or_else(|| "integration is missing a url".to_string())?;

            let provider = OpdsProvider::connect(url, integration.username, integration.password);
            Ok((ResolvedProvider::Opds(provider), integration_id))
        }
        IntegrationType::Komga => {
            let url = integration
                .url
                .ok_or_else(|| "integration is missing a url".to_string())?;

            let provider = KomgaProvider::connect(
                url,
                integration.api_key,
                integration.username,
                integration.password,
            )?;
            Ok((ResolvedProvider::Komga(provider), integration_id))
        }
        _ => Err("this integration type is not supported yet".to_string()),
    }
}
