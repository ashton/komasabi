use std::collections::{HashMap, HashSet};

use iced::widget::image;
use iced::{Element, Task};

use crate::models::{
    AsyncModel,
    collection::CollectionRef,
    library_item::LibraryItem,
    series_entry::{EntryKind, SeriesEntry, cover_key},
};
use crate::providers::{Provider, provider_for_integration};
use crate::repository::library as library_repository;

/// The two tabs shown above the contents grid. `Details` carries no data of its
/// own — it just renders the series' metadata in list form, as a counterpart
/// to the quick stats already in the header.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tab {
    Contents,
    Details,
}

/// Where a series was opened from, which decides where its entries come from.
#[derive(Clone, Debug, PartialEq)]
pub enum Origin {
    /// A source's collection: the entries are listed live by the source, and
    /// "Back" returns to the collection.
    Source(CollectionRef),
    /// The user's Library: the entries are the items of the series added from
    /// the source `integration_id`, which they're still read through.
    Library { integration_id: String },
}

impl Origin {
    /// The source the series belongs to.
    fn integration_id(&self) -> &str {
        match self {
            Origin::Source(location) => &location.integration_id,
            Origin::Library { integration_id } => integration_id,
        }
    }

    fn is_library(&self) -> bool {
        matches!(self, Origin::Library { .. })
    }
}

/// Whether an entry card offers to add the entry to the Library.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AddState {
    /// Not in the Library: offer to add it.
    Available,
    /// Being saved in the Library.
    Adding,
    /// Already in the Library.
    Added,
    /// Not offered at all (the series is the Library's own).
    Hidden,
}

/// How an entry's card is drawn, beyond the entry itself.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CardState {
    pub add: AddState,
    /// Whether the entry is in the Library, so its menu offers to delete it.
    pub in_library: bool,
    /// Whether the card's menu is open.
    pub menu_open: bool,
}

#[derive(Debug)]
pub struct SeriesDetail {
    /// The series' provider-side id, which the reader session needs to look up
    /// the entry after the one being read.
    series_external_id: String,
    series_name: String,
    series_pages: Option<u16>,
    origin: Origin,
    /// The series' own (large, header) cover, once it has loaded.
    series_cover: Option<image::Handle>,
    entries: AsyncModel<Vec<SeriesEntry>, String>,
    /// The series' items in the Library, keyed by [`SeriesEntry::cover_key`]:
    /// which entries are already in it, or, for the Library's own series, the
    /// items the entries were made from.
    library: HashMap<String, LibraryItem>,
    /// Cover keys of the entries being saved in the Library.
    adding: HashSet<String>,
    /// Decoded entry cover images, keyed by [`SeriesEntry::cover_key`].
    covers: HashMap<String, image::Handle>,
    /// cover keys whose cover is currently being fetched, so it isn't requested
    /// more than once.
    covers_in_flight: HashSet<String>,
    active_tab: Tab,
    /// Whether an entry is currently being prepared and opened in the reader.
    /// Guards against double-launching while pages are still downloading.
    launching: bool,
    /// The last reader-launch or add-to-library failure, shown as a status
    /// banner until the next attempt.
    launch_error: Option<String>,
    /// The cover key of the entry whose card menu is open, if any.
    menu: Option<String>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Message {
    EntriesLoaded(Result<Vec<SeriesEntry>, String>),
    SeriesCoverLoaded(Option<Vec<u8>>),
    /// Cover image bytes that have been loaded, keyed by [`SeriesEntry::cover_key`].
    /// Failed/omitted entries are simply absent.
    CoversLoaded(Vec<(String, Vec<u8>)>),
    /// The series' items in the Library have been loaded.
    LibraryLoaded(Result<Vec<LibraryItem>, String>),
    /// A Library series' items, once their progress was synced from the
    /// source.
    LibrarySynced(Vec<LibraryItem>),
    TabSelected(Tab),
    BackPressed,
    /// "Read" was chosen on a volume/chapter; prepare it and open it in the
    /// reader.
    EntrySelected(SeriesEntry),
    /// The reader-launch task finished (the reader window is open, or it failed).
    ReaderLaunched(Result<(), String>),
    /// "Add to library" was chosen on a volume/chapter: save it in the
    /// Library.
    AddToLibrary(SeriesEntry),
    /// An entry (by cover key) was saved in the Library, or failed to be.
    AddedToLibrary(String, Result<Box<LibraryItem>, String>),
    /// The reader showing a Library item closed, so its progress may have moved.
    LibraryReaderClosed,
    /// The "⋮" button of an entry's card (by cover key) was pressed: open its
    /// menu, or close it if it's the one open.
    MenuToggled(String),
    /// A click landed outside the open menu, closing it.
    MenuDismissed,
    /// "Delete" was chosen in an entry's menu: remove it from the Library.
    RemoveFromLibrary(SeriesEntry),
    /// An entry (by cover key) was removed from the Library, or failed to be.
    RemovedFromLibrary(String, Result<(), String>),
}

#[derive(Debug)]
pub enum Action {
    None,
    Run(Task<Message>),
    /// Navigate back to the carried collection's series.
    Back(CollectionRef),
    /// Navigate back to the Library (home).
    BackToLibrary,
}

impl SeriesDetail {
    /// A series browsed live from a source's collection.
    pub fn new(
        series_external_id: String,
        series_name: String,
        series_pages: Option<u16>,
        location: CollectionRef,
    ) -> (Self, Task<Message>) {
        let entries_task = Task::perform(
            crate::runtime::on_tokio(fetch_entries(
                location.integration_id.clone(),
                series_external_id.clone(),
            )),
            Message::EntriesLoaded,
        );

        let cover_task = Task::perform(
            crate::runtime::on_tokio(fetch_series_cover(
                location.integration_id.clone(),
                series_external_id.clone(),
            )),
            Message::SeriesCoverLoaded,
        );

        let library_task = load_library(&location.integration_id, &series_external_id);

        (
            Self::with_origin(
                series_external_id,
                series_name,
                series_pages,
                Origin::Source(location),
            ),
            Task::batch([entries_task, cover_task, library_task]),
        )
    }

    /// A series of the user's Library: the items added from it.
    pub fn from_library(
        integration_id: String,
        series_external_id: String,
        series_name: String,
    ) -> (Self, Task<Message>) {
        let library_task = load_library(&integration_id, &series_external_id);
        let cover_task = Task::perform(
            crate::runtime::on_tokio(fetch_series_cover(
                integration_id.clone(),
                series_external_id.clone(),
            )),
            Message::SeriesCoverLoaded,
        );

        (
            Self::with_origin(
                series_external_id,
                series_name,
                None,
                Origin::Library { integration_id },
            ),
            Task::batch([library_task, cover_task]),
        )
    }

    fn with_origin(
        series_external_id: String,
        series_name: String,
        series_pages: Option<u16>,
        origin: Origin,
    ) -> Self {
        Self {
            series_external_id,
            series_name,
            series_pages,
            origin,
            series_cover: None,
            entries: AsyncModel::Loading,
            library: HashMap::new(),
            adding: HashSet::new(),
            covers: HashMap::new(),
            covers_in_flight: HashSet::new(),
            active_tab: Tab::Contents,
            launching: false,
            launch_error: None,
            menu: None,
        }
    }

    pub fn update(&mut self, message: Message) -> Action {
        // Doing anything else on the screen closes an open card menu.
        if matches!(
            message,
            Message::TabSelected(_)
                | Message::BackPressed
                | Message::EntrySelected(_)
                | Message::AddToLibrary(_)
                | Message::RemoveFromLibrary(_)
                | Message::MenuDismissed
        ) {
            self.menu = None;
        }

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

            Message::LibraryLoaded(Ok(items)) => {
                // A Library series is shown as saved right away, then again
                // once synced with the source's progress.
                let sync = (self.origin.is_library() && !items.is_empty()).then(|| {
                    Task::perform(
                        crate::runtime::on_tokio(crate::library::sync_progress(items.clone())),
                        Message::LibrarySynced,
                    )
                });
                let covers = self.show_library(items);

                match (sync, covers) {
                    (None, None) => Action::None,
                    (sync, covers) => Action::Run(Task::batch(sync.into_iter().chain(covers))),
                }
            }
            Message::LibrarySynced(items) => match self.show_library(items) {
                Some(task) => Action::Run(task),
                None => Action::None,
            },
            Message::LibraryLoaded(Err(error)) => {
                if self.origin.is_library() {
                    self.entries = AsyncModel::Error(error);
                } else {
                    tracing::debug!("could not load the series' library items: {error}");
                }
                Action::None
            }

            Message::TabSelected(tab) => {
                self.active_tab = tab;
                Action::None
            }

            Message::BackPressed => match &self.origin {
                Origin::Source(location) => Action::Back(location.clone()),
                Origin::Library { .. } => Action::BackToLibrary,
            },

            Message::EntrySelected(entry) => {
                // Ignore clicks while a launch is already in flight; the reader
                // for the previous click is still being prepared.
                if self.launching {
                    return Action::None;
                }
                self.launch_error = None;

                match &self.origin {
                    Origin::Source(location) => {
                        self.launching = true;

                        Action::Run(Task::perform(
                            crate::runtime::on_tokio(crate::reader_launch::prepare_and_launch(
                                location.integration_id.clone(),
                                self.series_external_id.clone(),
                                self.series_name.clone(),
                                entry,
                            )),
                            Message::ReaderLaunched,
                        ))
                    }
                    Origin::Library { .. } => {
                        let Some(item) = self.library.get(&entry.cover_key()) else {
                            return Action::None;
                        };

                        match crate::reader_launch::launch_library_item(item.clone()) {
                            Ok(closed) => {
                                Action::Run(Task::perform(closed, |_| Message::LibraryReaderClosed))
                            }
                            Err(error) => {
                                tracing::debug!("failed to open reader: {error}");
                                self.launch_error = Some(error);
                                Action::None
                            }
                        }
                    }
                }
            }

            Message::ReaderLaunched(result) => {
                self.launching = false;
                if let Err(error) = result {
                    tracing::debug!("failed to open reader: {error}");
                    self.launch_error = Some(error);
                }
                Action::None
            }

            Message::AddToLibrary(entry) => {
                let key = entry.cover_key();
                if self.origin.is_library()
                    || self.library.contains_key(&key)
                    || !self.adding.insert(key.clone())
                {
                    return Action::None;
                }
                self.launch_error = None;

                Action::Run(Task::perform(
                    crate::runtime::on_tokio(crate::library::add_to_library(
                        self.origin.integration_id().to_string(),
                        self.series_external_id.clone(),
                        self.series_name.clone(),
                        entry,
                    )),
                    move |result| Message::AddedToLibrary(key.clone(), result.map(Box::new)),
                ))
            }

            Message::AddedToLibrary(key, result) => {
                self.adding.remove(&key);
                match result {
                    Ok(item) => {
                        self.library.insert(key, *item);
                    }
                    Err(error) => {
                        tracing::debug!("failed to add to the library: {error}");
                        self.launch_error = Some(format!("Could not add to library: {error}"));
                    }
                }
                Action::None
            }

            Message::LibraryReaderClosed => Action::Run(load_library(
                self.origin.integration_id(),
                &self.series_external_id,
            )),

            Message::MenuToggled(key) => {
                self.menu = (self.menu.as_ref() != Some(&key)).then_some(key);
                Action::None
            }
            Message::MenuDismissed => Action::None,

            Message::RemoveFromLibrary(entry) => {
                let key = entry.cover_key();
                let Some(item) = self.library.get(&key) else {
                    return Action::None;
                };
                self.launch_error = None;

                Action::Run(Task::perform(
                    library_repository::remove(item.id.clone()),
                    move |result| Message::RemovedFromLibrary(key.clone(), result),
                ))
            }

            Message::RemovedFromLibrary(key, Ok(())) => {
                self.library.remove(&key);
                if !self.origin.is_library() {
                    return Action::None;
                }

                // The Library's own series loses the entry, and once it has
                // none left, it's no longer in the Library to show.
                if let AsyncModel::Loaded(entries) = &mut self.entries {
                    entries.retain(|entry| entry.cover_key() != key);
                    if entries.is_empty() {
                        return Action::BackToLibrary;
                    }
                }
                Action::None
            }
            Message::RemovedFromLibrary(_, Err(error)) => {
                tracing::debug!("failed to remove from the library: {error}");
                self.launch_error = Some(format!("Could not remove from library: {error}"));
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
            &|entry: &SeriesEntry| self.card_state(entry),
            self.active_tab,
            status,
            self.menu.is_some(),
        )
    }

    /// How `entry`'s card is drawn.
    fn card_state(&self, entry: &SeriesEntry) -> CardState {
        let key = entry.cover_key();
        CardState {
            add: self.add_state(entry),
            in_library: self.library.contains_key(&key),
            menu_open: self.menu.as_ref() == Some(&key),
        }
    }

    /// Keeps the series' Library `items`, which, for the Library's own series,
    /// are the entries shown. Returns the task fetching any covers they need.
    fn show_library(&mut self, items: Vec<LibraryItem>) -> Option<Task<Message>> {
        let entries: Vec<SeriesEntry> = items.iter().map(LibraryItem::as_entry).collect();
        self.library = items
            .into_iter()
            .map(|item| (item.as_entry().cover_key(), item))
            .collect();

        if !self.origin.is_library() {
            return None;
        }
        self.entries = AsyncModel::Loaded(entries);
        self.request_entry_covers()
    }

    /// Whether `entry`'s card offers to add it to the Library.
    fn add_state(&self, entry: &SeriesEntry) -> AddState {
        let key = entry.cover_key();
        if self.origin.is_library() {
            AddState::Hidden
        } else if self.library.contains_key(&key) {
            AddState::Added
        } else if self.adding.contains(&key) {
            AddState::Adding
        } else {
            AddState::Available
        }
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

        // Library items are covered by their source, like its own entries.
        Some(Task::perform(
            crate::runtime::on_tokio(fetch_entry_covers(
                self.origin.integration_id().to_string(),
                pending,
            )),
            Message::CoversLoaded,
        ))
    }
}

/// Loads the Library's items of a series.
fn load_library(integration_id: &str, series_external_id: &str) -> Task<Message> {
    Task::perform(
        library_repository::series_items(
            integration_id.to_string(),
            series_external_id.to_string(),
        ),
        Message::LibraryLoaded,
    )
}

async fn fetch_entries(
    integration_id: String,
    series_external_id: String,
) -> Result<Vec<SeriesEntry>, String> {
    let (provider, _namespace) = provider_for_integration(&integration_id).await?;
    provider.list_series_entries(&series_external_id).await
}

/// Fetches the series' own cover, returning `None` on any error (a missing
/// header cover just renders the placeholder box rather than failing the page).
async fn fetch_series_cover(integration_id: String, series_external_id: String) -> Option<Vec<u8>> {
    let (provider, namespace) = provider_for_integration(&integration_id).await.ok()?;
    provider.cover(&namespace, &series_external_id).await.ok()
}

/// Fetches covers for the given entries, returning `(cover_key, bytes)` for each
/// one that resolved. Per-entry and whole-batch failures are logged and omitted
/// rather than surfaced, so a partial result still renders what it can.
async fn fetch_entry_covers(
    integration_id: String,
    entries: Vec<(String, EntryKind)>,
) -> Vec<(String, Vec<u8>)> {
    match load_entry_covers(integration_id, entries).await {
        Ok(covers) => covers,
        Err(error) => {
            tracing::debug!("failed to load entry covers: {error}");
            Vec::new()
        }
    }
}

async fn load_entry_covers(
    integration_id: String,
    entries: Vec<(String, EntryKind)>,
) -> Result<Vec<(String, Vec<u8>)>, String> {
    let (provider, namespace) = provider_for_integration(&integration_id).await?;

    let mut covers = Vec::new();
    for (external_id, kind) in entries {
        let result = match kind {
            EntryKind::Volume => provider.volume_cover(&namespace, &external_id).await,
            EntryKind::Chapter => provider.chapter_cover(&namespace, &external_id).await,
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
        Background, Border, ContentFit, Element, Font,
        Length::{Fill, Fixed},
        Theme, border,
        widget::{
            button, column, container, grid, hover, image, mouse_area, progress_bar, row,
            scrollable, stack, text,
        },
    };

    use super::{AddState, CardState, Tab};
    use crate::models::{
        AsyncModel,
        series_entry::{ReadState, SeriesEntry},
    };
    use crate::screen::card::{self, COVER_RADIUS};
    use crate::screen::cover_overlay;
    use crate::touch_scroll::touch_scroll;

    type Covers = HashMap<String, image::Handle>;
    /// How an entry's card is drawn (see [`CardState`]).
    type CardStateOf<'b> = &'b dyn Fn(&SeriesEntry) -> CardState;

    /// Width of the large header cover, in pixels (height follows the 2:3
    /// comic aspect ratio).
    const HEADER_COVER_WIDTH: f32 = 200.0;

    #[allow(clippy::too_many_arguments)]
    pub fn view<'a>(
        series_name: &'a str,
        series_pages: Option<u16>,
        series_cover: Option<&'a image::Handle>,
        entries: &'a AsyncModel<Vec<SeriesEntry>, String>,
        covers: &'a Covers,
        card_state: CardStateOf<'_>,
        active_tab: Tab,
        status: Option<&'a str>,
        menu_open: bool,
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
                card_state,
                noun,
            ));

        let screen = column![
            top_bar(),
            touch_scroll(scrollable(content).width(Fill).height(Fill))
        ]
        .width(Fill)
        .height(Fill);

        // With a card's menu open, a click anywhere the menu (or another
        // button) doesn't take closes it.
        if menu_open {
            mouse_area(screen)
                .on_press(super::Message::MenuDismissed)
                .into()
        } else {
            screen.into()
        }
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
        container(crate::screen::back_button::back_button(
            super::Message::BackPressed,
        ))
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
        container(cover_image(handle, 1.0, COVER_RADIUS.into()))
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
        card_state: CardStateOf<'_>,
        noun: Noun,
    ) -> Element<'a, super::Message> {
        match active_tab {
            Tab::Contents => contents_body(entries, covers, card_state),
            Tab::Details => details_body(series_name, series_pages, entries, noun),
        }
    }

    fn contents_body<'a>(
        entries: &'a AsyncModel<Vec<SeriesEntry>, String>,
        covers: &'a Covers,
        card_state: CardStateOf<'_>,
    ) -> Element<'a, super::Message> {
        match entries {
            AsyncModel::Loaded(entries) if entries.is_empty() => empty_state(),
            AsyncModel::Loaded(entries) => entries_grid(entries, covers, card_state),
            AsyncModel::Error(error) => error_state(error),
            AsyncModel::NotLoaded | AsyncModel::Loading => loading_state(),
        }
    }

    fn entries_grid<'a>(
        entries: &'a [SeriesEntry],
        covers: &'a Covers,
        card_state: CardStateOf<'_>,
    ) -> Element<'a, super::Message> {
        let cards = entries
            .iter()
            .map(|entry| entry_card(entry, covers.get(&entry.cover_key()), card_state(entry)));

        container(
            grid(cards)
                .fluid(160.0)
                .spacing(16.0)
                .height(grid::aspect_ratio(2.0, 3.0)),
        )
        .width(Fill)
        .into()
    }

    /// An entry's card: its cover, with a bar under it naming the entry and
    /// holding its menu.
    fn entry_card<'a>(
        entry: &'a SeriesEntry,
        cover: Option<&image::Handle>,
        state: CardState,
    ) -> Element<'a, super::Message> {
        // Only its top corners are rounded: the bar carries on below it.
        let cover = container(cover_image(
            cover,
            cover_opacity(entry.read_state),
            border::top(COVER_RADIUS),
        ))
        .width(Fill)
        .height(Fill)
        .clip(true)
        .style(|theme: &Theme| container::Style {
            border: Border {
                radius: border::top(COVER_RADIUS),
                ..Border::default()
            },
            ..cover_box_style(theme)
        });

        let cover: Element<'a, super::Message> = match progress_value(entry.read_state) {
            Some(value) => stack![cover, read_progress_bar(value)].into(),
            None => cover.into(),
        };

        // Hovering the cover shades it and offers what to do with the entry.
        let cover = hover(cover, cover_actions(entry, state.add));

        // The open menu sits over the cover, just above its "⋮" button.
        let cover: Element<'a, super::Message> = if state.menu_open {
            stack![cover, entry_menu(entry, state)].into()
        } else {
            cover
        };

        column![cover, card_bar(entry, state)]
            .width(Fill)
            .height(Fill)
            .into()
    }

    /// The bar under a card's cover: its name and page count, and the "⋮"
    /// button opening its menu, when the menu has anything in it.
    fn card_bar<'a>(entry: &'a SeriesEntry, state: CardState) -> Element<'a, super::Message> {
        // A name too long for the card is cut off, not wrapped.
        let name = column![
            text(&entry.label).size(13).wrapping(text::Wrapping::None),
            pages_label(entry.pages),
        ]
        .spacing(2);

        card::bar(
            "cloud",
            name.into(),
            has_menu(state).then(|| super::Message::MenuToggled(entry.cover_key())),
        )
    }

    /// Whether an entry's menu has anything to offer: deleting it from the
    /// Library, if it's in it.
    pub(super) fn has_menu(state: CardState) -> bool {
        state.in_library
    }

    /// An entry's open menu, laid over the bottom-right of its cover.
    fn entry_menu<'a>(entry: &SeriesEntry, state: CardState) -> Element<'a, super::Message> {
        let mut items: Vec<Element<'a, super::Message>> = Vec::new();
        if state.in_library {
            items.push(card::menu_item(
                "trash",
                "Delete",
                super::Message::RemoveFromLibrary(entry.clone()),
            ));
        }

        card::menu(items)
    }

    /// The shade laid over a hovered cover, with the entry's actions side by
    /// side: adding it to the Library (unless that's where it's shown) and
    /// reading it.
    fn cover_actions<'a>(entry: &SeriesEntry, add_state: AddState) -> Element<'a, super::Message> {
        let add = match add_state {
            AddState::Available => Some(cover_overlay::action(
                "circle-plus",
                "Add to library",
                Some(super::Message::AddToLibrary(entry.clone())),
            )),
            AddState::Adding => Some(cover_overlay::action("download", "Adding…", None)),
            AddState::Added => Some(cover_overlay::action("circle-check", "In library", None)),
            AddState::Hidden => None,
        };
        let read = cover_overlay::action(
            "book-open",
            "Read",
            Some(super::Message::EntrySelected(entry.clone())),
        );

        cover_overlay::shade(
            add.into_iter().chain([read]).collect(),
            border::top(COVER_RADIUS),
        )
    }

    /// How opaque an entry's cover is drawn: a read entry's is faded, so what's
    /// left to read stands out.
    pub(super) fn cover_opacity(state: ReadState) -> f32 {
        match state {
            ReadState::Read => 0.4,
            ReadState::Unread | ReadState::InProgress(_) => 1.0,
        }
    }

    /// How full an entry's progress bar is, or `None` for an unread entry,
    /// which gets no bar. A started entry always shows a sliver, so it never
    /// looks unread, even when its page count is unknown.
    pub(super) fn progress_value(state: ReadState) -> Option<f32> {
        const MIN_VISIBLE: f32 = 0.05;

        match state {
            ReadState::Unread => None,
            ReadState::InProgress(fraction) => {
                Some(fraction.unwrap_or(0.0).clamp(MIN_VISIBLE, 1.0))
            }
            ReadState::Read => Some(1.0),
        }
    }

    /// A thin bar laid over the bottom edge of a cover.
    fn read_progress_bar<'a>(value: f32) -> Element<'a, super::Message> {
        let bar = progress_bar(0.0..=1.0, value)
            .length(Fill)
            .girth(Fixed(4.0))
            .style(|theme: &Theme| {
                let palette = theme.extended_palette();

                progress_bar::Style {
                    background: Background::Color(iced::Color::BLACK.scale_alpha(0.45)),
                    bar: Background::Color(palette.primary.base.color),
                    border: Border {
                        radius: 2.0.into(),
                        ..Border::default()
                    },
                }
            });

        container(bar)
            .padding(8)
            .width(Fill)
            .height(Fill)
            .align_bottom(Fill)
            .into()
    }

    /// Renders a cover if its handle has loaded, otherwise nothing (the styled
    /// box around it stands in as the placeholder).
    fn cover_image<'a>(
        handle: Option<&image::Handle>,
        opacity: f32,
        radius: border::Radius,
    ) -> Element<'a, super::Message> {
        match handle {
            Some(handle) => image(handle.clone())
                .width(Fill)
                .height(Fill)
                .content_fit(ContentFit::Cover)
                .opacity(opacity)
                .border_radius(radius)
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
                radius: COVER_RADIUS.into(),
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
            read_state: Default::default(),
        }
    }

    fn chapter_entry(id: &str, label: &str, pages: Option<u16>) -> SeriesEntry {
        SeriesEntry {
            external_id: id.to_string(),
            kind: EntryKind::Chapter,
            label: label.to_string(),
            pages,
            read_state: Default::default(),
        }
    }

    fn location() -> CollectionRef {
        CollectionRef {
            integration_id: "source-1".to_string(),
            collection: crate::models::collection::Collection::new(
                "lib-1",
                "Manga",
                crate::models::collection::CollectionKind::Library,
            ),
        }
    }

    fn series_detail(entries: AsyncModel<Vec<SeriesEntry>, String>) -> SeriesDetail {
        SeriesDetail {
            entries,
            ..SeriesDetail::with_origin(
                "series-1".to_string(),
                "Beneath the Trees".to_string(),
                Some(183),
                Origin::Source(location()),
            )
        }
    }

    fn library_detail() -> SeriesDetail {
        SeriesDetail::with_origin(
            "1".to_string(),
            "Series 1".to_string(),
            None,
            Origin::Library {
                integration_id: "s".to_string(),
            },
        )
    }

    fn library_item(id: &str, pages: i32, read: i32) -> LibraryItem {
        crate::models::library_item::tests::item("s", "1", id, pages, read)
    }

    #[test]
    fn new_starts_in_loading_with_the_contents_tab_active() {
        let (detail, _task) = SeriesDetail::new(
            "1".to_string(),
            "One Piece".to_string(),
            Some(42),
            location(),
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
    fn back_pressed_returns_back_to_the_collection() {
        let mut detail = series_detail(AsyncModel::Loading);

        let action = detail.update(Message::BackPressed);

        assert!(matches!(action, Action::Back(location) if location == self::location()));
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
        let _ = ui
            .click(iced::widget::Id::new(crate::screen::back_button::ID))
            .expect("the back chevron should be found");

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

    #[test]
    fn read_covers_are_faded_and_started_ones_get_a_progress_bar() {
        use crate::models::series_entry::ReadState;
        use view_helper::{cover_opacity, progress_value};

        assert_eq!(cover_opacity(ReadState::Unread), 1.0);
        assert_eq!(cover_opacity(ReadState::InProgress(Some(0.5))), 1.0);
        assert!(cover_opacity(ReadState::Read) < 1.0);

        assert_eq!(progress_value(ReadState::Unread), None);
        assert_eq!(progress_value(ReadState::InProgress(Some(0.5))), Some(0.5));
        assert_eq!(progress_value(ReadState::Read), Some(1.0));
        // A started entry always shows something, even without a page count.
        assert!(progress_value(ReadState::InProgress(None)).unwrap() > 0.0);
        assert!(progress_value(ReadState::InProgress(Some(0.0))).unwrap() > 0.0);
    }

    #[test]
    fn view_renders_cards_in_every_read_state() {
        use crate::models::series_entry::ReadState;

        let with_state = |id, label, read_state| SeriesEntry {
            read_state,
            ..chapter_entry(id, label, Some(20))
        };
        let detail = series_detail(AsyncModel::Loaded(vec![
            with_state("1", "#1", ReadState::Read),
            with_state("2", "#2", ReadState::InProgress(Some(0.5))),
            with_state("3", "#3", ReadState::Unread),
        ]));

        let mut ui = simulator(detail.view());

        for label in ["#1", "#2", "#3"] {
            assert!(ui.find(label).is_ok(), "{label} card should render");
        }
    }

    #[test]
    fn hovering_a_cover_offers_to_add_the_entry_or_read_it() {
        let detail = series_detail(AsyncModel::Loaded(vec![chapter_entry("1", "#1", Some(24))]));
        let entry = chapter_entry("1", "#1", Some(24));

        let mut ui = simulator(detail.view());
        let _ = ui
            .click("Add to library")
            .expect("the add action should be found");
        let _ = ui.click("Read").expect("the read action should be found");
        let messages: Vec<_> = ui.into_messages().collect();

        assert_eq!(
            messages,
            vec![
                Message::AddToLibrary(entry.clone()),
                Message::EntrySelected(entry)
            ]
        );
    }

    #[test]
    fn entries_already_in_the_library_are_not_offered_again() {
        let mut detail = series_detail(AsyncModel::Loaded(vec![chapter_entry("1", "#1", None)]));
        let _ = detail.update(Message::LibraryLoaded(Ok(vec![library_item("1", 10, 0)])));

        assert_eq!(
            detail.add_state(&chapter_entry("1", "#1", None)),
            AddState::Added
        );
        let action = detail.update(Message::AddToLibrary(chapter_entry("1", "#1", None)));
        assert!(matches!(action, Action::None));

        let mut ui = simulator(detail.view());
        assert!(ui.find("In library").is_ok());
        assert!(ui.find("Add to library").is_err());
    }

    #[test]
    fn adding_to_the_library_saves_once_and_records_the_result() {
        let mut detail = series_detail(AsyncModel::Loaded(vec![chapter_entry("1", "#1", None)]));
        let entry = chapter_entry("1", "#1", None);

        // The task isn't run here, so nothing is saved.
        assert!(matches!(
            detail.update(Message::AddToLibrary(entry.clone())),
            Action::Run(_)
        ));
        assert_eq!(detail.add_state(&entry), AddState::Adding);
        // A second click while it's saved does nothing.
        assert!(matches!(
            detail.update(Message::AddToLibrary(entry.clone())),
            Action::None
        ));

        let _ = detail.update(Message::AddedToLibrary(
            "ch:1".to_string(),
            Ok(Box::new(library_item("1", 10, 0))),
        ));
        assert_eq!(detail.add_state(&entry), AddState::Added);
    }

    #[test]
    fn a_failed_add_is_reported_and_can_be_retried() {
        let mut detail = series_detail(AsyncModel::Loaded(vec![chapter_entry("1", "#1", None)]));
        let entry = chapter_entry("1", "#1", None);
        let _ = detail.update(Message::AddToLibrary(entry.clone()));

        let _ = detail.update(Message::AddedToLibrary(
            "ch:1".to_string(),
            Err("offline".to_string()),
        ));

        assert_eq!(detail.add_state(&entry), AddState::Available);
        assert!(detail.launch_error.as_deref().unwrap().contains("offline"));
    }

    #[test]
    fn a_library_series_lists_its_items_with_their_read_state() {
        let mut detail = library_detail();

        let _ = detail.update(Message::LibraryLoaded(Ok(vec![
            library_item("1", 10, 10),
            library_item("2", 10, 5),
            library_item("3", 10, 0),
        ])));

        let AsyncModel::Loaded(entries) = &detail.entries else {
            panic!("the items should be the entries");
        };
        let states: Vec<_> = entries.iter().map(|entry| entry.read_state).collect();
        use crate::models::series_entry::ReadState;
        assert_eq!(
            states,
            vec![
                ReadState::Read,
                ReadState::InProgress(Some(0.5)),
                ReadState::Unread
            ]
        );

        let mut ui = simulator(detail.view());
        assert!(ui.find("Series 1").is_ok());
        assert!(ui.find("#2").is_ok());
        // Library items can be read, but there's nothing to add.
        assert!(ui.find("Read").is_ok());
        assert!(ui.find("Add to library").is_err());
    }

    #[test]
    fn a_library_series_shows_the_progress_synced_from_its_source() {
        use crate::models::series_entry::ReadState;
        let mut detail = library_detail();

        // The sync task isn't run here.
        assert!(matches!(
            detail.update(Message::LibraryLoaded(Ok(vec![library_item("1", 10, 0)]))),
            Action::Run(_)
        ));
        let _ = detail.update(Message::LibrarySynced(vec![library_item("1", 10, 10)]));

        let AsyncModel::Loaded(entries) = &detail.entries else {
            panic!("the items should be the entries");
        };
        assert_eq!(entries[0].read_state, ReadState::Read);
    }

    #[test]
    fn a_sources_series_does_not_sync_its_library_items() {
        let mut detail = series_detail(AsyncModel::Loaded(vec![chapter_entry("1", "#1", None)]));

        let action = detail.update(Message::LibraryLoaded(Ok(vec![library_item("1", 10, 0)])));

        assert!(matches!(action, Action::None));
    }

    fn library_detail_with(items: Vec<LibraryItem>) -> SeriesDetail {
        let mut detail = library_detail();
        let _ = detail.update(Message::LibraryLoaded(Ok(items)));
        detail
    }

    #[test]
    fn the_menu_toggles_and_closes_on_a_click_elsewhere() {
        let mut detail = library_detail_with(vec![library_item("1", 10, 0)]);
        let entry = chapter_entry("1", "#1", Some(10));

        let _ = detail.update(Message::MenuToggled("ch:1".to_string()));
        assert!(detail.card_state(&entry).menu_open);
        let _ = detail.update(Message::MenuToggled("ch:1".to_string()));
        assert!(!detail.card_state(&entry).menu_open);

        let _ = detail.update(Message::MenuToggled("ch:1".to_string()));
        let _ = detail.update(Message::MenuDismissed);
        assert!(!detail.card_state(&entry).menu_open);
    }

    #[test]
    fn the_open_menu_offers_to_delete_the_item() {
        let mut detail = library_detail_with(vec![library_item("1", 10, 0)]);
        let entry = library_item("1", 10, 0).as_entry();

        let mut ui = simulator(detail.view());
        assert!(ui.find("Delete").is_err());
        drop(ui);

        let _ = detail.update(Message::MenuToggled("ch:1".to_string()));
        let mut ui = simulator(detail.view());
        let _ = ui.click("Delete").expect("the delete item should be found");
        let messages: Vec<_> = ui.into_messages().collect();

        assert_eq!(messages, vec![Message::RemoveFromLibrary(entry.clone())]);
        assert!(matches!(
            detail.update(Message::RemoveFromLibrary(entry)),
            Action::Run(_)
        ));
        // Choosing an item closes the menu.
        assert_eq!(detail.menu, None);
    }

    #[test]
    fn deleting_an_item_drops_it_from_a_library_series() {
        let mut detail =
            library_detail_with(vec![library_item("1", 10, 0), library_item("2", 10, 0)]);

        let action = detail.update(Message::RemovedFromLibrary("ch:1".to_string(), Ok(())));

        assert!(matches!(action, Action::None));
        let AsyncModel::Loaded(entries) = &detail.entries else {
            panic!("the entries should stay loaded");
        };
        assert_eq!(entries.len(), 1);
        assert!(!detail.library.contains_key("ch:1"));
    }

    #[test]
    fn deleting_the_last_item_goes_back_to_the_library() {
        let mut detail = library_detail_with(vec![library_item("1", 10, 0)]);

        let action = detail.update(Message::RemovedFromLibrary("ch:1".to_string(), Ok(())));

        assert!(matches!(action, Action::BackToLibrary));
    }

    #[test]
    fn deleting_from_a_sources_series_offers_to_add_it_again() {
        let mut detail = series_detail(AsyncModel::Loaded(vec![chapter_entry("1", "#1", None)]));
        let _ = detail.update(Message::LibraryLoaded(Ok(vec![library_item("1", 10, 0)])));

        let action = detail.update(Message::RemovedFromLibrary("ch:1".to_string(), Ok(())));

        assert!(matches!(action, Action::None));
        assert_eq!(
            detail.add_state(&chapter_entry("1", "#1", None)),
            AddState::Available
        );
    }

    #[test]
    fn a_failed_delete_is_reported() {
        let mut detail = library_detail_with(vec![library_item("1", 10, 0)]);

        let _ = detail.update(Message::RemovedFromLibrary(
            "ch:1".to_string(),
            Err("locked".to_string()),
        ));

        assert!(detail.launch_error.as_deref().unwrap().contains("locked"));
        assert!(detail.library.contains_key("ch:1"));
    }

    #[test]
    fn a_library_series_goes_back_to_the_library() {
        let mut detail = library_detail();

        assert!(matches!(
            detail.update(Message::BackPressed),
            Action::BackToLibrary
        ));
    }

    #[test]
    fn closing_the_reader_reloads_a_library_series_progress() {
        let mut detail = library_detail();

        assert!(matches!(
            detail.update(Message::LibraryReaderClosed),
            Action::Run(_)
        ));
    }
}
