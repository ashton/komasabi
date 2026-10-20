use serde::Deserialize;

/// A Komga book: one file (CBZ/CBR/PDF/EPUB) of a series.
#[derive(Deserialize, Debug, Default, PartialEq)]
#[serde(rename_all = "camelCase", default)]
pub struct Book {
    pub id: String,
    pub series_id: String,
    pub library_id: String,
    /// The file name without extension; [`BookMetadata::title`] is the display
    /// title.
    pub name: String,
    /// The book's 1-based position in its series, as Komga sorted it on disk.
    pub number: u32,
    pub size_bytes: u64,
    pub media: Media,
    pub metadata: BookMetadata,
    pub deleted: bool,
}

#[derive(Deserialize, Debug, Default, PartialEq)]
#[serde(rename_all = "camelCase", default)]
pub struct Media {
    /// `READY` once Komga has analyzed the file; `UNKNOWN`, `ERROR`,
    /// `UNSUPPORTED` or `OUTDATED` otherwise.
    pub status: String,
    pub media_type: String,
    pub pages_count: u32,
    /// `DIVINA` (image pages: comic archives), `PDF` or `EPUB`.
    pub media_profile: String,
    /// Whether an EPUB is made of full-page images and can be read page by
    /// page like a comic.
    pub epub_divina_compatible: bool,
}

#[derive(Deserialize, Debug, Default, PartialEq)]
#[serde(rename_all = "camelCase", default)]
pub struct BookMetadata {
    pub title: String,
    /// The book's number as displayed (e.g. "1", "1.5"), a string in Komga.
    pub number: String,
    pub number_sort: f32,
}

/// One page of a book, as listed by `/api/v1/books/{id}/pages`.
#[derive(Deserialize, Debug, Default, PartialEq)]
#[serde(rename_all = "camelCase", default)]
pub struct BookPage {
    /// The page's 1-based number, as the page endpoint expects it.
    pub number: u32,
    pub file_name: String,
    pub media_type: String,
    pub width: Option<u32>,
    pub height: Option<u32>,
    pub size_bytes: Option<u64>,
}
