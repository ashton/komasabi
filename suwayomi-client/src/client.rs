use std::fmt;

use reqwest::{Client, RequestBuilder, Response};
use serde::Deserialize;
use serde::de::DeserializeOwned;
use serde_json::{Value, json};
use tracing::debug;

mod category;
mod chapter;
mod manga;
mod server;

/// How requests to Suwayomi are authenticated. A server runs unauthenticated
/// by default; one configured with `BASIC_AUTH` expects the credentials on
/// every request.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Auth {
    Basic { username: String, password: String },
    None,
}

#[derive(Debug)]
pub enum Error {
    /// The request failed, or the server answered with a non-2xx status.
    Http(reqwest::Error),
    /// The server answered, but the GraphQL query failed; one message per
    /// error it reported.
    GraphQl(Vec<String>),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Http(error) => error.fmt(f),
            Error::GraphQl(messages) => write!(f, "GraphQL error: {}", messages.join("; ")),
        }
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Error::Http(error) => Some(error),
            Error::GraphQl(_) => None,
        }
    }
}

impl From<reqwest::Error> for Error {
    fn from(error: reqwest::Error) -> Self {
        Error::Http(error)
    }
}

impl Error {
    /// The HTTP status the server answered with, for an [`Error::Http`].
    pub fn status(&self) -> Option<reqwest::StatusCode> {
        match self {
            Error::Http(error) => error.status(),
            Error::GraphQl(_) => None,
        }
    }
}

#[derive(Deserialize)]
struct GraphQlResponse<T> {
    data: Option<T>,
    #[serde(default)]
    errors: Vec<GraphQlError>,
}

#[derive(Deserialize)]
struct GraphQlError {
    message: String,
}

pub struct SuwayomiClient {
    http_client: Client,
    base_url: String,
    auth: Auth,
}

impl SuwayomiClient {
    /// Builds a client for the Suwayomi server at `base_url` (e.g.
    /// `http://localhost:4567`; a trailing `/` is ignored).
    pub fn new(base_url: String, auth: Auth) -> Result<Self, reqwest::Error> {
        Ok(SuwayomiClient {
            http_client: Client::builder().build()?,
            base_url: base_url.trim_end_matches('/').to_string(),
            auth,
        })
    }

    fn authenticated(&self, request: RequestBuilder) -> RequestBuilder {
        match &self.auth {
            Auth::Basic { username, password } => request.basic_auth(username, Some(password)),
            Auth::None => request,
        }
    }

    /// A GET for a server-relative `path` (`/api/v1/...`), as the GraphQL API
    /// hands out for images.
    fn get(&self, path: &str) -> RequestBuilder {
        self.authenticated(self.http_client.get(format!("{}{path}", self.base_url)))
    }

    /// Runs a GraphQL `query` with `variables` and decodes its `data` into
    /// `T`. A response carrying `errors` fails, even when it also has data.
    async fn graphql<T: DeserializeOwned>(
        &self,
        query: &str,
        variables: Value,
    ) -> Result<T, Error> {
        let request = self.authenticated(
            self.http_client
                .post(format!("{}/api/graphql", self.base_url))
                .json(&json!({ "query": query, "variables": variables })),
        );

        let body = self
            .execute(request)
            .await?
            .json::<GraphQlResponse<T>>()
            .await;

        let body = match body {
            Ok(body) => body,
            Err(error) => {
                debug!("failed to decode GraphQL response: {error}");
                return Err(error.into());
            }
        };

        match (body.data, body.errors) {
            (Some(data), errors) if errors.is_empty() => Ok(data),
            (_, errors) if !errors.is_empty() => {
                let messages: Vec<String> = errors.into_iter().map(|e| e.message).collect();
                debug!("GraphQL errors: {messages:?}");
                Err(Error::GraphQl(messages))
            }
            _ => Err(Error::GraphQl(vec!["response has no data".to_string()])),
        }
    }

    /// Sends `request`, logging the method, URL, response status and any
    /// error at debug level, and turns a non-2xx status into an error.
    async fn execute(&self, request: RequestBuilder) -> Result<Response, reqwest::Error> {
        let request = match request.build() {
            Ok(request) => request,
            Err(error) => {
                debug!("failed to build request: {error}");
                return Err(error);
            }
        };

        let method = request.method().clone();
        let url = request.url().clone();

        debug!("{method} {url}");

        let response = match self.http_client.execute(request).await {
            Ok(response) => response,
            Err(error) => {
                debug!("{method} {url} failed: {error}");
                return Err(error);
            }
        };

        debug!("{method} {url} -> {}", response.status());

        if let Err(error) = response.error_for_status_ref() {
            let body = response.text().await.unwrap_or_default();
            debug!("response body: {body}");
            return Err(error);
        }

        Ok(response)
    }

    /// Sends `request` and returns the raw response body (an image).
    async fn execute_bytes(&self, request: RequestBuilder) -> Result<Vec<u8>, Error> {
        Ok(self.execute(request).await?.bytes().await?.to_vec())
    }
}

#[cfg(test)]
pub(crate) mod test_support {
    use super::{Auth, SuwayomiClient};
    use wiremock::MockServer;

    pub fn client_for(mock_server: &MockServer) -> SuwayomiClient {
        SuwayomiClient::new(mock_server.uri(), Auth::None).unwrap()
    }

    /// A `/api/graphql` response body wrapping `data`.
    pub fn data(data: &str) -> String {
        format!(r#"{{"data": {data}}}"#)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use test_support::client_for;
    use wiremock::matchers::{header, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    fn about_response() -> ResponseTemplate {
        ResponseTemplate::new(200).set_body_raw(
            test_support::data(r#"{"aboutServer": {"name": "Suwayomi-Server", "version": "v2"}}"#),
            "application/json",
        )
    }

    #[tokio::test]
    async fn basic_credentials_are_sent_as_an_authorization_header() {
        let mock_server = MockServer::start().await;

        // base64("reader:secret")
        Mock::given(method("POST"))
            .and(path("/api/graphql"))
            .and(header("Authorization", "Basic cmVhZGVyOnNlY3JldA=="))
            .respond_with(about_response())
            .expect(1)
            .mount(&mock_server)
            .await;

        let client = SuwayomiClient::new(
            mock_server.uri(),
            Auth::Basic {
                username: "reader".to_string(),
                password: "secret".to_string(),
            },
        )
        .unwrap();

        assert!(client.about().await.is_ok());
    }

    #[tokio::test]
    async fn a_trailing_slash_in_the_base_url_is_ignored() {
        let mock_server = MockServer::start().await;

        Mock::given(method("POST"))
            .and(path("/api/graphql"))
            .respond_with(about_response())
            .expect(1)
            .mount(&mock_server)
            .await;

        let client = SuwayomiClient::new(format!("{}/", mock_server.uri()), Auth::None).unwrap();

        assert!(client.about().await.is_ok());
    }

    #[tokio::test]
    async fn a_non_success_status_is_reported_as_an_http_error() {
        let mock_server = MockServer::start().await;

        Mock::given(method("POST"))
            .and(path("/api/graphql"))
            .respond_with(ResponseTemplate::new(401))
            .mount(&mock_server)
            .await;

        let error = client_for(&mock_server).about().await.unwrap_err();

        assert_eq!(error.status(), Some(reqwest::StatusCode::UNAUTHORIZED));
    }

    #[tokio::test]
    async fn graphql_errors_are_reported_even_alongside_data() {
        let mock_server = MockServer::start().await;

        Mock::given(method("POST"))
            .and(path("/api/graphql"))
            .respond_with(ResponseTemplate::new(200).set_body_raw(
                r#"{"data": null, "errors": [{"message": "boom"}, {"message": "bang"}]}"#,
                "application/json",
            ))
            .mount(&mock_server)
            .await;

        let error = client_for(&mock_server).about().await.unwrap_err();

        assert!(matches!(&error, Error::GraphQl(messages) if messages == &["boom", "bang"]));
        assert_eq!(error.to_string(), "GraphQL error: boom; bang");
    }

    #[tokio::test]
    async fn a_response_without_data_or_errors_is_an_error() {
        let mock_server = MockServer::start().await;

        Mock::given(method("POST"))
            .and(path("/api/graphql"))
            .respond_with(ResponseTemplate::new(200).set_body_raw("{}", "application/json"))
            .mount(&mock_server)
            .await;

        assert!(matches!(
            client_for(&mock_server).about().await,
            Err(Error::GraphQl(_))
        ));
    }
}
