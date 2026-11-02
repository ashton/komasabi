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
    /// How far the user has read this entry, per the backend's reading
    /// history. Backends that keep none leave every entry [`ReadState::Unread`].
    pub read_state: ReadState,
}

/// Where the user is in a series entry, shown on its card: a read entry's cover
/// is dimmed, and an in-progress one gets a progress bar.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub enum ReadState {
    #[default]
    Unread,
    /// Partly read; the fraction (`0.0..1.0`) of its pages read so far, or
    /// `None` when the entry's page count isn't known.
    InProgress(Option<f32>),
    Read,
}

impl ReadState {
    /// Derives the state from how many of an entry's `pages` have been read.
    /// Reading every page (or more, if the count is stale) counts as read.
    pub fn from_pages_read(pages_read: u32, pages: Option<u32>) -> Self {
        match pages {
            _ if pages_read == 0 => Self::Unread,
            Some(pages) if pages_read >= pages => Self::Read,
            Some(pages) => Self::InProgress(Some(pages_read as f32 / pages as f32)),
            None => Self::InProgress(None),
        }
    }
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

#[cfg(test)]
mod tests {
    use super::ReadState;

    #[test]
    fn read_state_from_pages_read() {
        assert_eq!(ReadState::from_pages_read(0, Some(20)), ReadState::Unread);
        assert_eq!(ReadState::from_pages_read(0, None), ReadState::Unread);
        assert_eq!(
            ReadState::from_pages_read(5, Some(20)),
            ReadState::InProgress(Some(0.25))
        );
        assert_eq!(
            ReadState::from_pages_read(5, None),
            ReadState::InProgress(None)
        );
        assert_eq!(ReadState::from_pages_read(20, Some(20)), ReadState::Read);
        assert_eq!(ReadState::from_pages_read(25, Some(20)), ReadState::Read);
    }
}
