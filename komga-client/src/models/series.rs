use serde::Deserialize;

#[derive(Deserialize, Debug, Default, PartialEq)]
#[serde(rename_all = "camelCase", default)]
pub struct Series {
    pub id: String,
    pub library_id: String,
    /// The series' folder name; [`SeriesMetadata::title`] is the display title.
    pub name: String,
    pub books_count: u32,
    pub metadata: SeriesMetadata,
    pub deleted: bool,
}

#[derive(Deserialize, Debug, Default, PartialEq)]
#[serde(rename_all = "camelCase", default)]
pub struct SeriesMetadata {
    pub title: String,
    pub title_sort: String,
    /// `ONGOING`, `ENDED`, `ABANDONED` or `HIATUS`.
    pub status: String,
    /// `LEFT_TO_RIGHT`, `RIGHT_TO_LEFT`, `VERTICAL`, `WEBTOON`, or empty when
    /// unset.
    pub reading_direction: Option<String>,
}
