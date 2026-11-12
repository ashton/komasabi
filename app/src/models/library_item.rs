/// A series in the user's own Library (see [`super::library::Library`]),
/// kept apart from [`super::collection_item::CollectionItem`], which is a
/// series as a source lists it.
#[allow(dead_code)] // Nothing fills the Library yet.
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
