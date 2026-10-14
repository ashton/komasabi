use std::collections::{HashMap, HashSet};

use iced::widget::image;
use iced::{Element, Task};

use crate::models::{AsyncModel, integration::IntegrationType, volume::Volume};
use crate::providers::kavita::provider::KavitaProvider;
use crate::repository::{integration as integration_repository, library as library_repository};

/// The two tabs shown above the volume grid. `Details` carries no data of its
/// own — it just renders the series' metadata in list form, as a counterpart
/// to the quick stats already in the header.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tab {
    Volumes,
    Details,
}

#[derive(Debug)]
pub struct SeriesDetail {
    series_name: String,
    series_pages: Option<u16>,
    /// The external id of the library the series belongs to, used to resolve
    /// the integration to fetch volumes and covers through.
    library_external_id: String,
    /// The series' own (large, header) cover, once it has loaded.
    series_cover: Option<image::Handle>,
    volumes: AsyncModel<Vec<Volume>, String>,
    /// Decoded volume cover images, keyed by volume `external_id`.
    covers: HashMap<String, image::Handle>,
    /// `external_id`s whose cover is currently being fetched, so it isn't
    /// requested more than once.
    covers_in_flight: HashSet<String>,
    active_tab: Tab,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Message {
    VolumesLoaded(Result<Vec<Volume>, String>),
    SeriesCoverLoaded(Option<Vec<u8>>),
    /// Cover image bytes that have been loaded, keyed by volume `external_id`.
    /// Failed/omitted ids are simply absent.
    VolumeCoversLoaded(Vec<(String, Vec<u8>)>),
    TabSelected(Tab),
    BackPressed,
}

#[derive(Debug)]
pub enum Action {
    None,
    Run(Task<Message>),
    /// Navigate back to the browse screen of the carried library.
    Back(String),
}

impl SeriesDetail {
    pub fn new(
        series_external_id: String,
        series_name: String,
        series_pages: Option<u16>,
        library_external_id: String,
    ) -> (Self, Task<Message>) {
        let volumes_task = Task::perform(
            crate::runtime::on_tokio(fetch_volumes(
                library_external_id.clone(),
                series_external_id.clone(),
            )),
            Message::VolumesLoaded,
        );

        let cover_task = Task::perform(
            crate::runtime::on_tokio(fetch_series_cover(
                library_external_id.clone(),
                series_external_id.clone(),
            )),
            Message::SeriesCoverLoaded,
        );

        (
            Self {
                series_name,
                series_pages,
                library_external_id,
                series_cover: None,
                volumes: AsyncModel::Loading,
                covers: HashMap::new(),
                covers_in_flight: HashSet::new(),
                active_tab: Tab::Volumes,
            },
            Task::batch([volumes_task, cover_task]),
        )
    }

    pub fn update(&mut self, message: Message) -> Action {
        match message {
            Message::VolumesLoaded(result) => {
                let loaded = result.is_ok();
                self.volumes = result.into();

                if loaded {
                    match self.request_volume_covers() {
                        Some(task) => Action::Run(task),
                        None => Action::None,
                    }
                } else {
                    Action::None
                }
            }

            Message::SeriesCoverLoaded(Some(bytes)) => {
                self.series_cover = Some(image::Handle::from_bytes(bytes));
                Action::None
            }
            Message::SeriesCoverLoaded(None) => Action::None,

            Message::VolumeCoversLoaded(covers) => {
                for (external_id, bytes) in covers {
                    self.covers_in_flight.remove(&external_id);
                    self.covers
                        .insert(external_id, image::Handle::from_bytes(bytes));
                }

                Action::None
            }

            Message::TabSelected(tab) => {
                self.active_tab = tab;
                Action::None
            }

            Message::BackPressed => Action::Back(self.library_external_id.clone()),
        }
    }

    pub fn view<'a>(&'a self) -> Element<'a, Message> {
        view_helper::view(
            &self.series_name,
            self.series_pages,
            self.series_cover.as_ref(),
            &self.volumes,
            &self.covers,
            self.active_tab,
        )
    }

    /// Builds a task to fetch covers for every loaded volume whose cover isn't
    /// already cached or in flight, marking them in flight. Returns `None`
    /// when there's nothing left to fetch.
    fn request_volume_covers(&mut self) -> Option<Task<Message>> {
        let AsyncModel::Loaded(volumes) = &self.volumes else {
            return None;
        };

        let volume_ids: Vec<String> = volumes
            .iter()
            .map(|volume| volume.external_id.clone())
            .filter(|id| !self.covers.contains_key(id) && !self.covers_in_flight.contains(id))
            .collect();

        if volume_ids.is_empty() {
            return None;
        }

        self.covers_in_flight.extend(volume_ids.iter().cloned());

        Some(Task::perform(
            crate::runtime::on_tokio(fetch_volume_covers(
                self.library_external_id.clone(),
                volume_ids,
            )),
            Message::VolumeCoversLoaded,
        ))
    }
}

/// Resolves the Kavita provider (and its integration id) for the library with
/// `library_external_id`, authenticating against the stored credentials.
async fn kavita_provider_for_library(
    library_external_id: &str,
) -> Result<(KavitaProvider, String), String> {
    let library = library_repository::find_by_external_id(library_external_id).await?;
    let integration_id = library
        .integration_id
        .ok_or_else(|| "library has no associated integration".to_string())?;
    let integration = integration_repository::find(&integration_id).await?;

    match integration.integration_type {
        IntegrationType::Kavita => {
            let url = integration
                .url
                .ok_or_else(|| "integration is missing a url".to_string())?;
            let api_key = integration
                .api_key
                .ok_or_else(|| "integration is missing an api key".to_string())?;

            let provider = KavitaProvider::authenticate(url, api_key).await?;
            Ok((provider, integration_id))
        }
        _ => Err("this integration type is not supported yet".to_string()),
    }
}

async fn fetch_volumes(
    library_external_id: String,
    series_external_id: String,
) -> Result<Vec<Volume>, String> {
    let series_id = series_external_id
        .parse::<u16>()
        .map_err(|_| format!("series id {series_external_id} is not a valid Kavita id"))?;
    let (provider, _integration_id) = kavita_provider_for_library(&library_external_id).await?;
    provider.list_volumes(series_id).await
}

/// Fetches the series' own cover, returning `None` on any error (a missing
/// header cover just renders the placeholder box rather than failing the page).
async fn fetch_series_cover(
    library_external_id: String,
    series_external_id: String,
) -> Option<Vec<u8>> {
    let series_id = series_external_id.parse::<u16>().ok()?;
    let (provider, integration_id) = kavita_provider_for_library(&library_external_id)
        .await
        .ok()?;
    provider.cover(&integration_id, series_id).await.ok()
}

/// Fetches covers for the given volumes, returning `(external_id, bytes)` for
/// each one that resolved. Per-id and whole-batch failures are logged and
/// omitted rather than surfaced, so a partial result still renders what it can.
async fn fetch_volume_covers(
    library_external_id: String,
    volume_ids: Vec<String>,
) -> Vec<(String, Vec<u8>)> {
    match load_volume_covers(library_external_id, volume_ids).await {
        Ok(covers) => covers,
        Err(error) => {
            tracing::debug!("failed to load volume covers: {error}");
            Vec::new()
        }
    }
}

async fn load_volume_covers(
    library_external_id: String,
    volume_ids: Vec<String>,
) -> Result<Vec<(String, Vec<u8>)>, String> {
    let (provider, integration_id) = kavita_provider_for_library(&library_external_id).await?;

    let mut covers = Vec::new();
    for volume_id in volume_ids {
        let Ok(parsed) = volume_id.parse::<u16>() else {
            tracing::debug!("skipping cover for non-numeric volume id {volume_id}");
            continue;
        };

        match provider.volume_cover(&integration_id, parsed).await {
            Ok(bytes) => covers.push((volume_id, bytes)),
            Err(error) => {
                tracing::debug!("failed to fetch cover for volume {volume_id}: {error}")
            }
        }
    }

    Ok(covers)
}

mod view_helper {
    use std::collections::HashMap;

    use iced::{
        Background, Border, Center, ContentFit, Element, Font,
        Length::{Fill, Fixed},
        Theme,
        widget::{button, column, container, grid, image, row, scrollable, text},
    };

    use super::Tab;
    use crate::models::{AsyncModel, volume::Volume};

    type Covers = HashMap<String, image::Handle>;

    /// Width of the large header cover, in pixels (height follows the 2:3
    /// comic aspect ratio).
    const HEADER_COVER_WIDTH: f32 = 200.0;

    pub fn view<'a>(
        series_name: &'a str,
        series_pages: Option<u16>,
        series_cover: Option<&'a image::Handle>,
        volumes: &'a AsyncModel<Vec<Volume>, String>,
        covers: &'a Covers,
        active_tab: Tab,
    ) -> Element<'a, super::Message> {
        let volume_count = match volumes {
            AsyncModel::Loaded(volumes) => Some(volumes.len()),
            _ => None,
        };

        let content = column![
            header(series_name, series_pages, volume_count, series_cover),
            tab_bar(active_tab),
            tab_content(active_tab, series_name, series_pages, volumes, covers),
        ]
        .spacing(24)
        .padding(24)
        .width(Fill);

        column![
            top_bar(),
            scrollable(content).width(Fill).height(Fill),
        ]
        .width(Fill)
        .height(Fill)
        .into()
    }

    fn top_bar<'a>() -> Element<'a, super::Message> {
        container(button("Back").on_press(super::Message::BackPressed))
            .padding(20)
            .into()
    }

    fn header<'a>(
        series_name: &'a str,
        series_pages: Option<u16>,
        volume_count: Option<usize>,
        series_cover: Option<&'a image::Handle>,
    ) -> Element<'a, super::Message> {
        row![
            header_cover(series_cover),
            column![
                text(series_name).size(28).font(Font {
                    weight: iced::font::Weight::Bold,
                    ..Font::default()
                }),
                stats(series_pages, volume_count),
            ]
            .spacing(16)
            .width(Fill),
        ]
        .spacing(24)
        .into()
    }

    fn stats<'a>(
        series_pages: Option<u16>,
        volume_count: Option<usize>,
    ) -> Element<'a, super::Message> {
        let mut chips = row![].spacing(8);

        if let Some(count) = volume_count {
            chips = chips.push(stat_chip(format!(
                "{count} {}",
                if count == 1 { "volume" } else { "volumes" }
            )));
        }

        if let Some(pages) = series_pages {
            chips = chips.push(stat_chip(format!("{pages} pages")));
        }

        chips.into()
    }

    fn stat_chip<'a>(label: String) -> Element<'a, super::Message> {
        container(text(label).size(13))
            .padding([4, 10])
            .style(|theme: &Theme| {
                let palette = theme.extended_palette();

                container::Style {
                    background: Some(Background::Color(palette.secondary.weak.color)),
                    text_color: Some(palette.secondary.weak.text),
                    border: Border {
                        radius: 999.0.into(),
                        ..Border::default()
                    },
                    ..container::Style::default()
                }
            })
            .into()
    }

    fn header_cover<'a>(handle: Option<&image::Handle>) -> Element<'a, super::Message> {
        container(cover_image(handle))
            .width(Fixed(HEADER_COVER_WIDTH))
            .height(Fixed(HEADER_COVER_WIDTH * 3.0 / 2.0))
            .clip(true)
            .style(cover_box_style)
            .into()
    }

    fn tab_bar<'a>(active_tab: Tab) -> Element<'a, super::Message> {
        row![
            tab_button("Volumes", Tab::Volumes, active_tab),
            tab_button("Details", Tab::Details, active_tab),
        ]
        .spacing(8)
        .into()
    }

    fn tab_button<'a>(
        label: &'a str,
        tab: Tab,
        active_tab: Tab,
    ) -> Element<'a, super::Message> {
        let is_active = tab == active_tab;

        button(text(label).size(15))
            .on_press(super::Message::TabSelected(tab))
            .padding([6, 12])
            .style(move |theme: &Theme, status| {
                let palette = theme.extended_palette();
                let mut style = button::text(theme, status);

                if is_active {
                    style.background =
                        Some(Background::Color(palette.background.weak.color));
                    style.text_color = palette.primary.strong.color;
                    style.border = Border {
                        radius: 6.0.into(),
                        ..Border::default()
                    };
                }

                style
            })
            .into()
    }

    fn tab_content<'a>(
        active_tab: Tab,
        series_name: &'a str,
        series_pages: Option<u16>,
        volumes: &'a AsyncModel<Vec<Volume>, String>,
        covers: &'a Covers,
    ) -> Element<'a, super::Message> {
        match active_tab {
            Tab::Volumes => volumes_body(volumes, covers),
            Tab::Details => details_body(series_name, series_pages, volumes),
        }
    }

    fn volumes_body<'a>(
        volumes: &'a AsyncModel<Vec<Volume>, String>,
        covers: &'a Covers,
    ) -> Element<'a, super::Message> {
        match volumes {
            AsyncModel::Loaded(volumes) if volumes.is_empty() => empty_state(),
            AsyncModel::Loaded(volumes) => volumes_grid(volumes, covers),
            AsyncModel::Error(error) => error_state(error),
            AsyncModel::NotLoaded | AsyncModel::Loading => loading_state(),
        }
    }

    fn volumes_grid<'a>(
        volumes: &'a [Volume],
        covers: &'a Covers,
    ) -> Element<'a, super::Message> {
        let cards = volumes
            .iter()
            .map(|volume| volume_card(volume, covers.get(&volume.external_id)));

        container(
            grid(cards)
                .fluid(160.0)
                .spacing(16.0)
                .height(grid::aspect_ratio(2.0, 3.0)),
        )
        .width(Fill)
        .into()
    }

    fn volume_card<'a>(
        volume: &'a Volume,
        cover: Option<&image::Handle>,
    ) -> Element<'a, super::Message> {
        column![
            container(cover_image(cover))
                .width(Fill)
                .height(Fill)
                .clip(true)
                .style(cover_box_style),
            text(volume_label(&volume.name)).size(13),
            pages_label(volume.pages),
        ]
        .width(Fill)
        .spacing(4)
        .align_x(Center)
        .into()
    }

    /// Kavita volume names are usually just the number (e.g. `"1"`); prefix
    /// them so the card reads "Volume 1" rather than a bare "1". Non-numeric
    /// or empty names (specials, loose chapters) are shown as-is, falling back
    /// to a plain "Volume" when blank.
    fn volume_label(name: &str) -> String {
        let name = name.trim();

        if name.is_empty() {
            "Volume".to_string()
        } else if name.chars().all(|c| c.is_ascii_digit() || c == '.') {
            format!("Volume {name}")
        } else {
            name.to_string()
        }
    }

    /// Renders a cover if its handle has loaded, otherwise nothing (the styled
    /// box around it stands in as the placeholder).
    fn cover_image<'a>(handle: Option<&image::Handle>) -> Element<'a, super::Message> {
        match handle {
            Some(handle) => image(handle.clone())
                .width(Fill)
                .height(Fill)
                .content_fit(ContentFit::Cover)
                .into(),
            None => row![].into(),
        }
    }

    fn cover_box_style(theme: &Theme) -> container::Style {
        container::Style {
            background: Some(Background::Color(
                theme.extended_palette().background.strong.color,
            )),
            border: Border {
                radius: 8.0.into(),
                ..Border::default()
            },
            ..container::Style::default()
        }
    }

    fn pages_label<'a>(pages: Option<u16>) -> Element<'a, super::Message> {
        match pages {
            Some(pages) => text(format!("{pages} pages"))
                .size(11)
                .style(text::secondary)
                .into(),
            None => row![].into(),
        }
    }

    fn details_body<'a>(
        series_name: &'a str,
        series_pages: Option<u16>,
        volumes: &'a AsyncModel<Vec<Volume>, String>,
    ) -> Element<'a, super::Message> {
        let volume_count = match volumes {
            AsyncModel::Loaded(volumes) => volumes.len().to_string(),
            AsyncModel::Loading | AsyncModel::NotLoaded => "—".to_string(),
            AsyncModel::Error(_) => "—".to_string(),
        };

        let pages = series_pages
            .map(|pages| pages.to_string())
            .unwrap_or_else(|| "—".to_string());

        column![
            detail_row("Title", series_name.to_string()),
            detail_row("Volumes", volume_count),
            detail_row("Pages", pages),
        ]
        .spacing(12)
        .width(Fill)
        .into()
    }

    fn detail_row<'a>(label: &'a str, value: String) -> Element<'a, super::Message> {
        column![
            text(label).size(13).style(text::secondary),
            text(value).size(15),
        ]
        .spacing(2)
        .into()
    }

    fn empty_state<'a>() -> Element<'a, super::Message> {
        container(text("Nenhum volume encontrado nesta série"))
            .center_x(Fill)
            .padding(40)
            .into()
    }

    fn loading_state<'a>() -> Element<'a, super::Message> {
        container(row![]).padding(40).into()
    }

    fn error_state<'a>(error: &'a str) -> Element<'a, super::Message> {
        container(text(error)).center_x(Fill).padding(40).into()
    }
}

#[cfg(test)]
mod tests {
    use iced_test::simulator;

    use super::*;

    fn volume(name: &str, pages: Option<u16>) -> Volume {
        Volume {
            external_id: name.to_string(),
            name: name.to_string(),
            series_id: "1".to_string(),
            pages,
        }
    }

    fn series_detail(volumes: AsyncModel<Vec<Volume>, String>) -> SeriesDetail {
        SeriesDetail {
            series_name: "Beneath the Trees".to_string(),
            series_pages: Some(183),
            library_external_id: "lib-1".to_string(),
            series_cover: None,
            volumes,
            covers: HashMap::new(),
            covers_in_flight: HashSet::new(),
            active_tab: Tab::Volumes,
        }
    }

    #[test]
    fn new_starts_in_loading_with_the_volumes_tab_active() {
        let (detail, _task) = SeriesDetail::new(
            "1".to_string(),
            "One Piece".to_string(),
            Some(42),
            "lib-1".to_string(),
        );

        assert!(matches!(detail.volumes, AsyncModel::Loading));
        assert_eq!(detail.active_tab, Tab::Volumes);
        assert!(detail.series_cover.is_none());
    }

    #[test]
    fn volumes_loaded_ok_stores_volumes_and_requests_covers() {
        let mut detail = series_detail(AsyncModel::Loading);

        let action = detail.update(Message::VolumesLoaded(Ok(vec![volume("1", Some(24))])));

        assert!(matches!(action, Action::Run(_)));
        assert!(matches!(detail.volumes, AsyncModel::Loaded(ref v) if v.len() == 1));
        assert!(detail.covers_in_flight.contains("1"));
    }

    #[test]
    fn volumes_loaded_empty_does_not_request_covers() {
        let mut detail = series_detail(AsyncModel::Loading);

        let action = detail.update(Message::VolumesLoaded(Ok(vec![])));

        assert!(matches!(action, Action::None));
    }

    #[test]
    fn volumes_loaded_error_stores_the_error() {
        let mut detail = series_detail(AsyncModel::Loading);

        let action = detail.update(Message::VolumesLoaded(Err("boom".to_string())));

        assert!(matches!(action, Action::None));
        assert!(matches!(detail.volumes, AsyncModel::Error(ref e) if e == "boom"));
    }

    #[test]
    fn series_cover_loaded_stores_the_handle() {
        let mut detail = series_detail(AsyncModel::Loading);

        let action = detail.update(Message::SeriesCoverLoaded(Some(vec![1, 2, 3, 4])));

        assert!(matches!(action, Action::None));
        assert!(detail.series_cover.is_some());
    }

    #[test]
    fn series_cover_loaded_none_leaves_the_cover_empty() {
        let mut detail = series_detail(AsyncModel::Loading);

        let action = detail.update(Message::SeriesCoverLoaded(None));

        assert!(matches!(action, Action::None));
        assert!(detail.series_cover.is_none());
    }

    #[test]
    fn volume_covers_loaded_inserts_handles_and_clears_in_flight() {
        let mut detail = series_detail(AsyncModel::Loaded(vec![volume("1", None)]));
        detail.covers_in_flight.insert("1".to_string());

        let action = detail.update(Message::VolumeCoversLoaded(vec![(
            "1".to_string(),
            vec![1, 2, 3, 4],
        )]));

        assert!(matches!(action, Action::None));
        assert!(detail.covers.contains_key("1"));
        assert!(!detail.covers_in_flight.contains("1"));
    }

    #[test]
    fn request_volume_covers_skips_cached_and_in_flight_volumes() {
        let mut detail = series_detail(AsyncModel::Loaded(vec![
            volume("cached", None),
            volume("in-flight", None),
            volume("fresh", None),
        ]));
        detail
            .covers
            .insert("cached".to_string(), image::Handle::from_bytes(vec![0]));
        detail.covers_in_flight.insert("in-flight".to_string());

        let task = detail.request_volume_covers();

        assert!(task.is_some());
        assert!(detail.covers_in_flight.contains("fresh"));
        assert!(detail.covers_in_flight.contains("in-flight"));
        assert!(!detail.covers_in_flight.contains("cached"));
    }

    #[test]
    fn tab_selected_switches_the_active_tab() {
        let mut detail = series_detail(AsyncModel::Loaded(vec![]));

        let action = detail.update(Message::TabSelected(Tab::Details));

        assert!(matches!(action, Action::None));
        assert_eq!(detail.active_tab, Tab::Details);
    }

    #[test]
    fn back_pressed_returns_back_with_the_library_id() {
        let mut detail = series_detail(AsyncModel::Loading);

        let action = detail.update(Message::BackPressed);

        assert!(matches!(action, Action::Back(id) if id == "lib-1"));
    }

    #[test]
    fn view_renders_the_series_name_and_volume_cards() {
        let detail = series_detail(AsyncModel::Loaded(vec![volume("1", Some(24))]));

        let mut ui = simulator(detail.view());

        assert!(ui.find("Beneath the Trees").is_ok());
        assert!(ui.find("Volume 1").is_ok());
        assert!(ui.find("24 pages").is_ok());
    }

    #[test]
    fn view_renders_the_volume_count_in_the_header() {
        let detail = series_detail(AsyncModel::Loaded(vec![
            volume("1", None),
            volume("2", None),
        ]));

        let mut ui = simulator(detail.view());

        assert!(ui.find("2 volumes").is_ok());
    }

    #[test]
    fn view_renders_the_empty_state_when_there_are_no_volumes() {
        let detail = series_detail(AsyncModel::Loaded(vec![]));

        let mut ui = simulator(detail.view());

        assert!(ui.find("Nenhum volume encontrado nesta série").is_ok());
    }

    #[test]
    fn view_renders_the_error_state_when_loading_failed() {
        let detail = series_detail(AsyncModel::Error("boom".to_string()));

        let mut ui = simulator(detail.view());

        assert!(ui.find("boom").is_ok());
    }

    #[test]
    fn clicking_back_sends_back_pressed_message() {
        let detail = series_detail(AsyncModel::Loading);

        let mut ui = simulator(detail.view());
        let _ = ui.click("Back").expect("Back button should be found");

        let messages: Vec<_> = ui.into_messages().collect();

        assert_eq!(messages, vec![Message::BackPressed]);
    }

    #[test]
    fn clicking_the_details_tab_sends_tab_selected() {
        let detail = series_detail(AsyncModel::Loaded(vec![]));

        let mut ui = simulator(detail.view());
        let _ = ui.click("Details").expect("Details tab should be found");

        let messages: Vec<_> = ui.into_messages().collect();

        assert_eq!(messages, vec![Message::TabSelected(Tab::Details)]);
    }

    #[test]
    fn details_tab_lists_the_series_metadata() {
        let mut detail = series_detail(AsyncModel::Loaded(vec![volume("1", None)]));
        detail.active_tab = Tab::Details;

        let mut ui = simulator(detail.view());

        assert!(ui.find("Title").is_ok());
        assert!(ui.find("Volumes").is_ok());
        assert!(ui.find("Pages").is_ok());
    }
}
