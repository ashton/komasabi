/// One card in a collection's grid: a series, as any source lists it. Providers
/// adapt their own wire types into this, so screens never see a backend's
/// models.
#[derive(Clone, Debug, PartialEq)]
pub struct CollectionItem {
    /// The item's provider-side id (a Kavita series id, stringified, a Komga
    /// series id, …), used to fetch its cover and entries, and as the cache key
    /// for them.
    pub external_id: String,
    pub name: String,
    pub cover: String,
    pub pages: Option<u16>,
}
