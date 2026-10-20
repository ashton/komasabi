use super::KomgaClient;
use crate::models::user::{ClaimStatus, User};

impl KomgaClient {
    /// The authenticated user (Komga serves users under `/api/v2`). Fails with
    /// `401` when the credentials are wrong, which makes it the cheapest
    /// authentication check.
    pub async fn me(&self) -> Result<User, reqwest::Error> {
        self.execute_json(self.get("/api/v2/users/me")).await
    }

    /// Whether the server has been claimed. Served without authentication, so
    /// it doubles as a reachability check for a Komga server.
    pub async fn claim_status(&self) -> Result<ClaimStatus, reqwest::Error> {
        self.execute_json(self.get("/api/v1/claim")).await
    }
}

#[cfg(test)]
mod tests {
    use crate::client::test_support::client_for;
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    #[tokio::test]
    async fn me_parses_response() {
        let mock_server = MockServer::start().await;

        Mock::given(method("GET"))
            .and(path("/api/v2/users/me"))
            .respond_with(ResponseTemplate::new(200).set_body_raw(
                r#"{"id": "U1", "email": "reader@example.com", "roles": ["ADMIN"]}"#,
                "application/json",
            ))
            .mount(&mock_server)
            .await;

        let user = client_for(&mock_server).me().await.unwrap();

        assert_eq!(user.email, "reader@example.com");
        assert_eq!(user.roles, vec!["ADMIN".to_string()]);
    }

    #[tokio::test]
    async fn me_errors_when_unauthorized() {
        let mock_server = MockServer::start().await;

        Mock::given(method("GET"))
            .and(path("/api/v2/users/me"))
            .respond_with(ResponseTemplate::new(401))
            .mount(&mock_server)
            .await;

        assert!(client_for(&mock_server).me().await.is_err());
    }

    #[tokio::test]
    async fn claim_status_parses_response() {
        let mock_server = MockServer::start().await;

        Mock::given(method("GET"))
            .and(path("/api/v1/claim"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_raw(r#"{"isClaimed": true}"#, "application/json"),
            )
            .mount(&mock_server)
            .await;

        assert!(
            client_for(&mock_server)
                .claim_status()
                .await
                .unwrap()
                .is_claimed
        );
    }
}
