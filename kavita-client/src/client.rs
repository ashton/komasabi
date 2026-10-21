use reqwest::{Client, RequestBuilder, Response, Url};
use serde::de::DeserializeOwned;
use tracing::debug;

mod account;
mod chapter;
mod health;
mod library;
mod reader;
mod series;
mod volume;

pub struct KavitaClient {
    http_client: Client,
    base_url: String,
}

impl KavitaClient {
    pub fn new(base_url: String, http_client: Client) -> Self {
        KavitaClient {
            base_url,
            http_client,
        }
    }

    /// Sends `request`, logging the method, URL, response status and any
    /// error at debug level.
    async fn execute(&self, request: RequestBuilder) -> Result<Response, reqwest::Error> {
        let request = match request.build() {
            Ok(request) => request,
            Err(error) => {
                debug!("failed to build request: {error}");
                return Err(error);
            }
        };

        let method = request.method().clone();
        let url = redact_api_key(request.url());

        debug!("{method} {url}");

        let result = self.http_client.execute(request).await;

        match &result {
            Ok(response) => debug!("{method} {url} -> {}", response.status()),
            Err(error) => debug!("{method} {url} failed: {error}"),
        }

        result
    }

    /// Sends `request` and decodes the JSON response body into `T`, logging
    /// the method, URL, response status and any error (request or decode)
    /// at debug level.
    ///
    /// A non-2xx status is reported as an error without attempting to
    /// decode `T` from it — Kavita's error responses are a validation
    /// problem object, not the requested shape, so trying to decode them
    /// as `T` would just mask the real status error behind a confusing
    /// decode failure. The raw body is still logged so the actual
    /// validation message is visible.
    async fn execute_json<T: DeserializeOwned>(
        &self,
        request: RequestBuilder,
    ) -> Result<T, reqwest::Error> {
        let response = self.execute(request).await?;

        if let Err(error) = response.error_for_status_ref() {
            let body = response.text().await.unwrap_or_default();
            debug!("response body: {body}");
            return Err(error);
        }

        let body = response.json::<T>().await;

        if let Err(error) = &body {
            debug!("failed to decode response body: {error}");
        }

        body
    }
}

/// Masks the `apiKey` query parameter, if present, before a URL is logged.
/// `authenticate` passes the Kavita API key as part of the URL itself
/// (Kavita's endpoint takes it as a query param, not a header), so logging
/// requests verbatim would leak it into the logs.
fn redact_api_key(url: &Url) -> Url {
    if !url.query_pairs().any(|(key, _)| key == "apiKey") {
        return url.clone();
    }

    let redacted_pairs: Vec<(String, String)> = url
        .query_pairs()
        .map(|(key, value)| {
            if key == "apiKey" {
                (key.into_owned(), "REDACTED".to_string())
            } else {
                (key.into_owned(), value.into_owned())
            }
        })
        .collect();

    let mut redacted = url.clone();
    redacted
        .query_pairs_mut()
        .clear()
        .extend_pairs(redacted_pairs);
    redacted
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn redact_api_key_masks_the_api_key_query_param() {
        let url = Url::parse(
            "http://localhost:5000/api/Plugin/authenticate?apiKey=secret&pluginName=Komasabi",
        )
        .unwrap();

        let redacted = redact_api_key(&url);

        assert!(!redacted.as_str().contains("secret"));
        assert!(redacted.as_str().contains("apiKey=REDACTED"));
        assert!(redacted.as_str().contains("pluginName=Komasabi"));
    }

    #[test]
    fn redact_api_key_leaves_other_urls_unchanged() {
        let url = Url::parse("http://localhost:5000/api/Library/libraries").unwrap();

        let redacted = redact_api_key(&url);

        assert_eq!(redacted, url);
    }
}
