use std::collections::{HashMap, HashSet};

use iced::widget::image;
use iced::{Element, Task};

use crate::models::{
    AsyncModel,
    series_entry::{EntryKind, SeriesEntry, cover_key},
};
use crate::providers::{Provider, provider_for_library};

/// The two tabs shown above the contents grid. `Details` carries no data of its
/// own — it just renders the series' metadata in list form, as a counterpart
/// to the quick stats already in the header.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tab {
    Contents,
    Details,
}

#[derive(Debug)]
pub struct SeriesDetail {
    series_name: String,
    series_pages: Option<u16>,
    /// The external id of the library the series belongs to, used to resolve
    /// the integration to fetch entries and covers through.
    library_external_id: String,
    /// The series' own (large, header) cover, once it has loaded.
    series_cover: Option<image::Handle>,
    entries: AsyncModel<Vec<SeriesEntry>, String>,
    /// Decoded entry cover images, keyed by [`SeriesEntry::cover_key`].
    covers: HashMap<String, image::Handle>,
    /// cover keys whose cover is currently being fetched, so it isn't requested
    /// more than once.
    covers_in_flight: HashSet<String>,
    active_tab: Tab,
    /// Whether an entry is currently being prepared and opened in the reader.
    /// Guards against double-launching while pages are still downloading.
    launching: bool,
    /// The last reader-launch failure, shown as a status banner until the next
    /// attempt.
    launch_error: Option<String>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Message {
    EntriesLoaded(Result<Vec<SeriesEntry>, String>),
    SeriesCoverLoaded(Option<Vec<u8>>),
    /// Cover image bytes that have been loaded, keyed by [`SeriesEntry::cover_key`].
    /// Failed/omitted entries are simply absent.
    CoversLoaded(Vec<(String, Vec<u8>)>),
    TabSelected(Tab),
    BackPressed,
    /// A volume/chapter card was clicked; prepare it and open it in the reader.
    EntrySelected(SeriesEntry),
    /// The reader-launch task finished (the reader window is open, or it failed).
    ReaderLaunched(Result<(), String>),
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
        let entries_task = Task::perform(
            crate::runtime::on_tokio(fetch_entries(
                library_external_id.clone(),
                series_external_id.clone(),
            )),
            Message::EntriesLoaded,
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
                entries: AsyncModel::Loading,
                covers: HashMap::new(),
                covers_in_flight: HashSet::new(),
                active_tab: Tab::Contents,
                launching: false,
                launch_error: None,
            },
            Task::batch([entries_task, cover_task]),
        )
    }

    pub fn update(&mut self, message: Message) -> Action {
        match message {
            Message::EntriesLoaded(result) => {
                let loaded = result.is_ok();
                self.entries = result.into();

                if loaded {
                    match self.request_entry_covers() {
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

            Message::CoversLoaded(covers) => {
                for (key, bytes) in covers {
                    self.covers_in_flight.remove(&key);
                    self.covers.insert(key, image::Handle::from_bytes(bytes));
                }

                Action::None
            }

            Message::TabSelected(tab) => {
                self.active_tab = tab;
                Action::None
            }

            Message::BackPressed => Action::Back(self.library_external_id.clone()),

            Message::EntrySelected(entry) => {
                // Ignore clicks while a launch is already in flight; the reader
                // for the previous click is still being prepared.
                if self.launching {
                    return Action::None;
                }

                self.launching = true;
                self.launch_error = None;

                Action::Run(Task::perform(
                    crate::runtime::on_tokio(crate::reader_launch::prepare_and_launch(
                        self.library_external_id.clone(),
                        self.series_name.clone(),
                        entry,
                    )),
                    Message::ReaderLaunched,
                ))
            }

            Message::ReaderLaunched(result) => {
                self.launching = false;
                if let Err(error) = result {
                    tracing::debug!("failed to open reader: {error}");
                    self.launch_error = Some(error);
                }
                Action::None
            }
        }
    }

    pub fn view<'a>(&'a self) -> Element<'a, Message> {
        let status = if self.launching {
            Some("Preparando leitor…")
        } else {
            self.launch_error.as_deref()
        };

        view_helper::view(
            &self.series_name,
            self.series_pages,
            self.series_cover.as_ref(),
            &self.entries,
            &self.covers,
            self.active_tab,
            status,
        )
    }

    /// Builds a task to fetch covers for every loaded entry whose cover isn't
    /// already cached or in flight, marking them in flight. Returns `None`
    /// when there's nothing left to fetch.
    fn request_entry_covers(&mut self) -> Option<Task<Message>> {
        let AsyncModel::Loaded(entries) = &self.entries else {
            return None;
        };

        let pending: Vec<(String, EntryKind)> = entries
            .iter()
            .filter(|entry| {
                let key = entry.cover_key();
                !self.covers.contains_key(&key) && !self.covers_in_flight.contains(&key)
            })
            .map(|entry| (entry.external_id.clone(), entry.kind))
            .collect();

        if pending.is_empty() {
            return None;
        }

        self.covers_in_flight
            .extend(pending.iter().map(|(id, kind)| cover_key(*kind, id)));

        Some(Task::perform(
            crate::runtime::on_tokio(fetch_entry_covers(
                self.library_external_id.clone(),
                pending,
            )),
            Message::CoversLoaded,
        ))
    }
}

async fn fetch_entries(
    library_external_id: String,
    series_external_id: String,
) -> Result<Vec<SeriesEntry>, String> {
    let series_id = series_external_id
        .parse::<u16>()
        .map_err(|_| format!("series id {series_external_id} is not a valid Kavita id"))?;
    let (provider, _integration_id) = provider_for_library(&library_external_id).await?;
    provider.list_series_entries(series_id).await
}

/// Fetches the series' own cover, returning `None` on any error (a missing
/// header cover just renders the placeholder box rather than failing the page).
async fn fetch_series_cover(
    library_external_id: String,
    series_external_id: String,
) -> Option<Vec<u8>> {
    let series_id = series_external_id.parse::<u16>().ok()?;
    let (provider, integration_id) = provider_for_library(&library_external_id).await.ok()?;
    provider.cover(&integration_id, series_id).await.ok()
}

/// Fetches covers for the given entries, returning `(cover_key, bytes)` for each
/// one that resolved. Per-entry and whole-batch failures are logged and omitted
/// rather than surfaced, so a partial result still renders what it can.
async fn fetch_entry_covers(
    library_external_id: String,
    entries: Vec<(String, EntryKind)>,
) -> Vec<(String, Vec<u8>)> {
    match load_entry_covers(library_external_id, entries).await {
        Ok(covers) => covers,
        Err(error) => {
            tracing::debug!("failed to load entry covers: {error}");
            Vec::new()
        }
    }
}

async fn load_entry_covers(
    library_external_id: String,
    entries: Vec<(String, EntryKind)>,
) -> Result<Vec<(String, Vec<u8>)>, String> {
    let (provider, integration_id) = provider_for_library(&library_external_id).await?;

    let mut covers = Vec::new();
    for (external_id, kind) in entries {
        let Ok(parsed) = external_id.parse::<u16>() else {
            tracing::debug!("skipping cover for non-numeric id {external_id}");
            continue;
        };

        let result = match kind {
            EntryKind::Volume => provider.volume_cover(&integration_id, parsed).await,
            EntryKind::Chapter => provider.chapter_cover(&integration_id, parsed).await,
        };

        match result {
            Ok(bytes) => covers.push((cover_key(kind, &external_id), bytes)),
            Err(error) => tracing::debug!("failed to fetch cover for {external_id}: {error}"),
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
    use crate::models::{AsyncModel, series_entry::SeriesEntry};

    type Covers = HashMap<String, image::Handle>;

    /// Width of the large header cover, in pixels (height follows the 2:3
    /// comic aspect ratio).
    const HEADER_COVER_WIDTH: f32 = 200.0;

    pub fn view<'a>(
        series_name: &'a str,
        series_pages: Option<u16>,
        series_cover: Option<&'a image::Handle>,
        entries: &'a AsyncModel<Vec<SeriesEntry>, String>,
        covers: &'a Covers,
        active_tab: Tab,
        status: Option<&'a str>,
    ) -> Element<'a, super::Message> {
        let count = match entries {
            AsyncModel::Loaded(entries) => Some(entries.len()),
            _ => None,
        };
        let noun = contents_noun(entries);

        let mut content = column![header(series_name, series_pages, count, noun, series_cover)]
            .spacing(24)
            .padding(24)
            .width(Fill);

        if let Some(status) = status {
            content = content.push(status_banner(status));
        }

        content = content
            .push(tab_bar(active_tab, noun.tab))
            .push(tab_content(
                active_tab,
                series_name,
                series_pages,
                entries,
                covers,
                noun,
            ));

        column![top_bar(), scrollable(content).width(Fill).height(Fill)]
            .width(Fill)
            .height(Fill)
            .into()
    }

    /// A one-line status banner (reader preparing / last launch error).
    fn status_banner<'a>(status: &'a str) -> Element<'a, super::Message> {
        container(text(status).size(13))
            .padding(8)
            .width(Fill)
            .into()
    }

    /// How to name a series' contents in the UI, derived from what the entries
    /// actually are: all-volumes, all-chapters, or a mix.
    #[derive(Clone, Copy)]
    struct Noun {
        /// Tab/section label, e.g. "Volumes".
        tab: &'static str,
        /// Singular form for the count chip, e.g. "volume".
        singular: &'static str,
    }

    fn contents_noun(entries: &AsyncModel<Vec<SeriesEntry>, String>) -> Noun {
        use crate::models::series_entry::EntryKind;

        let AsyncModel::Loaded(entries) = entries else {
            return Noun {
                tab: "Contents",
                singular: "item",
            };
        };

        let has_volume = entries.iter().any(|e| e.kind == EntryKind::Volume);
        let has_chapter = entries.iter().any(|e| e.kind == EntryKind::Chapter);

        match (has_volume, has_chapter) {
            (true, false) => Noun {
                tab: "Volumes",
                singular: "volume",
            },
            (false, true) => Noun {
                tab: "Chapters",
                singular: "chapter",
            },
            // Mixed, or empty — a neutral label covers both.
            _ => Noun {
                tab: "Contents",
                singular: "item",
            },
        }
    }

    fn top_bar<'a>() -> Element<'a, super::Message> {
        container(button("Back").on_press(super::Message::BackPressed))
            .padding(20)
            .into()
    }

    fn header<'a>(
        series_name: &'a str,
        series_pages: Option<u16>,
        count: Option<usize>,
        noun: Noun,
        series_cover: Option<&'a image::Handle>,
    ) -> Element<'a, super::Message> {
        row![
            header_cover(series_cover),
            column![
                text(series_name).size(28).font(Font {
                    weight: iced::font::Weight::Bold,
                    ..Font::default()
                }),
                stats(series_pages, count, noun),
            ]
            .spacing(16)
            .width(Fill),
        ]
        .spacing(24)
        .into()
    }

    fn stats<'a>(
        series_pages: Option<u16>,
        count: Option<usize>,
        noun: Noun,
    ) -> Element<'a, super::Message> {
        let mut chips = row![].spacing(8);

        if let Some(count) = count {
            let unit = if count == 1 {
                noun.singular.to_string()
            } else {
                format!("{}s", noun.singular)
            };
            chips = chips.push(stat_chip(format!("{count} {unit}")));
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

    fn tab_bar<'a>(active_tab: Tab, contents_label: &'a str) -> Element<'a, super::Message> {
        row![
            tab_button(contents_label, Tab::Contents, active_tab),
            tab_button("Details", Tab::Details, active_tab),
        ]
        .spacing(8)
        .into()
    }

    fn tab_button<'a>(label: &'a str, tab: Tab, active_tab: Tab) -> Element<'a, super::Message> {
        let is_active = tab == active_tab;

        button(text(label).size(15))
            .on_press(super::Message::TabSelected(tab))
            .padding([6, 12])
            .style(move |theme: &Theme, status| {
                let palette = theme.extended_palette();
                let mut style = button::text(theme, status);

                if is_active {
                    style.background = Some(Background::Color(palette.background.weak.color));
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
        entries: &'a AsyncModel<Vec<SeriesEntry>, String>,
        covers: &'a Covers,
        noun: Noun,
    ) -> Element<'a, super::Message> {
        match active_tab {
            Tab::Contents => contents_body(entries, covers),
            Tab::Details => details_body(series_name, series_pages, entries, noun),
        }
    }

    fn contents_body<'a>(
        entries: &'a AsyncModel<Vec<SeriesEntry>, String>,
        covers: &'a Covers,
    ) -> Element<'a, super::Message> {
        match entries {
            AsyncModel::Loaded(entries) if entries.is_empty() => empty_state(),
            AsyncModel::Loaded(entries) => entries_grid(entries, covers),
            AsyncModel::Error(error) => error_state(error),
            AsyncModel::NotLoaded | AsyncModel::Loading => loading_state(),
        }
    }

    fn entries_grid<'a>(
        entries: &'a [SeriesEntry],
        covers: &'a Covers,
    ) -> Element<'a, super::Message> {
        let cards = entries
            .iter()
            .map(|entry| entry_card(entry, covers.get(&entry.cover_key())));

        container(
            grid(cards)
                .fluid(160.0)
                .spacing(16.0)
                .height(grid::aspect_ratio(2.0, 3.0)),
        )
        .width(Fill)
        .into()
    }

    fn entry_card<'a>(
        entry: &'a SeriesEntry,
        cover: Option<&image::Handle>,
    ) -> Element<'a, super::Message> {
        let content = column![
            container(cover_image(cover))
                .width(Fill)
                .height(Fill)
                .clip(true)
                .style(cover_box_style),
            text(&entry.label).size(13),
            pages_label(entry.pages),
        ]
        .width(Fill)
        .height(Fill)
        .spacing(4)
        .align_x(Center);

        button(content)
            .on_press(super::Message::EntrySelected(entry.clone()))
            .padding(0)
            .width(Fill)
            .height(Fill)
            .style(button::text)
            .into()
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
        entries: &'a AsyncModel<Vec<SeriesEntry>, String>,
        noun: Noun,
    ) -> Element<'a, super::Message> {
        let count = match entries {
            AsyncModel::Loaded(entries) => entries.len().to_string(),
            AsyncModel::Loading | AsyncModel::NotLoaded | AsyncModel::Error(_) => "—".to_string(),
        };

        let pages = series_pages
            .map(|pages| pages.to_string())
            .unwrap_or_else(|| "—".to_string());

        column![
            detail_row("Title", series_name.to_string()),
            detail_row(noun.tab, count),
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
        container(text("Nenhum conteúdo encontrado nesta série"))
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

    fn volume_entry(id: &str, label: &str, pages: Option<u16>) -> SeriesEntry {
        SeriesEntry {
            external_id: id.to_string(),
            kind: EntryKind::Volume,
            label: label.to_string(),
            pages,
        }
    }

    fn chapter_entry(id: &str, label: &str, pages: Option<u16>) -> SeriesEntry {
        SeriesEntry {
            external_id: id.to_string(),
            kind: EntryKind::Chapter,
            label: label.to_string(),
            pages,
        }
    }

    fn series_detail(entries: AsyncModel<Vec<SeriesEntry>, String>) -> SeriesDetail {
        SeriesDetail {
            series_name: "Beneath the Trees".to_string(),
            series_pages: Some(183),
            library_external_id: "lib-1".to_string(),
            series_cover: None,
            entries,
            covers: HashMap::new(),
            covers_in_flight: HashSet::new(),
            active_tab: Tab::Contents,
            launching: false,
            launch_error: None,
        }
    }

    #[test]
    fn new_starts_in_loading_with_the_contents_tab_active() {
        let (detail, _task) = SeriesDetail::new(
            "1".to_string(),
            "One Piece".to_string(),
            Some(42),
            "lib-1".to_string(),
        );

        assert!(matches!(detail.entries, AsyncModel::Loading));
        assert_eq!(detail.active_tab, Tab::Contents);
        assert!(detail.series_cover.is_none());
    }

    #[test]
    fn entries_loaded_ok_stores_entries_and_requests_covers() {
        let mut detail = series_detail(AsyncModel::Loading);

        let action = detail.update(Message::EntriesLoaded(Ok(vec![chapter_entry(
            "1",
            "#1",
            Some(24),
        )])));

        assert!(matches!(action, Action::Run(_)));
        assert!(matches!(detail.entries, AsyncModel::Loaded(ref e) if e.len() == 1));
        assert!(detail.covers_in_flight.contains("ch:1"));
    }

    #[test]
    fn entries_loaded_empty_does_not_request_covers() {
        let mut detail = series_detail(AsyncModel::Loading);

        let action = detail.update(Message::EntriesLoaded(Ok(vec![])));

        assert!(matches!(action, Action::None));
    }

    #[test]
    fn entry_selected_is_ignored_while_a_launch_is_already_in_flight() {
        let mut detail = series_detail(AsyncModel::Loaded(vec![]));
        detail.launching = true;

        let action = detail.update(Message::EntrySelected(chapter_entry("1", "#1", Some(24))));

        // No new launch is kicked off; the in-flight one is left alone.
        assert!(matches!(action, Action::None));
        assert!(detail.launching);
    }

    #[test]
    fn reader_launched_error_is_surfaced_and_clears_the_launching_flag() {
        let mut detail = series_detail(AsyncModel::Loaded(vec![]));
        detail.launching = true;

        let action = detail.update(Message::ReaderLaunched(Err("boom".to_string())));

        assert!(matches!(action, Action::None));
        assert!(!detail.launching);
        assert_eq!(detail.launch_error.as_deref(), Some("boom"));
    }

    #[test]
    fn entries_loaded_error_stores_the_error() {
        let mut detail = series_detail(AsyncModel::Loading);

        let action = detail.update(Message::EntriesLoaded(Err("boom".to_string())));

        assert!(matches!(action, Action::None));
        assert!(matches!(detail.entries, AsyncModel::Error(ref e) if e == "boom"));
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
    fn covers_loaded_inserts_handles_and_clears_in_flight() {
        let mut detail = series_detail(AsyncModel::Loaded(vec![chapter_entry("1", "#1", None)]));
        detail.covers_in_flight.insert("ch:1".to_string());

        let action = detail.update(Message::CoversLoaded(vec![(
            "ch:1".to_string(),
            vec![1, 2, 3, 4],
        )]));

        assert!(matches!(action, Action::None));
        assert!(detail.covers.contains_key("ch:1"));
        assert!(!detail.covers_in_flight.contains("ch:1"));
    }

    #[test]
    fn request_entry_covers_marks_volume_and_chapter_covers_by_kind() {
        let mut detail = series_detail(AsyncModel::Loaded(vec![
            volume_entry("5", "Volume 1", None),
            chapter_entry("5", "#1", None),
        ]));

        let task = detail.request_entry_covers();

        assert!(task.is_some());
        // Same numeric id, different kinds — must not collide.
        assert!(detail.covers_in_flight.contains("vol:5"));
        assert!(detail.covers_in_flight.contains("ch:5"));
    }

    #[test]
    fn request_entry_covers_skips_cached_and_in_flight_entries() {
        let mut detail = series_detail(AsyncModel::Loaded(vec![
            chapter_entry("cached", "#1", None),
            chapter_entry("in-flight", "#2", None),
            chapter_entry("fresh", "#3", None),
        ]));
        detail
            .covers
            .insert("ch:cached".to_string(), image::Handle::from_bytes(vec![0]));
        detail.covers_in_flight.insert("ch:in-flight".to_string());

        let task = detail.request_entry_covers();

        assert!(task.is_some());
        assert!(detail.covers_in_flight.contains("ch:fresh"));
        assert!(detail.covers_in_flight.contains("ch:in-flight"));
        assert!(!detail.covers_in_flight.contains("ch:cached"));
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
    fn view_renders_the_series_name_and_entry_cards() {
        let detail = series_detail(AsyncModel::Loaded(vec![chapter_entry("1", "#1", Some(24))]));

        let mut ui = simulator(detail.view());

        assert!(ui.find("Beneath the Trees").is_ok());
        assert!(ui.find("#1").is_ok());
        assert!(ui.find("24 pages").is_ok());
    }

    #[test]
    fn view_names_the_tab_and_count_volumes_when_entries_are_volumes() {
        let detail = series_detail(AsyncModel::Loaded(vec![
            volume_entry("1", "Volume 1", None),
            volume_entry("2", "Volume 2", None),
        ]));

        let mut ui = simulator(detail.view());

        assert!(ui.find("Volumes").is_ok());
        assert!(ui.find("2 volumes").is_ok());
    }

    #[test]
    fn view_names_the_tab_and_count_chapters_when_entries_are_chapters() {
        let detail = series_detail(AsyncModel::Loaded(vec![
            chapter_entry("1", "#1", None),
            chapter_entry("2", "#2", None),
        ]));

        let mut ui = simulator(detail.view());

        assert!(ui.find("Chapters").is_ok());
        assert!(ui.find("2 chapters").is_ok());
    }

    #[test]
    fn view_uses_a_neutral_label_for_mixed_contents() {
        let detail = series_detail(AsyncModel::Loaded(vec![
            volume_entry("1", "Volume 1", None),
            chapter_entry("2", "#2", None),
        ]));

        let mut ui = simulator(detail.view());

        assert!(ui.find("Contents").is_ok());
        assert!(ui.find("2 items").is_ok());
    }

    #[test]
    fn view_renders_the_empty_state_when_there_are_no_entries() {
        let detail = series_detail(AsyncModel::Loaded(vec![]));

        let mut ui = simulator(detail.view());

        assert!(ui.find("Nenhum conteúdo encontrado nesta série").is_ok());
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
        let mut detail = series_detail(AsyncModel::Loaded(vec![chapter_entry("1", "#1", None)]));
        detail.active_tab = Tab::Details;

        let mut ui = simulator(detail.view());

        assert!(ui.find("Title").is_ok());
        assert!(ui.find("Pages").is_ok());
    }
}
