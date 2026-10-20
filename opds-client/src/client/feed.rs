use reqwest::header::CONTENT_TYPE;

use super::OpdsClient;
use crate::error::Error;
use crate::format;
use crate::models::feed::Feed;

impl OpdsClient {
    /// Fetches the catalog's root feed.
    pub async fn catalog(&self) -> Result<Feed, Error> {
        self.feed(&self.base_url.clone()).await
    }

    /// Fetches and parses the feed at `href`, in whichever OPDS version the
    /// server answers. Hrefs in the result are resolved against the final
    /// (post-redirect) URL.
    pub async fn feed(&self, href: &str) -> Result<Feed, Error> {
        let response = self.execute(self.get_feed(self.url(href)?)).await?;

        let url = response.url().clone();
        let content_type = response
            .headers()
            .get(CONTENT_TYPE)
            .and_then(|value| value.to_str().ok())
            .map(str::to_string);
        let body = response.bytes().await?;

        format::parse_feed(&body, content_type.as_deref(), &url).inspect_err(|error| {
            tracing::debug!("failed to decode feed {url}: {error}");
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use reqwest::Client;
    use wiremock::matchers::{header_regex, method, path, query_param};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    fn client_for(mock_server: &MockServer) -> OpdsClient {
        OpdsClient::new(format!("{}/opds", mock_server.uri()), Client::new())
    }

    #[tokio::test]
    async fn catalog_parses_an_opds2_feed_and_asks_for_json_first() {
        let mock_server = MockServer::start().await;

        Mock::given(method("GET"))
            .and(path("/opds"))
            .and(header_regex("accept", "^application/opds\\+json"))
            .respond_with(ResponseTemplate::new(200).set_body_raw(
                r#"{"metadata": {"title": "Root"},
                    "navigation": [{"href": "series", "title": "Series"}]}"#,
                "application/opds+json",
            ))
            .mount(&mock_server)
            .await;

        let feed = client_for(&mock_server).catalog().await.unwrap();

        assert_eq!(feed.metadata.title, "Root");
        assert_eq!(
            feed.navigation[0].link.href,
            format!("{}/series", mock_server.uri())
        );
    }

    #[tokio::test]
    async fn feed_parses_an_atom_feed_served_as_plain_xml() {
        let mock_server = MockServer::start().await;

        Mock::given(method("GET"))
            .and(path("/opds/series/4"))
            .and(query_param("pageNumber", "2"))
            .respond_with(ResponseTemplate::new(200).set_body_raw(
                r#"<?xml version="1.0"?>
                <feed xmlns="http://www.w3.org/2005/Atom">
                  <title>Series</title>
                  <entry><id>1</id><title>Issue 1</title>
                    <link rel="http://opds-spec.org/acquisition" type="application/x-cbz" href="1.cbz"/>
                  </entry>
                </feed>"#,
                "application/xml",
            ))
            .mount(&mock_server)
            .await;

        let feed = client_for(&mock_server)
            .feed("/opds/series/4?pageNumber=2")
            .await
            .unwrap();

        assert_eq!(feed.publications[0].metadata.title, "Issue 1");
        assert_eq!(
            feed.publications[0].links[0].href,
            format!("{}/opds/series/1.cbz", mock_server.uri())
        );
    }

    #[tokio::test]
    async fn feed_sends_basic_auth_when_configured() {
        let mock_server = MockServer::start().await;

        Mock::given(method("GET"))
            .and(path("/opds"))
            // base64("user:secret")
            .and(header_regex("authorization", "^Basic dXNlcjpzZWNyZXQ=$"))
            .respond_with(ResponseTemplate::new(200).set_body_raw(
                r#"{"metadata": {"title": "Private"}}"#,
                "application/opds+json",
            ))
            .mount(&mock_server)
            .await;

        let client = client_for(&mock_server).with_basic_auth("user".into(), Some("secret".into()));

        assert_eq!(client.catalog().await.unwrap().metadata.title, "Private");
    }

    #[tokio::test]
    async fn feed_errors_on_failure_status() {
        let mock_server = MockServer::start().await;

        Mock::given(method("GET"))
            .respond_with(ResponseTemplate::new(401))
            .mount(&mock_server)
            .await;

        assert!(matches!(
            client_for(&mock_server).catalog().await,
            Err(Error::Http(_))
        ));
    }

    #[tokio::test]
    async fn feed_errors_on_a_non_opds_body() {
        let mock_server = MockServer::start().await;

        Mock::given(method("GET"))
            .respond_with(ResponseTemplate::new(200).set_body_raw("hello", "text/plain"))
            .mount(&mock_server)
            .await;

        assert!(matches!(
            client_for(&mock_server).catalog().await,
            Err(Error::UnsupportedFormat(_))
        ));
    }
}
