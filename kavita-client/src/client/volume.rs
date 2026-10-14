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

    /// A realistically-shaped Kavita volume, including a nested chapter with a
    /// file and a person. Mirrors what the live API returns: camelCase keys
    /// and `...Utc`/`created`/`releaseDate` datetimes serialized *without* a
    /// timezone offset (e.g. `2026-10-10T05:44:01.71`), which is why those
    /// fields are modelled as `NaiveDateTime` rather than `DateTime<Utc>`.
    const VOLUME_JSON: &str = r#"[{
        "id": 1605,
        "name": "1",
        "coverImage": "v1605_c2274.png",
        "pages": 183,
        "pagesRead": 26,
        "seriesId": 345,
        "minNumber": 1,
        "maxNumber": 1,
        "minHoursToRead": 1,
        "maxHoursToRead": 1,
        "avgHoursToRead": 1.0,
        "wordCount": 0,
        "chapters": [{
            "id": 2274,
            "range": "1",
            "number": "1",
            "minNumber": 1.0,
            "maxNumber": 1.0,
            "sortOrder": 1.0,
            "pages": 30,
            "isSpecial": false,
            "title": "1",
            "pagesRead": 30,
            "totalReads": 0,
            "lastReadingProgressUtc": "2026-10-10T06:04:43.856902",
            "lastReadingProgress": "2026-10-10T03:04:43.8569004",
            "volumeId": 1605,
            "createdUtc": "2026-10-10T05:44:01.7183536",
            "lastModifiedUtc": "2026-10-10T05:44:03.2533084",
            "created": "2026-10-10T02:44:01.7183532",
            "releaseDate": "2023-12-01T00:00:00",
            "coverImage": "v1605_c2274.png",
            "files": [{
                "id": 2299,
                "filePath": "/comics/Beneath the Trees 001.cbr",
                "pages": 30,
                "bytes": 76502155,
                "format": 1,
                "created": "2026-10-10T02:44:01.7184349",
                "extension": ".cbr"
            }],
            "ageRating": 0,
            "writers": [],
            "coverArtists": [],
            "publishers": [{
                "id": 161,
                "name": "IDW",
                "coverImage": null,
                "description": null,
                "roles": []
            }],
            "characters": [],
            "pencillers": [],
            "inkers": [],
            "imprints": [],
            "colorists": [],
            "letterers": [],
            "editors": [],
            "translators": [],
            "teams": [],
            "locations": [],
            "genres": [],
            "publicationStatus": 2,
            "format": 1
        }]
    }]"#;

    #[tokio::test]
    async fn list_volumes_from_series_decodes_a_realistic_response() {
        let mock_server = MockServer::start().await;

        Mock::given(method("GET"))
            .and(path("/api/Series/volumes"))
            .and(query_param("seriesId", "345"))
            .respond_with(ResponseTemplate::new(200).set_body_raw(VOLUME_JSON, "application/json"))
            .mount(&mock_server)
            .await;

        let client = client_for(&mock_server);
        let volumes = client.list_volumes_from_series(345).await.unwrap();

        assert_eq!(volumes.len(), 1);
        assert_eq!(volumes[0].id, 1605);
        assert_eq!(volumes[0].series_id, 345);
        assert_eq!(volumes[0].pages, 183);
        assert_eq!(volumes[0].chapters.len(), 1);
        assert_eq!(volumes[0].chapters[0].files.len(), 1);
    }

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
