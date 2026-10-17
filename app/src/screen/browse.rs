use std::collections::{HashMap, HashSet};
use std::path::PathBuf;

use iced::advanced::widget::Id as WidgetId;
use iced::widget::image;
use iced::{Element, Task};

use crate::models::{AsyncModel, library_item::LibraryItem};
use crate::providers::{Provider, provider_for_library};
use crate::repository::library as library_repository;

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

#[derive(Debug)]
pub struct Browse {
    items: AsyncModel<Vec<Option<LibraryItem>>, String>,
    pagination: Option<Pagination>,
    /// Decoded cover images, keyed by item `external_id`. Populated lazily as
    /// pages load (see [`Browse::request_page_covers`]) and pruned alongside
    /// page eviction so it doesn't grow without bound.
    covers: HashMap<String, image::Handle>,
    /// `external_id`s whose cover is currently being fetched, so a page isn't
    /// requested more than once per session.
    covers_in_flight: HashSet<String>,
    /// Whether this library accepts imported items (true for local,
    /// filesystem-backed libraries). Gates the "Import items" control.
    importable: bool,
}

#[derive(Debug)]
struct Pagination {
    external_id: String,
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
    ItemsLoaded(Result<Vec<LibraryItem>, String>),
    NextPageLoaded(Result<Vec<LibraryItem>, String>),
    PageRefetched(usize, Result<Vec<LibraryItem>, String>),
    /// Cover image bytes that have been loaded for a page, keyed by item
    /// `external_id`. Failed/omitted ids are simply absent.
    CoversLoaded(Vec<(String, Vec<u8>)>),
    Scrolled(ScrollMetrics),
    ContentOverflowChecked(bool),
    /// A series card was clicked; carries the item's `external_id`.
    SeriesSelected(String),
    BackPressed,
    /// The "Import items" button was pressed; opens the file picker.
    ImportPressed,
    /// Files chosen in the picker to import into this library (`None` if
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
        library_external_id: String,
    },
    BackPressed,
}

impl Browse {
    /// Opens a remote (integration-backed) library for browsing. Items can't be
    /// imported into it, so the import control is hidden.
    pub fn new_remote(external_id: String) -> (Self, Task<Message>) {
        Self::new_paginated(external_id, false)
    }

    /// Opens a local (filesystem-backed) library for browsing. Browsing works
    /// identically to a remote library — the provider is resolved the same way —
    /// but it additionally offers importing more items into the library.
    pub fn new_local(external_id: String) -> (Self, Task<Message>) {
        Self::new_paginated(external_id, true)
    }

    /// Shared setup for both library kinds: start on the first page and kick off
    /// its load. `importable` is the only thing that differs between them.
    fn new_paginated(external_id: String, importable: bool) -> (Self, Task<Message>) {
        (
            Self {
                items: AsyncModel::Loading,
                pagination: Some(Pagination {
                    external_id: external_id.clone(),
                    next_page: 2,
                    page_size: PAGE_SIZE,
                    has_more: true,
                    loading_next: false,
                    refetching_pages: std::collections::HashSet::new(),
                }),
                covers: HashMap::new(),
                covers_in_flight: HashSet::new(),
                importable,
            },
            Task::perform(
                crate::runtime::on_tokio(fetch_remote_items_page(external_id, 1, PAGE_SIZE)),
                Message::ItemsLoaded,
            ),
        )
    }

    pub fn update(&mut self, message: Message) -> Action {
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
            // the library is exhausted.
            Message::ContentOverflowChecked(overflows) => {
                if overflows {
                    return Action::None;
                }

                self.request_next_page().unwrap_or(Action::None)
            }

            Message::SeriesSelected(external_id) => {
                let Some(library_external_id) =
                    self.pagination.as_ref().map(|p| p.external_id.clone())
                else {
                    return Action::None;
                };

                let Some(item) = self.loaded_item(&external_id) else {
                    return Action::None;
                };

                Action::OpenSeries {
                    series_external_id: item.external_id.clone(),
                    series_name: item.name.clone(),
                    series_pages: item.pages,
                    library_external_id,
                }
            }

            Message::BackPressed => Action::BackPressed,

            Message::ImportPressed => {
                Action::Run(Task::perform(pick_files(), Message::ImportSourcesPicked))
            }

            Message::ImportSourcesPicked(None) => Action::None,
            Message::ImportSourcesPicked(Some(sources)) if sources.is_empty() => Action::None,
            Message::ImportSourcesPicked(Some(sources)) => {
                let Some(external_id) = self.pagination.as_ref().map(|p| p.external_id.clone())
                else {
                    return Action::None;
                };

                Action::Run(Task::perform(
                    import_into_library(external_id, sources),
                    Message::ItemsImported,
                ))
            }

            Message::ItemsImported(Ok(count)) => {
                tracing::debug!("imported {count} item(s) into the library");
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
        let external_id = pagination.external_id.clone();

        self.items = AsyncModel::Loading;
        self.covers.clear();
        self.covers_in_flight.clear();

        Action::Run(Task::perform(
            crate::runtime::on_tokio(fetch_remote_items_page(external_id, 1, PAGE_SIZE)),
            Message::ItemsLoaded,
        ))
    }

    /// Finds a currently-loaded item by its `external_id`, if present.
    fn loaded_item(&self, external_id: &str) -> Option<&LibraryItem> {
        let AsyncModel::Loaded(items) = &self.items else {
            return None;
        };

        items
            .iter()
            .flatten()
            .find(|item| item.external_id == external_id)
    }

    pub fn view<'a>(&'a self) -> Element<'a, Message> {
        view_helper::view(&self.items, &self.covers, self.importable)
    }

    /// Requests the next page, if pagination is set up for it, there isn't
    /// already a request in flight, and there's more to fetch.
    fn request_next_page(&mut self) -> Option<Action> {
        let pagination = self.pagination.as_mut()?;

        if pagination.loading_next || !pagination.has_more {
            return None;
        }

        pagination.loading_next = true;

        let external_id = pagination.external_id.clone();
        let page_number = pagination.next_page;
        let page_size = pagination.page_size;

        Some(Action::Run(Task::perform(
            crate::runtime::on_tokio(fetch_remote_items_page(external_id, page_number, page_size)),
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

        let external_id = pagination.external_id.clone();
        let page_number = page_index as u32 + 1;
        let page_size = pagination.page_size;

        Some(Task::perform(
            crate::runtime::on_tokio(fetch_remote_items_page(external_id, page_number, page_size)),
            move |result| Message::PageRefetched(page_index, result),
        ))
    }

    /// Builds a task to fetch covers for every currently-loaded item whose
    /// cover isn't already cached or in flight, marking them in flight.
    /// Returns `None` when there's no remote library or nothing left to fetch.
    fn request_page_covers(&mut self) -> Option<Task<Message>> {
        let library_external_id = self.pagination.as_ref()?.external_id.clone();

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
            crate::runtime::on_tokio(fetch_page_covers(library_external_id, series_ids)),
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

async fn fetch_remote_items_page(
    external_id: String,
    page_number: u32,
    page_size: u32,
) -> Result<Vec<LibraryItem>, String> {
    let (provider, _namespace) = provider_for_library(&external_id).await?;
    provider
        .list_items(&external_id, Some(page_number), Some(page_size))
        .await
}

/// Opens the system picker for one or more comic/book files to import, `None` if
/// the user cancels.
async fn pick_files() -> Option<Vec<PathBuf>> {
    let home = std::env::var("HOME").unwrap_or_else(|_| "/".to_string());

    rfd::AsyncFileDialog::new()
        .set_directory(home)
        .add_filter("Comics & Books", &["cbz", "cbr", "zip", "pdf", "epub", "doc"])
        .pick_files()
        .await
        .map(|handles| {
            handles
                .into_iter()
                .map(|handle| handle.path().to_path_buf())
                .collect()
        })
}

/// Copies each picked file into the library's folder so the next scan surfaces
/// it as a new series — the same structural rule used when first importing a
/// folder library. Only folder-backed libraries can take new items; a
/// single-file library has no folder to grow into. Existing files are left
/// untouched (never overwritten). Returns how many items were newly copied in.
async fn import_into_library(external_id: String, sources: Vec<PathBuf>) -> Result<usize, String> {
    let library = library_repository::find_by_external_id(&external_id).await?;
    let folder = library
        .folder
        .ok_or_else(|| "this library has no folder to import into".to_string())?;
    let root = PathBuf::from(folder);

    if !root.is_dir() {
        return Err("cannot import items into a single-file library".to_string());
    }

    let mut imported = 0;
    for source in sources {
        let Some(name) = source.file_name() else {
            continue;
        };
        let destination = root.join(name);

        if destination.exists() {
            tracing::debug!("skipping import of {}: already present", destination.display());
            continue;
        }

        std::fs::copy(&source, &destination)
            .map_err(|error| format!("could not copy {}: {error}", source.display()))?;
        imported += 1;
    }

    Ok(imported)
}

/// Fetches covers for a page's series, returning `(external_id, bytes)` for
/// each one that resolved. Per-id and whole-batch failures are logged and
/// omitted rather than surfaced, so a partial result still renders what it can.
async fn fetch_page_covers(
    library_external_id: String,
    series_ids: Vec<String>,
) -> Vec<(String, Vec<u8>)> {
    match load_page_covers(library_external_id, series_ids).await {
        Ok(covers) => covers,
        Err(error) => {
            tracing::debug!("failed to load page covers: {error}");
            Vec::new()
        }
    }
}

async fn load_page_covers(
    library_external_id: String,
    series_ids: Vec<String>,
) -> Result<Vec<(String, Vec<u8>)>, String> {
    let (provider, namespace) = provider_for_library(&library_external_id).await?;

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
        Length::Fill,
        Theme,
        widget::{button, column, container, grid, image, row, scrollable, text},
    };

    use crate::models::{AsyncModel, library_item::LibraryItem};

    type Covers = HashMap<String, image::Handle>;

    pub fn view<'a>(
        items: &'a AsyncModel<Vec<Option<LibraryItem>>, String>,
        covers: &'a Covers,
        importable: bool,
    ) -> Element<'a, super::Message> {
        column![top_bar(importable), body(items, covers)]
            .width(Fill)
            .height(Fill)
            .into()
    }

    fn top_bar<'a>(importable: bool) -> Element<'a, super::Message> {
        let mut bar = row![button("Back").on_press(super::Message::BackPressed)].spacing(10);

        if importable {
            bar = bar.push(button("Import items").on_press(super::Message::ImportPressed));
        }

        container(bar).padding(20).into()
    }

    fn body<'a>(
        items: &'a AsyncModel<Vec<Option<LibraryItem>>, String>,
        covers: &'a Covers,
    ) -> Element<'a, super::Message> {
        match items {
            AsyncModel::Loaded(items) if items.is_empty() => empty_state(),
            AsyncModel::Loaded(items) => items_scrollable(items, covers),
            AsyncModel::Error(error) => error_state(error),
            AsyncModel::NotLoaded | AsyncModel::Loading => container(row![]).into(),
        }
    }

    fn items_scrollable<'a>(
        items: &'a [Option<LibraryItem>],
        covers: &'a Covers,
    ) -> Element<'a, super::Message> {
        scrollable(items_grid(items, covers))
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
            })
            .into()
    }

    fn items_grid<'a>(
        items: &'a [Option<LibraryItem>],
        covers: &'a Covers,
    ) -> Element<'a, super::Message> {
        let cards = items.iter().map(|item| match item {
            Some(item) => item_card(item, covers.get(&item.external_id)),
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

    fn item_card<'a>(
        item: &'a LibraryItem,
        cover: Option<&image::Handle>,
    ) -> Element<'a, super::Message> {
        let content = column![
            thumbnail(cover),
            text(&item.name).size(13),
            pages_label(item.pages)
        ]
        .width(Fill)
        .height(Fill)
        .spacing(4)
        .align_x(Center);

        button(content)
            .on_press(super::Message::SeriesSelected(item.external_id.clone()))
            .padding(0)
            .width(Fill)
            .height(Fill)
            .style(button::text)
            .into()
    }

    /// Placeholder for an item whose data has been evicted from memory
    /// because it was scrolled far off-screen; keeps the grid's item count
    /// (and thus its scroll geometry) stable.
    fn evicted_card<'a>() -> Element<'a, super::Message> {
        container(thumbnail(None)).width(Fill).height(Fill).into()
    }

    /// Renders an item's cover if it has loaded, otherwise the same styled
    /// grey box used as a placeholder (and for evicted items).
    fn thumbnail<'a>(handle: Option<&image::Handle>) -> Element<'a, super::Message> {
        let inner: Element<'a, super::Message> = match handle {
            Some(handle) => image(handle.clone())
                .width(Fill)
                .height(Fill)
                .content_fit(ContentFit::Cover)
                .into(),
            None => row![].into(),
        };

        container(inner)
            .width(Fill)
            .height(Fill)
            .clip(true)
            .style(|theme: &Theme| container::Style {
                background: Some(Background::Color(
                    theme.extended_palette().background.strong.color,
                )),
                border: Border {
                    radius: 8.0.into(),
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

    fn library_item(name: &str, pages: Option<u16>) -> LibraryItem {
        LibraryItem {
            external_id: name.to_string(),
            name: name.to_string(),
            cover: String::new(),
            library_id: "1".to_string(),
            pages,
        }
    }

    /// Builds a `Browse` with empty cover state, so tests only have to specify
    /// the `items`/`pagination` they care about.
    fn browse(
        items: AsyncModel<Vec<Option<LibraryItem>>, String>,
        pagination: Option<Pagination>,
    ) -> Browse {
        Browse {
            items,
            pagination,
            covers: HashMap::new(),
            covers_in_flight: HashSet::new(),
            importable: false,
        }
    }

    /// An importable (local, filesystem-backed) `Browse`, for the import tests.
    fn importable_browse(
        items: AsyncModel<Vec<Option<LibraryItem>>, String>,
        pagination: Option<Pagination>,
    ) -> Browse {
        Browse {
            importable: true,
            ..browse(items, pagination)
        }
    }

    fn pagination(has_more: bool, loading_next: bool) -> Pagination {
        Pagination {
            external_id: "lib-1".to_string(),
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
    fn new_local_sets_up_paginated_browsing_and_allows_importing() {
        let (browse, _task) = Browse::new_local("library-id".to_string());

        assert!(matches!(browse.items, AsyncModel::Loading));
        let pagination = browse.pagination.expect("pagination should be set up");
        assert_eq!(pagination.external_id, "library-id");
        assert_eq!(pagination.next_page, 2);
        assert!(browse.importable);
    }

    #[test]
    fn new_remote_starts_pagination_at_the_second_page() {
        let (browse, _task) = Browse::new_remote("external-id".to_string());

        let pagination = browse.pagination.expect("pagination should be set up");
        assert_eq!(pagination.next_page, 2);
        assert!(pagination.has_more);
        assert!(!pagination.loading_next);
        assert!(!browse.importable);
    }

    #[test]
    fn update_items_loaded_stores_result_and_checks_the_viewport() {
        let mut browse = browse(AsyncModel::Loading, Some(pagination(true, false)));

        let action = browse.update(Message::ItemsLoaded(Ok(vec![library_item(
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
            .map(|i| library_item(&format!("Item {i}"), None))
            .collect();

        browse.update(Message::ItemsLoaded(Ok(full_page)));

        assert!(browse.pagination.unwrap().has_more);
    }

    #[test]
    fn update_items_loaded_with_a_short_page_clears_has_more() {
        let mut browse = browse(AsyncModel::Loading, Some(pagination(true, false)));

        browse.update(Message::ItemsLoaded(Ok(vec![library_item(
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
            AsyncModel::Loaded(vec![Some(library_item("One Piece", Some(42)))]),
            Some(pagination(true, false)),
        );

        let action = browse.update(Message::SeriesSelected("One Piece".to_string()));

        match action {
            Action::OpenSeries {
                series_external_id,
                series_name,
                series_pages,
                library_external_id,
            } => {
                assert_eq!(series_external_id, "One Piece");
                assert_eq!(series_name, "One Piece");
                assert_eq!(series_pages, Some(42));
                assert_eq!(library_external_id, "lib-1");
            }
            other => panic!("expected OpenSeries, got {other:?}"),
        }
    }

    #[test]
    fn update_series_selected_for_an_unknown_item_returns_none() {
        let mut browse = browse(
            AsyncModel::Loaded(vec![Some(library_item("One Piece", None))]),
            Some(pagination(true, false)),
        );

        let action = browse.update(Message::SeriesSelected("Berserk".to_string()));

        assert!(matches!(action, Action::None));
    }

    #[test]
    fn update_series_selected_without_pagination_returns_none() {
        let mut browse = browse(
            AsyncModel::Loaded(vec![Some(library_item("One Piece", None))]),
            None,
        );

        let action = browse.update(Message::SeriesSelected("One Piece".to_string()));

        assert!(matches!(action, Action::None));
    }

    #[test]
    fn clicking_an_item_card_sends_series_selected() {
        let browse = browse(
            AsyncModel::Loaded(vec![Some(library_item("One Piece", Some(42)))]),
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
    fn update_back_pressed_returns_back_pressed_action() {
        let mut browse = browse(AsyncModel::Loading, None);

        let action = browse.update(Message::BackPressed);

        assert!(matches!(action, Action::BackPressed));
    }

    #[test]
    fn view_shows_the_import_button_only_for_importable_libraries() {
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
            AsyncModel::Loaded(vec![Some(library_item("Old", None))]),
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
            AsyncModel::Loaded(vec![Some(library_item("Still here", None))]),
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
                    .map(|i| Some(library_item(&format!("Item {i}"), None)))
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
                    .map(|i| Some(library_item(&format!("Item {i}"), None)))
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
            AsyncModel::Loaded(vec![Some(library_item("One Piece", None))]),
            Some(pagination(true, true)),
        );

        let action = browse.update(Message::Scrolled(scroll_metrics(0.95)));

        assert!(matches!(action, Action::None));
    }

    #[test]
    fn scrolling_near_the_bottom_without_more_pages_does_nothing() {
        let mut browse = browse(
            AsyncModel::Loaded(vec![Some(library_item("One Piece", None))]),
            Some(pagination(false, false)),
        );

        let action = browse.update(Message::Scrolled(scroll_metrics(0.95)));

        assert!(matches!(action, Action::None));
    }

    #[test]
    fn next_page_loaded_appends_items_and_advances_pagination() {
        let mut browse = browse(
            AsyncModel::Loaded(vec![Some(library_item("One Piece", None))]),
            Some(pagination(true, true)),
        );

        let action = browse.update(Message::NextPageLoaded(Ok(vec![library_item(
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
            AsyncModel::Loaded(vec![Some(library_item("One Piece", None))]),
            Some(pagination(true, false)),
        );

        let action = browse.update(Message::ContentOverflowChecked(true));

        assert!(matches!(action, Action::None));
        assert!(!browse.pagination.unwrap().loading_next);
    }

    #[test]
    fn content_overflow_checked_false_requests_the_next_page_to_fill_the_viewport() {
        let mut browse = browse(
            AsyncModel::Loaded(vec![Some(library_item("One Piece", None))]),
            Some(pagination(true, false)),
        );

        let action = browse.update(Message::ContentOverflowChecked(false));

        assert!(matches!(action, Action::Run(_)));
        assert!(browse.pagination.unwrap().loading_next);
    }

    #[test]
    fn content_overflow_checked_false_without_more_pages_does_nothing() {
        let mut browse = browse(
            AsyncModel::Loaded(vec![Some(library_item("One Piece", None))]),
            Some(pagination(false, false)),
        );

        let action = browse.update(Message::ContentOverflowChecked(false));

        assert!(matches!(action, Action::None));
    }

    #[test]
    fn content_overflow_checked_false_while_already_loading_does_nothing() {
        let mut browse = browse(
            AsyncModel::Loaded(vec![Some(library_item("One Piece", None))]),
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
            AsyncModel::Loaded(vec![Some(library_item("One Piece", None))]),
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
            .map(|i| Some(library_item(&format!("Item {i}"), None)))
            .collect();

        let mut pagination_state = pagination(true, false);
        pagination_state.next_page = KEEP_PAGES_LOADED as u32 + 1;

        let mut browse = browse(AsyncModel::Loaded(items), Some(pagination_state));

        let new_page: Vec<_> = (0..PAGE_SIZE)
            .map(|i| library_item(&format!("New {i}"), None))
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
            .map(|i| Some(library_item(&format!("Item {i}"), None)))
            .collect();

        let mut browse = browse(AsyncModel::Loaded(items), Some(pagination(true, false)));

        let new_page: Vec<_> = (0..PAGE_SIZE)
            .map(|i| library_item(&format!("New {i}"), None))
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
            .map(|i| Some(library_item(&format!("Item {i}"), None)))
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
            .map(|i| Some(library_item(&format!("Item {i}"), None)))
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
            .map(|i| Some(library_item(&format!("Item {i}"), None)))
            .collect();

        let mut browse = browse(AsyncModel::Loaded(items), Some(pagination(false, false)));

        let action = browse.update(Message::Scrolled(scroll_metrics_at_page(60, 0)));

        assert!(matches!(action, Action::None));
        assert!(browse.pagination.unwrap().refetching_pages.is_empty());
    }

    #[test]
    fn scrolling_onto_an_evicted_page_already_being_refetched_does_not_request_again() {
        let mut items: Vec<_> = (0..(PAGE_SIZE * 3))
            .map(|i| Some(library_item(&format!("Item {i}"), None)))
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
            .map(|i| Some(library_item(&format!("Item {i}"), None)))
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
            .map(|i| Some(library_item(&format!("Item {i}"), None)))
            .collect();
        for slot in &mut items[0..PAGE_SIZE as usize] {
            *slot = None;
        }

        let mut pagination_state = pagination(false, false);
        pagination_state.refetching_pages.insert(0);

        let mut browse = browse(AsyncModel::Loaded(items), Some(pagination_state));

        let refetched: Vec<_> = (0..PAGE_SIZE)
            .map(|i| library_item(&format!("Refetched {i}"), None))
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
        let items = vec![None, Some(library_item("One Piece", None))];

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
            AsyncModel::Loaded(vec![Some(library_item("One Piece", None))]),
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
                Some(library_item("cached", None)),
                Some(library_item("in-flight", None)),
                Some(library_item("fresh", None)),
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
            AsyncModel::Loaded(vec![Some(library_item("One Piece", None))]),
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
            AsyncModel::Loaded(vec![None, Some(library_item("still-here", None))]),
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
            AsyncModel::Loaded(vec![Some(library_item("One Piece", Some(42)))]),
            None,
        );

        let mut ui = simulator(browse.view());

        assert!(ui.find("One Piece").is_ok());
        assert!(ui.find("42 pages").is_ok());
    }

    #[test]
    fn view_does_not_render_pages_when_absent() {
        let browse = browse(
            AsyncModel::Loaded(vec![Some(library_item("One Piece", None))]),
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
        let _ = ui.click("Back").expect("Back button should be found");

        let messages: Vec<_> = ui.into_messages().collect();

        assert_eq!(messages, vec![Message::BackPressed]);
    }

    #[test]
    fn view_renders_items_with_a_visible_non_collapsed_size() {
        let browse = browse(
            AsyncModel::Loaded(vec![Some(library_item("One Piece", Some(42)))]),
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
            .map(|i| Some(library_item(&format!("Item {i}"), None)))
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
