pub mod integration;
pub mod job;
pub mod library;
pub mod library_item;
pub mod volume;

#[derive(Debug, Default)]
pub enum AsyncModel<T, E> {
    #[default]
    NotLoaded,
    Loading,
    Loaded(T),
    Error(E),
}

impl<T, E> From<Result<T, E>> for AsyncModel<T, E> {
    fn from(result: Result<T, E>) -> Self {
        match result {
            Ok(value) => AsyncModel::Loaded(value),
            Err(error) => AsyncModel::Error(error),
        }
    }
}
