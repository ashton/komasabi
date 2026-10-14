pub mod browse;
pub mod home;
pub mod integration;
pub mod library;
pub mod series_detail;
pub mod settings;

pub use browse::Browse;
pub use home::Home;
pub use integration::Integrations;
pub use library::Library;
pub use series_detail::SeriesDetail;
pub use settings::Settings;

pub enum Screen {
    Home(Home),
    Library(Library),
    Browse(Browse),
    SeriesDetail(SeriesDetail),
    Integrations(Integrations),
    Settings(Settings),
}
