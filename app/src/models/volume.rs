/// A single volume within a series, adapted from a provider's wire type into a
/// UI-agnostic shape (see [`crate::adapter::VolumeAdapter`]).
#[derive(Clone, Debug, PartialEq)]
pub struct Volume {
    /// The volume's provider-side id (a Kavita volume id, stringified), used to
    /// fetch its cover and as the cache key for it.
    pub external_id: String,
    pub name: String,
    pub series_id: String,
    pub pages: Option<u16>,
}
