use serde::Deserialize;

/// A collection of series.
#[derive(Deserialize, Debug, Default, PartialEq)]
#[serde(rename_all = "camelCase", default)]
pub struct Collection {
    pub id: String,
    pub name: String,
    /// Whether the series keep a manual order rather than sorting by title.
    pub ordered: bool,
    pub series_ids: Vec<String>,
}
