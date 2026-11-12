pub mod browse;
pub mod collections;
pub mod home;
pub mod integration;
pub mod library;
pub mod series_detail;
pub mod settings;

pub use browse::Browse;
pub use collections::Collections;
pub use home::Home;
pub use integration::Integrations;
pub use library::Library;
pub use series_detail::SeriesDetail;
pub use settings::Settings;

pub enum Screen {
    Home(Home),
    /// The user's own Library.
    Library(Library),
    /// The list of sources (integrations), and the forms that add one.
    Integrations(Integrations),
    /// One source's collections.
    Collections(Collections),
    /// One collection's series.
    Browse(Browse),
    SeriesDetail(SeriesDetail),
    Settings(Settings),
}
