use kavita_client::client::KavitaClient;
use reqwest::header::{AUTHORIZATION, HeaderMap, HeaderValue};

use crate::{
    adapter::{ItemAdapter, LibraryAdapter},
    models::{
        library::Library,
        library_item::LibraryItem,
        series_entry::{EntryKind, SeriesEntry},
    },
    providers::{
        PageRef, Provider,
        kavita::adapter::{KavitaItemAdapter, KavitaLibraryAdapter, series_entries},
    },
};

pub struct KavitaProvider {
    client: KavitaClient,
    /// The Kavita API key, retained because the `/api/Reader/image` endpoint
    /// authorizes on it as a query parameter rather than the bearer header.
    api_key: String,
}

impl KavitaProvider {
    /// Exchanges the given API key for a JWT and builds a `KavitaProvider`
    /// that authenticates all subsequent requests with it.
    pub async fn authenticate(url: String, api_key: String) -> Result<Self, String> {
        let unauthenticated_client = KavitaClient::new(url.clone(), reqwest::Client::new());
        let user = unauthenticated_client
            .authenticate(&api_key, "Sabita")
            .await
            .map_err(crate::error::log_and_stringify)?;
        let token = user
            .token
            .ok_or_else(|| "kavita did not return an auth token".to_string())?;

        let mut headers = HeaderMap::new();
        let value = HeaderValue::from_str(&format!("Bearer {token}"))
            .map_err(crate::error::log_and_stringify)?;
        headers.insert(AUTHORIZATION, value);

        let http_client = reqwest::Client::builder()
            .default_headers(headers)
            .build()
            .map_err(crate::error::log_and_stringify)?;

        Ok(Self {
            client: KavitaClient::new(url, http_client),
            api_key,
        })
    }

    /// Checks whether a Kavita server at `url` is reachable, without
    /// requiring authentication.
    pub async fn ping(url: String) -> Result<(), String> {
        let client = KavitaClient::new(url, reqwest::Client::new());
        client
            .health()
            .await
            .map_err(crate::error::log_and_stringify)
    }

    /// Checks whether `api_key` is valid for the Kavita server at `url`.
    pub async fn check_authentication(url: String, api_key: String) -> Result<(), String> {
        Self::authenticate(url, api_key).await.map(|_| ())
    }

    pub async fn list_libraries(&self, integration_id: &str) -> Result<Vec<Library>, String> {
        let libraries = self
            .client
            .list_libraries()
            .await
            .map_err(crate::error::log_and_stringify)?;

        Ok(libraries
            .into_iter()
            .map(|library| KavitaLibraryAdapter::adapt_library(library, integration_id))
            .collect())
    }

    pub async fn list_series_of_library(
        &self,
        library_id: String,
        page_number: Option<u32>,
        page_size: Option<u32>,
    ) -> Result<Vec<LibraryItem>, String> {
        let series = self
            .client
            .list_series_of_library_paginated(library_id, page_number, page_size)
            .await
            .map_err(crate::error::log_and_stringify)?;

        Ok(series
            .into_iter()
            .map(KavitaItemAdapter::adapt_item)
            .collect())
    }

    /// Lists a series' contents as display entries (numbered volumes and/or
    /// loose chapters), using the same `/api/Series/volumes` call as before —
    /// Kavita nests each volume's chapters in that response. See
    /// [`series_entries`] for how volumes vs chapters are chosen.
    pub async fn list_series_entries(&self, series_id: u16) -> Result<Vec<SeriesEntry>, String> {
        let volumes = self
            .client
            .list_volumes_from_series(series_id)
            .await
            .map_err(crate::error::log_and_stringify)?;

        Ok(series_entries(volumes))
    }

    /// Resolves a series entry into the ordered pages to read. A chapter entry
    /// is its own pages; a volume entry is the concatenation of its chapters'
    /// pages, in `sort_order`. The volume's chapters come nested in the single
    /// `/api/Volume` response, so a volume needs no per-chapter round trips.
    pub async fn reader_pages(&self, entry: &SeriesEntry) -> Result<Vec<PageRef>, String> {
        let id = entry
            .external_id
            .parse::<u16>()
            .map_err(|_| format!("entry id {} is not a valid Kavita id", entry.external_id))?;

        match entry.kind {
            EntryKind::Chapter => {
                let chapter = self
                    .client
                    .chapter_detail(id)
                    .await
                    .map_err(crate::error::log_and_stringify)?;

                Ok(chapter_pages(chapter.id, chapter.pages))
            }
            EntryKind::Volume => {
                let volume = self
                    .client
                    .volume_detail(id)
                    .await
                    .map_err(crate::error::log_and_stringify)?;

                let mut chapters = volume.chapters;
                chapters.sort_by(|a, b| {
                    a.sort_order
                        .partial_cmp(&b.sort_order)
                        .unwrap_or(std::cmp::Ordering::Equal)
                });

                Ok(chapters
                    .into_iter()
                    .flat_map(|chapter| chapter_pages(chapter.id, chapter.pages))
                    .collect())
            }
        }
    }

    /// Fetches the bytes of a single reader page. Not cached here — the reader
    /// launcher persists downloaded pages to the on-disk page cache instead.
    pub async fn reader_image(&self, chapter_id: u16, page: u16) -> Result<Vec<u8>, String> {
        self.client
            .reader_image(chapter_id, page, &self.api_key)
            .await
            .map_err(crate::error::log_and_stringify)
    }

    /// Returns the cover image bytes for a volume, reading from the on-disk
    /// cache first and only hitting Kavita on a miss (writing the result back).
    /// `integration_id` namespaces the cache key; the `vol-` segment keeps
    /// volume covers from colliding with series covers (which key on
    /// `{integration_id}-{id}`) or chapter covers (`-ch-`).
    pub async fn volume_cover(
        &self,
        integration_id: &str,
        volume_id: u16,
    ) -> Result<Vec<u8>, String> {
        self.cached_cover(
            format!("{integration_id}-vol-{volume_id}"),
            self.client.volume_cover(volume_id),
        )
        .await
    }

    /// Returns the cover image bytes for a chapter, cached like
    /// [`volume_cover`](Self::volume_cover) but under a `ch-` key segment.
    pub async fn chapter_cover(
        &self,
        integration_id: &str,
        chapter_id: u16,
    ) -> Result<Vec<u8>, String> {
        self.cached_cover(
            format!("{integration_id}-ch-{chapter_id}"),
            self.client.chapter_cover(chapter_id),
        )
        .await
    }

    /// Shared cover-cache path: return the on-disk bytes for `key` on a hit,
    /// otherwise await `fetch`, write the result back, and return it.
    async fn cached_cover(
        &self,
        key: String,
        fetch: impl std::future::Future<Output = Result<Vec<u8>, reqwest::Error>>,
    ) -> Result<Vec<u8>, String> {
        if let Some(bytes) = crate::cache::read_cover(&key) {
            return Ok(bytes);
        }

        let bytes = fetch.await.map_err(crate::error::log_and_stringify)?;

        if let Err(error) = crate::cache::write_cover(&key, &bytes) {
            tracing::debug!("failed to cache cover for {key}: {error}");
        }

        Ok(bytes)
    }

    /// Returns the cover image bytes for a series, reading from the on-disk
    /// cache first and only hitting Kavita on a miss (writing the result back).
    /// `integration_id` namespaces the cache key so series ids from different
    /// Kavita servers can't collide.
    pub async fn cover(&self, integration_id: &str, series_id: u16) -> Result<Vec<u8>, String> {
        self.cached_cover(
            format!("{integration_id}-{series_id}"),
            self.client.series_cover(series_id),
        )
        .await
    }
}

/// Expands a chapter into its 0-based page refs. The chapter id is stringified
/// into the backend-opaque [`PageRef::chapter_id`]; the trait's `reader_image`
/// parses it back to a Kavita id.
fn chapter_pages(chapter_id: u16, pages: u16) -> Vec<PageRef> {
    (0..pages)
        .map(|page| PageRef {
            chapter_id: chapter_id.to_string(),
            page,
        })
        .collect()
}

/// Exposes `KavitaProvider`'s operations through the backend-agnostic
/// [`Provider`] trait. Each method defers to the inherent one of the same name
/// (inherent methods win name resolution, so there's no recursion), letting
/// [`ResolvedProvider`](crate::providers::ResolvedProvider) treat every backend
/// uniformly.
impl Provider for KavitaProvider {
    async fn list_items(
        &self,
        library_external_id: &str,
        page: Option<u32>,
        page_size: Option<u32>,
    ) -> Result<Vec<LibraryItem>, String> {
        self.list_series_of_library(library_external_id.to_string(), page, page_size)
            .await
    }

    async fn list_series_entries(&self, series_id: &str) -> Result<Vec<SeriesEntry>, String> {
        self.list_series_entries(parse_kavita_id(series_id)?).await
    }

    async fn cover(&self, integration_id: &str, series_id: &str) -> Result<Vec<u8>, String> {
        self.cover(integration_id, parse_kavita_id(series_id)?).await
    }

    async fn volume_cover(&self, integration_id: &str, volume_id: &str) -> Result<Vec<u8>, String> {
        self.volume_cover(integration_id, parse_kavita_id(volume_id)?)
            .await
    }

    async fn chapter_cover(
        &self,
        integration_id: &str,
        chapter_id: &str,
    ) -> Result<Vec<u8>, String> {
        self.chapter_cover(integration_id, parse_kavita_id(chapter_id)?)
            .await
    }

    async fn reader_pages(&self, entry: &SeriesEntry) -> Result<Vec<PageRef>, String> {
        self.reader_pages(entry).await
    }

    async fn reader_image(&self, chapter_id: &str, page: u16) -> Result<Vec<u8>, String> {
        self.reader_image(parse_kavita_id(chapter_id)?, page).await
    }
}

/// Parses a backend-agnostic string id back into the numeric id Kavita's API
/// uses, failing with a descriptive error rather than silently.
fn parse_kavita_id(id: &str) -> Result<u16, String> {
    id.parse::<u16>()
        .map_err(|_| format!("id {id} is not a valid Kavita id"))
}
