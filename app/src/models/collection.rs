/// Anything within a source that gathers several series: a Kavita or Komga
/// library, collection or reading list, a Suwayomi category, an OPDS feed, or a
/// local folder. Collections aren't stored. A source lists its own live from the
/// backend (see [`crate::providers::Provider::list_collections`]), so the app
/// only ever handles this provider-agnostic shape.
#[derive(Clone, Debug, PartialEq)]
pub struct Collection {
    /// The collection's provider-defined id. Only the provider that listed it
    /// knows how to read it back (it may encode what kind of collection it is).
    pub external_id: String,
    pub name: String,
    pub kind: CollectionKind,
    /// Whether files can be imported into it (a local folder can grow; a remote
    /// server's collections can't). See
    /// [`crate::providers::Provider::import_files`].
    pub importable: bool,
}

/// What a collection is, which the collections screen groups them by. Only the
/// generic notions are named here, never a backend's own terms. Each kind is
/// only produced by the providers that have it, which a build can leave out
/// (see the crate's cargo features), hence the `dead_code` allowance.
#[allow(dead_code)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum CollectionKind {
    Library,
    Collection,
    ReadingList,
    Category,
    Catalog,
    Folder,
}

impl CollectionKind {
    /// The heading for a group of collections of this kind.
    pub fn group_label(&self) -> &'static str {
        match self {
            CollectionKind::Library => "Libraries",
            CollectionKind::Collection => "Collections",
            CollectionKind::ReadingList => "Reading lists",
            CollectionKind::Category => "Categories",
            CollectionKind::Catalog => "Catalogs",
            CollectionKind::Folder => "Folders",
        }
    }
}

impl Collection {
    pub fn new(
        external_id: impl Into<String>,
        name: impl Into<String>,
        kind: CollectionKind,
    ) -> Self {
        Self {
            external_id: external_id.into(),
            name: name.into(),
            kind,
            importable: false,
        }
    }
}

/// A collection together with the source (integration) it belongs to: all a
/// screen needs to list its series, or to get back to it.
#[derive(Clone, Debug, PartialEq)]
pub struct CollectionRef {
    pub integration_id: String,
    pub collection: Collection,
}
