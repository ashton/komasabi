use crate::{client::KavitaClient, models::volume::Volume};

impl KavitaClient {
    pub async fn list_volumes_from_series(
        &self,
        series_id: u16,
    ) -> Result<Vec<Volume>, reqwest::Error> {
        self.execute_json(self.http_client.get(format!(
            "{}/api/Series/volumes?seriesId={}",
            self.base_url, series_id
        )))
        .await
    }

    pub async fn volume_detail(&self, volume_id: u16) -> Result<Volume, reqwest::Error> {
        self.execute_json(self.http_client.get(format!(
            "{}/api/Volume?volumeId={}",
            self.base_url, volume_id
        )))
        .await
    }

    pub async fn volume_cover(&self, volume_id: u16) -> Result<Vec<u8>, reqwest::Error> {
        let response = self
            .execute(
                self.http_client
                    .get(format!("{}/api/image/volume-cover", self.base_url))
                    .query(&[("volumeId", volume_id)]),
            )
            .await?;

        Ok(response.error_for_status()?.bytes().await?.to_vec())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use reqwest::Client;
    use wiremock::matchers::{method, path, query_param};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    fn client_for(mock_server: &MockServer) -> KavitaClient {
        KavitaClient::new(mock_server.uri(), Client::new())
    }

    #[tokio::test]
    async fn volume_cover_fetches_raw_image_bytes() {
        let mock_server = MockServer::start().await;
        let image_bytes: &[u8] = &[0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A];

        Mock::given(method("GET"))
            .and(path("/api/image/volume-cover"))
            .and(query_param("volumeId", "1"))
            .respond_with(ResponseTemplate::new(200).set_body_raw(image_bytes, "image/png"))
            .mount(&mock_server)
            .await;

        let client = client_for(&mock_server);
        let bytes = client.volume_cover(1).await.unwrap();

        assert_eq!(bytes, image_bytes);
    }

    #[tokio::test]
    async fn volume_cover_errors_on_non_success_status() {
        let mock_server = MockServer::start().await;

        Mock::given(method("GET"))
            .and(path("/api/image/volume-cover"))
            .respond_with(ResponseTemplate::new(404))
            .mount(&mock_server)
            .await;

        let client = client_for(&mock_server);
        let result = client.volume_cover(1).await;

        assert!(result.is_err());
    }
}
