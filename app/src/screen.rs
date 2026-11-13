mod back_button;
pub mod browse;
mod card;
pub mod collections;
mod cover_overlay;
pub mod downloads;
pub mod home;
pub mod integration;
pub mod series_detail;
pub mod settings;

pub use browse::Browse;
pub use collections::Collections;
pub use downloads::Downloads;
pub use home::Home;
pub use integration::Integrations;
pub use series_detail::SeriesDetail;
pub use settings::Settings;

pub enum Screen {
    /// The user's Library: the series of the items they've added.
    Home(Home),
    /// The list of sources (integrations), and the forms that add one.
    Integrations(Integrations),
    /// One source's collections.
    Collections(Collections),
    /// One collection's series.
    Browse(Browse),
    SeriesDetail(SeriesDetail),
    /// Every downloaded volume/chapter.
    Downloads(Downloads),
    Settings(Settings),
}
