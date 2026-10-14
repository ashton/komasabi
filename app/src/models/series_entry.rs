/// One card in the series-detail grid. A series is displayed either as its
/// numbered **volumes** or, when it has none, as the individual **chapters**
/// (issues) of its loose-leaf volume — see
/// [`crate::providers::kavita::adapter::series_entries`] for the heuristic. An
/// entry abstracts over which it is, carrying only what the grid renders plus
/// the `kind` needed to fetch the right kind of cover.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EntryKind {
    Volume,
    Chapter,
}

#[derive(Clone, Debug, PartialEq)]
pub struct SeriesEntry {
    /// The entry's provider-side id (a Kavita volume *or* chapter id,
    /// stringified, per `kind`), used to fetch its cover and as the cache key.
    pub external_id: String,
    pub kind: EntryKind,
    /// Pre-resolved display label (e.g. "Volume 1", "#3", "Special").
    pub label: String,
    pub pages: Option<u16>,
}

impl SeriesEntry {
    /// Key under which this entry's cover is cached in memory. Volume and
    /// chapter ids come from separate Kavita id spaces and can coincide, so the
    /// kind is folded in to keep a volume's cover from shadowing a chapter's.
    pub fn cover_key(&self) -> String {
        cover_key(self.kind, &self.external_id)
    }
}

/// Builds a [`SeriesEntry::cover_key`] from its parts, for callers that only
/// have the kind and id (e.g. after an async cover fetch).
pub fn cover_key(kind: EntryKind, external_id: &str) -> String {
    let tag = match kind {
        EntryKind::Volume => "vol",
        EntryKind::Chapter => "ch",
    };
    format!("{tag}:{external_id}")
}
