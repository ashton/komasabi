use serde::Deserialize;

/// What the server reports about itself (`aboutServer`).
#[derive(Deserialize, Debug, Default, PartialEq)]
#[serde(rename_all = "camelCase", default)]
pub struct AboutServer {
    pub name: String,
    pub version: String,
}
