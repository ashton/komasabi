use serde::Deserialize;

/// One page of a paginated Komga listing (Spring's `Page` shape). Komga's
/// page numbers are 0-based.
#[derive(Deserialize, Debug, Default, PartialEq)]
#[serde(rename_all = "camelCase", default)]
pub struct Page<T> {
    pub content: Vec<T>,
    pub number: u32,
    pub size: u32,
    pub total_pages: u32,
    pub total_elements: u64,
    pub first: bool,
    pub last: bool,
    pub empty: bool,
}
