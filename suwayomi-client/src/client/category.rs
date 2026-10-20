use serde::Deserialize;
use serde_json::json;

use super::{Error, SuwayomiClient};
use crate::models::{category::Category, manga::Manga};

#[derive(Deserialize)]
struct Nodes<T> {
    nodes: Vec<T>,
}

#[derive(Deserialize)]
struct CategoriesData {
    categories: Nodes<Category>,
}

#[derive(Deserialize)]
struct CategoryData {
    category: CategoryMangas,
}

#[derive(Deserialize)]
struct CategoryMangas {
    mangas: Nodes<Manga>,
}

const CATEGORIES_QUERY: &str = "query { categories(orderBy: ORDER) { nodes { \
    id name order default mangas { totalCount } } } }";

const CATEGORY_MANGAS_QUERY: &str = "query ($id: Int!) { category(id: $id) { mangas { \
    nodes { id title thumbnailUrl inLibrary } } } }";

impl SuwayomiClient {
    /// Lists the library's categories, in the user's order.
    pub async fn list_categories(&self) -> Result<Vec<Category>, Error> {
        let data: CategoriesData = self.graphql(CATEGORIES_QUERY, json!({})).await?;

        Ok(data.categories.nodes)
    }

    /// Lists every manga in a category. Suwayomi doesn't page a category's
    /// mangas, so this returns them all at once.
    pub async fn list_category_mangas(&self, category_id: i32) -> Result<Vec<Manga>, Error> {
        let data: CategoryData = self
            .graphql(CATEGORY_MANGAS_QUERY, json!({ "id": category_id }))
            .await?;

        Ok(data.category.mangas.nodes)
    }
}

#[cfg(test)]
mod tests {
    use crate::client::test_support::{client_for, data};
    use crate::models::category::{Category, MangaCount};
    use crate::models::manga::Manga;
    use wiremock::matchers::{body_partial_json, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    #[tokio::test]
    async fn list_categories_parses_response() {
        let mock_server = MockServer::start().await;

        Mock::given(method("POST"))
            .and(path("/api/graphql"))
            .respond_with(ResponseTemplate::new(200).set_body_raw(
                data(
                    r#"{"categories": {"nodes": [
                        {"id": 0, "name": "Default", "order": 0, "default": true,
                         "mangas": {"totalCount": 55}},
                        {"id": 1, "name": "Completed", "order": 1, "default": false,
                         "mangas": {"totalCount": 0}}
                    ]}}"#,
                ),
                "application/json",
            ))
            .mount(&mock_server)
            .await;

        let categories = client_for(&mock_server).list_categories().await.unwrap();

        assert_eq!(
            categories,
            vec![
                Category {
                    id: 0,
                    name: "Default".to_string(),
                    order: 0,
                    default: true,
                    mangas: MangaCount { total_count: 55 },
                },
                Category {
                    id: 1,
                    name: "Completed".to_string(),
                    order: 1,
                    default: false,
                    mangas: MangaCount { total_count: 0 },
                },
            ]
        );
    }

    #[tokio::test]
    async fn list_category_mangas_sends_the_category_id_and_parses_response() {
        let mock_server = MockServer::start().await;

        Mock::given(method("POST"))
            .and(path("/api/graphql"))
            .and(body_partial_json(
                serde_json::json!({"variables": {"id": 3}}),
            ))
            .respond_with(ResponseTemplate::new(200).set_body_raw(
                data(
                    r#"{"category": {"mangas": {"nodes": [
                        {"id": 9, "title": "Voyeurs, Inc.",
                         "thumbnailUrl": "/api/v1/manga/9/thumbnail", "inLibrary": true},
                        {"id": 10, "title": "No Cover", "thumbnailUrl": null, "inLibrary": true}
                    ]}}}"#,
                ),
                "application/json",
            ))
            .expect(1)
            .mount(&mock_server)
            .await;

        let mangas = client_for(&mock_server)
            .list_category_mangas(3)
            .await
            .unwrap();

        assert_eq!(
            mangas,
            vec![
                Manga {
                    id: 9,
                    title: "Voyeurs, Inc.".to_string(),
                    thumbnail_url: Some("/api/v1/manga/9/thumbnail".to_string()),
                    in_library: true,
                },
                Manga {
                    id: 10,
                    title: "No Cover".to_string(),
                    thumbnail_url: None,
                    in_library: true,
                },
            ]
        );
    }

    #[tokio::test]
    async fn list_category_mangas_errors_on_malformed_body() {
        let mock_server = MockServer::start().await;

        Mock::given(method("POST"))
            .and(path("/api/graphql"))
            .respond_with(ResponseTemplate::new(200).set_body_raw("not json", "application/json"))
            .mount(&mock_server)
            .await;

        assert!(
            client_for(&mock_server)
                .list_category_mangas(0)
                .await
                .is_err()
        );
    }
}
