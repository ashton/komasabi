use serde::Deserialize;
use serde_json::json;

use super::{Error, SuwayomiClient};
use crate::models::chapter::{Chapter, ChapterPages};

#[derive(Deserialize)]
struct ChaptersData {
    chapters: Nodes,
}

#[derive(Deserialize)]
struct Nodes {
    nodes: Vec<Chapter>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct FetchChapterPagesData {
    fetch_chapter_pages: ChapterPages,
}

const CHAPTER_FIELDS: &str =
    "id mangaId name chapterNumber sourceOrder pageCount isRead isDownloaded";

impl SuwayomiClient {
    /// Lists a manga's chapters in reading order (first chapter first).
    pub async fn list_chapters(&self, manga_id: i32) -> Result<Vec<Chapter>, Error> {
        let query = format!(
            "query ($mangaId: Int!) {{ chapters(condition: {{mangaId: $mangaId}}, \
             orderBy: SOURCE_ORDER, orderByType: ASC) {{ nodes {{ {CHAPTER_FIELDS} }} }} }}"
        );

        let data: ChaptersData = self.graphql(&query, json!({ "mangaId": manga_id })).await?;

        Ok(data.chapters.nodes)
    }

    /// Resolves a chapter's pages, having the server fetch the page list from
    /// the chapter's source when it isn't downloaded. This is what makes the
    /// page images servable, so it must run before [`Self::chapter_page`].
    pub async fn fetch_chapter_pages(&self, chapter_id: i32) -> Result<ChapterPages, Error> {
        let query = format!(
            "mutation ($chapterId: Int!) {{ fetchChapterPages(input: {{chapterId: $chapterId}}) \
             {{ pages chapter {{ {CHAPTER_FIELDS} }} }} }}"
        );

        let data: FetchChapterPagesData = self
            .graphql(&query, json!({ "chapterId": chapter_id }))
            .await?;

        Ok(data.fetch_chapter_pages)
    }

    /// Fetches the image bytes of one page of a chapter. The chapter is
    /// addressed by its manga and its [`Chapter::source_order`]; `page` is
    /// 0-based.
    pub async fn chapter_page(
        &self,
        manga_id: i32,
        source_order: i32,
        page: u32,
    ) -> Result<Vec<u8>, Error> {
        self.execute_bytes(self.get(&format!(
            "/api/v1/manga/{manga_id}/chapter/{source_order}/page/{page}"
        )))
        .await
    }
}

#[cfg(test)]
mod tests {
    use crate::client::test_support::{client_for, data};
    use wiremock::matchers::{body_partial_json, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    const CHAPTER_JSON: &str = r#"{"id": 45, "mangaId": 25, "name": "Ch. 1",
        "chapterNumber": 1.0, "sourceOrder": 1, "pageCount": 30,
        "isRead": false, "isDownloaded": true}"#;

    #[tokio::test]
    async fn list_chapters_sends_the_manga_id_and_parses_response() {
        let mock_server = MockServer::start().await;

        Mock::given(method("POST"))
            .and(path("/api/graphql"))
            .and(body_partial_json(
                serde_json::json!({"variables": {"mangaId": 25}}),
            ))
            .respond_with(ResponseTemplate::new(200).set_body_raw(
                data(&format!(r#"{{"chapters": {{"nodes": [{CHAPTER_JSON}]}}}}"#)),
                "application/json",
            ))
            .expect(1)
            .mount(&mock_server)
            .await;

        let chapters = client_for(&mock_server).list_chapters(25).await.unwrap();

        assert_eq!(chapters.len(), 1);
        assert_eq!(chapters[0].id, 45);
        assert_eq!(chapters[0].manga_id, 25);
        assert_eq!(chapters[0].name, "Ch. 1");
        assert_eq!(chapters[0].source_order, 1);
        assert_eq!(chapters[0].page_count, 30);
        assert!(chapters[0].is_downloaded);
    }

    #[tokio::test]
    async fn fetch_chapter_pages_sends_the_chapter_id_and_parses_response() {
        let mock_server = MockServer::start().await;

        Mock::given(method("POST"))
            .and(path("/api/graphql"))
            .and(body_partial_json(
                serde_json::json!({"variables": {"chapterId": 45}}),
            ))
            .respond_with(ResponseTemplate::new(200).set_body_raw(
                data(&format!(
                    r#"{{"fetchChapterPages": {{
                        "pages": ["/api/v1/manga/25/chapter/1/page/0",
                                  "/api/v1/manga/25/chapter/1/page/1"],
                        "chapter": {CHAPTER_JSON}}}}}"#
                )),
                "application/json",
            ))
            .expect(1)
            .mount(&mock_server)
            .await;

        let pages = client_for(&mock_server)
            .fetch_chapter_pages(45)
            .await
            .unwrap();

        assert_eq!(pages.pages.len(), 2);
        assert_eq!(pages.chapter.source_order, 1);
        assert_eq!(pages.chapter.manga_id, 25);
    }

    #[tokio::test]
    async fn chapter_page_addresses_the_chapter_by_source_order() {
        let mock_server = MockServer::start().await;
        let image: &[u8] = &[0x89, 0x50, 0x4E, 0x47];

        Mock::given(method("GET"))
            .and(path("/api/v1/manga/25/chapter/1/page/0"))
            .respond_with(ResponseTemplate::new(200).set_body_raw(image, "image/png"))
            .mount(&mock_server)
            .await;

        assert_eq!(
            client_for(&mock_server)
                .chapter_page(25, 1, 0)
                .await
                .unwrap(),
            image
        );
    }
}
