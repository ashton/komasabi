use serde::Deserialize;

#[derive(Deserialize, Debug, Default, PartialEq)]
#[serde(rename_all = "camelCase", default)]
pub struct Manga {
    pub id: i32,
    pub title: String,
    /// A server-relative path to the cover image, when the manga has one.
    pub thumbnail_url: Option<String>,
    pub in_library: bool,
}
