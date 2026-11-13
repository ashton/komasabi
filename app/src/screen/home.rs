//! The home screen: the user's Library, as the series of the items they've
//! added from their sources. Opening a series shows the items added from it
//! (see [`super::SeriesDetail::from_library`]).

use std::collections::{HashMap, HashSet};

use iced::widget::image;
use iced::{Element, Task};

use crate::models::{
    AsyncModel,
    library_item::{LibraryItem, LibrarySeries, group_into_series},
};
use crate::repository::library as library_repository;
use crate::screen::card::DownloadState;

#[derive(Debug, Default)]
pub struct Home {
    series: AsyncModel<Vec<LibrarySeries>, String>,
    /// Decoded series covers, keyed by [`series_key`].
    covers: HashMap<String, image::Handle>,
    /// The [`series_key`] of the series whose card menu is open, if any.
    menu: Option<String>,
    /// The last remove-from-library or download failure, shown until the next
    /// attempt.
    error: Option<String>,
    /// [`series_key`]s of the series downloaded in full.
    downloaded: HashSet<String>,
    /// [`series_key`]s of the series being downloaded.
    downloading: HashSet<String>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Message {
    ItemsLoaded(Result<Vec<LibraryItem>, String>),
    /// The items, once their progress was synced from their sources.
    ItemsSynced(Vec<LibraryItem>),
    /// Series cover bytes, keyed by [`series_key`].
    CoversLoaded(Vec<(String, Vec<u8>)>),
    SeriesSelected(LibrarySeries),
    /// The "⋮" button of a series' card (by [`series_key`]) was pressed: open
    /// its menu, or close it if it's the one open.
    MenuToggled(String),
    /// A click landed outside the open menu, closing it.
    MenuDismissed,
    /// "Remove from library" was chosen in a series' menu: remove all of its
    /// items from the Library.
    RemoveSeries(LibrarySeries),
    /// A series (by [`series_key`]) was removed from the Library, or failed
    /// to be.
    SeriesRemoved(String, Result<usize, String>),
    /// The series downloaded in full, as `(source id, series id)`s.
    DownloadedSeriesLoaded(HashSet<(String, String)>),
    /// "Download" was chosen in a series' menu: download each of its
    /// volumes/chapters for reading offline.
    DownloadSeries(LibrarySeries),
    /// A series (by [`series_key`]) finished downloading, carrying how many of
    /// its entries were downloaded, or why some couldn't be.
    SeriesDownloaded(String, Result<usize, String>),
}

#[derive(Debug)]
pub enum Action {
    None,
    Run(Task<Message>),
    /// Open a series of the Library.
    OpenSeries {
        integration_id: String,
        series_external_id: String,
        series_name: String,
    },
}

/// The key a series' cover is kept under: series ids are only unique within
/// their source.
fn series_key(series: &LibrarySeries) -> String {
    cover_key(&series.integration_id, &series.series_external_id)
}

fn cover_key(source_id: &str, series_id: &str) -> String {
    format!("{source_id}/{series_id}")
}

impl Home {
    pub fn new() -> (Self, Task<Message>) {
        (
            Self {
                series: AsyncModel::Loading,
                covers: HashMap::new(),
                menu: None,
                error: None,
                downloaded: HashSet::new(),
                downloading: HashSet::new(),
            },
            Task::batch([
                Task::perform(library_repository::all(), Message::ItemsLoaded),
                Task::perform(
                    crate::downloads::all_downloaded_series(),
                    Message::DownloadedSeriesLoaded,
                ),
            ]),
        )
    }

    pub fn update(&mut self, message: Message) -> Action {
        // Doing anything else on the screen closes an open card menu.
        if matches!(
            message,
            Message::SeriesSelected(_)
                | Message::RemoveSeries(_)
                | Message::DownloadSeries(_)
                | Message::MenuDismissed
        ) {
            self.menu = None;
        }

        match message {
            Message::ItemsLoaded(result) => {
                // Shown as saved right away, then again once synced.
                let sync = result
                    .as_ref()
                    .ok()
                    .filter(|items| !items.is_empty())
                    .map(|items| {
                        Task::perform(
                            crate::runtime::on_tokio(crate::library::sync_progress(items.clone())),
                            Message::ItemsSynced,
                        )
                    });
                let series = result.map(|items| group_into_series(&items));
                let covers: Vec<(String, String)> = match &series {
                    Ok(series) => series
                        .iter()
                        .filter(|series| !self.covers.contains_key(&series_key(series)))
                        .map(|series| {
                            (
                                series.integration_id.clone(),
                                series.series_external_id.clone(),
                            )
                        })
                        .collect(),
                    Err(_) => Vec::new(),
                };
                self.series = series.into();

                let mut tasks: Vec<Task<Message>> = sync.into_iter().collect();
                if !covers.is_empty() {
                    tasks.push(Task::perform(
                        crate::runtime::on_tokio(fetch_covers(covers)),
                        Message::CoversLoaded,
                    ));
                }
                if tasks.is_empty() {
                    Action::None
                } else {
                    Action::Run(Task::batch(tasks))
                }
            }

            Message::ItemsSynced(items) => {
                self.series = AsyncModel::Loaded(group_into_series(&items));
                Action::None
            }

            Message::CoversLoaded(covers) => {
                for (key, bytes) in covers {
                    self.covers.insert(key, image::Handle::from_bytes(bytes));
                }
                Action::None
            }

            Message::SeriesSelected(series) => Action::OpenSeries {
                integration_id: series.integration_id,
                series_external_id: series.series_external_id,
                series_name: series.name,
            },

            Message::MenuToggled(key) => {
                self.menu = (self.menu.as_ref() != Some(&key)).then_some(key);
                Action::None
            }
            Message::MenuDismissed => Action::None,

            Message::RemoveSeries(series) => {
                self.error = None;
                let key = series_key(&series);

                Action::Run(Task::perform(
                    library_repository::remove_series(
                        series.integration_id,
                        series.series_external_id,
                    ),
                    move |result| Message::SeriesRemoved(key.clone(), result),
                ))
            }

            Message::SeriesRemoved(key, Ok(removed)) => {
                tracing::debug!("removed {removed} item(s) of {key} from the library");
                if let AsyncModel::Loaded(series) = &mut self.series {
                    series.retain(|series| series_key(series) != key);
                }
                self.covers.remove(&key);
                Action::None
            }
            Message::SeriesRemoved(key, Err(error)) => {
                tracing::debug!("failed to remove {key} from the library: {error}");
                self.error = Some(format!("Could not remove from library: {error}"));
                Action::None
            }

            Message::DownloadedSeriesLoaded(series) => {
                self.downloaded.extend(
                    series
                        .iter()
                        .map(|(source, series)| cover_key(source, series)),
                );
                Action::None
            }

            Message::DownloadSeries(series) => {
                let key = series_key(&series);
                if self.downloaded.contains(&key) || !self.downloading.insert(key.clone()) {
                    return Action::None;
                }
                self.error = None;

                Action::Run(Task::perform(
                    crate::runtime::on_tokio(crate::downloads::download_series(
                        series.integration_id,
                        series.series_external_id,
                        series.name,
                    )),
                    move |result| Message::SeriesDownloaded(key.clone(), result),
                ))
            }

            Message::SeriesDownloaded(key, result) => {
                self.downloading.remove(&key);
                match result {
                    Ok(downloaded) => {
                        tracing::debug!("downloaded {downloaded} item(s) of {key}");
                        self.downloaded.insert(key);
                    }
                    Err(error) => {
                        tracing::debug!("failed to download {key}: {error}");
                        self.error = Some(format!("Could not download: {error}"));
                    }
                }
                Action::None
            }
        }
    }

    /// Whether the series with the [`series_key`] `key` is downloaded.
    fn download_state(&self, key: &str) -> DownloadState {
        if self.downloading.contains(key) {
            DownloadState::Downloading
        } else if self.downloaded.contains(key) {
            DownloadState::Downloaded
        } else {
            DownloadState::NotDownloaded
        }
    }

    pub fn view<'a>(&'a self) -> Element<'a, Message> {
        view_helper::view(
            &self.series,
            &self.covers,
            self.menu.as_deref(),
            self.error.as_deref(),
            &|key: &str| self.download_state(key),
        )
    }
}

/// Fetches the covers of the given `(source id, series id)`s from their
/// sources, keyed by [`series_key`]. A source is resolved once for all its
/// series, and covers that can't be fetched are left out.
async fn fetch_covers(series: Vec<(String, String)>) -> Vec<(String, Vec<u8>)> {
    use crate::providers::{Provider, provider_for_integration};

    let mut by_source: Vec<(String, Vec<String>)> = Vec::new();
    for (source, series_id) in series {
        match by_source.iter_mut().find(|(id, _)| *id == source) {
            Some((_, ids)) => ids.push(series_id),
            None => by_source.push((source, vec![series_id])),
        }
    }

    let mut covers = Vec::new();
    for (source, series_ids) in by_source {
        let (provider, namespace) = match provider_for_integration(&source).await {
            Ok(resolved) => resolved,
            Err(error) => {
                tracing::debug!("no covers from source {source}: {error}");
                continue;
            }
        };
        for series_id in series_ids {
            match provider.cover(&namespace, &series_id).await {
                Ok(bytes) => covers.push((cover_key(&source, &series_id), bytes)),
                Err(error) => tracing::debug!("no cover for series {series_id}: {error}"),
            }
        }
    }
    covers
}

mod view_helper {
    use std::collections::HashMap;

    use iced::{
        Background, Border, Center, ContentFit, Element, Font,
        Length::{Fill, Shrink},
        Theme, border,
        widget::{
            button, column, container, grid, image, mouse_area, row, scrollable, stack, svg, text,
        },
    };

    use super::{Message, series_key};
    use crate::icons;
    use crate::models::{AsyncModel, library_item::LibrarySeries};
    use crate::screen::card::{self, COVER_RADIUS, DownloadState};

    /// Whether a series (by [`series_key`]) is downloaded.
    type DownloadStateOf<'b> = &'b dyn Fn(&str) -> DownloadState;
    use crate::touch_scroll::touch_scroll;

    pub fn view<'a>(
        series: &'a AsyncModel<Vec<LibrarySeries>, String>,
        covers: &'a HashMap<String, image::Handle>,
        open_menu: Option<&str>,
        error: Option<&'a str>,
        download_state: DownloadStateOf<'_>,
    ) -> Element<'a, Message> {
        let body: Element<'a, Message> = match series {
            // Centered on the whole screen, with no title above it.
            AsyncModel::Loaded(series) if series.is_empty() => return empty_state(),
            AsyncModel::Loaded(series) => series_grid(series, covers, open_menu, download_state),
            AsyncModel::Error(error) => container(text(error)).center_x(Fill).padding(40).into(),
            AsyncModel::NotLoaded | AsyncModel::Loading => container(row![]).padding(40).into(),
        };

        let title = text("Library").size(28).font(Font {
            weight: iced::font::Weight::Bold,
            ..Font::default()
        });

        let mut content = column![title].spacing(24).padding(24).width(Fill);
        if let Some(error) = error {
            content = content.push(text(error).size(13));
        }

        let screen = touch_scroll(scrollable(content.push(body)).width(Fill).height(Fill));

        // With a card's menu open, a click anywhere the menu (or another
        // button) doesn't take closes it.
        if open_menu.is_some() {
            mouse_area(screen).on_press(Message::MenuDismissed).into()
        } else {
            screen.into()
        }
    }

    fn series_grid<'a>(
        series: &'a [LibrarySeries],
        covers: &'a HashMap<String, image::Handle>,
        open_menu: Option<&str>,
        download_state: DownloadStateOf<'_>,
    ) -> Element<'a, Message> {
        let cards = series.iter().map(|series| {
            let key = series_key(series);
            let menu_open = open_menu == Some(key.as_str());
            series_card(series, covers.get(&key), menu_open, download_state(&key))
        });

        grid(cards)
            .fluid(160.0)
            .spacing(16.0)
            .height(grid::aspect_ratio(2.0, 3.0))
            .into()
    }

    /// A series' card: its cover, which opens the series, over a bar holding
    /// its title (a link to the series too), item count and menu.
    fn series_card<'a>(
        series: &'a LibrarySeries,
        cover: Option<&image::Handle>,
        menu_open: bool,
        download: DownloadState,
    ) -> Element<'a, Message> {
        // Only its top corners are rounded: the bar carries on below it.
        let image: Element<'a, Message> = match cover {
            Some(handle) => image(handle.clone())
                .width(Fill)
                .height(Fill)
                .content_fit(ContentFit::Cover)
                .border_radius(border::top(COVER_RADIUS))
                .into(),
            None => row![].into(),
        };
        let cover = container(image)
            .width(Fill)
            .height(Fill)
            .clip(true)
            .style(|theme: &Theme| container::Style {
                background: Some(Background::Color(
                    theme.extended_palette().background.strong.color,
                )),
                border: Border {
                    radius: border::top(COVER_RADIUS),
                    ..Border::default()
                },
                ..container::Style::default()
            });
        let cover: Element<'a, Message> = button(cover)
            .on_press(Message::SeriesSelected(series.clone()))
            .padding(0)
            .width(Fill)
            .height(Fill)
            .style(button::text)
            .into();

        // The open menu sits over the cover, just above its "⋮" button.
        let cover = if menu_open {
            stack![
                cover,
                card::menu(
                    download
                        .menu_item(Message::DownloadSeries(series.clone()))
                        .into_iter()
                        .chain([card::menu_item(
                            "trash",
                            "Remove from library",
                            Message::RemoveSeries(series.clone()),
                        )])
                        .collect()
                )
            ]
            .into()
        } else {
            cover
        };

        column![cover, card_bar(series, download)]
            .width(Fill)
            .height(Fill)
            .into()
    }

    /// The bar under a series' cover: its title, a link to the series, its
    /// item count, and the "⋮" button opening its menu.
    fn card_bar<'a>(series: &'a LibrarySeries, download: DownloadState) -> Element<'a, Message> {
        // A title too long for the card is cut off (by the bar), not wrapped.
        let title = button(text(&series.name).size(13).wrapping(text::Wrapping::None))
            .on_press(Message::SeriesSelected(series.clone()))
            .padding(0)
            .width(Shrink)
            .style(|theme: &Theme, status| {
                let palette = theme.extended_palette();
                let text_color = match status {
                    button::Status::Hovered | button::Status::Pressed => palette.primary.base.color,
                    _ => palette.background.weak.text,
                };

                button::Style {
                    text_color,
                    ..button::Style::default()
                }
            });
        let name = column![
            title,
            text(items_label(series)).size(11).style(text::secondary),
        ]
        .spacing(2);

        card::bar(
            download.icon(),
            name.into(),
            Some(Message::MenuToggled(series_key(series))),
        )
    }

    /// "3 items · 1 read", or just the count when nothing's read yet.
    pub(super) fn items_label(series: &LibrarySeries) -> String {
        let noun = if series.items == 1 { "item" } else { "items" };
        match series.items_read {
            0 => format!("{} {noun}", series.items),
            read => format!("{} {noun} · {read} read", series.items),
        }
    }

    fn empty_state<'a>() -> Element<'a, Message> {
        let content = column![
            svg(svg::Handle::from_memory(icons::BOOK_OPEN))
                .height(64)
                .width(64)
                .style(|theme: &Theme, _status| svg::Style {
                    color: Some(theme.extended_palette().background.neutral.color),
                }),
            text!("Sua biblioteca está vazia, adicione volumes ou capítulos a partir de uma fonte")
                .center(),
        ]
        .spacing(20)
        .max_width(420)
        .align_x(Center);

        container(content).center(Fill).into()
    }
}

#[cfg(test)]
mod tests {
    use iced_test::simulator;

    use super::*;
    use crate::models::library_item::tests::item;

    fn loaded(items: Vec<LibraryItem>) -> Home {
        let mut home = Home::default();
        let _ = home.update(Message::ItemsLoaded(Ok(items)));
        home
    }

    #[test]
    fn new_starts_loading() {
        let (home, _task) = Home::new();

        assert!(matches!(home.series, AsyncModel::Loading));
    }

    #[test]
    fn view_lists_the_series_of_the_added_items() {
        let home = loaded(vec![
            item("s", "1", "1", 10, 10),
            item("s", "1", "2", 10, 0),
            item("s", "2", "3", 10, 0),
        ]);

        let mut ui = simulator(home.view());

        assert!(ui.find("Series 1").is_ok());
        assert!(ui.find("2 items · 1 read").is_ok());
        assert!(ui.find("Series 2").is_ok());
        assert!(ui.find("1 item").is_ok());
    }

    #[test]
    fn loading_items_syncs_their_progress_and_shows_the_synced_state() {
        let mut home = Home::default();

        // The sync task isn't run here.
        assert!(matches!(
            home.update(Message::ItemsLoaded(Ok(vec![item("s", "1", "1", 10, 0)]))),
            Action::Run(_)
        ));
        let _ = home.update(Message::ItemsSynced(vec![item("s", "1", "1", 10, 10)]));

        let mut ui = simulator(home.view());
        assert!(ui.find("1 item · 1 read").is_ok());
    }

    #[test]
    fn view_shows_the_empty_state_without_items() {
        let home = loaded(vec![]);

        let mut ui = simulator(home.view());

        assert!(
            ui.find(
                "Sua biblioteca está vazia, adicione volumes ou capítulos a partir de uma fonte"
            )
            .is_ok()
        );
    }

    #[test]
    fn every_series_card_has_a_menu_offering_to_remove_it() {
        let mut home = loaded(vec![item("s", "1", "1", 10, 0)]);
        let series = match &home.series {
            AsyncModel::Loaded(series) => series[0].clone(),
            _ => panic!("the series should be loaded"),
        };

        let mut ui = simulator(home.view());
        assert!(ui.find("Remove from library").is_err());
        drop(ui);

        let _ = home.update(Message::MenuToggled("s/1".to_string()));
        let mut ui = simulator(home.view());
        let _ = ui
            .click("Remove from library")
            .expect("the remove item should be found");
        let messages: Vec<_> = ui.into_messages().collect();
        assert!(messages.contains(&Message::RemoveSeries(series.clone())));

        assert!(matches!(
            home.update(Message::RemoveSeries(series)),
            Action::Run(_)
        ));
        // Choosing an item closes the menu.
        assert_eq!(home.menu, None);
    }

    #[test]
    fn the_menu_toggles_and_closes_on_a_click_elsewhere() {
        let mut home = loaded(vec![item("s", "1", "1", 10, 0)]);

        let _ = home.update(Message::MenuToggled("s/1".to_string()));
        assert_eq!(home.menu.as_deref(), Some("s/1"));
        let _ = home.update(Message::MenuToggled("s/1".to_string()));
        assert_eq!(home.menu, None);

        let _ = home.update(Message::MenuToggled("s/1".to_string()));
        let _ = home.update(Message::MenuDismissed);
        assert_eq!(home.menu, None);
    }

    #[test]
    fn a_removed_series_leaves_the_library() {
        let mut home = loaded(vec![item("s", "1", "1", 10, 0), item("s", "2", "1", 10, 0)]);

        let _ = home.update(Message::SeriesRemoved("s/1".to_string(), Ok(1)));

        let AsyncModel::Loaded(series) = &home.series else {
            panic!("the series should be loaded");
        };
        assert_eq!(series.len(), 1);
        assert_eq!(series[0].series_external_id, "2");
    }

    #[test]
    fn a_failed_removal_is_reported_and_keeps_the_series() {
        let mut home = loaded(vec![item("s", "1", "1", 10, 0)]);

        let _ = home.update(Message::SeriesRemoved(
            "s/1".to_string(),
            Err("locked".to_string()),
        ));

        assert!(matches!(&home.series, AsyncModel::Loaded(series) if series.len() == 1));
        let mut ui = simulator(home.view());
        assert!(ui.find("Could not remove from library: locked").is_ok());
    }

    #[test]
    fn a_series_is_offered_a_download_until_it_is_downloaded() {
        let mut home = loaded(vec![item("s", "1", "1", 10, 0)]);
        let series = match &home.series {
            AsyncModel::Loaded(series) => series[0].clone(),
            _ => panic!("the series should be loaded"),
        };
        assert_eq!(home.download_state("s/1").icon(), "cloud");

        let _ = home.update(Message::MenuToggled("s/1".to_string()));
        let mut ui = simulator(home.view());
        let _ = ui
            .click("Download")
            .expect("the download item should be found");
        let messages: Vec<_> = ui.into_messages().collect();
        assert!(messages.contains(&Message::DownloadSeries(series.clone())));

        assert!(matches!(
            home.update(Message::DownloadSeries(series.clone())),
            Action::Run(_)
        ));
        assert_eq!(home.download_state("s/1"), DownloadState::Downloading);
        assert!(matches!(
            home.update(Message::DownloadSeries(series.clone())),
            Action::None
        ));

        let _ = home.update(Message::SeriesDownloaded("s/1".to_string(), Ok(1)));
        assert_eq!(home.download_state("s/1").icon(), "download");
        let _ = home.update(Message::MenuToggled("s/1".to_string()));
        let mut ui = simulator(home.view());
        assert!(ui.find("Download").is_err());
        assert!(ui.find("Remove from library").is_ok());
    }

    #[test]
    fn series_downloaded_earlier_show_as_downloaded() {
        let mut home = loaded(vec![item("s", "1", "1", 10, 0)]);

        let _ = home.update(Message::DownloadedSeriesLoaded(HashSet::from([(
            "s".to_string(),
            "1".to_string(),
        )])));

        assert_eq!(home.download_state("s/1"), DownloadState::Downloaded);
    }

    #[test]
    fn clicking_a_series_opens_it() {
        let mut home = loaded(vec![item("s", "1", "1", 10, 0)]);

        let mut ui = simulator(home.view());
        let _ = ui
            .click("Series 1")
            .expect("the series card should be found");
        let messages: Vec<_> = ui.into_messages().collect();
        assert_eq!(messages.len(), 1);

        let action = home.update(messages[0].clone());
        assert!(matches!(
            action,
            Action::OpenSeries { ref integration_id, ref series_external_id, ref series_name }
                if integration_id == "s" && series_external_id == "1" && series_name == "Series 1"
        ));
    }
}
