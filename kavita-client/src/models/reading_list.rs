use serde::Deserialize;

/// A reading list (`ReadingListDto`): an ordered list of chapters, possibly
/// from many series.
#[derive(Deserialize, Debug, Default, PartialEq)]
#[serde(rename_all = "camelCase", default)]
pub struct ReadingList {
    pub id: u32,
    pub title: String,
    pub summary: Option<String>,
    pub promoted: bool,
    pub cover_image: Option<String>,
    pub item_count: u32,
}

/// One chapter of a reading list (`ReadingListItemDto`), in the list's order.
#[derive(Deserialize, Debug, Default, PartialEq)]
#[serde(rename_all = "camelCase", default)]
pub struct ReadingListItem {
    pub id: u32,
    pub order: u32,
    pub chapter_id: u32,
    pub series_id: u16,
    pub series_name: String,
    pub volume_id: u32,
    pub library_id: u16,
    pub title: String,
    pub pages_read: u32,
    pub pages_total: u32,
}
