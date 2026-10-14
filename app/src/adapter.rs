use crate::models::{library::Library, library_item::LibraryItem, volume::Volume};

pub trait ItemAdapter<T> {
    fn adapt_item(source: T) -> LibraryItem;
}

pub trait LibraryAdapter<T> {
    fn adapt_library(source: T, integration_id: &str) -> Library;
}

pub trait VolumeAdapter<T> {
    fn adapt_volume(source: T) -> Volume;
}
