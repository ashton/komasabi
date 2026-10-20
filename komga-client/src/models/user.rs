use serde::Deserialize;

#[derive(Deserialize, Debug, Default, PartialEq)]
#[serde(rename_all = "camelCase", default)]
pub struct User {
    pub id: String,
    pub email: String,
    pub roles: Vec<String>,
}

/// Whether the server has been set up with its first admin account, from the
/// anonymous `/api/v1/claim` endpoint.
#[derive(Deserialize, Debug, Default, PartialEq)]
#[serde(rename_all = "camelCase", default)]
pub struct ClaimStatus {
    pub is_claimed: bool,
}
