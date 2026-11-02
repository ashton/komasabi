use serde::{Deserialize, Serialize};

/// Where a chapter sits in Kavita's hierarchy, from `/api/Reader/chapter-info`.
/// Only the ids reading progress needs (plus the page count) are decoded; the
/// endpoint returns more (series name, format, page dimensions, …), which is
/// ignored.
#[derive(Deserialize, Debug, Clone, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ChapterInfo {
    pub volume_id: u16,
    pub series_id: u16,
    pub library_id: u16,
    pub pages: u16,
}

/// The body of `POST /api/Reader/progress`: the user has read `page_num` pages
/// of `chapter_id`. Kavita needs the chapter's whole ancestry (volume, series,
/// library) alongside it. A chapter counts as read once `page_num` reaches its
/// page count.
#[derive(Serialize, Debug, Clone, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Progress {
    pub volume_id: u16,
    pub chapter_id: u16,
    pub page_num: u16,
    pub series_id: u16,
    pub library_id: u16,
}
