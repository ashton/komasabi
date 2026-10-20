use serde::Deserialize;

#[derive(Deserialize, Debug, Default, PartialEq)]
#[serde(rename_all = "camelCase", default)]
pub struct Chapter {
    /// Server-wide chapter id, used by GraphQL.
    pub id: i32,
    pub manga_id: i32,
    pub name: String,
    pub chapter_number: f64,
    /// The chapter's 1-based position within its manga, in reading order. The
    /// image endpoints address a chapter by this, not by `id`.
    pub source_order: i32,
    /// `-1` until the chapter's pages have been fetched from its source.
    pub page_count: i32,
    pub is_read: bool,
    pub is_downloaded: bool,
}

/// The result of resolving a chapter's pages (`fetchChapterPages`).
#[derive(Deserialize, Debug, Default, PartialEq)]
#[serde(rename_all = "camelCase", default)]
pub struct ChapterPages {
    /// Server-relative paths to each page image, in reading order.
    pub pages: Vec<String>,
    pub chapter: Chapter,
}
