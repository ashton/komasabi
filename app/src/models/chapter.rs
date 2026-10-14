/// A single chapter (or comic issue) within a series, adapted from a
/// provider's wire type into a UI-agnostic shape (see
/// [`crate::adapter::ChapterAdapter`]). Chapters are what the series-detail
/// grid actually renders; they're read from the volumes the provider returns.
#[derive(Clone, Debug, PartialEq)]
pub struct Chapter {
    /// The chapter's provider-side id (a Kavita chapter id, stringified), used
    /// to fetch its cover and as the cache key for it.
    pub external_id: String,
    /// The chapter's number/range within the series (e.g. `"1"`, `"1-5"`).
    pub range: String,
    /// A human-facing title, when the provider has one distinct from the bare
    /// number (otherwise `None`).
    pub title: Option<String>,
    pub is_special: bool,
    pub pages: Option<u16>,
}
