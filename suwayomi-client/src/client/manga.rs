use super::{Error, SuwayomiClient};

impl SuwayomiClient {
    /// A manga's cover image.
    pub async fn manga_thumbnail(&self, manga_id: i32) -> Result<Vec<u8>, Error> {
        self.execute_bytes(self.get(&format!("/api/v1/manga/{manga_id}/thumbnail")))
            .await
    }
}

#[cfg(test)]
mod tests {
    use crate::client::test_support::client_for;
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    #[tokio::test]
    async fn manga_thumbnail_returns_the_image_bytes() {
        let mock_server = MockServer::start().await;
        let image: &[u8] = &[0xFF, 0xD8, 0xFF];

        Mock::given(method("GET"))
            .and(path("/api/v1/manga/9/thumbnail"))
            .respond_with(ResponseTemplate::new(200).set_body_raw(image, "image/jpeg"))
            .mount(&mock_server)
            .await;

        assert_eq!(
            client_for(&mock_server).manga_thumbnail(9).await.unwrap(),
            image
        );
    }

    #[tokio::test]
    async fn manga_thumbnail_errors_on_not_found() {
        let mock_server = MockServer::start().await;

        Mock::given(method("GET"))
            .and(path("/api/v1/manga/9/thumbnail"))
            .respond_with(ResponseTemplate::new(404))
            .mount(&mock_server)
            .await;

        assert!(client_for(&mock_server).manga_thumbnail(9).await.is_err());
    }
}
