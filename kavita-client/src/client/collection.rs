use super::KavitaClient;
use crate::models::collection::KavitaCollection;

impl KavitaClient {
    /// Lists the collections the user can see: their own and those promoted by
    /// other users.
    pub async fn list_collections(&self) -> Result<Vec<KavitaCollection>, reqwest::Error> {
        self.execute_json(
            self.http_client
                .get(format!("{}/api/Collection", self.base_url))
                .query(&[("ownedOnly", "false")]),
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

    #[tokio::test]
    async fn list_collections_parses_response() {
        let mock_server = MockServer::start().await;

        Mock::given(method("GET"))
            .and(path("/api/Collection"))
            .and(query_param("ownedOnly", "false"))
            .respond_with(ResponseTemplate::new(200).set_body_raw(
                r#"[{"id": 4, "title": "Favorites", "summary": null, "promoted": false,
                    "coverImage": "c4.png", "itemCount": 12, "owner": "admin", "source": 0}]"#,
                "application/json",
            ))
            .mount(&mock_server)
            .await;

        let client = KavitaClient::new(mock_server.uri(), Client::new());
        let collections = client.list_collections().await.unwrap();

        assert_eq!(
            collections,
            vec![KavitaCollection {
                id: 4,
                title: "Favorites".to_string(),
                summary: None,
                promoted: false,
                cover_image: Some("c4.png".to_string()),
                item_count: 12,
            }]
        );
    }
}
