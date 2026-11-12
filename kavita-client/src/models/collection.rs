use serde::Deserialize;

/// A user's collection of series (`AppUserCollectionDto`), what Kavita's UI
/// calls a collection and its series filter calls a "collection tag".
#[derive(Deserialize, Debug, Default, PartialEq)]
#[serde(rename_all = "camelCase", default)]
pub struct KavitaCollection {
    pub id: u32,
    pub title: String,
    pub summary: Option<String>,
    pub promoted: bool,
    pub cover_image: Option<String>,
    pub item_count: u32,
}
