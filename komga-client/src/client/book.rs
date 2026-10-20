use super::KomgaClient;
use crate::models::book::{Book, BookPage};

impl KomgaClient {
    pub async fn book(&self, book_id: &str) -> Result<Book, reqwest::Error> {
        self.execute_json(self.get(&format!("/api/v1/books/{book_id}")))
            .await
    }

    pub async fn book_thumbnail(&self, book_id: &str) -> Result<Vec<u8>, reqwest::Error> {
        self.execute_bytes(self.get(&format!("/api/v1/books/{book_id}/thumbnail")))
            .await
    }

    /// Lists a book's pages, with each page's 1-based number and media type.
    pub async fn book_pages(&self, book_id: &str) -> Result<Vec<BookPage>, reqwest::Error> {
        self.execute_json(self.get(&format!("/api/v1/books/{book_id}/pages")))
            .await
    }

    /// Fetches the image bytes of one page of a book. `number` is 1-based, as
    /// Komga numbers pages.
    pub async fn book_page(&self, book_id: &str, number: u32) -> Result<Vec<u8>, reqwest::Error> {
        self.execute_bytes(self.get(&format!("/api/v1/books/{book_id}/pages/{number}")))
            .await
    }
}

#[cfg(test)]
mod tests {
    use crate::client::test_support::client_for;
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    #[tokio::test]
    async fn book_parses_response() {
        let mock_server = MockServer::start().await;

        Mock::given(method("GET"))
            .and(path("/api/v1/books/B1"))
            .respond_with(ResponseTemplate::new(200).set_body_raw(
                r#"{"id": "B1", "seriesId": "S1", "libraryId": "L1", "name": "Alpha Saga v01",
                    "number": 1, "sizeBytes": 1024,
                    "media": {"status": "READY", "mediaType": "application/zip",
                              "pagesCount": 3, "mediaProfile": "DIVINA",
                              "epubDivinaCompatible": false},
                    "metadata": {"title": "Alpha Saga v01", "number": "1", "numberSort": 1.0},
                    "deleted": false}"#,
                "application/json",
            ))
            .mount(&mock_server)
            .await;

        let book = client_for(&mock_server).book("B1").await.unwrap();

        assert_eq!(book.series_id, "S1");
        assert_eq!(book.media.status, "READY");
        assert_eq!(book.media.pages_count, 3);
        assert_eq!(book.metadata.number, "1");
    }

    #[tokio::test]
    async fn book_pages_parses_response() {
        let mock_server = MockServer::start().await;

        Mock::given(method("GET"))
            .and(path("/api/v1/books/B1/pages"))
            .respond_with(ResponseTemplate::new(200).set_body_raw(
                r#"[{"number": 1, "fileName": "001.png", "mediaType": "image/png",
                     "width": 60, "height": 90, "sizeBytes": 120},
                    {"number": 2, "fileName": "002.png", "mediaType": "image/png"}]"#,
                "application/json",
            ))
            .mount(&mock_server)
            .await;

        let pages = client_for(&mock_server).book_pages("B1").await.unwrap();

        assert_eq!(pages.len(), 2);
        assert_eq!(pages[0].number, 1);
        assert_eq!(pages[0].width, Some(60));
        assert_eq!(pages[1].file_name, "002.png");
        assert_eq!(pages[1].width, None);
    }

    #[tokio::test]
    async fn book_page_fetches_the_one_based_page() {
        let mock_server = MockServer::start().await;
        let image_bytes: &[u8] = &[0x89, 0x50, 0x4E, 0x47];

        Mock::given(method("GET"))
            .and(path("/api/v1/books/B1/pages/3"))
            .respond_with(ResponseTemplate::new(200).set_body_raw(image_bytes, "image/png"))
            .mount(&mock_server)
            .await;

        let bytes = client_for(&mock_server).book_page("B1", 3).await.unwrap();

        assert_eq!(bytes, image_bytes);
    }

    #[tokio::test]
    async fn book_page_errors_on_non_success_status() {
        let mock_server = MockServer::start().await;

        Mock::given(method("GET"))
            .and(path("/api/v1/books/B1/pages/9"))
            .respond_with(ResponseTemplate::new(404))
            .mount(&mock_server)
            .await;

        assert!(client_for(&mock_server).book_page("B1", 9).await.is_err());
    }

    #[tokio::test]
    async fn book_thumbnail_fetches_raw_bytes() {
        let mock_server = MockServer::start().await;
        let image_bytes: &[u8] = &[0xFF, 0xD8, 0xFF, 0xE0];

        Mock::given(method("GET"))
            .and(path("/api/v1/books/B1/thumbnail"))
            .respond_with(ResponseTemplate::new(200).set_body_raw(image_bytes, "image/jpeg"))
            .mount(&mock_server)
            .await;

        let bytes = client_for(&mock_server).book_thumbnail("B1").await.unwrap();

        assert_eq!(bytes, image_bytes);
    }
}
