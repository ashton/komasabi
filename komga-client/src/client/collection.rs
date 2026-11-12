use super::KomgaClient;
use crate::models::{collection::Collection, page::Page, series::Series};

impl KomgaClient {
    /// Lists every collection the authenticated user can access (unpaged).
    pub async fn list_collections(&self) -> Result<Vec<Collection>, reqwest::Error> {
        let page: Page<Collection> = self
            .execute_json(
                self.get("/api/v1/collections")
                    .query(&[("unpaged", "true")]),
            )
            .await?;

        Ok(page.content)
    }

    /// Lists one page of a collection's series, in the collection's order.
    /// `page` is 0-based, as Komga numbers them.
    pub async fn list_series_of_collection(
        &self,
        collection_id: &str,
        page: u32,
        size: u32,
    ) -> Result<Page<Series>, reqwest::Error> {
        self.execute_json(
            self.get(&format!("/api/v1/collections/{collection_id}/series"))
                .query(&[("page", page.to_string()), ("size", size.to_string())]),
        )
        .await
    }
}

#[cfg(test)]
mod tests {
    use crate::client::test_support::client_for;
    use wiremock::matchers::{method, path, query_param};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    #[tokio::test]
    async fn list_collections_parses_the_unpaged_response() {
        let mock_server = MockServer::start().await;

        Mock::given(method("GET"))
            .and(path("/api/v1/collections"))
            .and(query_param("unpaged", "true"))
            .respond_with(ResponseTemplate::new(200).set_body_raw(
                r#"{"content": [{"id": "C1", "name": "Classics", "ordered": false,
                    "seriesIds": ["S1", "S2"], "filtered": false}],
                    "number": 0, "size": 1, "totalPages": 1, "totalElements": 1,
                    "first": true, "last": true, "empty": false}"#,
                "application/json",
            ))
            .mount(&mock_server)
            .await;

        let collections = client_for(&mock_server).list_collections().await.unwrap();

        assert_eq!(collections.len(), 1);
        assert_eq!(collections[0].id, "C1");
        assert_eq!(collections[0].name, "Classics");
        assert_eq!(collections[0].series_ids, vec!["S1", "S2"]);
    }

    #[tokio::test]
    async fn list_series_of_collection_sends_paging() {
        let mock_server = MockServer::start().await;

        Mock::given(method("GET"))
            .and(path("/api/v1/collections/C1/series"))
            .and(query_param("page", "1"))
            .and(query_param("size", "20"))
            .respond_with(ResponseTemplate::new(200).set_body_raw(
                r#"{"content": [{"id": "S1", "name": "Alpha"}], "number": 1, "last": true}"#,
                "application/json",
            ))
            .mount(&mock_server)
            .await;

        let page = client_for(&mock_server)
            .list_series_of_collection("C1", 1, 20)
            .await
            .unwrap();

        assert_eq!(page.content.len(), 1);
        assert_eq!(page.content[0].id, "S1");
    }
}
