use serde::Deserialize;
use serde_json::json;

use super::{Error, SuwayomiClient};
use crate::models::server::AboutServer;

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct AboutData {
    about_server: AboutServer,
}

impl SuwayomiClient {
    /// The server's name and version. Cheap, so it doubles as a
    /// reachability (and, behind basic auth, credentials) check.
    pub async fn about(&self) -> Result<AboutServer, Error> {
        let data: AboutData = self
            .graphql("query { aboutServer { name version } }", json!({}))
            .await?;

        Ok(data.about_server)
    }
}

#[cfg(test)]
mod tests {
    use crate::client::test_support::{client_for, data};
    use crate::models::server::AboutServer;
    use wiremock::matchers::{body_partial_json, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    #[tokio::test]
    async fn about_queries_the_server_and_parses_the_response() {
        let mock_server = MockServer::start().await;

        Mock::given(method("POST"))
            .and(path("/api/graphql"))
            .and(body_partial_json(
                serde_json::json!({"query": "query { aboutServer { name version } }"}),
            ))
            .respond_with(ResponseTemplate::new(200).set_body_raw(
                data(r#"{"aboutServer": {"name": "Suwayomi-Server", "version": "v2.3.2243"}}"#),
                "application/json",
            ))
            .expect(1)
            .mount(&mock_server)
            .await;

        assert_eq!(
            client_for(&mock_server).about().await.unwrap(),
            AboutServer {
                name: "Suwayomi-Server".to_string(),
                version: "v2.3.2243".to_string(),
            }
        );
    }
}
