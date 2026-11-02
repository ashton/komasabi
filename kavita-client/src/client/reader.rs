use crate::client::KavitaClient;
use crate::models::reader::{ChapterInfo, Progress};

impl KavitaClient {
    /// Fetches the raw bytes of a single reader page. `page` is 0-based and must
    /// be less than the chapter's `pages` count.
    ///
    /// Unlike the other endpoints, this one requires `api_key` as a query
    /// parameter: Kavita's `/api/Reader/image` is built to be used as an
    /// `<img src>` URL, so it authorizes on the key in the query rather than the
    /// `Authorization` header (omitting it yields a 400, not a 401). The bearer
    /// header is still sent, it's just not what this endpoint checks.
    pub async fn reader_image(
        &self,
        chapter_id: u16,
        page: u16,
        api_key: &str,
    ) -> Result<Vec<u8>, reqwest::Error> {
        let response = self
            .execute(
                self.http_client
                    .get(format!("{}/api/Reader/image", self.base_url))
                    .query(&[("chapterId", chapter_id), ("page", page)])
                    .query(&[("apiKey", api_key)]),
            )
            .await?;

        Ok(response.error_for_status()?.bytes().await?.to_vec())
    }

    /// Looks up a chapter's place in the library (its volume, series and library
    /// ids) and its page count, which [`save_progress`](Self::save_progress)
    /// needs alongside the chapter id.
    pub async fn chapter_info(&self, chapter_id: u16) -> Result<ChapterInfo, reqwest::Error> {
        self.execute_json(
            self.http_client
                .get(format!("{}/api/Reader/chapter-info", self.base_url))
                .query(&[("chapterId", chapter_id)]),
        )
        .await
    }

    /// Records the user's reading progress in a chapter. Kavita keeps one
    /// progress row per user and chapter, so each call overwrites the last.
    pub async fn save_progress(&self, progress: &Progress) -> Result<(), reqwest::Error> {
        self.execute(
            self.http_client
                .post(format!("{}/api/Reader/progress", self.base_url))
                .json(progress),
        )
        .await?
        .error_for_status()?;

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use reqwest::Client;
    use wiremock::matchers::{body_json, method, path, query_param};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    fn client_for(mock_server: &MockServer) -> KavitaClient {
        KavitaClient::new(mock_server.uri(), Client::new())
    }

    #[tokio::test]
    async fn reader_image_fetches_raw_page_bytes() {
        let mock_server = MockServer::start().await;
        let image_bytes: &[u8] = &[0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A];

        Mock::given(method("GET"))
            .and(path("/api/Reader/image"))
            .and(query_param("chapterId", "42"))
            .and(query_param("page", "3"))
            .and(query_param("apiKey", "secret-key"))
            .respond_with(ResponseTemplate::new(200).set_body_raw(image_bytes, "image/png"))
            .mount(&mock_server)
            .await;

        let client = client_for(&mock_server);
        let bytes = client.reader_image(42, 3, "secret-key").await.unwrap();

        assert_eq!(bytes, image_bytes);
    }

    #[tokio::test]
    async fn reader_image_errors_on_non_success_status() {
        let mock_server = MockServer::start().await;

        Mock::given(method("GET"))
            .and(path("/api/Reader/image"))
            .respond_with(ResponseTemplate::new(404))
            .mount(&mock_server)
            .await;

        let client = client_for(&mock_server);
        let result = client.reader_image(42, 0, "secret-key").await;

        assert!(result.is_err());
    }

    #[tokio::test]
    async fn chapter_info_decodes_the_chapter_ancestry() {
        let mock_server = MockServer::start().await;

        Mock::given(method("GET"))
            .and(path("/api/Reader/chapter-info"))
            .and(query_param("chapterId", "42"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "chapterNumber": "3",
                "volumeNumber": "1",
                "volumeId": 7,
                "seriesName": "Some Series",
                "seriesFormat": 1,
                "seriesId": 9,
                "libraryId": 2,
                "libraryType": 0,
                "chapterTitle": "",
                "pages": 24,
                "fileName": "ch3.cbz",
                "isSpecial": false
            })))
            .mount(&mock_server)
            .await;

        let client = client_for(&mock_server);
        let info = client.chapter_info(42).await.unwrap();

        assert_eq!(
            info,
            ChapterInfo {
                volume_id: 7,
                series_id: 9,
                library_id: 2,
                pages: 24,
            }
        );
    }

    #[tokio::test]
    async fn save_progress_posts_the_progress_body() {
        let mock_server = MockServer::start().await;

        Mock::given(method("POST"))
            .and(path("/api/Reader/progress"))
            .and(body_json(serde_json::json!({
                "volumeId": 7,
                "chapterId": 42,
                "pageNum": 5,
                "seriesId": 9,
                "libraryId": 2
            })))
            .respond_with(ResponseTemplate::new(200).set_body_json(true))
            .expect(1)
            .mount(&mock_server)
            .await;

        let client = client_for(&mock_server);
        client
            .save_progress(&Progress {
                volume_id: 7,
                chapter_id: 42,
                page_num: 5,
                series_id: 9,
                library_id: 2,
            })
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn save_progress_errors_on_non_success_status() {
        let mock_server = MockServer::start().await;

        Mock::given(method("POST"))
            .and(path("/api/Reader/progress"))
            .respond_with(ResponseTemplate::new(401))
            .mount(&mock_server)
            .await;

        let client = client_for(&mock_server);
        let result = client
            .save_progress(&Progress {
                volume_id: 7,
                chapter_id: 42,
                page_num: 5,
                series_id: 9,
                library_id: 2,
            })
            .await;

        assert!(result.is_err());
    }
}
