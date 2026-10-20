use super::KomgaClient;
use crate::models::{book::Book, page::Page, series::Series};

impl KomgaClient {
    /// Lists one page of a library's series, sorted by title. `page` is
    /// 0-based, as Komga numbers them.
    pub async fn list_series_of_library(
        &self,
        library_id: &str,
        page: u32,
        size: u32,
    ) -> Result<Page<Series>, reqwest::Error> {
        self.execute_json(self.get("/api/v1/series").query(&[
            ("library_id", library_id),
            ("page", &page.to_string()),
            ("size", &size.to_string()),
            ("sort", "metadata.titleSort,asc"),
        ]))
        .await
    }

    pub async fn series(&self, series_id: &str) -> Result<Series, reqwest::Error> {
        self.execute_json(self.get(&format!("/api/v1/series/{series_id}")))
            .await
    }

    /// Lists every book of a series (unpaged), in reading order.
    pub async fn list_books_of_series(&self, series_id: &str) -> Result<Vec<Book>, reqwest::Error> {
        let page: Page<Book> = self
            .execute_json(
                self.get(&format!("/api/v1/series/{series_id}/books"))
                    .query(&[("unpaged", "true"), ("sort", "metadata.numberSort,asc")]),
            )
            .await?;

        Ok(page.content)
    }

    pub async fn series_thumbnail(&self, series_id: &str) -> Result<Vec<u8>, reqwest::Error> {
        self.execute_bytes(self.get(&format!("/api/v1/series/{series_id}/thumbnail")))
            .await
    }
}

#[cfg(test)]
mod tests {
    use crate::client::test_support::client_for;
    use wiremock::matchers::{method, path, query_param};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    const SERIES_JSON: &str = r#"{
        "id": "0B8RAKD1N9Y4M",
        "libraryId": "0B8RAK3Q2Y1VX",
        "name": "Alpha Saga",
        "url": "/data/manga/Alpha Saga",
        "booksCount": 2,
        "booksReadCount": 0,
        "metadata": {
            "status": "ONGOING",
            "title": "Alpha Saga",
            "titleSort": "Alpha Saga",
            "readingDirection": "RIGHT_TO_LEFT",
            "genres": []
        },
        "deleted": false
    }"#;

    #[tokio::test]
    async fn list_series_of_library_sends_paging_and_parses_the_page() {
        let mock_server = MockServer::start().await;

        Mock::given(method("GET"))
            .and(path("/api/v1/series"))
            .and(query_param("library_id", "0B8RAK3Q2Y1VX"))
            .and(query_param("page", "2"))
            .and(query_param("size", "20"))
            .and(query_param("sort", "metadata.titleSort,asc"))
            .respond_with(ResponseTemplate::new(200).set_body_raw(
                format!(
                    r#"{{"content": [{SERIES_JSON}], "number": 2, "size": 20,
                        "totalPages": 3, "totalElements": 41,
                        "first": false, "last": false, "empty": false}}"#
                ),
                "application/json",
            ))
            .mount(&mock_server)
            .await;

        let page = client_for(&mock_server)
            .list_series_of_library("0B8RAK3Q2Y1VX", 2, 20)
            .await
            .unwrap();

        assert_eq!(page.number, 2);
        assert_eq!(page.total_elements, 41);
        assert_eq!(page.content.len(), 1);
        let series = &page.content[0];
        assert_eq!(series.id, "0B8RAKD1N9Y4M");
        assert_eq!(series.library_id, "0B8RAK3Q2Y1VX");
        assert_eq!(series.books_count, 2);
        assert_eq!(series.metadata.title, "Alpha Saga");
        assert_eq!(
            series.metadata.reading_direction.as_deref(),
            Some("RIGHT_TO_LEFT")
        );
    }

    #[tokio::test]
    async fn list_books_of_series_requests_every_book_in_number_order() {
        let mock_server = MockServer::start().await;

        Mock::given(method("GET"))
            .and(path("/api/v1/series/S1/books"))
            .and(query_param("unpaged", "true"))
            .and(query_param("sort", "metadata.numberSort,asc"))
            .respond_with(ResponseTemplate::new(200).set_body_raw(
                r#"{"content": [
                    {"id": "B1", "seriesId": "S1", "name": "v01", "number": 1,
                     "media": {"status": "READY", "mediaType": "application/zip",
                               "pagesCount": 3, "mediaProfile": "DIVINA"},
                     "metadata": {"title": "Volume 1", "number": "1", "numberSort": 1.0}},
                    {"id": "B2", "seriesId": "S1", "name": "v02", "number": 2,
                     "media": {"status": "READY", "pagesCount": 4, "mediaProfile": "DIVINA"},
                     "metadata": {"title": "Volume 2", "number": "2", "numberSort": 2.0}}
                ], "number": 0, "size": 2, "totalPages": 1, "totalElements": 2}"#,
                "application/json",
            ))
            .mount(&mock_server)
            .await;

        let books = client_for(&mock_server)
            .list_books_of_series("S1")
            .await
            .unwrap();

        assert_eq!(books.len(), 2);
        assert_eq!(books[0].id, "B1");
        assert_eq!(books[0].media.pages_count, 3);
        assert_eq!(books[0].media.media_profile, "DIVINA");
        assert_eq!(books[1].metadata.title, "Volume 2");
        assert_eq!(books[1].metadata.number_sort, 2.0);
    }

    #[tokio::test]
    async fn series_parses_response() {
        let mock_server = MockServer::start().await;

        Mock::given(method("GET"))
            .and(path("/api/v1/series/0B8RAKD1N9Y4M"))
            .respond_with(ResponseTemplate::new(200).set_body_raw(SERIES_JSON, "application/json"))
            .mount(&mock_server)
            .await;

        let series = client_for(&mock_server)
            .series("0B8RAKD1N9Y4M")
            .await
            .unwrap();

        assert_eq!(series.name, "Alpha Saga");
    }

    #[tokio::test]
    async fn series_thumbnail_fetches_raw_bytes() {
        let mock_server = MockServer::start().await;
        let image_bytes: &[u8] = &[0xFF, 0xD8, 0xFF, 0xE0];

        Mock::given(method("GET"))
            .and(path("/api/v1/series/S1/thumbnail"))
            .respond_with(ResponseTemplate::new(200).set_body_raw(image_bytes, "image/jpeg"))
            .mount(&mock_server)
            .await;

        let bytes = client_for(&mock_server)
            .series_thumbnail("S1")
            .await
            .unwrap();

        assert_eq!(bytes, image_bytes);
    }
}
