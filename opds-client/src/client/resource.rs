use std::io::Write;
use std::path::Path;

use super::OpdsClient;
use crate::error::Error;
use crate::format;
use crate::models::{link::Link, manifest::Manifest};

impl OpdsClient {
    /// Fetches a Readium Web Publication / Divina manifest.
    pub async fn manifest(&self, href: &str) -> Result<Manifest, Error> {
        let response = self.execute(self.get(self.url(href)?)).await?;
        let url = response.url().clone();
        let body = response.bytes().await?;

        format::parse_manifest(&body, &url)
    }

    /// Downloads a resource's raw bytes: a cover, a page image, an archive.
    pub async fn bytes(&self, href: &str) -> Result<Vec<u8>, Error> {
        let response = self.execute(self.get(self.url(href)?)).await?;
        Ok(response.bytes().await?.to_vec())
    }

    /// Streams the file at `href` (typically an open-access acquisition link)
    /// into `destination`, chunk by chunk, so large archives never sit in
    /// memory whole. Returns the number of bytes written.
    pub async fn download(&self, href: &str, destination: &Path) -> Result<u64, Error> {
        let mut response = self.execute(self.get(self.url(href)?)).await?;
        let mut file = std::fs::File::create(destination)?;
        let mut written = 0;

        while let Some(chunk) = response.chunk().await? {
            file.write_all(&chunk)?;
            written += chunk.len() as u64;
        }

        file.flush()?;
        Ok(written)
    }

    /// Downloads the 0-based `page` of an OPDS-PSE page `stream` link.
    pub async fn stream_page(&self, stream: &Link, page: u32) -> Result<Vec<u8>, Error> {
        self.bytes(&stream.expand_page(page)).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use reqwest::Client;
    use wiremock::matchers::{method, path, query_param};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    fn client_for(mock_server: &MockServer) -> OpdsClient {
        OpdsClient::new(mock_server.uri(), Client::new())
    }

    #[tokio::test]
    async fn stream_page_expands_the_page_template() {
        let mock_server = MockServer::start().await;

        Mock::given(method("GET"))
            .and(path("/image"))
            .and(query_param("chapterId", "75"))
            .and(query_param("pageNumber", "3"))
            .respond_with(ResponseTemplate::new(200).set_body_raw(vec![1, 2, 3], "image/jpeg"))
            .mount(&mock_server)
            .await;

        let stream = Link {
            href: format!(
                "{}/image?chapterId=75&pageNumber={{pageNumber}}",
                mock_server.uri()
            ),
            ..Link::default()
        };

        let bytes = client_for(&mock_server)
            .stream_page(&stream, 3)
            .await
            .unwrap();

        assert_eq!(bytes, vec![1, 2, 3]);
    }

    #[tokio::test]
    async fn download_streams_the_file_to_disk() {
        let mock_server = MockServer::start().await;
        let body: Vec<u8> = (0..=255u8).cycle().take(100_000).collect();

        Mock::given(method("GET"))
            .and(path("/download/book.cbz"))
            .respond_with(
                ResponseTemplate::new(200).set_body_raw(body.clone(), "application/x-cbz"),
            )
            .mount(&mock_server)
            .await;

        let destination =
            std::env::temp_dir().join(format!("opds-client-download-{}.cbz", std::process::id()));

        let written = client_for(&mock_server)
            .download("/download/book.cbz", &destination)
            .await
            .unwrap();

        assert_eq!(written, body.len() as u64);
        assert_eq!(std::fs::read(&destination).unwrap(), body);
        let _ = std::fs::remove_file(destination);
    }

    #[tokio::test]
    async fn download_errors_on_failure_status_without_creating_the_file() {
        let mock_server = MockServer::start().await;

        Mock::given(method("GET"))
            .respond_with(ResponseTemplate::new(404))
            .mount(&mock_server)
            .await;

        let destination =
            std::env::temp_dir().join(format!("opds-client-missing-{}.cbz", std::process::id()));

        let result = client_for(&mock_server)
            .download("/download/missing.cbz", &destination)
            .await;

        assert!(matches!(result, Err(Error::Http(_))));
        assert!(!destination.exists());
    }

    #[tokio::test]
    async fn manifest_parses_the_reading_order() {
        let mock_server = MockServer::start().await;

        Mock::given(method("GET"))
            .and(path("/pub/1/manifest.json"))
            .respond_with(ResponseTemplate::new(200).set_body_raw(
                r#"{"metadata": {"title": "Comic"},
                    "readingOrder": [{"href": "1.jpg", "type": "image/jpeg"}]}"#,
                "application/divina+json",
            ))
            .mount(&mock_server)
            .await;

        let manifest = client_for(&mock_server)
            .manifest("/pub/1/manifest.json")
            .await
            .unwrap();

        assert_eq!(manifest.metadata.title, "Comic");
        assert_eq!(
            manifest.pages().next().unwrap().href,
            format!("{}/pub/1/1.jpg", mock_server.uri())
        );
    }
}
