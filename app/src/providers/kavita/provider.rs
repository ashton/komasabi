use kavita_client::client::KavitaClient;
use reqwest::header::{AUTHORIZATION, HeaderMap, HeaderValue};

use crate::{
    adapter::{ItemAdapter, LibraryAdapter, VolumeAdapter},
    models::{library::Library, library_item::LibraryItem, volume::Volume},
    providers::kavita::adapter::{KavitaItemAdapter, KavitaLibraryAdapter, KavitaVolumeAdapter},
};

pub struct KavitaProvider {
    client: KavitaClient,
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

    pub async fn list_volumes(&self, series_id: u16) -> Result<Vec<Volume>, String> {
        let volumes = self
            .client
            .list_volumes_from_series(series_id)
            .await
            .map_err(crate::error::log_and_stringify)?;

        Ok(volumes
            .into_iter()
            .map(KavitaVolumeAdapter::adapt_volume)
            .collect())
    }

    /// Returns the cover image bytes for a volume, reading from the on-disk
    /// cache first and only hitting Kavita on a miss (writing the result back).
    /// `integration_id` namespaces the cache key so volume ids from different
    /// Kavita servers can't collide; the `vol-` segment keeps volume covers
    /// from colliding with series covers (which key on `{integration_id}-{id}`).
    pub async fn volume_cover(
        &self,
        integration_id: &str,
        volume_id: u16,
    ) -> Result<Vec<u8>, String> {
        let key = format!("{integration_id}-vol-{volume_id}");

        if let Some(bytes) = crate::cache::read_cover(&key) {
            return Ok(bytes);
        }

        let bytes = self
            .client
            .volume_cover(volume_id)
            .await
            .map_err(crate::error::log_and_stringify)?;

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
        let key = format!("{integration_id}-{series_id}");

        if let Some(bytes) = crate::cache::read_cover(&key) {
            return Ok(bytes);
        }

        let bytes = self
            .client
            .series_cover(series_id)
            .await
            .map_err(crate::error::log_and_stringify)?;

        if let Err(error) = crate::cache::write_cover(&key, &bytes) {
            tracing::debug!("failed to cache cover for {key}: {error}");
        }

        Ok(bytes)
    }
}
