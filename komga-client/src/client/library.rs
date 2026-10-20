use super::KomgaClient;
use crate::models::library::Library;

impl KomgaClient {
    /// Lists the libraries the authenticated user can access.
    pub async fn list_libraries(&self) -> Result<Vec<Library>, reqwest::Error> {
        self.execute_json(self.get("/api/v1/libraries")).await
    }
}

#[cfg(test)]
mod tests {
    use crate::client::test_support::client_for;
    use crate::models::library::Library;
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    #[tokio::test]
    async fn list_libraries_parses_response() {
        let mock_server = MockServer::start().await;

        Mock::given(method("GET"))
            .and(path("/api/v1/libraries"))
            .respond_with(ResponseTemplate::new(200).set_body_raw(
                r#"[{
                    "id": "0B8RAK3Q2Y1VX",
                    "name": "Manga",
                    "root": "/data/manga",
                    "importComicInfoBook": true,
                    "scanInterval": "EVERY_6H",
                    "unavailable": false
                }]"#,
                "application/json",
            ))
            .mount(&mock_server)
            .await;

        let libraries = client_for(&mock_server).list_libraries().await.unwrap();

        assert_eq!(
            libraries,
            vec![Library {
                id: "0B8RAK3Q2Y1VX".to_string(),
                name: "Manga".to_string(),
                root: "/data/manga".to_string(),
                unavailable: false,
            }]
        );
    }

    #[tokio::test]
    async fn list_libraries_errors_on_malformed_body() {
        let mock_server = MockServer::start().await;

        Mock::given(method("GET"))
            .and(path("/api/v1/libraries"))
            .respond_with(ResponseTemplate::new(200).set_body_raw("not json", "application/json"))
            .mount(&mock_server)
            .await;

        assert!(client_for(&mock_server).list_libraries().await.is_err());
    }
}
