use komga_client::client::{Auth, KomgaClient};

use crate::{
    adapter::{ItemAdapter, LibraryAdapter},
    models::{library::Library, library_item::LibraryItem, series_entry::SeriesEntry},
    providers::{
        PageRef, Provider,
        komga::adapter::{KomgaItemAdapter, KomgaLibraryAdapter, book_entry},
    },
};

/// Page size used when a caller lists items without asking for one, matching
/// the browse screen's.
const DEFAULT_PAGE_SIZE: u32 = 20;

/// A [`Provider`] over a Komga server. Komga's library → series → book
/// hierarchy maps directly onto the app's library → item → entry one; a book
/// is read page by page through Komga's page endpoint.
pub struct KomgaProvider {
    client: KomgaClient,
}

/// Picks how to authenticate: an API key when one is given, otherwise the
/// username (Komga's account email) and password.
fn auth(api_key: Option<String>, username: Option<String>, password: Option<String>) -> Auth {
    let non_blank = |value: Option<String>| value.filter(|value| !value.trim().is_empty());

    match (non_blank(api_key), non_blank(username)) {
        (Some(api_key), _) => Auth::ApiKey(api_key),
        (None, Some(username)) => Auth::Basic {
            username,
            password: password.unwrap_or_default(),
        },
        (None, None) => Auth::None,
    }
}

impl KomgaProvider {
    pub fn connect(
        url: String,
        api_key: Option<String>,
        username: Option<String>,
        password: Option<String>,
    ) -> Result<Self, String> {
        let client = KomgaClient::new(url, auth(api_key, username, password))
            .map_err(crate::error::log_and_stringify)?;

        Ok(Self { client })
    }

    /// Checks whether a Komga server is reachable at `url`, without
    /// authenticating.
    pub async fn ping(url: String) -> Result<(), String> {
        Self::connect(url, None, None, None)?
            .client
            .claim_status()
            .await
            .map(|_| ())
            .map_err(crate::error::log_and_stringify)
    }

    /// Checks whether the given credentials are accepted by the Komga server
    /// at `url`.
    pub async fn check_authentication(
        url: String,
        api_key: Option<String>,
        username: Option<String>,
        password: Option<String>,
    ) -> Result<(), String> {
        Self::connect(url, api_key, username, password)?
            .client
            .me()
            .await
            .map(|_| ())
            .map_err(crate::error::log_and_stringify)
    }

    pub async fn list_libraries(&self, integration_id: &str) -> Result<Vec<Library>, String> {
        let libraries = self
            .client
            .list_libraries()
            .await
            .map_err(crate::error::log_and_stringify)?;

        Ok(libraries
            .into_iter()
            .filter(|library| !library.unavailable)
            .map(|library| KomgaLibraryAdapter::adapt_library(library, integration_id))
            .collect())
    }

    /// Returns cover bytes from the on-disk cache for `key`, or awaits `fetch`
    /// on a miss and writes the result back.
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

    /// A book's thumbnail, cached under a `book-` key segment so it can't
    /// collide with a series cover.
    async fn book_cover(&self, integration_id: &str, book_id: &str) -> Result<Vec<u8>, String> {
        self.cached_cover(
            format!("{integration_id}-book-{book_id}"),
            self.client.book_thumbnail(book_id),
        )
        .await
    }
}

impl Provider for KomgaProvider {
    /// Lists one page of a library's series. The browse screen's pages are
    /// 1-based; Komga's are 0-based.
    async fn list_items(
        &self,
        library_external_id: &str,
        page: Option<u32>,
        page_size: Option<u32>,
    ) -> Result<Vec<LibraryItem>, String> {
        let page = page.unwrap_or(1).saturating_sub(1);
        let size = page_size.unwrap_or(DEFAULT_PAGE_SIZE);

        let series = self
            .client
            .list_series_of_library(library_external_id, page, size)
            .await
            .map_err(crate::error::log_and_stringify)?;

        Ok(series
            .content
            .into_iter()
            .filter(|series| !series.deleted)
            .map(KomgaItemAdapter::adapt_item)
            .collect())
    }

    async fn list_series_entries(&self, series_id: &str) -> Result<Vec<SeriesEntry>, String> {
        let books = self
            .client
            .list_books_of_series(series_id)
            .await
            .map_err(crate::error::log_and_stringify)?;

        Ok(books
            .into_iter()
            .filter(|book| !book.deleted)
            .map(book_entry)
            .collect())
    }

    async fn cover(&self, integration_id: &str, series_id: &str) -> Result<Vec<u8>, String> {
        self.cached_cover(
            format!("{integration_id}-{series_id}"),
            self.client.series_thumbnail(series_id),
        )
        .await
    }

    async fn volume_cover(&self, integration_id: &str, volume_id: &str) -> Result<Vec<u8>, String> {
        self.book_cover(integration_id, volume_id).await
    }

    async fn chapter_cover(
        &self,
        integration_id: &str,
        chapter_id: &str,
    ) -> Result<Vec<u8>, String> {
        self.book_cover(integration_id, chapter_id).await
    }

    /// A book's pages, refreshed from the server so the count reflects its
    /// current analysis. Only books Komga can serve as images (comic archives,
    /// PDFs, and image-only EPUBs) can be read.
    async fn reader_pages(&self, entry: &SeriesEntry) -> Result<Vec<PageRef>, String> {
        let book = self
            .client
            .book(&entry.external_id)
            .await
            .map_err(crate::error::log_and_stringify)?;

        if book.media.status != "READY" {
            return Err(format!(
                "Komga hasn't analyzed this book yet (status {})",
                book.media.status
            ));
        }

        if book.media.media_profile == "EPUB" && !book.media.epub_divina_compatible {
            return Err("this EPUB is a text book and can't be read page by page".to_string());
        }

        let pages = u16::try_from(book.media.pages_count)
            .map_err(|_| format!("book {} has too many pages", book.id))?;

        Ok((0..pages)
            .map(|page| PageRef {
                chapter_id: book.id.clone(),
                page,
            })
            .collect())
    }

    /// Fetches one page; `page` is 0-based, Komga's page numbers are 1-based.
    async fn reader_image(&self, chapter_id: &str, page: u16) -> Result<Vec<u8>, String> {
        self.client
            .book_page(chapter_id, u32::from(page) + 1)
            .await
            .map_err(crate::error::log_and_stringify)
    }
}

#[cfg(test)]
mod tests {
    use wiremock::matchers::{header, method, path, query_param};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    use super::*;
    use crate::models::series_entry::EntryKind;

    fn provider_for(server: &MockServer) -> KomgaProvider {
        KomgaProvider::connect(server.uri(), Some("key".to_string()), None, None).unwrap()
    }

    fn entry(id: &str) -> SeriesEntry {
        SeriesEntry {
            external_id: id.to_string(),
            kind: EntryKind::Chapter,
            label: "v01".to_string(),
            pages: None,
        }
    }

    fn book_json(status: &str, profile: &str, pages: u32) -> String {
        format!(
            r#"{{"id": "B1", "seriesId": "S1", "name": "v01", "number": 1,
                "media": {{"status": "{status}", "pagesCount": {pages},
                          "mediaProfile": "{profile}", "epubDivinaCompatible": false}},
                "metadata": {{"title": "v01", "number": "1", "numberSort": 1.0}}}}"#
        )
    }

    #[test]
    fn auth_prefers_the_api_key_then_basic_credentials() {
        assert_eq!(
            auth(Some("key".into()), Some("me".into()), Some("pw".into())),
            Auth::ApiKey("key".into())
        );
        assert_eq!(
            auth(Some(" ".into()), Some("me".into()), Some("pw".into())),
            Auth::Basic {
                username: "me".into(),
                password: "pw".into()
            }
        );
        assert_eq!(auth(None, None, None), Auth::None);
    }

    #[tokio::test]
    async fn list_items_maps_the_one_based_browse_page_to_komgas_zero_based_page() {
        let server = MockServer::start().await;

        Mock::given(method("GET"))
            .and(path("/api/v1/series"))
            .and(query_param("library_id", "L1"))
            .and(query_param("page", "1"))
            .and(query_param("size", "20"))
            .and(header("X-API-Key", "key"))
            .respond_with(ResponseTemplate::new(200).set_body_raw(
                r#"{"content": [
                    {"id": "S1", "libraryId": "L1", "name": "alpha",
                     "metadata": {"title": "Alpha Saga"}},
                    {"id": "S2", "libraryId": "L1", "name": "gone", "deleted": true}
                ], "number": 1, "size": 20}"#,
                "application/json",
            ))
            .expect(1)
            .mount(&server)
            .await;

        let items = Provider::list_items(&provider_for(&server), "L1", Some(2), Some(20))
            .await
            .unwrap();

        assert_eq!(items.len(), 1);
        assert_eq!(items[0].external_id, "S1");
        assert_eq!(items[0].name, "Alpha Saga");
    }

    #[tokio::test]
    async fn list_series_entries_lists_the_series_books() {
        let server = MockServer::start().await;

        Mock::given(method("GET"))
            .and(path("/api/v1/series/S1/books"))
            .respond_with(ResponseTemplate::new(200).set_body_raw(
                format!(r#"{{"content": [{}]}}"#, book_json("READY", "DIVINA", 3)),
                "application/json",
            ))
            .mount(&server)
            .await;

        let entries = Provider::list_series_entries(&provider_for(&server), "S1")
            .await
            .unwrap();

        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].external_id, "B1");
        assert_eq!(entries[0].pages, Some(3));
    }

    #[tokio::test]
    async fn reader_pages_expands_a_book_into_zero_based_pages() {
        let server = MockServer::start().await;

        Mock::given(method("GET"))
            .and(path("/api/v1/books/B1"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_raw(book_json("READY", "DIVINA", 3), "application/json"),
            )
            .mount(&server)
            .await;

        let pages = Provider::reader_pages(&provider_for(&server), &entry("B1"))
            .await
            .unwrap();

        assert_eq!(
            pages,
            (0..3)
                .map(|page| PageRef {
                    chapter_id: "B1".to_string(),
                    page,
                })
                .collect::<Vec<_>>()
        );
    }

    #[tokio::test]
    async fn reader_pages_refuses_unanalyzed_books_and_text_epubs() {
        let server = MockServer::start().await;

        Mock::given(method("GET"))
            .and(path("/api/v1/books/B1"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_raw(book_json("UNKNOWN", "DIVINA", 0), "application/json"),
            )
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/api/v1/books/B2"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_raw(book_json("READY", "EPUB", 12), "application/json"),
            )
            .mount(&server)
            .await;

        let provider = provider_for(&server);

        assert!(
            Provider::reader_pages(&provider, &entry("B1"))
                .await
                .is_err()
        );
        assert!(
            Provider::reader_pages(&provider, &entry("B2"))
                .await
                .is_err()
        );
    }

    #[tokio::test]
    async fn reader_image_requests_the_one_based_page() {
        let server = MockServer::start().await;
        let image: &[u8] = &[0x89, 0x50, 0x4E, 0x47];

        Mock::given(method("GET"))
            .and(path("/api/v1/books/B1/pages/1"))
            .respond_with(ResponseTemplate::new(200).set_body_raw(image, "image/png"))
            .mount(&server)
            .await;

        let bytes = Provider::reader_image(&provider_for(&server), "B1", 0)
            .await
            .unwrap();

        assert_eq!(bytes, image);
    }

    #[tokio::test]
    async fn list_libraries_skips_unavailable_libraries() {
        let server = MockServer::start().await;

        Mock::given(method("GET"))
            .and(path("/api/v1/libraries"))
            .respond_with(ResponseTemplate::new(200).set_body_raw(
                r#"[{"id": "L1", "name": "Manga", "unavailable": false},
                    {"id": "L2", "name": "Gone", "unavailable": true}]"#,
                "application/json",
            ))
            .mount(&server)
            .await;

        let libraries = provider_for(&server)
            .list_libraries("integration-id")
            .await
            .unwrap();

        assert_eq!(libraries.len(), 1);
        assert_eq!(libraries[0].external_id, Some("L1".to_string()));
    }

    /// Exercises the provider against a real Komga server:
    /// `KOMGA_LIVE_URL=<url> KOMGA_LIVE_API_KEY=<key> cargo test -p komasabi-app komga_live -- --ignored --nocapture`.
    #[tokio::test]
    #[ignore]
    async fn komga_live() {
        let url = std::env::var("KOMGA_LIVE_URL").expect("KOMGA_LIVE_URL");
        let api_key = std::env::var("KOMGA_LIVE_API_KEY").ok();

        KomgaProvider::ping(url.clone()).await.unwrap();
        KomgaProvider::check_authentication(url.clone(), api_key.clone(), None, None)
            .await
            .unwrap();

        let provider = KomgaProvider::connect(url, api_key, None, None).unwrap();
        let libraries = provider.list_libraries("live").await.unwrap();
        println!(
            "libraries: {:?}",
            libraries.iter().map(|l| &l.name).collect::<Vec<_>>()
        );

        let library_id = libraries[0].external_id.clone().unwrap();
        let items = Provider::list_items(&provider, &library_id, Some(1), Some(20))
            .await
            .unwrap();
        println!(
            "series: {:?}",
            items.iter().map(|i| &i.name).collect::<Vec<_>>()
        );

        let entries = Provider::list_series_entries(&provider, &items[0].external_id)
            .await
            .unwrap();
        println!("entries: {entries:?}");

        let pages = Provider::reader_pages(&provider, &entries[0])
            .await
            .unwrap();
        let image = Provider::reader_image(&provider, &pages[0].chapter_id, pages[0].page)
            .await
            .unwrap();
        println!("{} pages, first page is {} bytes", pages.len(), image.len());
        assert!(!image.is_empty());
    }
}
