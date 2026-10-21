use suwayomi_client::client::{Auth, SuwayomiClient};

use crate::{
    adapter::{ItemAdapter, LibraryAdapter},
    models::{library::Library, library_item::LibraryItem, series_entry::SeriesEntry},
    providers::{
        PageRef, Provider,
        suwayomi::adapter::{
            SuwayomiItemAdapter, SuwayomiLibraryAdapter, category_id, chapter_entry,
            page_chapter_id, parse_chapter_external_id, parse_page_chapter_id,
        },
    },
};

/// Page size used when a caller lists items without asking for one, matching
/// the browse screen's.
const DEFAULT_PAGE_SIZE: u32 = 20;

/// A [`Provider`] over a Suwayomi server. Its categories are the libraries,
/// their mangas the items, and a manga's chapters its entries; a chapter is
/// read page by page through the server's page images.
pub struct SuwayomiProvider {
    client: SuwayomiClient,
}

/// Basic auth when a username is given, otherwise none (Suwayomi's default).
fn auth(username: Option<String>, password: Option<String>) -> Auth {
    match username.filter(|value| !value.trim().is_empty()) {
        Some(username) => Auth::Basic {
            username,
            password: password.unwrap_or_default(),
        },
        None => Auth::None,
    }
}

fn parse_manga_id(series_id: &str) -> Result<i32, String> {
    series_id
        .parse()
        .map_err(|_| format!("{series_id} is not a Suwayomi manga id"))
}

impl SuwayomiProvider {
    pub fn connect(
        url: String,
        username: Option<String>,
        password: Option<String>,
    ) -> Result<Self, String> {
        let client = SuwayomiClient::new(url, auth(username, password))
            .map_err(crate::error::log_and_stringify)?;

        Ok(Self { client })
    }

    /// Checks that a Suwayomi server answers at `url` with these credentials,
    /// returning its name and version (e.g. "Suwayomi-Server v2.3.2243").
    pub async fn check_server(
        url: String,
        username: Option<String>,
        password: Option<String>,
    ) -> Result<String, String> {
        let about = Self::connect(url, username, password)?
            .client
            .about()
            .await
            .map_err(crate::error::log_and_stringify)?;

        Ok(format!("{} {}", about.name, about.version)
            .trim()
            .to_string())
    }

    /// Lists the server's categories as libraries. The default category is
    /// skipped while it's empty (every manga is in another category), as
    /// Suwayomi's own UI does.
    pub async fn list_libraries(&self, integration_id: &str) -> Result<Vec<Library>, String> {
        let categories = self
            .client
            .list_categories()
            .await
            .map_err(crate::error::log_and_stringify)?;

        Ok(categories
            .into_iter()
            .filter(|category| !category.default || category.mangas.total_count > 0)
            .map(|category| SuwayomiLibraryAdapter::adapt_library(category, integration_id))
            .collect())
    }
}

impl Provider for SuwayomiProvider {
    /// Lists one page of a category's mangas. Suwayomi returns a category's
    /// mangas all at once, so the 1-based page is cut out of the full list.
    async fn list_items(
        &self,
        library_external_id: &str,
        page: Option<u32>,
        page_size: Option<u32>,
    ) -> Result<Vec<LibraryItem>, String> {
        let page = page.unwrap_or(1).saturating_sub(1) as usize;
        let size = page_size.unwrap_or(DEFAULT_PAGE_SIZE) as usize;

        let mangas = self
            .client
            .list_category_mangas(category_id(library_external_id)?)
            .await
            .map_err(crate::error::log_and_stringify)?;

        Ok(mangas
            .into_iter()
            .skip(page * size)
            .take(size)
            .map(|manga| SuwayomiItemAdapter::adapt_item((manga, library_external_id)))
            .collect())
    }

    async fn list_series_entries(&self, series_id: &str) -> Result<Vec<SeriesEntry>, String> {
        let chapters = self
            .client
            .list_chapters(parse_manga_id(series_id)?)
            .await
            .map_err(crate::error::log_and_stringify)?;

        Ok(chapters.into_iter().map(chapter_entry).collect())
    }

    async fn cover(&self, integration_id: &str, series_id: &str) -> Result<Vec<u8>, String> {
        let key = format!("{integration_id}-{series_id}");

        if let Some(bytes) = crate::cache::read_cover(&key) {
            return Ok(bytes);
        }

        let bytes = self
            .client
            .manga_thumbnail(parse_manga_id(series_id)?)
            .await
            .map_err(crate::error::log_and_stringify)?;

        if let Err(error) = crate::cache::write_cover(&key, &bytes) {
            tracing::debug!("failed to cache cover for {key}: {error}");
        }

        Ok(bytes)
    }

    async fn volume_cover(
        &self,
        _integration_id: &str,
        volume_id: &str,
    ) -> Result<Vec<u8>, String> {
        Err(format!("Suwayomi has no volumes (asked for {volume_id})"))
    }

    /// Suwayomi has no chapter images, so a chapter card shows its manga's
    /// cover — cached under the series' key, so it's fetched once per manga.
    async fn chapter_cover(
        &self,
        integration_id: &str,
        chapter_id: &str,
    ) -> Result<Vec<u8>, String> {
        let (manga_id, _) = parse_chapter_external_id(chapter_id)?;

        self.cover(integration_id, &manga_id.to_string()).await
    }

    /// Has the server resolve the chapter's pages (fetching them from the
    /// chapter's source unless it's downloaded), which also makes the page
    /// images servable.
    async fn reader_pages(&self, entry: &SeriesEntry) -> Result<Vec<PageRef>, String> {
        let (_, chapter_id) = parse_chapter_external_id(&entry.external_id)?;

        let pages = self
            .client
            .fetch_chapter_pages(chapter_id)
            .await
            .map_err(crate::error::log_and_stringify)?;

        let count = u16::try_from(pages.pages.len())
            .map_err(|_| format!("chapter {chapter_id} has too many pages"))?;
        let chapter = page_chapter_id(pages.chapter.manga_id, pages.chapter.source_order);

        Ok((0..count)
            .map(|page| PageRef {
                chapter_id: chapter.clone(),
                page,
            })
            .collect())
    }

    async fn reader_image(&self, chapter_id: &str, page: u16) -> Result<Vec<u8>, String> {
        let (manga_id, source_order) = parse_page_chapter_id(chapter_id)?;

        self.client
            .chapter_page(manga_id, source_order, u32::from(page))
            .await
            .map_err(crate::error::log_and_stringify)
    }
}

#[cfg(test)]
mod tests {
    use wiremock::matchers::{body_partial_json, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    use super::*;
    use crate::models::series_entry::EntryKind;

    fn provider_for(server: &MockServer) -> SuwayomiProvider {
        SuwayomiProvider::connect(server.uri(), None, None).unwrap()
    }

    fn graphql(data: &str) -> ResponseTemplate {
        ResponseTemplate::new(200)
            .set_body_raw(format!(r#"{{"data": {data}}}"#), "application/json")
    }

    fn mangas_json(count: i32) -> String {
        let nodes: Vec<String> = (1..=count)
            .map(|id| format!(r#"{{"id": {id}, "title": "Manga {id}", "inLibrary": true}}"#))
            .collect();
        format!(
            r#"{{"category": {{"mangas": {{"nodes": [{}]}}}}}}"#,
            nodes.join(",")
        )
    }

    #[test]
    fn auth_uses_basic_credentials_only_with_a_username() {
        assert_eq!(
            auth(Some("me".into()), Some("pw".into())),
            Auth::Basic {
                username: "me".into(),
                password: "pw".into()
            }
        );
        assert_eq!(auth(Some(" ".into()), Some("pw".into())), Auth::None);
        assert_eq!(auth(None, None), Auth::None);
    }

    #[tokio::test]
    async fn check_server_reports_the_server_name_and_version() {
        let server = MockServer::start().await;

        Mock::given(method("POST"))
            .and(path("/api/graphql"))
            .respond_with(graphql(
                r#"{"aboutServer": {"name": "Suwayomi-Server", "version": "v2.3.2243"}}"#,
            ))
            .mount(&server)
            .await;

        assert_eq!(
            SuwayomiProvider::check_server(server.uri(), None, None).await,
            Ok("Suwayomi-Server v2.3.2243".to_string())
        );
    }

    #[tokio::test]
    async fn list_libraries_skips_an_empty_default_category() {
        let server = MockServer::start().await;

        Mock::given(method("POST"))
            .and(path("/api/graphql"))
            .respond_with(graphql(
                r#"{"categories": {"nodes": [
                    {"id": 0, "name": "Default", "default": true, "mangas": {"totalCount": 0}},
                    {"id": 1, "name": "Reading", "default": false, "mangas": {"totalCount": 4}},
                    {"id": 2, "name": "Later", "default": false, "mangas": {"totalCount": 0}}
                ]}}"#,
            ))
            .mount(&server)
            .await;

        let libraries = provider_for(&server)
            .list_libraries("integration-id")
            .await
            .unwrap();

        assert_eq!(
            libraries
                .iter()
                .map(|library| library.external_id.clone().unwrap())
                .collect::<Vec<_>>(),
            vec!["integration-id:1", "integration-id:2"]
        );
    }

    #[tokio::test]
    async fn list_items_cuts_the_requested_page_out_of_the_category() {
        let server = MockServer::start().await;

        Mock::given(method("POST"))
            .and(path("/api/graphql"))
            .and(body_partial_json(
                serde_json::json!({"variables": {"id": 3}}),
            ))
            .respond_with(graphql(&mangas_json(5)))
            .mount(&server)
            .await;

        let provider = provider_for(&server);
        let second = Provider::list_items(&provider, "integration-id:3", Some(2), Some(2))
            .await
            .unwrap();
        let past_the_end = Provider::list_items(&provider, "integration-id:3", Some(4), Some(2))
            .await
            .unwrap();

        assert_eq!(
            second
                .iter()
                .map(|item| item.name.as_str())
                .collect::<Vec<_>>(),
            vec!["Manga 3", "Manga 4"]
        );
        assert_eq!(second[0].external_id, "3");
        assert_eq!(second[0].library_id, "integration-id:3");
        assert!(past_the_end.is_empty());
    }

    #[tokio::test]
    async fn list_series_entries_lists_the_manga_chapters() {
        let server = MockServer::start().await;

        Mock::given(method("POST"))
            .and(path("/api/graphql"))
            .and(body_partial_json(
                serde_json::json!({"variables": {"mangaId": 25}}),
            ))
            .respond_with(graphql(
                r#"{"chapters": {"nodes": [
                    {"id": 45, "mangaId": 25, "name": "Ch. 1", "chapterNumber": 1.0,
                     "sourceOrder": 1, "pageCount": -1}
                ]}}"#,
            ))
            .mount(&server)
            .await;

        let entries = Provider::list_series_entries(&provider_for(&server), "25")
            .await
            .unwrap();

        assert_eq!(
            entries,
            vec![SeriesEntry {
                external_id: "25-45".to_string(),
                kind: EntryKind::Chapter,
                label: "Ch. 1".to_string(),
                pages: None,
            }]
        );
    }

    #[tokio::test]
    async fn reader_pages_addresses_pages_by_manga_and_source_order() {
        let server = MockServer::start().await;

        Mock::given(method("POST"))
            .and(path("/api/graphql"))
            .and(body_partial_json(
                serde_json::json!({"variables": {"chapterId": 45}}),
            ))
            .respond_with(graphql(
                r#"{"fetchChapterPages": {
                    "pages": ["/api/v1/manga/25/chapter/1/page/0",
                              "/api/v1/manga/25/chapter/1/page/1"],
                    "chapter": {"id": 45, "mangaId": 25, "sourceOrder": 1, "pageCount": 2}
                }}"#,
            ))
            .expect(1)
            .mount(&server)
            .await;

        let entry = SeriesEntry {
            external_id: "25-45".to_string(),
            kind: EntryKind::Chapter,
            label: "Ch. 1".to_string(),
            pages: None,
        };

        let pages = Provider::reader_pages(&provider_for(&server), &entry)
            .await
            .unwrap();

        assert_eq!(
            pages,
            (0..2)
                .map(|page| PageRef {
                    chapter_id: "25-1".to_string(),
                    page,
                })
                .collect::<Vec<_>>()
        );
    }

    #[tokio::test]
    async fn reader_image_fetches_the_zero_based_page() {
        let server = MockServer::start().await;
        let image: &[u8] = &[0xFF, 0xD8, 0xFF];

        Mock::given(method("GET"))
            .and(path("/api/v1/manga/25/chapter/1/page/0"))
            .respond_with(ResponseTemplate::new(200).set_body_raw(image, "image/jpeg"))
            .mount(&server)
            .await;

        let bytes = Provider::reader_image(&provider_for(&server), "25-1", 0)
            .await
            .unwrap();

        assert_eq!(bytes, image);
    }

    #[tokio::test]
    async fn volume_cover_is_refused() {
        let server = MockServer::start().await;

        assert!(
            Provider::volume_cover(&provider_for(&server), "integration-id", "1")
                .await
                .is_err()
        );
    }

    /// Exercises the provider against a real Suwayomi server:
    /// `SUWAYOMI_LIVE_URL=<url> cargo test -p komasabi suwayomi_live -- --ignored --nocapture`.
    #[tokio::test]
    #[ignore]
    async fn suwayomi_live() {
        let url = std::env::var("SUWAYOMI_LIVE_URL").expect("SUWAYOMI_LIVE_URL");
        let username = std::env::var("SUWAYOMI_LIVE_USERNAME").ok();
        let password = std::env::var("SUWAYOMI_LIVE_PASSWORD").ok();

        println!(
            "server: {}",
            SuwayomiProvider::check_server(url.clone(), username.clone(), password.clone())
                .await
                .unwrap()
        );

        let provider = SuwayomiProvider::connect(url, username, password).unwrap();
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
            "mangas: {:?}",
            items.iter().map(|i| &i.name).collect::<Vec<_>>()
        );

        let entries = Provider::list_series_entries(&provider, &items[0].external_id)
            .await
            .unwrap();
        println!("{} chapters, first: {:?}", entries.len(), entries.first());

        let cover = Provider::cover(&provider, "live", &items[0].external_id)
            .await
            .unwrap();
        println!("cover is {} bytes", cover.len());

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
