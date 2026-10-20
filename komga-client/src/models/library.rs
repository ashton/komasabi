use serde::Deserialize;

#[derive(Deserialize, Debug, Default, PartialEq)]
#[serde(rename_all = "camelCase", default)]
pub struct Library {
    pub id: String,
    pub name: String,
    /// The library's root folder on the Komga server.
    pub root: String,
    pub unavailable: bool,
}
