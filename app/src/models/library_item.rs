#[derive(Clone, Debug, PartialEq)]
pub struct LibraryItem {
    /// The item's provider-side id (a Kavita series id, stringified), used to
    /// fetch its cover and as the cache key for it.
    pub external_id: String,
    pub name: String,
    pub cover: String,
    pub library_id: String,
    pub pages: Option<u16>,
}
