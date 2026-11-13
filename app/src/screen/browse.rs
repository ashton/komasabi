use std::collections::{HashMap, HashSet};
use std::path::PathBuf;

use iced::advanced::widget::Id as WidgetId;
use iced::widget::image;
use iced::{Element, Task};

use crate::models::{AsyncModel, collection::CollectionRef, collection_item::CollectionItem};
use crate::providers::{Provider, provider_for_integration};
use crate::repository::library as library_repository;
use crate::screen::card::DownloadState;

/// Number of items fetched per page, and the unit eviction operates on:
/// pages are dropped from (or re-fetched back into) memory as a whole,
/// rather than item-by-item.
const PAGE_SIZE: u32 = 20;

/// How close to the bottom of the scrollable (as a fraction of its scrollable
/// range, 0.0 = top, 1.0 = bottom) the user must be before the next page is
/// requested.
const LOAD_NEXT_PAGE_THRESHOLD: f32 = 0.8;

/// How many of the most recently loaded pages to keep fully populated in
/// memory (4 pages * PAGE_SIZE = 80 items). Older pages are evicted, but
/// only once this many pages have been loaded ahead of them — eviction
/// tracks loading progress, not the live scroll position, so scrolling back
/// up by a page or two never shows placeholders.
const KEEP_PAGES_LOADED: usize = 4;

/// Identifies the items scrollable so it can be measured after a page loads,
/// to tell whether the loaded items actually overflow the viewport (see
/// [`measure_items_scrollable`]).
const ITEMS_SCROLLABLE_ID: &str = "browse-items";

/// A collection's series, as a paginated grid of cards.
#[derive(Debug)]
pub struct Browse {
    /// The collection being browsed, and its source.
    location: CollectionRef,
    items: AsyncModel<Vec<Option<CollectionItem>>, String>,
    pagination: Option<Pagination>,
    /// Decoded cover images, keyed by item `external_id`. Populated lazily as
    /// pages load (see [`Browse::request_page_covers`]) and pruned alongside
    /// page eviction so it doesn't grow without bound.
    covers: HashMap<String, image::Handle>,
    /// `external_id`s whose cover is currently being fetched, so a page isn't
    /// requested more than once per session.
    covers_in_flight: HashSet<String>,
    /// `external_id`s of the series being saved in the Library.
    adding_series: HashSet<String>,
    /// `external_id`s of this source's series that have items in the Library,
    /// loaded when the screen opens and kept up to date as series are added
    /// and removed.
    library_series: HashSet<String>,
    /// The last add-to/remove-from-library failure, shown as a status banner
    /// until the next attempt.
    add_error: Option<String>,
    /// The `external_id` of the series whose card menu is open, if any.
    menu: Option<String>,
    /// `external_id`s of this source's series downloaded in full, loaded when
    /// the screen opens and kept up to date as series are downloaded.
    downloaded_series: HashSet<String>,
    /// `external_id`s of the series being downloaded.
    downloading: HashSet<String>,
}

/// Whether a series card offers to add the whole series to the Library.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AddState {
    /// Not in the Library: offer to add it.
    Available,
    /// Its volumes/chapters are being saved in the Library.
    Adding,
    /// It has items in the Library.
    Added,
}

/// How a series' card is drawn, beyond the series itself.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CardState {
    pub add: AddState,
    pub download: DownloadState,
    /// Whether the card's menu is open.
    pub menu_open: bool,
}

#[derive(Debug)]
struct Pagination {
    next_page: u32,
    page_size: u32,
    has_more: bool,
    loading_next: bool,
    /// 0-based page indices currently being re-fetched after eviction (see
    /// `request_page_refetch`), keyed separately from `loading_next` since
    /// several evicted pages can be scrolled through in quick succession.
    refetching_pages: std::collections::HashSet<usize>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Message {
    ItemsLoaded(Result<Vec<CollectionItem>, String>),
    NextPageLoaded(Result<Vec<CollectionItem>, String>),
    PageRefetched(usize, Result<Vec<CollectionItem>, String>),
    /// Cover image bytes that have been loaded for a page, keyed by item
    /// `external_id`. Failed/omitted ids are simply absent.
    CoversLoaded(Vec<(String, Vec<u8>)>),
    Scrolled(ScrollMetrics),
    ContentOverflowChecked(bool),
    /// A series' title, or "Open" on its cover, was clicked; carries the
    /// item's `external_id`.
    SeriesSelected(String),
    /// "Add to library" was chosen on a series' cover (by `external_id`):
    /// save each of its volumes/chapters in the Library.
    AddSeriesToLibrary(String),
    /// A series (by `external_id`) was saved in the Library, carrying how many
    /// of its entries were newly added, or it failed to be.
    SeriesAddedToLibrary(String, Result<usize, String>),
    /// The ids of this source's series that have items in the Library.
    LibrarySeriesLoaded(Result<Vec<String>, String>),
    /// The "⋮" button of a series' card (by `external_id`) was pressed: open
    /// its menu, or close it if it's the one open.
    MenuToggled(String),
    /// A click landed outside the open menu, closing it.
    MenuDismissed,
    /// "Remove from library" was chosen in a series' menu: remove all of its
    /// items from the Library.
    RemoveSeriesFromLibrary(String),
    /// A series (by `external_id`) was removed from the Library, carrying how
    /// many items went with it, or it failed to be.
    SeriesRemovedFromLibrary(String, Result<usize, String>),
    /// The ids of this source's series downloaded in full.
    DownloadedSeriesLoaded(HashSet<String>),
    /// "Download" was chosen in a series' menu (by `external_id`): download
    /// each of its volumes/chapters for reading offline.
    DownloadSeries(String),
    /// A series (by `external_id`) finished downloading, carrying how many of
    /// its entries were downloaded, or why some couldn't be.
    SeriesDownloaded(String, Result<usize, String>),
    BackPressed,
    /// The "Import items" button was pressed; opens the file picker.
    ImportPressed,
    /// Files chosen in the picker to import into this collection (`None` if
    /// cancelled).
    ImportSourcesPicked(Option<Vec<PathBuf>>),
    /// The import finished, carrying how many items were added (or an error).
    ItemsImported(Result<usize, String>),
}

/// The scroll state needed to tell which pages are actually on-screen.
///
/// `relative_offset_y` (0.0 = top, 1.0 = bottom) is only valid as a measure
/// of *scroll-track* position, i.e. "how close to the bottom", because it's
/// normalized against the scrollable *range* (`content_height -
/// viewport_height`), not the content height itself. Using it to guess which
/// page is visible silently breaks whenever the viewport is a non-trivial
/// fraction of the content's height (which it always is here, since only
/// `KEEP_PAGES_LOADED` pages are ever kept intact) — so page visibility is
/// computed separately below from the absolute pixel offset instead.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ScrollMetrics {
    relative_offset_y: f32,
    absolute_offset_y: f32,
    viewport_height: f32,
    content_height: f32,
}

#[derive(Debug)]
pub enum Action {
    None,
    Run(Task<Message>),
    OpenSeries {
        series_external_id: String,
        series_name: String,
        series_pages: Option<u16>,
        location: CollectionRef,
    },
    /// Back to the collections of this collection's source.
    BackPressed(String),
}

impl Browse {
    /// Opens `location`'s collection, starting on its first page. A collection
    /// that's [`importable`](crate::models::collection::Collection::importable)
    /// also offers importing files into it.
    pub fn new(location: CollectionRef) -> (Self, Task<Message>) {
        let task = Task::batch([
            Task::perform(
                crate::runtime::on_tokio(fetch_items_page(location.clone(), 1, PAGE_SIZE)),
                Message::ItemsLoaded,
            ),
            Task::perform(
                library_repository::series_ids(location.integration_id.clone()),
                Message::LibrarySeriesLoaded,
            ),
            Task::perform(
                crate::downloads::downloaded_series(location.integration_id.clone()),
                Message::DownloadedSeriesLoaded,
            ),
        ]);

        (
            Self {
                location,
                items: AsyncModel::Loading,
                pagination: Some(Pagination {
                    next_page: 2,
                    page_size: PAGE_SIZE,
                    has_more: true,
                    loading_next: false,
                    refetching_pages: std::collections::HashSet::new(),
                }),
                covers: HashMap::new(),
                covers_in_flight: HashSet::new(),
                adding_series: HashSet::new(),
                library_series: HashSet::new(),
                add_error: None,
                menu: None,
                downloaded_series: HashSet::new(),
                downloading: HashSet::new(),
            },
            task,
        )
    }

    pub fn update(&mut self, message: Message) -> Action {
        // Doing anything else on the screen closes an open card menu.
        if matches!(
            message,
            Message::SeriesSelected(_)
                | Message::BackPressed
                | Message::ImportPressed
                | Message::AddSeriesToLibrary(_)
                | Message::RemoveSeriesFromLibrary(_)
                | Message::DownloadSeries(_)
                | Message::MenuDismissed
        ) {
            self.menu = None;
        }

        match message {
            Message::ItemsLoaded(result) => {
                let result = result.map(|items| {
                    let has_more = items.len() as u32 == PAGE_SIZE;
                    if let Some(pagination) = &mut self.pagination {
                        pagination.has_more = has_more;
                    }
                    items.into_iter().map(Some).collect::<Vec<_>>()
                });

                let loaded = result.is_ok();
                self.items = result.into();

                if loaded {
                    Action::Run(with_cover_task(
                        measure_items_scrollable(),
                        self.request_page_covers(),
                    ))
                } else {
                    Action::None
                }
            }

            Message::NextPageLoaded(Ok(new_items)) => {
                let mut last_loaded_page = None;

                if let Some(pagination) = &mut self.pagination {
                    pagination.loading_next = false;
                    pagination.has_more = new_items.len() as u32 == pagination.page_size;
                    pagination.next_page += 1;
                    last_loaded_page = Some((pagination.next_page as usize).saturating_sub(2));
                }

                if let AsyncModel::Loaded(items) = &mut self.items {
                    items.extend(new_items.into_iter().map(Some));

                    // Forward loading only ever happens once the user has
                    // scrolled near the bottom, so by the time a page this
                    // far ahead loads, pages this far behind are safely
                    // off-screen.
                    if let Some(last_loaded_page) = last_loaded_page {
                        let range = eviction_range(
                            items.len(),
                            PAGE_SIZE,
                            last_loaded_page,
                            KEEP_PAGES_LOADED,
                        );
                        if !range.is_empty() {
                            tracing::debug!(
                                "evicting pages for items[{range:?}] (last_loaded_page={last_loaded_page}, keep={KEEP_PAGES_LOADED})"
                            );
                        }
                        for slot in &mut items[range] {
                            *slot = None;
                        }
                    }
                }

                self.prune_covers();

                Action::Run(with_cover_task(
                    measure_items_scrollable(),
                    self.request_page_covers(),
                ))
            }

            Message::NextPageLoaded(Err(_)) => {
                if let Some(pagination) = &mut self.pagination {
                    pagination.loading_next = false;
                }

                Action::None
            }

            Message::CoversLoaded(covers) => {
                for (external_id, bytes) in covers {
                    self.covers_in_flight.remove(&external_id);
                    self.covers
                        .insert(external_id, image::Handle::from_bytes(bytes));
                }

                Action::None
            }

            Message::Scrolled(metrics) => {
                let mut pages_needing_refetch = Vec::new();

                if let AsyncModel::Loaded(items) = &self.items
                    && let Some(visible_pages) = visible_page_range(
                        items.len(),
                        PAGE_SIZE,
                        metrics.absolute_offset_y,
                        metrics.viewport_height,
                        metrics.content_height,
                    )
                {
                    for page_index in visible_pages {
                        let start = page_index * PAGE_SIZE as usize;
                        let end = (start + PAGE_SIZE as usize).min(items.len());

                        if items[start..end].iter().any(Option::is_none) {
                            pages_needing_refetch.push(page_index);
                        }
                    }
                }

                let refetch_tasks: Vec<_> = pages_needing_refetch
                    .into_iter()
                    .filter_map(|page_index| self.request_page_refetch(page_index))
                    .collect();

                if !refetch_tasks.is_empty() {
                    return Action::Run(Task::batch(refetch_tasks));
                }

                if !is_near_bottom(metrics.relative_offset_y, LOAD_NEXT_PAGE_THRESHOLD) {
                    return Action::None;
                }

                self.request_next_page().unwrap_or(Action::None)
            }

            Message::PageRefetched(page_index, Ok(refetched_items)) => {
                if let Some(pagination) = &mut self.pagination {
                    pagination.refetching_pages.remove(&page_index);
                }

                tracing::debug!(
                    "re-fetched page {} with {} item(s)",
                    page_index + 1,
                    refetched_items.len()
                );

                if let AsyncModel::Loaded(items) = &mut self.items {
                    let start = page_index * PAGE_SIZE as usize;

                    for (offset, item) in refetched_items.into_iter().enumerate() {
                        if let Some(slot) = items.get_mut(start + offset) {
                            *slot = Some(item);
                        }
                    }
                }

                match self.request_page_covers() {
                    Some(task) => Action::Run(task),
                    None => Action::None,
                }
            }

            Message::PageRefetched(page_index, Err(error)) => {
                if let Some(pagination) = &mut self.pagination {
                    pagination.refetching_pages.remove(&page_index);
                }

                tracing::debug!("re-fetching page {} failed: {error}", page_index + 1);

                Action::None
            }

            // The items scrollable doesn't overflow its viewport, so a
            // scroll gesture can never happen on its own: eagerly request
            // another page, as if the user had scrolled to the bottom. This
            // repeats (via the Action::Run chain from NextPageLoaded) until
            // either enough items are loaded to overflow the viewport, or
            // the collection is exhausted.
            Message::ContentOverflowChecked(overflows) => {
                if overflows {
                    return Action::None;
                }

                self.request_next_page().unwrap_or(Action::None)
            }

            Message::SeriesSelected(external_id) => {
                let Some(item) = self.loaded_item(&external_id) else {
                    return Action::None;
                };

                Action::OpenSeries {
                    series_external_id: item.external_id.clone(),
                    series_name: item.name.clone(),
                    series_pages: item.pages,
                    location: self.location.clone(),
                }
            }

            Message::AddSeriesToLibrary(external_id) => {
                let Some(item) = self.loaded_item(&external_id) else {
                    return Action::None;
                };
                let series_name = item.name.clone();
                if self.library_series.contains(&external_id)
                    || !self.adding_series.insert(external_id.clone())
                {
                    return Action::None;
                }
                self.add_error = None;

                Action::Run(Task::perform(
                    crate::runtime::on_tokio(crate::library::add_series_to_library(
                        self.location.integration_id.clone(),
                        external_id.clone(),
                        series_name,
                    )),
                    move |result| Message::SeriesAddedToLibrary(external_id.clone(), result),
                ))
            }

            Message::SeriesAddedToLibrary(external_id, result) => {
                self.adding_series.remove(&external_id);
                match result {
                    Ok(added) => {
                        tracing::debug!("added {added} item(s) of {external_id} to the library");
                        self.library_series.insert(external_id);
                    }
                    Err(error) => {
                        tracing::debug!("failed to add {external_id} to the library: {error}");
                        self.add_error = Some(format!("Could not add to library: {error}"));
                    }
                }
                Action::None
            }

            Message::LibrarySeriesLoaded(Ok(series)) => {
                self.library_series.extend(series);
                Action::None
            }
            Message::LibrarySeriesLoaded(Err(error)) => {
                tracing::debug!("failed to load the library's series: {error}");
                Action::None
            }

            Message::DownloadedSeriesLoaded(series) => {
                self.downloaded_series.extend(series);
                Action::None
            }

            Message::DownloadSeries(external_id) => {
                let Some(item) = self.loaded_item(&external_id) else {
                    return Action::None;
                };
                let series_name = item.name.clone();
                if self.downloaded_series.contains(&external_id)
                    || !self.downloading.insert(external_id.clone())
                {
                    return Action::None;
                }
                self.add_error = None;

                Action::Run(Task::perform(
                    crate::runtime::on_tokio(crate::downloads::download_series(
                        self.location.integration_id.clone(),
                        external_id.clone(),
                        series_name,
                    )),
                    move |result| Message::SeriesDownloaded(external_id.clone(), result),
                ))
            }

            Message::SeriesDownloaded(external_id, result) => {
                self.downloading.remove(&external_id);
                match result {
                    Ok(downloaded) => {
                        tracing::debug!("downloaded {downloaded} item(s) of {external_id}");
                        self.downloaded_series.insert(external_id);
                    }
                    Err(error) => {
                        tracing::debug!("failed to download {external_id}: {error}");
                        self.add_error = Some(format!("Could not download: {error}"));
                    }
                }
                Action::None
            }

            Message::MenuToggled(external_id) => {
                self.menu = (self.menu.as_ref() != Some(&external_id)).then_some(external_id);
                Action::None
            }
            Message::MenuDismissed => Action::None,

            Message::RemoveSeriesFromLibrary(external_id) => {
                if !self.library_series.contains(&external_id) {
                    return Action::None;
                }
                self.add_error = None;

                Action::Run(Task::perform(
                    library_repository::remove_series(
                        self.location.integration_id.clone(),
                        external_id.clone(),
                    ),
                    move |result| Message::SeriesRemovedFromLibrary(external_id.clone(), result),
                ))
            }

            Message::SeriesRemovedFromLibrary(external_id, Ok(removed)) => {
                tracing::debug!("removed {removed} item(s) of {external_id} from the library");
                self.library_series.remove(&external_id);
                Action::None
            }
            Message::SeriesRemovedFromLibrary(external_id, Err(error)) => {
                tracing::debug!("failed to remove {external_id} from the library: {error}");
                self.add_error = Some(format!("Could not remove from library: {error}"));
                Action::None
            }

            Message::BackPressed => Action::BackPressed(self.location.integration_id.clone()),

            Message::ImportPressed => {
                Action::Run(Task::perform(pick_files(), Message::ImportSourcesPicked))
            }

            Message::ImportSourcesPicked(None) => Action::None,
            Message::ImportSourcesPicked(Some(sources)) if sources.is_empty() => Action::None,
            Message::ImportSourcesPicked(Some(sources)) => Action::Run(Task::perform(
                crate::runtime::on_tokio(import_files(self.location.clone(), sources)),
                Message::ItemsImported,
            )),

            Message::ItemsImported(Ok(count)) => {
                tracing::debug!("imported {count} item(s) into the collection");
                // Re-scan from the first page so the new series show up.
                self.reload()
            }
            Message::ItemsImported(Err(error)) => {
                tracing::debug!("failed to import items: {error}");
                Action::None
            }
        }
    }

    /// Resets to the first page and reloads it, used after items are imported so
    /// the freshly-added series appear.
    fn reload(&mut self) -> Action {
        let Some(pagination) = &mut self.pagination else {
            return Action::None;
        };

        pagination.next_page = 2;
        pagination.has_more = true;
        pagination.loading_next = false;
        pagination.refetching_pages.clear();

        self.items = AsyncModel::Loading;
        self.covers.clear();
        self.covers_in_flight.clear();

        Action::Run(Task::perform(
            crate::runtime::on_tokio(fetch_items_page(self.location.clone(), 1, PAGE_SIZE)),
            Message::ItemsLoaded,
        ))
    }

    /// Finds a currently-loaded item by its `external_id`, if present.
    fn loaded_item(&self, external_id: &str) -> Option<&CollectionItem> {
        let AsyncModel::Loaded(items) = &self.items else {
            return None;
        };

        items
            .iter()
            .flatten()
            .find(|item| item.external_id == external_id)
    }

    pub fn view<'a>(&'a self) -> Element<'a, Message> {
        view_helper::view(
            &self.location.collection.name,
            &self.items,
            &self.covers,
            &|external_id: &str| self.card_state(external_id),
            self.location.collection.importable,
            self.add_error.as_deref(),
            self.menu.is_some(),
        )
    }

    /// How the series `external_id`'s card is drawn.
    fn card_state(&self, external_id: &str) -> CardState {
        let download = if self.downloading.contains(external_id) {
            DownloadState::Downloading
        } else if self.downloaded_series.contains(external_id) {
            DownloadState::Downloaded
        } else {
            DownloadState::NotDownloaded
        };

        CardState {
            add: self.add_state(external_id),
            download,
            menu_open: self.menu.as_deref() == Some(external_id),
        }
    }

    /// Whether the series `external_id`'s card offers to add it to the
    /// Library.
    fn add_state(&self, external_id: &str) -> AddState {
        if self.adding_series.contains(external_id) {
            AddState::Adding
        } else if self.library_series.contains(external_id) {
            AddState::Added
        } else {
            AddState::Available
        }
    }

    /// Requests the next page, if pagination is set up for it, there isn't
    /// already a request in flight, and there's more to fetch.
    fn request_next_page(&mut self) -> Option<Action> {
        let pagination = self.pagination.as_mut()?;

        if pagination.loading_next || !pagination.has_more {
            return None;
        }

        pagination.loading_next = true;

        let page_number = pagination.next_page;
        let page_size = pagination.page_size;

        Some(Action::Run(Task::perform(
            crate::runtime::on_tokio(fetch_items_page(
                self.location.clone(),
                page_number,
                page_size,
            )),
            Message::NextPageLoaded,
        )))
    }

    /// Re-fetches a single (0-based) page that was previously evicted, if it
    /// isn't already being re-fetched. Returns a bare `Task` (rather than an
    /// `Action`) so callers can request several pages at once and combine
    /// them with `Task::batch`.
    fn request_page_refetch(&mut self, page_index: usize) -> Option<Task<Message>> {
        let pagination = self.pagination.as_mut()?;

        if pagination.refetching_pages.contains(&page_index) {
            return None;
        }

        pagination.refetching_pages.insert(page_index);
        tracing::debug!(
            "re-fetching evicted page {} (0-based index {page_index})",
            page_index + 1
        );

        let page_number = page_index as u32 + 1;
        let page_size = pagination.page_size;

        Some(Task::perform(
            crate::runtime::on_tokio(fetch_items_page(
                self.location.clone(),
                page_number,
                page_size,
            )),
            move |result| Message::PageRefetched(page_index, result),
        ))
    }

    /// Builds a task to fetch covers for every currently-loaded item whose
    /// cover isn't already cached or in flight, marking them in flight.
    /// Returns `None` when there's nothing left to fetch.
    fn request_page_covers(&mut self) -> Option<Task<Message>> {
        let AsyncModel::Loaded(items) = &self.items else {
            return None;
        };

        let series_ids: Vec<String> = items
            .iter()
            .flatten()
            .map(|item| item.external_id.clone())
            .filter(|id| !self.covers.contains_key(id) && !self.covers_in_flight.contains(id))
            .collect();

        if series_ids.is_empty() {
            return None;
        }

        self.covers_in_flight.extend(series_ids.iter().cloned());

        Some(Task::perform(
            crate::runtime::on_tokio(fetch_page_covers(
                self.location.integration_id.clone(),
                series_ids,
            )),
            Message::CoversLoaded,
        ))
    }

    /// Drops cached covers whose item is no longer loaded (e.g. was evicted),
    /// keeping the cover map bounded to roughly the live item window.
    fn prune_covers(&mut self) {
        let AsyncModel::Loaded(items) = &self.items else {
            return;
        };

        let present: HashSet<&str> = items
            .iter()
            .flatten()
            .map(|item| item.external_id.as_str())
            .collect();

        self.covers.retain(|id, _| present.contains(id.as_str()));
    }
}

/// Combines the viewport-measurement task that already runs after a page loads
/// with an optional cover-fetch task, so both run from a single `Action::Run`.
fn with_cover_task(measure: Task<Message>, covers: Option<Task<Message>>) -> Task<Message> {
    match covers {
        Some(covers) => Task::batch([measure, covers]),
        None => measure,
    }
}

/// Returns the (flat, page-aligned) index range of items old enough to be
/// evicted from memory: everything before the last `keep_pages_loaded`
/// pages, relative to `last_loaded_page` (the most recently loaded page).
fn eviction_range(
    total_items: usize,
    page_size: u32,
    last_loaded_page: usize,
    keep_pages_loaded: usize,
) -> std::ops::Range<usize> {
    let evict_before_page = last_loaded_page.saturating_sub(keep_pages_loaded.saturating_sub(1));

    0..(evict_before_page * page_size as usize).min(total_items)
}

/// Returns the inclusive range of (0-based) pages currently visible within
/// the scrollable's viewport, from the *absolute* pixel scroll offset,
/// viewport height and content height (all taken straight from iced's
/// `Viewport`), assuming pages contribute roughly equal height to the grid
/// since iced doesn't expose which individual items are actually on-screen.
///
/// Deliberately doesn't use `relative_offset`, which is normalized against
/// the scrollable *range* (`content_height - viewport_height`) rather than
/// the content height itself — using it here would misidentify the visible
/// page by an amount proportional to how much of the content the viewport
/// covers.
fn visible_page_range(
    total_items: usize,
    page_size: u32,
    absolute_offset_y: f32,
    viewport_height: f32,
    content_height: f32,
) -> Option<std::ops::RangeInclusive<usize>> {
    if total_items == 0 || page_size == 0 || content_height <= 0.0 {
        return None;
    }

    let total_pages = total_items.div_ceil(page_size as usize);
    let page_height = content_height / total_pages as f32;

    if page_height <= 0.0 {
        return None;
    }

    let top_page = (absolute_offset_y.max(0.0) / page_height) as usize;
    let bottom_page =
        ((absolute_offset_y.max(0.0) + viewport_height.max(0.0)) / page_height) as usize;

    Some(top_page.min(total_pages - 1)..=bottom_page.min(total_pages - 1))
}

fn is_near_bottom(relative_offset_y: f32, threshold: f32) -> bool {
    relative_offset_y >= threshold
}

/// Builds a task that measures the items scrollable's viewport against its
/// content and reports whether the content overflows it (i.e. whether
/// there's anything to scroll at all).
fn measure_items_scrollable() -> Task<Message> {
    iced::advanced::widget::operate(viewport_probe::overflow_probe(WidgetId::new(
        ITEMS_SCROLLABLE_ID,
    )))
    .map(Message::ContentOverflowChecked)
}

/// A widget [`Operation`](iced::advanced::widget::Operation) that finds a
/// scrollable by [`Id`](WidgetId) and reports whether its content overflows
/// its viewport, without needing to track viewport state across updates.
mod viewport_probe {
    use iced::Rectangle;
    use iced::advanced::widget::operation::{Outcome, Scrollable};
    use iced::advanced::widget::{Id, Operation};

    struct OverflowProbe {
        target: Id,
        overflows: Option<bool>,
    }

    impl Operation<bool> for OverflowProbe {
        fn traverse(&mut self, operate: &mut dyn FnMut(&mut dyn Operation<bool>)) {
            operate(self);
        }

        fn scrollable(
            &mut self,
            id: Option<&Id>,
            bounds: Rectangle,
            content_bounds: Rectangle,
            _translation: iced::Vector,
            _state: &mut dyn Scrollable,
        ) {
            if Some(&self.target) == id {
                self.overflows = Some(content_bounds.height > bounds.height);
            }
        }

        fn finish(&self) -> Outcome<bool> {
            match self.overflows {
                Some(overflows) => Outcome::Some(overflows),
                None => Outcome::None,
            }
        }
    }

    pub fn overflow_probe(target: Id) -> impl Operation<bool> {
        OverflowProbe {
            target,
            overflows: None,
        }
    }

    #[cfg(test)]
    mod tests {
        use iced::Size;
        use iced::advanced::widget::operation::scrollable::{AbsoluteOffset, RelativeOffset};

        use super::*;

        struct NoopScrollableState;

        impl Scrollable for NoopScrollableState {
            fn snap_to(&mut self, _offset: RelativeOffset<Option<f32>>) {}
            fn scroll_to(&mut self, _offset: AbsoluteOffset<Option<f32>>) {}
            fn scroll_by(
                &mut self,
                _offset: AbsoluteOffset,
                _bounds: Rectangle,
                _content_bounds: Rectangle,
            ) {
            }
        }

        fn rectangle(height: f32) -> Rectangle {
            Rectangle::new(iced::Point::ORIGIN, Size::new(100.0, height))
        }

        #[test]
        fn reports_overflow_when_content_is_taller_than_the_viewport() {
            let target = Id::new("items");
            let mut probe = overflow_probe(target.clone());

            probe.scrollable(
                Some(&target),
                rectangle(400.0),
                rectangle(1200.0),
                iced::Vector::default(),
                &mut NoopScrollableState,
            );

            assert!(matches!(probe.finish(), Outcome::Some(true)));
        }

        #[test]
        fn reports_no_overflow_when_content_fits_the_viewport() {
            let target = Id::new("items");
            let mut probe = overflow_probe(target.clone());

            probe.scrollable(
                Some(&target),
                rectangle(800.0),
                rectangle(400.0),
                iced::Vector::default(),
                &mut NoopScrollableState,
            );

            assert!(matches!(probe.finish(), Outcome::Some(false)));
        }

        #[test]
        fn ignores_scrollables_with_a_different_id() {
            let target = Id::new("items");
            let other = Id::new("something-else");
            let mut probe = overflow_probe(target);

            probe.scrollable(
                Some(&other),
                rectangle(400.0),
                rectangle(1200.0),
                iced::Vector::default(),
                &mut NoopScrollableState,
            );

            assert!(matches!(probe.finish(), Outcome::None));
        }
    }
}

async fn fetch_items_page(
    location: CollectionRef,
    page_number: u32,
    page_size: u32,
) -> Result<Vec<CollectionItem>, String> {
    let (provider, _namespace) = provider_for_integration(&location.integration_id).await?;
    provider
        .list_items(
            &location.collection.external_id,
            Some(page_number),
            Some(page_size),
        )
        .await
}

/// Opens the system picker for one or more comic/book files to import, `None` if
/// the user cancels.
async fn pick_files() -> Option<Vec<PathBuf>> {
    let home = std::env::var("HOME").unwrap_or_else(|_| "/".to_string());

    rfd::AsyncFileDialog::new()
        .set_directory(home)
        .add_filter(
            "Comics & Books",
            &["cbz", "cbr", "zip", "pdf", "epub", "doc"],
        )
        .pick_files()
        .await
        .map(|handles| {
            handles
                .into_iter()
                .map(|handle| handle.path().to_path_buf())
                .collect()
        })
}

/// Imports the picked files into the collection, through its source's
/// provider. Returns how many were newly added.
async fn import_files(location: CollectionRef, files: Vec<PathBuf>) -> Result<usize, String> {
    let (provider, _namespace) = provider_for_integration(&location.integration_id).await?;
    provider
        .import_files(&location.collection.external_id, &files)
        .await
}

/// Fetches covers for a page's series, returning `(external_id, bytes)` for
/// each one that resolved. Per-id and whole-batch failures are logged and
/// omitted rather than surfaced, so a partial result still renders what it can.
async fn fetch_page_covers(
    integration_id: String,
    series_ids: Vec<String>,
) -> Vec<(String, Vec<u8>)> {
    match load_page_covers(integration_id, series_ids).await {
        Ok(covers) => covers,
        Err(error) => {
            tracing::debug!("failed to load page covers: {error}");
            Vec::new()
        }
    }
}

async fn load_page_covers(
    integration_id: String,
    series_ids: Vec<String>,
) -> Result<Vec<(String, Vec<u8>)>, String> {
    let (provider, namespace) = provider_for_integration(&integration_id).await?;

    let mut covers = Vec::new();
    for series_id in series_ids {
        match provider.cover(&namespace, &series_id).await {
            Ok(bytes) => covers.push((series_id, bytes)),
            Err(error) => {
                tracing::debug!("failed to fetch cover for series {series_id}: {error}")
            }
        }
    }

    Ok(covers)
}

mod view_helper {
    use std::collections::HashMap;

    use iced::{
        Background, Border, Center, ContentFit, Element,
        Length::{Fill, Shrink},
        Theme, border,
        widget::{
            button, column, container, grid, hover, image, mouse_area, row, scrollable, stack, text,
        },
    };

    use super::{AddState, CardState, DownloadState};
    use crate::models::{AsyncModel, collection_item::CollectionItem};
    use crate::screen::card::{self, COVER_RADIUS};
    use crate::screen::cover_overlay;

    type Covers = HashMap<String, image::Handle>;
    /// How a series' card (by `external_id`) is drawn (see [`CardState`]).
    type CardStateOf<'b> = &'b dyn Fn(&str) -> CardState;

    pub fn view<'a>(
        title: &'a str,
        items: &'a AsyncModel<Vec<Option<CollectionItem>>, String>,
        covers: &'a Covers,
        card_state: CardStateOf<'_>,
        importable: bool,
        status: Option<&'a str>,
        menu_open: bool,
    ) -> Element<'a, super::Message> {
        let mut screen = column![top_bar(title, importable)];
        if let Some(status) = status {
            screen = screen.push(container(text(status).size(13)).padding([0, 20]));
        }

        let screen = screen
            .push(body(items, covers, card_state))
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

    fn top_bar<'a>(title: &'a str, importable: bool) -> Element<'a, super::Message> {
        let mut bar = row![
            crate::screen::back_button::back_button(super::Message::BackPressed),
            text(title).size(20),
            iced::widget::space::horizontal(),
        ]
        .spacing(10)
        .align_y(Center);

        if importable {
            bar = bar.push(button("Import items").on_press(super::Message::ImportPressed));
        }

        container(bar).padding(20).into()
    }

    fn body<'a>(
        items: &'a AsyncModel<Vec<Option<CollectionItem>>, String>,
        covers: &'a Covers,
        card_state: CardStateOf<'_>,
    ) -> Element<'a, super::Message> {
        match items {
            AsyncModel::Loaded(items) if items.is_empty() => empty_state(),
            AsyncModel::Loaded(items) => items_scrollable(items, covers, card_state),
            AsyncModel::Error(error) => error_state(error),
            AsyncModel::NotLoaded | AsyncModel::Loading => container(row![]).into(),
        }
    }

    fn items_scrollable<'a>(
        items: &'a [Option<CollectionItem>],
        covers: &'a Covers,
        card_state: CardStateOf<'_>,
    ) -> Element<'a, super::Message> {
        crate::touch_scroll::touch_scroll(
            scrollable(items_grid(items, covers, card_state))
                .id(iced::advanced::widget::Id::new(super::ITEMS_SCROLLABLE_ID))
                .width(Fill)
                .height(Fill)
                .on_scroll(|viewport| {
                    super::Message::Scrolled(super::ScrollMetrics {
                        relative_offset_y: viewport.relative_offset().y,
                        absolute_offset_y: viewport.absolute_offset().y,
                        viewport_height: viewport.bounds().height,
                        content_height: viewport.content_bounds().height,
                    })
                }),
        )
        .into()
    }

    fn items_grid<'a>(
        items: &'a [Option<CollectionItem>],
        covers: &'a Covers,
        card_state: CardStateOf<'_>,
    ) -> Element<'a, super::Message> {
        let cards = items.iter().map(|item| match item {
            Some(item) => item_card(
                item,
                covers.get(&item.external_id),
                card_state(&item.external_id),
            ),
            None => evicted_card(),
        });

        container(
            grid(cards)
                .fluid(160.0)
                .spacing(16.0)
                .height(grid::aspect_ratio(2.0, 3.0)),
        )
        .width(Fill)
        .padding(20)
        .into()
    }

    /// A series' card: its cover, which offers the series' actions while
    /// hovered, over a bar holding its title, a link to the series, and its
    /// menu while it's in the Library.
    fn item_card<'a>(
        item: &'a CollectionItem,
        cover: Option<&image::Handle>,
        state: CardState,
    ) -> Element<'a, super::Message> {
        // Only its top corners are rounded: the bar carries on below it.
        let cover = hover(
            thumbnail(cover, border::top(COVER_RADIUS)),
            cover_actions(item, state.add),
        );

        // The open menu sits over the cover, just above its "⋮" button.
        let cover: Element<'a, super::Message> = if state.menu_open {
            stack![cover, series_menu(item, state)].into()
        } else {
            cover
        };

        column![cover, card_bar(item, state)]
            .width(Fill)
            .height(Fill)
            .into()
    }

    /// The bar under a series' cover: its title and page count, and the "⋮"
    /// button opening its menu, when the series is in the Library.
    fn card_bar<'a>(item: &'a CollectionItem, state: CardState) -> Element<'a, super::Message> {
        let name = column![title_link(item), pages_label(item.pages)].spacing(2);

        card::bar(
            state.download.icon(),
            name.into(),
            has_menu(state).then(|| super::Message::MenuToggled(item.external_id.clone())),
        )
    }

    /// Whether a series' menu has anything to offer: downloading it, unless
    /// it's downloaded, and removing it from the Library, if it's in it.
    pub(super) fn has_menu(state: CardState) -> bool {
        state.add == AddState::Added || state.download != DownloadState::Downloaded
    }

    /// A series' open menu, laid over the bottom-right of its cover.
    fn series_menu<'a>(item: &CollectionItem, state: CardState) -> Element<'a, super::Message> {
        let mut items: Vec<Element<'a, super::Message>> = state
            .download
            .menu_item(super::Message::DownloadSeries(item.external_id.clone()))
            .into_iter()
            .collect();
        if state.add == AddState::Added {
            items.push(card::menu_item(
                "trash",
                "Remove from library",
                super::Message::RemoveSeriesFromLibrary(item.external_id.clone()),
            ));
        }
        card::menu(items)
    }

    /// The shade laid over a hovered series cover, with its actions side by
    /// side: adding all of its volumes/chapters to the Library, and opening it.
    fn cover_actions<'a>(
        item: &CollectionItem,
        add_state: AddState,
    ) -> Element<'a, super::Message> {
        let add = match add_state {
            AddState::Available => cover_overlay::action(
                "circle-plus",
                "Add to library",
                Some(super::Message::AddSeriesToLibrary(item.external_id.clone())),
            ),
            AddState::Adding => cover_overlay::action("download", "Adding…", None),
            AddState::Added => cover_overlay::action("circle-check", "In library", None),
        };
        let open = cover_overlay::action(
            "folder-open",
            "Open",
            Some(super::Message::SeriesSelected(item.external_id.clone())),
        );

        cover_overlay::shade(vec![add, open], border::top(COVER_RADIUS))
    }

    /// A series' title, as a link to the series: plain text that takes the
    /// primary color while hovered.
    fn title_link<'a>(item: &'a CollectionItem) -> Element<'a, super::Message> {
        // A title too long for the card is cut off (by the bar), not wrapped.
        button(text(&item.name).size(13).wrapping(text::Wrapping::None))
            .on_press(super::Message::SeriesSelected(item.external_id.clone()))
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
            })
            .into()
    }

    /// Placeholder for an item whose data has been evicted from memory
    /// because it was scrolled far off-screen; keeps the grid's item count
    /// (and thus its scroll geometry) stable.
    fn evicted_card<'a>() -> Element<'a, super::Message> {
        container(thumbnail(None, COVER_RADIUS.into()))
            .width(Fill)
            .height(Fill)
            .into()
    }

    /// Renders an item's cover if it has loaded, otherwise the same styled
    /// grey box used as a placeholder (and for evicted items).
    fn thumbnail<'a>(
        handle: Option<&image::Handle>,
        radius: border::Radius,
    ) -> Element<'a, super::Message> {
        let inner: Element<'a, super::Message> = match handle {
            Some(handle) => image(handle.clone())
                .width(Fill)
                .height(Fill)
                .content_fit(ContentFit::Cover)
                .border_radius(radius)
                .into(),
            None => row![].into(),
        };

        container(inner)
            .width(Fill)
            .height(Fill)
            .clip(true)
            .style(move |theme: &Theme| container::Style {
                background: Some(Background::Color(
                    theme.extended_palette().background.strong.color,
                )),
                border: Border {
                    radius,
                    ..Border::default()
                },
                ..container::Style::default()
            })
            .into()
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

    fn empty_state<'a>() -> Element<'a, super::Message> {
        container(text("Nenhum item encontrado nesta biblioteca"))
            .center(Fill)
            .into()
    }

    fn error_state<'a>(error: &'a str) -> Element<'a, super::Message> {
        container(text(error)).center(Fill).into()
    }
}

#[cfg(test)]
mod tests {
    use iced_test::simulator;

    use super::*;

    fn collection_item(name: &str, pages: Option<u16>) -> CollectionItem {
        CollectionItem {
            external_id: name.to_string(),
            name: name.to_string(),
            cover: String::new(),
            pages,
        }
    }

    /// Builds a `Browse` with empty cover state, so tests only have to specify
    /// the `items`/`pagination` they care about.
    fn location(importable: bool) -> CollectionRef {
        CollectionRef {
            integration_id: "source-1".to_string(),
            collection: crate::models::collection::Collection {
                importable,
                ..crate::models::collection::Collection::new(
                    "lib-1",
                    "Manga",
                    crate::models::collection::CollectionKind::Library,
                )
            },
        }
    }

    fn browse(
        items: AsyncModel<Vec<Option<CollectionItem>>, String>,
        pagination: Option<Pagination>,
    ) -> Browse {
        Browse {
            location: location(false),
            items,
            pagination,
            covers: HashMap::new(),
            covers_in_flight: HashSet::new(),
            adding_series: HashSet::new(),
            library_series: HashSet::new(),
            add_error: None,
            menu: None,
            downloaded_series: HashSet::new(),
            downloading: HashSet::new(),
        }
    }

    /// A `Browse` of an importable collection (a local folder), for the import
    /// tests.
    fn importable_browse(
        items: AsyncModel<Vec<Option<CollectionItem>>, String>,
        pagination: Option<Pagination>,
    ) -> Browse {
        Browse {
            location: location(true),
            ..browse(items, pagination)
        }
    }

    fn pagination(has_more: bool, loading_next: bool) -> Pagination {
        Pagination {
            next_page: 2,
            page_size: PAGE_SIZE,
            has_more,
            loading_next,
            refetching_pages: std::collections::HashSet::new(),
        }
    }

    /// Scroll metrics for a given scroll-track position, placing the
    /// "visible" page at page 0. Only `relative_offset_y` matters for tests
    /// that exercise the near-bottom (forward prefetch) check.
    fn scroll_metrics(relative_offset_y: f32) -> ScrollMetrics {
        ScrollMetrics {
            relative_offset_y,
            absolute_offset_y: 0.0,
            viewport_height: 0.0,
            content_height: 100.0,
        }
    }

    /// Scroll metrics that place exactly `page_index` at the top of a
    /// viewport tall enough to show only that one page, given `total_items`
    /// loaded so far. Used by tests that need precise control over which
    /// page is considered "visible".
    fn scroll_metrics_at_page(total_items: usize, page_index: usize) -> ScrollMetrics {
        const PAGE_HEIGHT: f32 = 100.0;

        let total_pages = (total_items as u32).div_ceil(PAGE_SIZE).max(1) as f32;
        let content_height = total_pages * PAGE_HEIGHT;
        let viewport_height = PAGE_HEIGHT / 2.0;
        let absolute_offset_y = page_index as f32 * PAGE_HEIGHT;
        let scrollable_range = (content_height - viewport_height).max(1.0);

        ScrollMetrics {
            relative_offset_y: (absolute_offset_y / scrollable_range).clamp(0.0, 1.0),
            absolute_offset_y,
            viewport_height,
            content_height,
        }
    }

    #[test]
    fn new_starts_pagination_at_the_second_page() {
        let (browse, _task) = Browse::new(location(false));

        assert!(matches!(browse.items, AsyncModel::Loading));
        let pagination = browse.pagination.expect("pagination should be set up");
        assert_eq!(pagination.next_page, 2);
        assert!(pagination.has_more);
        assert!(!pagination.loading_next);
    }

    #[test]
    fn view_shows_the_collection_name() {
        let browse = browse(AsyncModel::Loaded(vec![]), Some(pagination(false, false)));

        let mut ui = simulator(browse.view());

        assert!(ui.find("Manga").is_ok());
    }

    #[test]
    fn update_items_loaded_stores_result_and_checks_the_viewport() {
        let mut browse = browse(AsyncModel::Loading, Some(pagination(true, false)));

        let action = browse.update(Message::ItemsLoaded(Ok(vec![collection_item(
            "One Piece",
            Some(42),
        )])));

        assert!(matches!(action, Action::Run(_)));
        assert!(matches!(browse.items, AsyncModel::Loaded(ref items) if items.len() == 1));
    }

    #[test]
    fn update_items_loaded_with_a_full_page_keeps_has_more_true() {
        let mut browse = browse(AsyncModel::Loading, Some(pagination(true, false)));

        let full_page: Vec<_> = (0..PAGE_SIZE)
            .map(|i| collection_item(&format!("Item {i}"), None))
            .collect();

        browse.update(Message::ItemsLoaded(Ok(full_page)));

        assert!(browse.pagination.unwrap().has_more);
    }

    #[test]
    fn update_items_loaded_with_a_short_page_clears_has_more() {
        let mut browse = browse(AsyncModel::Loading, Some(pagination(true, false)));

        browse.update(Message::ItemsLoaded(Ok(vec![collection_item(
            "Only one", None,
        )])));

        assert!(!browse.pagination.unwrap().has_more);
    }

    #[test]
    fn update_items_loaded_stores_error_and_returns_no_action() {
        let mut browse = browse(AsyncModel::Loading, Some(pagination(true, false)));

        let action = browse.update(Message::ItemsLoaded(Err("boom".to_string())));

        assert!(matches!(action, Action::None));
        assert!(matches!(browse.items, AsyncModel::Error(ref error) if error == "boom"));
    }

    #[test]
    fn update_series_selected_returns_open_series_with_item_metadata() {
        let mut browse = browse(
            AsyncModel::Loaded(vec![Some(collection_item("One Piece", Some(42)))]),
            Some(pagination(true, false)),
        );

        let action = browse.update(Message::SeriesSelected("One Piece".to_string()));

        match action {
            Action::OpenSeries {
                series_external_id,
                series_name,
                series_pages,
                location,
            } => {
                assert_eq!(series_external_id, "One Piece");
                assert_eq!(series_name, "One Piece");
                assert_eq!(series_pages, Some(42));
                assert_eq!(location.integration_id, "source-1");
                assert_eq!(location.collection.external_id, "lib-1");
            }
            other => panic!("expected OpenSeries, got {other:?}"),
        }
    }

    #[test]
    fn update_series_selected_for_an_unknown_item_returns_none() {
        let mut browse = browse(
            AsyncModel::Loaded(vec![Some(collection_item("One Piece", None))]),
            Some(pagination(true, false)),
        );

        let action = browse.update(Message::SeriesSelected("Berserk".to_string()));

        assert!(matches!(action, Action::None));
    }

    #[test]
    fn clicking_a_series_title_sends_series_selected() {
        let browse = browse(
            AsyncModel::Loaded(vec![Some(collection_item("One Piece", Some(42)))]),
            Some(pagination(true, false)),
        );

        let mut ui = simulator(browse.view());
        let _ = ui.click("One Piece").expect("item card should be found");

        let messages: Vec<_> = ui.into_messages().collect();

        assert_eq!(
            messages,
            vec![Message::SeriesSelected("One Piece".to_string())]
        );
    }

    #[test]
    fn hovering_a_cover_offers_to_add_the_series_or_open_it() {
        let browse = browse(
            AsyncModel::Loaded(vec![Some(collection_item("One Piece", Some(42)))]),
            Some(pagination(true, false)),
        );

        let mut ui = simulator(browse.view());
        let _ = ui
            .click("Add to library")
            .expect("the add action should be found");
        let _ = ui.click("Open").expect("the open action should be found");
        let messages: Vec<_> = ui.into_messages().collect();

        assert_eq!(
            messages,
            vec![
                Message::AddSeriesToLibrary("One Piece".to_string()),
                Message::SeriesSelected("One Piece".to_string()),
            ]
        );
    }

    #[test]
    fn adding_a_series_saves_it_once_and_records_the_result() {
        let mut browse = browse(
            AsyncModel::Loaded(vec![Some(collection_item("One Piece", None))]),
            Some(pagination(false, false)),
        );

        // The task isn't run here, so nothing is saved.
        assert!(matches!(
            browse.update(Message::AddSeriesToLibrary("One Piece".to_string())),
            Action::Run(_)
        ));
        assert_eq!(browse.add_state("One Piece"), AddState::Adding);
        // A second click while it's saved does nothing.
        assert!(matches!(
            browse.update(Message::AddSeriesToLibrary("One Piece".to_string())),
            Action::None
        ));

        let _ = browse.update(Message::SeriesAddedToLibrary(
            "One Piece".to_string(),
            Ok(3),
        ));
        assert_eq!(browse.add_state("One Piece"), AddState::Added);

        let mut ui = simulator(browse.view());
        assert!(ui.find("In library").is_ok());
        assert!(ui.find("Add to library").is_err());
    }

    #[test]
    fn adding_an_unknown_series_does_nothing() {
        let mut browse = browse(
            AsyncModel::Loaded(vec![Some(collection_item("One Piece", None))]),
            Some(pagination(false, false)),
        );

        let action = browse.update(Message::AddSeriesToLibrary("Berserk".to_string()));

        assert!(matches!(action, Action::None));
        assert_eq!(browse.add_state("Berserk"), AddState::Available);
    }

    #[test]
    fn a_failed_series_add_is_reported_and_can_be_retried() {
        let mut browse = browse(
            AsyncModel::Loaded(vec![Some(collection_item("One Piece", None))]),
            Some(pagination(false, false)),
        );
        let _ = browse.update(Message::AddSeriesToLibrary("One Piece".to_string()));

        let _ = browse.update(Message::SeriesAddedToLibrary(
            "One Piece".to_string(),
            Err("offline".to_string()),
        ));

        assert_eq!(browse.add_state("One Piece"), AddState::Available);
        let mut ui = simulator(browse.view());
        assert!(ui.find("Could not add to library: offline").is_ok());
    }

    #[test]
    fn series_already_in_the_library_show_as_in_it_with_a_menu() {
        let mut browse = browse(
            AsyncModel::Loaded(vec![
                Some(collection_item("One Piece", None)),
                Some(collection_item("Berserk", None)),
            ]),
            Some(pagination(false, false)),
        );

        let _ = browse.update(Message::LibrarySeriesLoaded(Ok(vec![
            "One Piece".to_string(),
        ])));

        assert_eq!(browse.add_state("One Piece"), AddState::Added);
        assert_eq!(browse.add_state("Berserk"), AddState::Available);
        assert!(view_helper::has_menu(browse.card_state("One Piece")));
        // Re-adding a series that's in the Library isn't offered.
        assert!(matches!(
            browse.update(Message::AddSeriesToLibrary("One Piece".to_string())),
            Action::None
        ));
    }

    #[test]
    fn a_series_not_downloaded_offers_a_download_with_a_cloud_icon() {
        let mut browse = browse(
            AsyncModel::Loaded(vec![Some(collection_item("One Piece", None))]),
            Some(pagination(false, false)),
        );
        let state = browse.card_state("One Piece");
        assert_eq!(state.download, DownloadState::NotDownloaded);
        assert_eq!(state.download.icon(), "cloud");
        // Not in the Library, but downloadable, so it has a menu.
        assert!(view_helper::has_menu(state));

        let _ = browse.update(Message::MenuToggled("One Piece".to_string()));
        let mut ui = simulator(browse.view());
        let _ = ui
            .click("Download")
            .expect("the download item should be found");
        let messages: Vec<_> = ui.into_messages().collect();
        assert!(messages.contains(&Message::DownloadSeries("One Piece".to_string())));
    }

    #[test]
    fn downloading_a_series_runs_once_then_shows_it_downloaded() {
        let mut browse = browse(
            AsyncModel::Loaded(vec![Some(collection_item("One Piece", None))]),
            Some(pagination(false, false)),
        );

        assert!(matches!(
            browse.update(Message::DownloadSeries("One Piece".to_string())),
            Action::Run(_)
        ));
        assert_eq!(
            browse.card_state("One Piece").download,
            DownloadState::Downloading
        );
        assert!(matches!(
            browse.update(Message::DownloadSeries("One Piece".to_string())),
            Action::None
        ));

        let _ = browse.update(Message::SeriesDownloaded("One Piece".to_string(), Ok(4)));
        let state = browse.card_state("One Piece");
        assert_eq!(state.download, DownloadState::Downloaded);
        assert_eq!(state.download.icon(), "download");
        // Downloaded and not in the Library: nothing left for its menu.
        assert!(!view_helper::has_menu(state));
        assert!(matches!(
            browse.update(Message::DownloadSeries("One Piece".to_string())),
            Action::None
        ));
    }

    #[test]
    fn a_downloaded_series_is_not_offered_a_download() {
        let mut browse = browse(
            AsyncModel::Loaded(vec![Some(collection_item("One Piece", None))]),
            Some(pagination(false, false)),
        );
        let _ = browse.update(Message::LibrarySeriesLoaded(Ok(vec![
            "One Piece".to_string(),
        ])));
        let _ = browse.update(Message::DownloadedSeriesLoaded(HashSet::from([
            "One Piece".to_string(),
        ])));
        let _ = browse.update(Message::MenuToggled("One Piece".to_string()));

        let mut ui = simulator(browse.view());
        assert!(ui.find("Download").is_err());
        assert!(ui.find("Remove from library").is_ok());
    }

    #[test]
    fn a_failed_download_is_reported_and_can_be_retried() {
        let mut browse = browse(
            AsyncModel::Loaded(vec![Some(collection_item("One Piece", None))]),
            Some(pagination(false, false)),
        );
        let _ = browse.update(Message::DownloadSeries("One Piece".to_string()));

        let _ = browse.update(Message::SeriesDownloaded(
            "One Piece".to_string(),
            Err("1 of 2 could not be downloaded".to_string()),
        ));

        assert_eq!(
            browse.card_state("One Piece").download,
            DownloadState::NotDownloaded
        );
        let mut ui = simulator(browse.view());
        assert!(
            ui.find("Could not download: 1 of 2 could not be downloaded")
                .is_ok()
        );
    }

    #[test]
    fn the_menu_toggles_and_closes_on_a_click_elsewhere() {
        let mut browse = browse(
            AsyncModel::Loaded(vec![Some(collection_item("One Piece", None))]),
            Some(pagination(false, false)),
        );
        let _ = browse.update(Message::LibrarySeriesLoaded(Ok(vec![
            "One Piece".to_string(),
        ])));

        let _ = browse.update(Message::MenuToggled("One Piece".to_string()));
        assert!(browse.card_state("One Piece").menu_open);
        let _ = browse.update(Message::MenuToggled("One Piece".to_string()));
        assert!(!browse.card_state("One Piece").menu_open);

        let _ = browse.update(Message::MenuToggled("One Piece".to_string()));
        let _ = browse.update(Message::MenuDismissed);
        assert!(!browse.card_state("One Piece").menu_open);
    }

    #[test]
    fn the_open_menu_offers_to_remove_the_series() {
        let mut browse = browse(
            AsyncModel::Loaded(vec![Some(collection_item("One Piece", None))]),
            Some(pagination(false, false)),
        );
        let _ = browse.update(Message::LibrarySeriesLoaded(Ok(vec![
            "One Piece".to_string(),
        ])));

        let mut ui = simulator(browse.view());
        assert!(ui.find("Remove from library").is_err());
        drop(ui);

        let _ = browse.update(Message::MenuToggled("One Piece".to_string()));
        let mut ui = simulator(browse.view());
        let _ = ui
            .click("Remove from library")
            .expect("the remove item should be found");
        let messages: Vec<_> = ui.into_messages().collect();

        assert!(messages.contains(&Message::RemoveSeriesFromLibrary("One Piece".to_string())));
    }

    #[test]
    fn removing_a_series_from_the_library_offers_adding_it_again() {
        let mut browse = browse(
            AsyncModel::Loaded(vec![Some(collection_item("One Piece", None))]),
            Some(pagination(false, false)),
        );
        let _ = browse.update(Message::LibrarySeriesLoaded(Ok(vec![
            "One Piece".to_string(),
        ])));
        let _ = browse.update(Message::MenuToggled("One Piece".to_string()));

        // The task isn't run here, so nothing is removed.
        assert!(matches!(
            browse.update(Message::RemoveSeriesFromLibrary("One Piece".to_string())),
            Action::Run(_)
        ));
        assert!(!browse.card_state("One Piece").menu_open);

        let _ = browse.update(Message::SeriesRemovedFromLibrary(
            "One Piece".to_string(),
            Ok(3),
        ));
        assert_eq!(browse.add_state("One Piece"), AddState::Available);
    }

    #[test]
    fn a_failed_removal_is_reported_and_keeps_the_series() {
        let mut browse = browse(
            AsyncModel::Loaded(vec![Some(collection_item("One Piece", None))]),
            Some(pagination(false, false)),
        );
        let _ = browse.update(Message::LibrarySeriesLoaded(Ok(vec![
            "One Piece".to_string(),
        ])));

        let _ = browse.update(Message::SeriesRemovedFromLibrary(
            "One Piece".to_string(),
            Err("locked".to_string()),
        ));

        assert_eq!(browse.add_state("One Piece"), AddState::Added);
        let mut ui = simulator(browse.view());
        assert!(ui.find("Could not remove from library: locked").is_ok());
    }

    #[test]
    fn removing_a_series_not_in_the_library_does_nothing() {
        let mut browse = browse(
            AsyncModel::Loaded(vec![Some(collection_item("One Piece", None))]),
            Some(pagination(false, false)),
        );

        let action = browse.update(Message::RemoveSeriesFromLibrary("One Piece".to_string()));

        assert!(matches!(action, Action::None));
    }

    #[test]
    fn update_back_pressed_returns_back_pressed_action() {
        let mut browse = browse(AsyncModel::Loading, None);

        let action = browse.update(Message::BackPressed);

        assert!(matches!(action, Action::BackPressed(id) if id == "source-1"));
    }

    #[test]
    fn view_shows_the_import_button_only_for_importable_collections() {
        let remote = browse(AsyncModel::Loaded(vec![]), Some(pagination(false, false)));
        let mut ui = simulator(remote.view());
        assert!(ui.find("Import items").is_err());

        let local = importable_browse(AsyncModel::Loaded(vec![]), Some(pagination(false, false)));
        let mut ui = simulator(local.view());
        assert!(ui.find("Import items").is_ok());
    }

    #[test]
    fn clicking_import_sends_import_pressed() {
        let browse = importable_browse(AsyncModel::Loaded(vec![]), Some(pagination(false, false)));

        let mut ui = simulator(browse.view());
        let _ = ui
            .click("Import items")
            .expect("Import items button should be found");

        let messages: Vec<_> = ui.into_messages().collect();

        assert_eq!(messages, vec![Message::ImportPressed]);
    }

    #[test]
    fn import_pressed_runs_a_task() {
        let mut browse =
            importable_browse(AsyncModel::Loaded(vec![]), Some(pagination(false, false)));

        let action = browse.update(Message::ImportPressed);

        assert!(matches!(action, Action::Run(_)));
    }

    #[test]
    fn import_sources_picked_none_does_nothing() {
        let mut browse =
            importable_browse(AsyncModel::Loaded(vec![]), Some(pagination(false, false)));

        let action = browse.update(Message::ImportSourcesPicked(None));

        assert!(matches!(action, Action::None));
    }

    #[test]
    fn import_sources_picked_empty_does_nothing() {
        let mut browse =
            importable_browse(AsyncModel::Loaded(vec![]), Some(pagination(false, false)));

        let action = browse.update(Message::ImportSourcesPicked(Some(Vec::new())));

        assert!(matches!(action, Action::None));
    }

    #[test]
    fn import_sources_picked_runs_an_import_task() {
        let mut browse =
            importable_browse(AsyncModel::Loaded(vec![]), Some(pagination(false, false)));

        let action = browse.update(Message::ImportSourcesPicked(Some(vec![PathBuf::from(
            "/tmp/One Shot.cbz",
        )])));

        assert!(matches!(action, Action::Run(_)));
    }

    #[test]
    fn items_imported_ok_resets_to_the_first_page_and_reloads() {
        let mut browse = importable_browse(
            AsyncModel::Loaded(vec![Some(collection_item("Old", None))]),
            Some(pagination(false, true)),
        );

        let action = browse.update(Message::ItemsImported(Ok(2)));

        assert!(matches!(action, Action::Run(_)));
        assert!(matches!(browse.items, AsyncModel::Loading));
        let pagination = browse.pagination.unwrap();
        assert_eq!(pagination.next_page, 2);
        assert!(pagination.has_more);
        assert!(!pagination.loading_next);
    }

    #[test]
    fn items_imported_err_keeps_the_current_list() {
        let mut browse = importable_browse(
            AsyncModel::Loaded(vec![Some(collection_item("Still here", None))]),
            Some(pagination(false, false)),
        );

        let action = browse.update(Message::ItemsImported(Err("boom".to_string())));

        assert!(matches!(action, Action::None));
        assert!(matches!(browse.items, AsyncModel::Loaded(ref items) if items.len() == 1));
    }

    #[test]
    fn scrolling_near_the_bottom_requests_the_next_page() {
        let mut browse = browse(
            AsyncModel::Loaded(
                (0..PAGE_SIZE)
                    .map(|i| Some(collection_item(&format!("Item {i}"), None)))
                    .collect(),
            ),
            Some(pagination(true, false)),
        );

        let action = browse.update(Message::Scrolled(scroll_metrics(0.9)));

        assert!(matches!(action, Action::Run(_)));
        assert!(browse.pagination.unwrap().loading_next);
    }

    #[test]
    fn scrolling_away_from_the_bottom_does_not_request_a_page() {
        let mut browse = browse(
            AsyncModel::Loaded(
                (0..PAGE_SIZE)
                    .map(|i| Some(collection_item(&format!("Item {i}"), None)))
                    .collect(),
            ),
            Some(pagination(true, false)),
        );

        let action = browse.update(Message::Scrolled(scroll_metrics(0.3)));

        assert!(matches!(action, Action::None));
        assert!(!browse.pagination.unwrap().loading_next);
    }

    #[test]
    fn scrolling_near_the_bottom_while_a_page_is_already_loading_does_nothing() {
        let mut browse = browse(
            AsyncModel::Loaded(vec![Some(collection_item("One Piece", None))]),
            Some(pagination(true, true)),
        );

        let action = browse.update(Message::Scrolled(scroll_metrics(0.95)));

        assert!(matches!(action, Action::None));
    }

    #[test]
    fn scrolling_near_the_bottom_without_more_pages_does_nothing() {
        let mut browse = browse(
            AsyncModel::Loaded(vec![Some(collection_item("One Piece", None))]),
            Some(pagination(false, false)),
        );

        let action = browse.update(Message::Scrolled(scroll_metrics(0.95)));

        assert!(matches!(action, Action::None));
    }

    #[test]
    fn next_page_loaded_appends_items_and_advances_pagination() {
        let mut browse = browse(
            AsyncModel::Loaded(vec![Some(collection_item("One Piece", None))]),
            Some(pagination(true, true)),
        );

        let action = browse.update(Message::NextPageLoaded(Ok(vec![collection_item(
            "Berserk", None,
        )])));

        assert!(matches!(action, Action::Run(_)));

        let AsyncModel::Loaded(items) = &browse.items else {
            panic!("items should be loaded");
        };
        assert_eq!(items.len(), 2);
        assert_eq!(items[1].as_ref().unwrap().name, "Berserk");

        let pagination = browse.pagination.unwrap();
        assert_eq!(pagination.next_page, 3);
        assert!(!pagination.loading_next);
        assert!(!pagination.has_more);
    }

    #[test]
    fn content_overflow_checked_true_does_not_request_a_page() {
        let mut browse = browse(
            AsyncModel::Loaded(vec![Some(collection_item("One Piece", None))]),
            Some(pagination(true, false)),
        );

        let action = browse.update(Message::ContentOverflowChecked(true));

        assert!(matches!(action, Action::None));
        assert!(!browse.pagination.unwrap().loading_next);
    }

    #[test]
    fn content_overflow_checked_false_requests_the_next_page_to_fill_the_viewport() {
        let mut browse = browse(
            AsyncModel::Loaded(vec![Some(collection_item("One Piece", None))]),
            Some(pagination(true, false)),
        );

        let action = browse.update(Message::ContentOverflowChecked(false));

        assert!(matches!(action, Action::Run(_)));
        assert!(browse.pagination.unwrap().loading_next);
    }

    #[test]
    fn content_overflow_checked_false_without_more_pages_does_nothing() {
        let mut browse = browse(
            AsyncModel::Loaded(vec![Some(collection_item("One Piece", None))]),
            Some(pagination(false, false)),
        );

        let action = browse.update(Message::ContentOverflowChecked(false));

        assert!(matches!(action, Action::None));
    }

    #[test]
    fn content_overflow_checked_false_while_already_loading_does_nothing() {
        let mut browse = browse(
            AsyncModel::Loaded(vec![Some(collection_item("One Piece", None))]),
            Some(pagination(true, true)),
        );

        let action = browse.update(Message::ContentOverflowChecked(false));

        assert!(matches!(action, Action::None));
    }

    #[test]
    fn content_overflow_checked_false_without_pagination_does_nothing() {
        let mut browse = browse(AsyncModel::Loaded(Vec::new()), None);

        let action = browse.update(Message::ContentOverflowChecked(false));

        assert!(matches!(action, Action::None));
    }

    #[test]
    fn next_page_loaded_error_resets_loading_flag_without_dropping_items() {
        let mut browse = browse(
            AsyncModel::Loaded(vec![Some(collection_item("One Piece", None))]),
            Some(pagination(true, true)),
        );

        browse.update(Message::NextPageLoaded(Err("boom".to_string())));

        assert!(matches!(browse.items, AsyncModel::Loaded(ref items) if items.len() == 1));
        assert!(!browse.pagination.unwrap().loading_next);
    }

    #[test]
    fn next_page_loaded_evicts_pages_outside_the_keep_window() {
        // KEEP_PAGES_LOADED pages are already loaded (indices 0..KEEP_PAGES_LOADED);
        // loading one more page should push page 0 out of the window.
        let items: Vec<_> = (0..(PAGE_SIZE * KEEP_PAGES_LOADED as u32))
            .map(|i| Some(collection_item(&format!("Item {i}"), None)))
            .collect();

        let mut pagination_state = pagination(true, false);
        pagination_state.next_page = KEEP_PAGES_LOADED as u32 + 1;

        let mut browse = browse(AsyncModel::Loaded(items), Some(pagination_state));

        let new_page: Vec<_> = (0..PAGE_SIZE)
            .map(|i| collection_item(&format!("New {i}"), None))
            .collect();
        browse.update(Message::NextPageLoaded(Ok(new_page)));

        let AsyncModel::Loaded(items) = &browse.items else {
            panic!("items should be loaded");
        };

        assert!(items[0..PAGE_SIZE as usize].iter().all(Option::is_none));
        assert!(
            items[PAGE_SIZE as usize..(PAGE_SIZE as usize * 2)]
                .iter()
                .all(Option::is_some)
        );
    }

    #[test]
    fn next_page_loaded_keeps_everything_within_the_keep_window() {
        let items: Vec<_> = (0..PAGE_SIZE)
            .map(|i| Some(collection_item(&format!("Item {i}"), None)))
            .collect();

        let mut browse = browse(AsyncModel::Loaded(items), Some(pagination(true, false)));

        let new_page: Vec<_> = (0..PAGE_SIZE)
            .map(|i| collection_item(&format!("New {i}"), None))
            .collect();
        browse.update(Message::NextPageLoaded(Ok(new_page)));

        let AsyncModel::Loaded(items) = &browse.items else {
            panic!("items should be loaded");
        };

        assert!(items.iter().all(Option::is_some));
    }

    #[test]
    fn scrolling_does_not_evict_anything_on_its_own() {
        let items: Vec<_> = (0..(PAGE_SIZE * 3))
            .map(|i| Some(collection_item(&format!("Item {i}"), None)))
            .collect();

        let mut browse = browse(AsyncModel::Loaded(items), Some(pagination(false, false)));

        browse.update(Message::Scrolled(scroll_metrics(0.99)));

        let AsyncModel::Loaded(items) = &browse.items else {
            panic!("items should be loaded");
        };

        assert!(items.iter().all(Option::is_some));
    }

    #[test]
    fn eviction_range_is_empty_for_an_empty_list() {
        assert_eq!(eviction_range(0, PAGE_SIZE, 10, KEEP_PAGES_LOADED), 0..0);
    }

    #[test]
    fn visible_page_range_is_none_for_an_empty_list() {
        assert_eq!(visible_page_range(0, PAGE_SIZE, 0.0, 100.0, 100.0), None);
    }

    #[test]
    fn visible_page_range_uses_absolute_offset_not_the_scroll_track_fraction() {
        // 10 pages of content, each 100px tall (content_height = 1000), with
        // a viewport that's 40% of that (400px) — a large enough fraction
        // that relative_offset_y (normalized against content_height -
        // viewport_height = 600) diverges sharply from the absolute pixel
        // position. Scrolled so the viewport's top sits at page 5 (y=500).
        let range =
            visible_page_range(PAGE_SIZE as usize * 10, PAGE_SIZE, 500.0, 400.0, 1000.0).unwrap();

        // The naive (buggy) relative-offset formula would have computed
        // relative_offset_y = 500.0 / 600.0 ≈ 0.833, then
        // (0.833 * 10 pages) = page 8 — three pages off from the truth.
        assert_eq!(*range.start(), 5);
        assert_eq!(*range.end(), 9);
    }

    #[test]
    fn visible_page_range_clamps_to_the_last_page() {
        let range =
            visible_page_range(PAGE_SIZE as usize * 3, PAGE_SIZE, 10_000.0, 50.0, 300.0).unwrap();

        assert_eq!(range, 2..=2);
    }

    #[test]
    fn scrolling_back_onto_an_evicted_page_refetches_it() {
        let mut items: Vec<_> = (0..(PAGE_SIZE * 3))
            .map(|i| Some(collection_item(&format!("Item {i}"), None)))
            .collect();
        for slot in &mut items[0..PAGE_SIZE as usize] {
            *slot = None;
        }

        let mut browse = browse(AsyncModel::Loaded(items), Some(pagination(false, false)));

        // Scroll back up onto the evicted page 0.
        let action = browse.update(Message::Scrolled(scroll_metrics_at_page(60, 0)));

        assert!(matches!(action, Action::Run(_)));
        assert!(browse.pagination.unwrap().refetching_pages.contains(&0));
    }

    #[test]
    fn scrolling_onto_a_page_that_is_not_evicted_does_not_refetch() {
        let items: Vec<_> = (0..(PAGE_SIZE * 3))
            .map(|i| Some(collection_item(&format!("Item {i}"), None)))
            .collect();

        let mut browse = browse(AsyncModel::Loaded(items), Some(pagination(false, false)));

        let action = browse.update(Message::Scrolled(scroll_metrics_at_page(60, 0)));

        assert!(matches!(action, Action::None));
        assert!(browse.pagination.unwrap().refetching_pages.is_empty());
    }

    #[test]
    fn scrolling_onto_an_evicted_page_already_being_refetched_does_not_request_again() {
        let mut items: Vec<_> = (0..(PAGE_SIZE * 3))
            .map(|i| Some(collection_item(&format!("Item {i}"), None)))
            .collect();
        for slot in &mut items[0..PAGE_SIZE as usize] {
            *slot = None;
        }

        let mut browse = browse(AsyncModel::Loaded(items), Some(pagination(false, false)));

        browse.update(Message::Scrolled(scroll_metrics_at_page(60, 0)));
        let action = browse.update(Message::Scrolled(scroll_metrics_at_page(60, 0)));

        assert!(matches!(action, Action::None));
    }

    #[test]
    fn scrolling_with_multiple_evicted_pages_visible_refetches_all_of_them() {
        let mut items: Vec<_> = (0..(PAGE_SIZE * 3))
            .map(|i| Some(collection_item(&format!("Item {i}"), None)))
            .collect();
        for slot in &mut items[0..(PAGE_SIZE as usize * 2)] {
            *slot = None;
        }

        let mut browse = browse(AsyncModel::Loaded(items), Some(pagination(false, false)));

        // A tall viewport spanning pages 0 and 1 at once, both evicted.
        let metrics = ScrollMetrics {
            relative_offset_y: 0.0,
            absolute_offset_y: 0.0,
            viewport_height: 150.0,
            content_height: 300.0,
        };
        let action = browse.update(Message::Scrolled(metrics));

        assert!(matches!(action, Action::Run(_)));
        let pagination = browse.pagination.unwrap();
        assert!(pagination.refetching_pages.contains(&0));
        assert!(pagination.refetching_pages.contains(&1));
    }

    #[test]
    fn page_refetched_ok_splices_items_back_at_the_right_position_and_clears_the_flag() {
        let mut items: Vec<_> = (0..(PAGE_SIZE * 2))
            .map(|i| Some(collection_item(&format!("Item {i}"), None)))
            .collect();
        for slot in &mut items[0..PAGE_SIZE as usize] {
            *slot = None;
        }

        let mut pagination_state = pagination(false, false);
        pagination_state.refetching_pages.insert(0);

        let mut browse = browse(AsyncModel::Loaded(items), Some(pagination_state));

        let refetched: Vec<_> = (0..PAGE_SIZE)
            .map(|i| collection_item(&format!("Refetched {i}"), None))
            .collect();

        let action = browse.update(Message::PageRefetched(0, Ok(refetched)));

        // A successful refetch now also kicks off a cover fetch for the
        // spliced-in page, so it returns a task rather than `Action::None`.
        assert!(matches!(action, Action::Run(_)));
        assert!(!browse.pagination.unwrap().refetching_pages.contains(&0));

        let AsyncModel::Loaded(items) = &browse.items else {
            panic!("items should be loaded");
        };
        assert_eq!(items[0].as_ref().unwrap().name, "Refetched 0");
        assert!(items[0..PAGE_SIZE as usize].iter().all(Option::is_some));
        assert!(items[PAGE_SIZE as usize..].iter().all(|item| {
            item.as_ref()
                .is_some_and(|item| item.name.starts_with("Item"))
        }));
    }

    #[test]
    fn page_refetched_err_clears_the_flag_without_touching_items() {
        let items = vec![None, Some(collection_item("One Piece", None))];

        let mut pagination_state = pagination(false, false);
        pagination_state.refetching_pages.insert(0);

        let mut browse = browse(AsyncModel::Loaded(items), Some(pagination_state));

        let action = browse.update(Message::PageRefetched(0, Err("boom".to_string())));

        assert!(matches!(action, Action::None));
        assert!(!browse.pagination.unwrap().refetching_pages.contains(&0));
        assert!(matches!(browse.items, AsyncModel::Loaded(ref items) if items[0].is_none()));
    }

    #[test]
    fn covers_loaded_inserts_handles_and_clears_in_flight() {
        let mut browse = browse(
            AsyncModel::Loaded(vec![Some(collection_item("One Piece", None))]),
            Some(pagination(false, false)),
        );
        browse.covers_in_flight.insert("One Piece".to_string());

        let action = browse.update(Message::CoversLoaded(vec![(
            "One Piece".to_string(),
            vec![1, 2, 3, 4],
        )]));

        assert!(matches!(action, Action::None));
        assert!(browse.covers.contains_key("One Piece"));
        assert!(!browse.covers_in_flight.contains("One Piece"));
    }

    #[test]
    fn request_page_covers_skips_items_already_cached_or_in_flight() {
        let mut browse = browse(
            AsyncModel::Loaded(vec![
                Some(collection_item("cached", None)),
                Some(collection_item("in-flight", None)),
                Some(collection_item("fresh", None)),
            ]),
            Some(pagination(false, false)),
        );
        browse
            .covers
            .insert("cached".to_string(), image::Handle::from_bytes(vec![0]));
        browse.covers_in_flight.insert("in-flight".to_string());

        // Only "fresh" should newly enter the in-flight set.
        let task = browse.request_page_covers();

        assert!(task.is_some());
        assert!(browse.covers_in_flight.contains("fresh"));
        assert!(browse.covers_in_flight.contains("in-flight"));
        assert!(!browse.covers_in_flight.contains("cached"));
    }

    #[test]
    fn request_page_covers_returns_none_when_everything_is_already_covered() {
        let mut browse = browse(
            AsyncModel::Loaded(vec![Some(collection_item("One Piece", None))]),
            Some(pagination(false, false)),
        );
        browse
            .covers
            .insert("One Piece".to_string(), image::Handle::from_bytes(vec![0]));

        assert!(browse.request_page_covers().is_none());
    }

    #[test]
    fn prune_covers_drops_entries_for_evicted_items() {
        let mut browse = browse(
            AsyncModel::Loaded(vec![None, Some(collection_item("still-here", None))]),
            Some(pagination(false, false)),
        );
        browse
            .covers
            .insert("evicted".to_string(), image::Handle::from_bytes(vec![0]));
        browse
            .covers
            .insert("still-here".to_string(), image::Handle::from_bytes(vec![0]));

        browse.prune_covers();

        assert!(browse.covers.contains_key("still-here"));
        assert!(!browse.covers.contains_key("evicted"));
    }

    #[test]
    fn view_renders_item_name_and_pages_when_loaded() {
        let browse = browse(
            AsyncModel::Loaded(vec![Some(collection_item("One Piece", Some(42)))]),
            None,
        );

        let mut ui = simulator(browse.view());

        assert!(ui.find("One Piece").is_ok());
        assert!(ui.find("42 pages").is_ok());
    }

    #[test]
    fn view_does_not_render_pages_when_absent() {
        let browse = browse(
            AsyncModel::Loaded(vec![Some(collection_item("One Piece", None))]),
            None,
        );

        let mut ui = simulator(browse.view());

        assert!(ui.find("One Piece").is_ok());
        assert!(ui.find("pages").is_err());
    }

    #[test]
    fn view_renders_error_message_when_loading_failed() {
        let browse = browse(AsyncModel::Error("boom".to_string()), None);

        let mut ui = simulator(browse.view());

        assert!(ui.find("boom").is_ok());
    }

    #[test]
    fn clicking_back_sends_back_pressed_message() {
        let browse = browse(AsyncModel::Loading, None);

        let mut ui = simulator(browse.view());
        let _ = ui
            .click(iced::widget::Id::new(crate::screen::back_button::ID))
            .expect("the back chevron should be found");

        let messages: Vec<_> = ui.into_messages().collect();

        assert_eq!(messages, vec![Message::BackPressed]);
    }

    #[test]
    fn view_renders_items_with_a_visible_non_collapsed_size() {
        let browse = browse(
            AsyncModel::Loaded(vec![Some(collection_item("One Piece", Some(42)))]),
            None,
        );

        let mut ui = simulator(browse.view());
        let bounds = ui
            .find("One Piece")
            .expect("item name should be found")
            .visible_bounds()
            .expect("item name should be visible");

        assert!(bounds.height > 10.0, "height was {}", bounds.height);
    }

    #[test]
    fn scrolling_past_the_bottom_of_a_tall_grid_requests_the_next_page() {
        let items: Vec<_> = (0..PAGE_SIZE)
            .map(|i| Some(collection_item(&format!("Item {i}"), None)))
            .collect();

        let browse = browse(AsyncModel::Loaded(items), Some(pagination(true, false)));

        let mut ui = simulator(browse.view());
        ui.point_at(iced::Point::new(100.0, 300.0));
        ui.simulate([iced::Event::Mouse(iced::mouse::Event::WheelScrolled {
            delta: iced::mouse::ScrollDelta::Pixels {
                x: 0.0,
                y: -100_000.0,
            },
        })]);

        let messages: Vec<_> = ui.into_messages().collect();

        assert!(
            messages.iter().any(|message| matches!(
                message,
                Message::Scrolled(metrics) if metrics.relative_offset_y >= LOAD_NEXT_PAGE_THRESHOLD
            )),
            "expected a Scrolled message near the bottom, got {messages:?}"
        );
    }
}
