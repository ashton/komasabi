use crate::models::{collection::Collection, collection_item::CollectionItem};

/// Converts a backend's series type into this app's [`CollectionItem`].
pub trait ItemAdapter<T> {
    fn adapt_item(source: T) -> CollectionItem;
}

/// Converts a backend's grouping of series (a library, collection, reading list,
/// category, feed, …) into this app's [`Collection`].
pub trait CollectionAdapter<T> {
    fn adapt_collection(source: T) -> Collection;
}
