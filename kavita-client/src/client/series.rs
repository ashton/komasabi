use super::KavitaClient;
use crate::models::series::Series;
use crate::models::series_filter::{
    SeriesFilter, SeriesFilterCombination, SeriesFilterComparison, SeriesFilterEntityType,
    SeriesFilterField, SeriesFilterStatement, SeriesSortField, SeriesSortOption,
};
use serde::Serialize;

#[derive(Serialize)]
struct SeriesFilterParams {
    #[serde(rename = "PageNumber", skip_serializing_if = "Option::is_none")]
    page_number: Option<u32>,
    #[serde(rename = "PageSize", skip_serializing_if = "Option::is_none")]
    page_size: Option<u32>,
}

impl KavitaClient {
    pub async fn list_series_with_filter(
        &self,
        filter: &SeriesFilter,
        page_number: Option<u32>,
        page_size: Option<u32>,
    ) -> Result<Vec<Series>, reqwest::Error> {
        self.execute_json(
            self.http_client
                .post(format!("{}/api/Series/v2", self.base_url))
                .query(&SeriesFilterParams {
                    page_number,
                    page_size,
                })
                .json(filter),
        )
        .await
    }

    pub async fn list_series_of_library_paginated(
        &self,
        library_id: String,
        page_number: Option<u32>,
        page_size: Option<u32>,
    ) -> Result<Vec<Series>, reqwest::Error> {
        let filter = SeriesFilter {
            id: 0,
            name: None,
            limit_to: 0,
            combination: SeriesFilterCombination::And,
            entity_type: SeriesFilterEntityType::Series,
            sort_options: SeriesSortOption {
                sort_field: SeriesSortField::SortName,
                is_ascending: true,
            },
            statements: vec![SeriesFilterStatement {
                comparison: SeriesFilterComparison::Equal,
                field: SeriesFilterField::Libraries,
                value: Some(library_id),
            }],
        };

        self.execute_json(
            self.http_client
                .post(format!("{}/api/Series/v2", self.base_url))
                .query(&SeriesFilterParams {
                    page_number,
                    page_size,
                })
                .json(&filter),
        )
        .await
    }

    pub async fn list_series_of_library(
        &self,
        library_id: String,
    ) -> Result<Vec<Series>, reqwest::Error> {
        self.list_series_of_library_paginated(library_id, None, None)
            .await
    }

    pub async fn series_cover(&self, series_id: u16) -> Result<Vec<u8>, reqwest::Error> {
        let response = self
            .execute(
                self.http_client
                    .get(format!("{}/api/image/series-cover", self.base_url))
                    .query(&[("seriesId", series_id)]),
            )
            .await?;

        Ok(response.error_for_status()?.bytes().await?.to_vec())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::series_filter::{
        SeriesFilterCombination, SeriesFilterEntityType, SeriesSortField, SeriesSortOption,
    };
    use reqwest::Client;
    use wiremock::matchers::{body_json, method, path, query_param};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    fn client_for(mock_server: &MockServer) -> KavitaClient {
        KavitaClient::new(mock_server.uri(), Client::new())
    }

    fn sample_filter() -> SeriesFilter {
        SeriesFilter {
            id: 0,
            name: None,
            statements: vec![],
            combination: SeriesFilterCombination::And,
            sort_options: SeriesSortOption {
                sort_field: SeriesSortField::SortName,
                is_ascending: true,
            },
            entity_type: SeriesFilterEntityType::Series,
            limit_to: 0,
        }
    }

    const SERIES_JSON: &str = r#"{
        "id": 1,
        "name": "My Series",
        "originalName": "My Series",
        "localizedName": "My Series",
        "coverImage": "1.png",
        "sortName": "My Series",
        "pages": 100,
        "userRating": 0.0,
        "hasUserRated": false,
        "totalReads": 0,
        "pagesRead": 0,
        "latestReadDate": "2024-01-15T10:30:00",
        "format": 1,
        "libraryId": 1,
        "libraryName": "Comics",
        "minHoursToRead": 1,
        "maxHoursToRead": 2,
        "avgHoursToRead": 1.5,
        "folderPath": "/data/comics/my-series",
        "isBlacklisted": false
    }"#;

    #[tokio::test]
    async fn series_v2_sends_filter_and_parses_response() {
        let mock_server = MockServer::start().await;
        let filter = sample_filter();
        let body = format!("[{}]", SERIES_JSON);

        Mock::given(method("POST"))
            .and(path("/api/Series/v2"))
            .and(query_param("PageNumber", "1"))
            .and(query_param("PageSize", "20"))
            .and(body_json(&filter))
            .respond_with(ResponseTemplate::new(200).set_body_raw(body, "application/json"))
            .mount(&mock_server)
            .await;

        let client = client_for(&mock_server);
        let series = client
            .list_series_with_filter(&filter, Some(1), Some(20))
            .await
            .unwrap();

        let expected: Series = serde_json::from_str(SERIES_JSON).unwrap();
        assert_eq!(series, vec![expected]);
    }

    #[tokio::test]
    async fn series_v2_omits_pagination_params_when_none() {
        let mock_server = MockServer::start().await;
        let filter = sample_filter();

        Mock::given(method("POST"))
            .and(path("/api/Series/v2"))
            .respond_with(ResponseTemplate::new(200).set_body_raw("[]", "application/json"))
            .mount(&mock_server)
            .await;

        let client = client_for(&mock_server);
        let series = client
            .list_series_with_filter(&filter, None, None)
            .await
            .unwrap();

        assert!(series.is_empty());
    }

    fn library_filter(library_id: &str) -> SeriesFilter {
        SeriesFilter {
            id: 0,
            name: None,
            limit_to: 0,
            combination: SeriesFilterCombination::And,
            entity_type: SeriesFilterEntityType::Series,
            sort_options: SeriesSortOption {
                sort_field: SeriesSortField::SortName,
                is_ascending: true,
            },
            statements: vec![SeriesFilterStatement {
                comparison: SeriesFilterComparison::Equal,
                field: SeriesFilterField::Libraries,
                value: Some(library_id.to_string()),
            }],
        }
    }

    #[tokio::test]
    async fn list_series_of_library_paginated_sends_library_filter_and_pagination() {
        let mock_server = MockServer::start().await;
        let expected_filter = library_filter("3");
        let body = format!("[{}]", SERIES_JSON);

        Mock::given(method("POST"))
            .and(path("/api/Series/v2"))
            .and(query_param("PageNumber", "2"))
            .and(query_param("PageSize", "10"))
            .and(body_json(&expected_filter))
            .respond_with(ResponseTemplate::new(200).set_body_raw(body, "application/json"))
            .mount(&mock_server)
            .await;

        let client = client_for(&mock_server);
        let series = client
            .list_series_of_library_paginated("3".to_string(), Some(2), Some(10))
            .await
            .unwrap();

        let expected: Series = serde_json::from_str(SERIES_JSON).unwrap();
        assert_eq!(series, vec![expected]);
    }

    #[tokio::test]
    async fn list_series_of_library_paginated_omits_pagination_params_when_none() {
        let mock_server = MockServer::start().await;
        let expected_filter = library_filter("3");

        Mock::given(method("POST"))
            .and(path("/api/Series/v2"))
            .and(body_json(&expected_filter))
            .respond_with(ResponseTemplate::new(200).set_body_raw("[]", "application/json"))
            .mount(&mock_server)
            .await;

        let client = client_for(&mock_server);
        let series = client
            .list_series_of_library_paginated("3".to_string(), None, None)
            .await
            .unwrap();

        assert!(series.is_empty());
    }

    #[tokio::test]
    async fn series_cover_fetches_raw_image_bytes() {
        let mock_server = MockServer::start().await;
        let image_bytes: &[u8] = &[0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A];

        Mock::given(method("GET"))
            .and(path("/api/image/series-cover"))
            .and(query_param("seriesId", "1"))
            .respond_with(ResponseTemplate::new(200).set_body_raw(image_bytes, "image/png"))
            .mount(&mock_server)
            .await;

        let client = client_for(&mock_server);
        let bytes = client.series_cover(1).await.unwrap();

        assert_eq!(bytes, image_bytes);
    }

    #[tokio::test]
    async fn series_cover_errors_on_non_success_status() {
        let mock_server = MockServer::start().await;

        Mock::given(method("GET"))
            .and(path("/api/image/series-cover"))
            .respond_with(ResponseTemplate::new(404))
            .mount(&mock_server)
            .await;

        let client = client_for(&mock_server);
        let result = client.series_cover(1).await;

        assert!(result.is_err());
    }

    #[tokio::test]
    async fn list_series_of_library_sends_library_filter_without_pagination() {
        let mock_server = MockServer::start().await;
        let expected_filter = library_filter("7");
        let body = format!("[{}]", SERIES_JSON);

        Mock::given(method("POST"))
            .and(path("/api/Series/v2"))
            .and(body_json(&expected_filter))
            .respond_with(ResponseTemplate::new(200).set_body_raw(body, "application/json"))
            .mount(&mock_server)
            .await;

        let client = client_for(&mock_server);
        let series = client
            .list_series_of_library("7".to_string())
            .await
            .unwrap();

        let expected: Series = serde_json::from_str(SERIES_JSON).unwrap();
        assert_eq!(series, vec![expected]);
    }
}
