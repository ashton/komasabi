use super::KomgaClient;
use crate::models::{book::Book, page::Page, read_list::ReadList};

impl KomgaClient {
    /// Lists every read list the authenticated user can access (unpaged).
    pub async fn list_read_lists(&self) -> Result<Vec<ReadList>, reqwest::Error> {
        let page: Page<ReadList> = self
            .execute_json(self.get("/api/v1/readlists").query(&[("unpaged", "true")]))
            .await?;

        Ok(page.content)
    }

    /// Lists every book of a read list (unpaged), in the list's order.
    pub async fn list_books_of_read_list(
        &self,
        read_list_id: &str,
    ) -> Result<Vec<Book>, reqwest::Error> {
        let page: Page<Book> = self
            .execute_json(
                self.get(&format!("/api/v1/readlists/{read_list_id}/books"))
                    .query(&[("unpaged", "true")]),
            )
            .await?;

        Ok(page.content)
    }
}

#[cfg(test)]
mod tests {
    use crate::client::test_support::client_for;
    use wiremock::matchers::{method, path, query_param};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    #[tokio::test]
    async fn list_read_lists_parses_the_unpaged_response() {
        let mock_server = MockServer::start().await;

        Mock::given(method("GET"))
            .and(path("/api/v1/readlists"))
            .and(query_param("unpaged", "true"))
            .respond_with(ResponseTemplate::new(200).set_body_raw(
                r#"{"content": [{"id": "R1", "name": "Event", "summary": "",
                    "ordered": true, "bookIds": ["B1"], "filtered": false}]}"#,
                "application/json",
            ))
            .mount(&mock_server)
            .await;

        let lists = client_for(&mock_server).list_read_lists().await.unwrap();

        assert_eq!(lists.len(), 1);
        assert_eq!(lists[0].id, "R1");
        assert_eq!(lists[0].name, "Event");
        assert_eq!(lists[0].book_ids, vec!["B1"]);
    }

    #[tokio::test]
    async fn list_books_of_read_list_parses_series_of_each_book() {
        let mock_server = MockServer::start().await;

        Mock::given(method("GET"))
            .and(path("/api/v1/readlists/R1/books"))
            .and(query_param("unpaged", "true"))
            .respond_with(ResponseTemplate::new(200).set_body_raw(
                r#"{"content": [{"id": "B1", "seriesId": "S1", "seriesTitle": "Alpha",
                    "libraryId": "L1", "name": "Alpha 1", "number": 1}]}"#,
                "application/json",
            ))
            .mount(&mock_server)
            .await;

        let books = client_for(&mock_server)
            .list_books_of_read_list("R1")
            .await
            .unwrap();

        assert_eq!(books.len(), 1);
        assert_eq!(books[0].series_id, "S1");
        assert_eq!(books[0].series_title, "Alpha");
    }
}
