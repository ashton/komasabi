use reqwest::header::{HeaderMap, HeaderValue};
use reqwest::{Client, RequestBuilder, Response};
use serde::de::DeserializeOwned;
use tracing::debug;

mod book;
mod library;
mod series;
mod user;

/// How requests to Komga are authenticated. Komga accepts either an API key
/// (generated per user in its web UI) or the user's email and password.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Auth {
    ApiKey(String),
    Basic {
        username: String,
        password: String,
    },
    /// No credentials, for the few endpoints Komga serves anonymously.
    None,
}

pub struct KomgaClient {
    http_client: Client,
    base_url: String,
    auth: Auth,
}

impl KomgaClient {
    /// Builds a client for the Komga server at `base_url` (e.g.
    /// `http://localhost:25600`; a trailing `/` is ignored). An API key is
    /// sent as a default header on every request; basic-auth credentials are
    /// attached per request.
    pub fn new(base_url: String, auth: Auth) -> Result<Self, reqwest::Error> {
        let mut headers = HeaderMap::new();

        if let Auth::ApiKey(api_key) = &auth
            && let Ok(mut value) = HeaderValue::from_str(api_key)
        {
            value.set_sensitive(true);
            headers.insert("X-API-Key", value);
        }

        let http_client = Client::builder().default_headers(headers).build()?;

        Ok(KomgaClient {
            http_client,
            base_url: base_url.trim_end_matches('/').to_string(),
            auth,
        })
    }

    fn get(&self, path: &str) -> RequestBuilder {
        let request = self.http_client.get(format!("{}{path}", self.base_url));

        match &self.auth {
            Auth::Basic { username, password } => request.basic_auth(username, Some(password)),
            Auth::ApiKey(_) | Auth::None => request,
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

    /// Sends `request` and decodes the JSON response body into `T`.
    async fn execute_json<T: DeserializeOwned>(
        &self,
        request: RequestBuilder,
    ) -> Result<T, reqwest::Error> {
        let body = self.execute(request).await?.json::<T>().await;

        if let Err(error) = &body {
            debug!("failed to decode response body: {error}");
        }

        body
    }

    /// Sends `request` and returns the raw response body (an image).
    async fn execute_bytes(&self, request: RequestBuilder) -> Result<Vec<u8>, reqwest::Error> {
        Ok(self.execute(request).await?.bytes().await?.to_vec())
    }
}

#[cfg(test)]
pub(crate) mod test_support {
    use super::{Auth, KomgaClient};
    use wiremock::MockServer;

    pub fn client_for(mock_server: &MockServer) -> KomgaClient {
        KomgaClient::new(mock_server.uri(), Auth::ApiKey("secret-key".to_string())).unwrap()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use wiremock::matchers::{header, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    #[tokio::test]
    async fn api_key_is_sent_as_the_x_api_key_header() {
        let mock_server = MockServer::start().await;

        Mock::given(method("GET"))
            .and(path("/api/v1/libraries"))
            .and(header("X-API-Key", "secret-key"))
            .respond_with(ResponseTemplate::new(200).set_body_raw("[]", "application/json"))
            .expect(1)
            .mount(&mock_server)
            .await;

        let client =
            KomgaClient::new(mock_server.uri(), Auth::ApiKey("secret-key".to_string())).unwrap();

        assert!(client.list_libraries().await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn basic_credentials_are_sent_as_an_authorization_header() {
        let mock_server = MockServer::start().await;

        // base64("reader@example.com:secret")
        Mock::given(method("GET"))
            .and(path("/api/v1/libraries"))
            .and(header(
                "Authorization",
                "Basic cmVhZGVyQGV4YW1wbGUuY29tOnNlY3JldA==",
            ))
            .respond_with(ResponseTemplate::new(200).set_body_raw("[]", "application/json"))
            .expect(1)
            .mount(&mock_server)
            .await;

        let client = KomgaClient::new(
            mock_server.uri(),
            Auth::Basic {
                username: "reader@example.com".to_string(),
                password: "secret".to_string(),
            },
        )
        .unwrap();

        assert!(client.list_libraries().await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn a_trailing_slash_in_the_base_url_is_ignored() {
        let mock_server = MockServer::start().await;

        Mock::given(method("GET"))
            .and(path("/api/v1/libraries"))
            .respond_with(ResponseTemplate::new(200).set_body_raw("[]", "application/json"))
            .expect(1)
            .mount(&mock_server)
            .await;

        let client = KomgaClient::new(format!("{}/", mock_server.uri()), Auth::None).unwrap();

        assert!(client.list_libraries().await.is_ok());
    }

    #[tokio::test]
    async fn a_non_success_status_is_reported_as_an_error() {
        let mock_server = MockServer::start().await;

        Mock::given(method("GET"))
            .and(path("/api/v1/libraries"))
            .respond_with(ResponseTemplate::new(401))
            .mount(&mock_server)
            .await;

        let client = test_support::client_for(&mock_server);
        let error = client.list_libraries().await.unwrap_err();

        assert_eq!(error.status(), Some(reqwest::StatusCode::UNAUTHORIZED));
    }
}
