use reqwest::{Client, RequestBuilder, Response, Url, header::ACCEPT};
use tracing::debug;

use crate::error::Error;

mod feed;
mod resource;

/// Content negotiation for catalog documents: OPDS 2 preferred, OPDS 1 (Atom)
/// accepted, so a server offering both answers in JSON.
const FEED_ACCEPT: &str = "application/opds+json, application/json;q=0.9, \
    application/atom+xml;q=0.8, application/xml;q=0.7, */*;q=0.5";

pub struct OpdsClient {
    http_client: Client,
    /// The catalog's root (start) URL. Relative hrefs passed to any method are
    /// resolved against it.
    base_url: String,
    credentials: Option<Credentials>,
}

/// HTTP basic-auth credentials, sent with every request (Komga, Calibre-Web
/// and most self-hosted catalogs authenticate OPDS this way).
#[derive(Clone)]
struct Credentials {
    username: String,
    password: Option<String>,
}

impl OpdsClient {
    pub fn new(base_url: String, http_client: Client) -> Self {
        OpdsClient {
            http_client,
            base_url,
            credentials: None,
        }
    }

    /// Authenticates every request with HTTP basic auth.
    pub fn with_basic_auth(mut self, username: String, password: Option<String>) -> Self {
        self.credentials = Some(Credentials { username, password });
        self
    }

    /// Resolves `href` (absolute, or relative to the catalog root) to a URL.
    pub fn url(&self, href: &str) -> Result<Url, Error> {
        Ok(Url::parse(&self.base_url)?.join(href)?)
    }

    fn get(&self, url: Url) -> RequestBuilder {
        let request = self.http_client.get(url);
        match &self.credentials {
            Some(credentials) => {
                request.basic_auth(&credentials.username, credentials.password.as_ref())
            }
            None => request,
        }
    }

    fn get_feed(&self, url: Url) -> RequestBuilder {
        self.get(url).header(ACCEPT, FEED_ACCEPT)
    }

    /// Sends `request`, logging the method, URL and response status at debug
    /// level, and turning error statuses into errors.
    async fn execute(&self, request: RequestBuilder) -> Result<Response, Error> {
        let request = request.build().inspect_err(|error| {
            debug!("failed to build request: {error}");
        })?;

        debug!("{} {}", request.method(), redact_api_key(request.url()));

        let response = self
            .http_client
            .execute(request)
            .await
            .inspect_err(|error| debug!("request failed: {error}"))?;

        debug!("-> {}", response.status());

        Ok(response
            .error_for_status()
            .inspect_err(|error| debug!("error status: {error}"))?)
    }
}

/// Masks an `apiKey` query parameter before a URL is logged. Kavita's OPDS
/// image links authorize with one.
fn redact_api_key(url: &Url) -> Url {
    if !url
        .query_pairs()
        .any(|(key, _)| key.eq_ignore_ascii_case("apikey"))
    {
        return url.clone();
    }

    let pairs: Vec<(String, String)> = url
        .query_pairs()
        .map(|(key, value)| {
            if key.eq_ignore_ascii_case("apikey") {
                (key.into_owned(), "REDACTED".to_string())
            } else {
                (key.into_owned(), value.into_owned())
            }
        })
        .collect();

    let mut redacted = url.clone();
    redacted.query_pairs_mut().clear().extend_pairs(pairs);
    redacted
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn url_resolves_relative_hrefs_against_the_catalog_root() {
        let client = OpdsClient::new("http://server/opds/v1.2/catalog".into(), Client::new());

        assert_eq!(
            client.url("/opds/v1.2/series/1").unwrap().as_str(),
            "http://server/opds/v1.2/series/1"
        );
        assert_eq!(
            client.url("http://other/feed").unwrap().as_str(),
            "http://other/feed"
        );
    }

    #[test]
    fn redact_api_key_masks_the_api_key_query_param() {
        let url = Url::parse("http://h/api/image?chapterId=1&apiKey=secret").unwrap();

        let redacted = redact_api_key(&url);

        assert!(!redacted.as_str().contains("secret"));
        assert!(redacted.as_str().contains("chapterId=1"));
    }
}
