use crate::models::integration::IntegrationType;
use crate::models::series_entry::SeriesEntry;
use crate::repository::{integration as integration_repository, library as library_repository};

pub mod kavita;

use kavita::provider::KavitaProvider;

/// One page to fetch from a provider: the chapter it belongs to and its 0-based
/// index within that chapter. A provider flattens a [`SeriesEntry`] (a volume or
/// a chapter) into an ordered list of these; the reader launcher then fetches
/// each one's bytes. It carries no provider-specific detail, so the launcher
/// stays backend-agnostic.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PageRef {
    pub chapter_id: u16,
    pub page: u16,
}

/// The backend-agnostic operations the app performs against a library's source.
/// Kavita is the first implementation; others plug in by adding a
/// [`ResolvedProvider`] variant. Uses native `async fn`, so it is not
/// `dyn`-compatible — callers hold a [`ResolvedProvider`] (an `impl Provider`),
/// not a trait object.
pub trait Provider {
    async fn list_series_entries(&self, series_id: u16) -> Result<Vec<SeriesEntry>, String>;
    async fn cover(&self, integration_id: &str, series_id: u16) -> Result<Vec<u8>, String>;
    async fn volume_cover(&self, integration_id: &str, volume_id: u16) -> Result<Vec<u8>, String>;
    async fn chapter_cover(&self, integration_id: &str, chapter_id: u16)
    -> Result<Vec<u8>, String>;
    /// Resolves a volume/chapter entry into the ordered pages to read.
    async fn reader_pages(&self, entry: &SeriesEntry) -> Result<Vec<PageRef>, String>;
    /// Fetches the bytes of a single page.
    async fn reader_image(&self, chapter_id: u16, page: u16) -> Result<Vec<u8>, String>;
}

/// A provider resolved for a particular library, erasing the concrete backend
/// behind the [`Provider`] trait. One variant per supported integration type.
pub enum ResolvedProvider {
    Kavita(KavitaProvider),
}

impl Provider for ResolvedProvider {
    async fn list_series_entries(&self, series_id: u16) -> Result<Vec<SeriesEntry>, String> {
        match self {
            ResolvedProvider::Kavita(provider) => provider.list_series_entries(series_id).await,
        }
    }

    async fn cover(&self, integration_id: &str, series_id: u16) -> Result<Vec<u8>, String> {
        match self {
            ResolvedProvider::Kavita(provider) => provider.cover(integration_id, series_id).await,
        }
    }

    async fn volume_cover(&self, integration_id: &str, volume_id: u16) -> Result<Vec<u8>, String> {
        match self {
            ResolvedProvider::Kavita(provider) => {
                provider.volume_cover(integration_id, volume_id).await
            }
        }
    }

    async fn chapter_cover(
        &self,
        integration_id: &str,
        chapter_id: u16,
    ) -> Result<Vec<u8>, String> {
        match self {
            ResolvedProvider::Kavita(provider) => {
                provider.chapter_cover(integration_id, chapter_id).await
            }
        }
    }

    async fn reader_pages(&self, entry: &SeriesEntry) -> Result<Vec<PageRef>, String> {
        match self {
            ResolvedProvider::Kavita(provider) => provider.reader_pages(entry).await,
        }
    }

    async fn reader_image(&self, chapter_id: u16, page: u16) -> Result<Vec<u8>, String> {
        match self {
            ResolvedProvider::Kavita(provider) => provider.reader_image(chapter_id, page).await,
        }
    }
}

/// Resolves the provider (and its integration id) for the library with
/// `library_external_id`, authenticating against the stored credentials. The
/// `match` on the integration type is the single place new backends are wired.
pub async fn provider_for_library(
    library_external_id: &str,
) -> Result<(ResolvedProvider, String), String> {
    let library = library_repository::find_by_external_id(library_external_id).await?;
    let integration_id = library
        .integration_id
        .ok_or_else(|| "library has no associated integration".to_string())?;
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
        _ => Err("this integration type is not supported yet".to_string()),
    }
}
