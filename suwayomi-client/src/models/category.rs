use serde::Deserialize;

/// A library category. The `default` one (id `0`) is virtual: it holds every
/// library manga that isn't in any other category.
#[derive(Deserialize, Debug, Default, PartialEq)]
#[serde(rename_all = "camelCase", default)]
pub struct Category {
    pub id: i32,
    pub name: String,
    pub order: i32,
    pub default: bool,
    pub mangas: MangaCount,
}

#[derive(Deserialize, Debug, Default, PartialEq)]
#[serde(rename_all = "camelCase", default)]
pub struct MangaCount {
    pub total_count: u32,
}
