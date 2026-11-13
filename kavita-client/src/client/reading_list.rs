use super::KavitaClient;
use crate::models::reading_list::{ReadingList, ReadingListItem};

impl KavitaClient {
    /// Lists the reading lists the user can see, promoted ones included. Kavita
    /// pages this endpoint, but without paging params it returns them all.
    pub async fn list_reading_lists(&self) -> Result<Vec<ReadingList>, reqwest::Error> {
        self.execute_json(
            self.http_client
                .post(format!("{}/api/ReadingList/lists", self.base_url))
                .query(&[("includePromoted", "true"), ("sortByLastModified", "false")]),
        )
        .await
    }

    /// Lists a reading list's chapters, in the list's order.
    pub async fn reading_list_items(
        &self,
        reading_list_id: u32,
    ) -> Result<Vec<ReadingListItem>, reqwest::Error> {
        self.execute_json(
            self.http_client
                .get(format!("{}/api/ReadingList/items", self.base_url))
                .query(&[("readingListId", reading_list_id)]),
        )
        .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use reqwest::Client;
    use wiremock::matchers::{method, path, query_param};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    fn client_for(mock_server: &MockServer) -> KavitaClient {
        KavitaClient::new(mock_server.uri(), Client::new())
    }

    #[tokio::test]
    async fn list_reading_lists_includes_promoted_lists() {
        let mock_server = MockServer::start().await;

        Mock::given(method("POST"))
            .and(path("/api/ReadingList/lists"))
            .and(query_param("includePromoted", "true"))
            .respond_with(ResponseTemplate::new(200).set_body_raw(
                r#"[{"id": 2, "title": "Crossover", "promoted": true, "itemCount": 3}]"#,
                "application/json",
            ))
            .mount(&mock_server)
            .await;

        let lists = client_for(&mock_server).list_reading_lists().await.unwrap();

        assert_eq!(lists.len(), 1);
        assert_eq!(lists[0].id, 2);
        assert_eq!(lists[0].title, "Crossover");
        assert!(lists[0].promoted);
    }

    #[tokio::test]
    async fn reading_list_items_parses_response() {
        let mock_server = MockServer::start().await;

        Mock::given(method("GET"))
            .and(path("/api/ReadingList/items"))
            .and(query_param("readingListId", "2"))
            .respond_with(ResponseTemplate::new(200).set_body_raw(
                r#"[{"id": 1, "order": 0, "chapterId": 30, "seriesId": 7,
                     "seriesName": "Saga", "volumeId": 9, "libraryId": 1,
                     "title": "Chapter 1", "pagesRead": 0, "pagesTotal": 24}]"#,
                "application/json",
            ))
            .mount(&mock_server)
            .await;

        let items = client_for(&mock_server)
            .reading_list_items(2)
            .await
            .unwrap();

        assert_eq!(items.len(), 1);
        assert_eq!(items[0].series_id, 7);
        assert_eq!(items[0].series_name, "Saga");
        assert_eq!(items[0].chapter_id, 30);
    }
}
