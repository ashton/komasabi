//! The comic reader window: given a folder of images, display them one page at
//! a time with navigation, zoom, rotation and fit controls. It has no knowledge
//! of where the images came from — a host process (the Komasabi app) downloads a
//! chapter's pages into a folder and launches a reader process pointed at it.
//!
//! Arguments (see [`run`]): `<dir> [--page N] [--events <socket-name>]`
//! - `<dir>`: folder of page image files named `{index:05}.img`.
//! - `--page N`: 0-based page to open on (clamped).
//! - `--events <socket-name>`: `interprocess` namespaced socket the host is
//!   listening on. Its presence selects **hosted** mode: the reader opens a
//!   duplex channel over it, learns the session (title, page count) from an
//!   `OpenSession` command, loads each page as the host reports it ready with
//!   `PageReady`, and reports what's on screen back with `ShowingPage`. Omitted
//!   for **standalone** runs, which just enumerate `<dir>` in filename order.

use std::collections::{HashSet, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use iced::animation::{Animation, Easing};
use iced::futures::{SinkExt, Stream, StreamExt};
use iced::keyboard::{self, Key, key::Named};
use iced::time::{Duration, Instant};
use iced::widget::{
    button, column, container, float, image, mouse_area, opaque, row, space, stack, text,
};
use iced::{
    Border, Color, ContentFit, Element, Length::Fill, Radians, Rotation, Subscription, Task, Vector,
};

use crate::{
    AdjacentEntry, Direction, EntryKind, EventSender, ReaderCommand, ReaderEvent, ReaderLink,
};

const ZOOM_STEP: f32 = 1.25;
const MIN_ZOOM: f32 = 0.1;
const MAX_ZOOM: f32 = 10.0;
/// How many pages ahead of the current one to decode in the background. Reading
/// moves forward, so we warm the next pages more aggressively than the previous.
const PREFETCH_AHEAD: usize = 2;
/// How many decoded pages to keep around. Comfortably larger than the window a
/// single flip touches (current, a couple ahead, one behind) so local
/// back-and-forth navigation stays in cache.
const CACHE_CAPACITY: usize = 9;
/// How long the top and bottom bars stay on screen after a tap on the middle
/// of the page, before sliding away again.
const CHROME_VISIBLE_FOR: Duration = Duration::from_secs(5);
/// How long the bars take to slide in or out.
const CHROME_SLIDE: Duration = Duration::from_millis(250);
/// Size of the adjacent entry's cover in the end-of-entry popup (2:3, like a
/// comic cover).
const POPUP_COVER_WIDTH: f32 = 150.0;
const POPUP_COVER_HEIGHT: f32 = 225.0;
const APP_ICON: &[u8] = include_bytes!("../assets/komasabi-icon.png");

/// Runs the reader window until it's closed. `args` are the reader's own
/// arguments, without the program name (or the host's subcommand) in front.
pub fn run(args: impl IntoIterator<Item = String>) -> iced::Result {
    let config = Config::from_args(args);
    iced::application(
        move || Reader::new(config.clone()),
        Reader::update,
        Reader::view,
    )
    .subscription(Reader::subscription)
    .title("Komasabi Reader")
    .window(iced::window::Settings {
        icon: iced::window::icon::from_file_data(APP_ICON, None).ok(),
        ..Default::default()
    })
    .run()
}

/// How a page is scaled into the viewport.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Fit {
    /// Whole page visible, letterboxed (default).
    Contain,
    /// Fill the viewport, cropping overflow.
    Cover,
    /// Native pixel size.
    Original,
}

impl Fit {
    fn content_fit(self) -> ContentFit {
        match self {
            Fit::Contain => ContentFit::Contain,
            Fit::Cover => ContentFit::Cover,
            Fit::Original => ContentFit::None,
        }
    }

    fn next(self) -> Fit {
        match self {
            Fit::Contain => Fit::Cover,
            Fit::Cover => Fit::Original,
            Fit::Original => Fit::Contain,
        }
    }

    fn label(self) -> &'static str {
        match self {
            Fit::Contain => "Fit",
            Fit::Cover => "Fill",
            Fit::Original => "1:1",
        }
    }
}

struct Reader {
    /// Folder the page files live in; page `i` is `dir/{i:05}.img`.
    dir: PathBuf,
    /// Reading-order page paths. Sized by enumerating `dir` (standalone) or from
    /// `OpenSession`'s page count (hosted).
    pages: Vec<PathBuf>,
    /// Pages known to be on disk and therefore worth loading. All of them in
    /// standalone mode; grown by `PageReady` in hosted mode.
    ready: HashSet<usize>,
    current: usize,
    /// Page to open on once the session is known (from `--page`).
    start_page: usize,
    /// Decoded pages kept around so the current page and recently visited ones
    /// render without re-reading or re-decoding. Filled by background loads, not
    /// on the UI thread.
    cache: PageCache,
    /// Pages whose background load is in flight, so concurrent flips and
    /// prefetches don't queue the same page twice.
    inflight: HashSet<usize>,
    /// Session title parts, shown in the header once known (hosted mode).
    series: Option<String>,
    entry: Option<String>,
    /// Whether the session's entry is a volume or a chapter, when the host
    /// says, to word the end-of-entry popup.
    kind: Option<EntryKind>,
    /// Hosted mode, before `OpenSession` has arrived — there's nothing to show
    /// yet, but it isn't an empty folder either.
    awaiting_session: bool,
    zoom: f32,
    rotation_quarters: u8,
    fit: Fit,
    /// Emits events to the host; disabled until the socket connects (`Connected`)
    /// and in standalone mode.
    events: EventHandle,
    /// The host socket name in hosted mode; drives the command subscription.
    /// `None` in standalone mode.
    events_socket: Option<String>,
    /// Set when startup failed (bad args, unreadable/empty folder); shown
    /// instead of a page.
    error: Option<String>,
    /// Counts the sessions this window has shown (one per `OpenSession`), so a
    /// page load that finishes after the user moved on to the next entry is
    /// recognised as stale and dropped.
    session: u64,
    /// What the host said comes after this session's entry (hosted mode).
    next: Adjacent,
    /// What the host said comes before this session's entry (hosted mode).
    previous: Adjacent,
    /// The popup shown when the user tried to turn past the last page
    /// (`Next`) or the first (`Previous`), offering the entry that way.
    popup: Option<Direction>,
    /// Set once the user asked to open the entry that way, until the host
    /// opens its session (or offers it again, when it couldn't).
    opening: Option<Direction>,
    /// Whether the top and bottom bars ("chrome") are shown, animated so they
    /// slide in and out. Hidden until the user taps the middle of the page.
    chrome: Animation<bool>,
    /// Counts the times the chrome was shown, so the timer that hides it again
    /// is ignored once a later tap has restarted (or cancelled) it.
    chrome_shown: u64,
    /// The time the view is drawn at, advanced every frame while the chrome
    /// slides.
    now: Instant,
}

/// The entry on one side of the one being read, as far as the reader knows.
/// The host looks it up when the user reaches that end of the entry.
#[derive(Debug, Clone, PartialEq)]
enum Adjacent {
    /// Not known (yet): the host hasn't answered, or there's no host at all.
    Unknown,
    /// The host has this entry ready to open.
    Available(Offer),
    /// The host found nothing on that side.
    None,
}

/// An adjacent entry the reader can offer, with its cover ready to draw.
#[derive(Debug, Clone, PartialEq)]
struct Offer {
    label: String,
    kind: EntryKind,
    cover: Option<image::Handle>,
}

impl From<AdjacentEntry> for Offer {
    fn from(entry: AdjacentEntry) -> Self {
        Self {
            label: entry.label,
            kind: entry.kind,
            cover: entry.cover.map(image::Handle::from_path),
        }
    }
}

#[derive(Debug, Clone)]
enum Message {
    Next,
    Prev,
    First,
    Last,
    ZoomIn,
    ZoomOut,
    ZoomReset,
    Rotate,
    CycleFit,
    /// A background load for page `index` of session number `session`
    /// finished; `None` if it couldn't be read or decoded.
    PageLoaded {
        session: u64,
        index: usize,
        handle: Option<image::Handle>,
    },
    /// The user accepted the popup's offer, to read the entry it shows.
    OpenAdjacent,
    /// The user closed the popup, staying on the entry being read.
    DismissPopup,
    /// The middle of the page was clicked or tapped: show the bars, or hide
    /// them if they're showing.
    ToggleChrome,
    /// The timer started when the bars were shown for the `n`th time ran out.
    HideChrome(u64),
    /// A frame is being drawn while the bars slide.
    Frame(Instant),
    /// The command subscription connected; carries the handle for emitting
    /// events back to the host.
    Connected(EventHandle),
    /// A command arrived from the host over the socket.
    Command(ReaderCommand),
}

/// A cheap, cloneable handle around an [`EventSender`] so it can ride in a
/// [`Message`] (which must be `Clone` + `Debug`). The underlying sender is a
/// channel to a background writer thread; cloning just shares it.
#[derive(Clone)]
struct EventHandle(Arc<EventSender>);

impl EventHandle {
    fn new(events: EventSender) -> Self {
        Self(Arc::new(events))
    }

    fn disabled() -> Self {
        Self::new(EventSender::disabled())
    }

    fn send(&self, event: ReaderEvent) {
        self.0.send(event);
    }
}

impl std::fmt::Debug for EventHandle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("EventHandle")
    }
}

impl Reader {
    fn new(config: Result<Config, String>) -> (Self, Task<Message>) {
        match config {
            Ok(config) if config.events.is_some() => (Self::hosted(config), Task::none()),
            Ok(config) => {
                let mut reader = Self::standalone(config);
                let task = reader.ensure_loaded();
                (reader, task)
            }
            Err(error) => (Self::failed(error), Task::none()),
        }
    }

    /// Base reader with no pages and nothing in flight; the constructors fill in
    /// the mode-specific fields.
    fn blank(dir: PathBuf, start_page: usize, events_socket: Option<String>) -> Self {
        Self {
            dir,
            pages: Vec::new(),
            ready: HashSet::new(),
            current: 0,
            start_page,
            cache: PageCache::new(CACHE_CAPACITY),
            inflight: HashSet::new(),
            series: None,
            entry: None,
            kind: None,
            awaiting_session: false,
            zoom: 1.0,
            rotation_quarters: 0,
            fit: Fit::Contain,
            events: EventHandle::disabled(),
            events_socket,
            error: None,
            session: 0,
            next: Adjacent::Unknown,
            previous: Adjacent::Unknown,
            popup: None,
            opening: None,
            chrome: Animation::new(false)
                .duration(CHROME_SLIDE)
                .easing(Easing::EaseOutCubic),
            chrome_shown: 0,
            now: Instant::now(),
        }
    }

    /// A reader that only shows `error` — used when startup never got as far as
    /// a page folder.
    fn failed(error: String) -> Self {
        let mut reader = Self::blank(PathBuf::new(), 0, None);
        reader.error = Some(error);
        reader
    }

    /// A hosted reader: no pages until the host sends `OpenSession`, and a socket
    /// name for the command subscription to connect to.
    fn hosted(config: Config) -> Self {
        let mut reader = Self::blank(config.dir, config.start_page, config.events);
        reader.awaiting_session = true;
        reader
    }

    /// A standalone reader: enumerates `dir` up front, so every page is ready
    /// immediately and there's no host to talk to.
    fn standalone(config: Config) -> Self {
        let mut reader = Self::blank(config.dir, config.start_page, None);

        match list_pages(&reader.dir) {
            Ok(pages) if pages.is_empty() => {
                reader.error = Some(format!("no images found in {}", reader.dir.display()));
            }
            Ok(pages) => {
                reader.ready = (0..pages.len()).collect();
                reader.current = reader.start_page.min(pages.len() - 1);
                reader.pages = pages;
            }
            Err(error) => reader.error = Some(error),
        }

        reader
    }

    fn update(&mut self, message: Message) -> Task<Message> {
        match message {
            Message::Next => self.turn(Direction::Next),
            Message::Prev => self.turn(Direction::Previous),
            Message::First => {
                self.popup = None;
                self.go_to(0)
            }
            Message::Last => {
                self.popup = None;
                self.go_to(self.pages.len().saturating_sub(1))
            }
            Message::ZoomIn => {
                self.zoom = (self.zoom * ZOOM_STEP).min(MAX_ZOOM);
                Task::none()
            }
            Message::ZoomOut => {
                self.zoom = (self.zoom / ZOOM_STEP).max(MIN_ZOOM);
                Task::none()
            }
            Message::ZoomReset => {
                self.zoom = 1.0;
                Task::none()
            }
            Message::Rotate => {
                self.rotation_quarters = (self.rotation_quarters + 1) % 4;
                Task::none()
            }
            Message::CycleFit => {
                self.fit = self.fit.next();
                Task::none()
            }
            Message::PageLoaded {
                session,
                index,
                handle,
            } => {
                if session != self.session {
                    // A page of the entry the user has since moved on from.
                    return Task::none();
                }
                self.inflight.remove(&index);
                if let Some(handle) = handle {
                    self.cache.insert(index, handle);
                }
                Task::none()
            }
            Message::ToggleChrome => {
                self.now = Instant::now();
                if self.chrome.value() {
                    self.chrome.go_mut(false, self.now);
                    // Cancels the pending hide timer.
                    self.chrome_shown += 1;
                    Task::none()
                } else {
                    self.chrome.go_mut(true, self.now);
                    self.chrome_shown += 1;
                    hide_chrome_after(CHROME_VISIBLE_FOR, self.chrome_shown)
                }
            }
            Message::HideChrome(shown) => {
                if shown == self.chrome_shown && self.chrome.value() {
                    self.now = Instant::now();
                    self.chrome.go_mut(false, self.now);
                }
                Task::none()
            }
            Message::Frame(now) => {
                self.now = now;
                Task::none()
            }
            Message::Connected(events) => {
                self.events = events;
                Task::none()
            }
            Message::Command(command) => self.handle_command(command),
            Message::OpenAdjacent => {
                if let Some(direction) = self.popup {
                    self.open_adjacent(direction);
                }
                Task::none()
            }
            Message::DismissPopup => {
                if self.opening.is_none() {
                    self.popup = None;
                }
                Task::none()
            }
        }
    }

    /// Applies a host command: `OpenSession` installs the session and starts
    /// loading the opening page; `PageReady` marks a page available and loads it
    /// if it's one we currently want.
    fn handle_command(&mut self, command: ReaderCommand) -> Task<Message> {
        match command {
            ReaderCommand::OpenSession {
                series,
                entry,
                pages,
                kind,
                dir,
                start_page,
            } => {
                if let Some(dir) = dir {
                    // An adjacent entry: a new folder, read from the page the
                    // host asks for, with nothing of the previous entry's
                    // carried over.
                    self.dir = dir;
                    self.start_page = start_page;
                    self.ready.clear();
                    self.cache = PageCache::new(CACHE_CAPACITY);
                    self.inflight.clear();
                }
                self.session += 1;
                self.next = Adjacent::Unknown;
                self.previous = Adjacent::Unknown;
                self.popup = None;
                self.opening = None;
                self.series = Some(series);
                self.entry = Some(entry);
                self.kind = kind;
                self.pages = (0..pages).map(|i| self.page_path(i)).collect();
                self.current = self.start_page.min(pages.saturating_sub(1));
                self.awaiting_session = false;
                self.error = None;
                if !self.pages.is_empty() {
                    self.events
                        .send(ReaderEvent::ShowingPage { page: self.current });
                }
                self.ensure_loaded()
            }
            ReaderCommand::PageReady { index } => {
                self.ready.insert(index);
                if self.wants(index) {
                    if let Some(task) = self.request(index) {
                        return task;
                    }
                }
                Task::none()
            }
            ReaderCommand::Adjacent { direction, entry } => {
                *self.adjacent_mut(direction) = match entry {
                    Some(entry) => Adjacent::Available(entry.into()),
                    None => Adjacent::None,
                };
                // An answer while opening means the host couldn't open it and
                // is offering it again.
                if self.opening == Some(direction) {
                    self.opening = None;
                }
                Task::none()
            }
        }
    }

    fn adjacent(&self, direction: Direction) -> &Adjacent {
        match direction {
            Direction::Next => &self.next,
            Direction::Previous => &self.previous,
        }
    }

    fn adjacent_mut(&mut self, direction: Direction) -> &mut Adjacent {
        match direction {
            Direction::Next => &mut self.next,
            Direction::Previous => &mut self.previous,
        }
    }

    /// Whether the reader is on the page at the `direction` end of the session:
    /// its last page going forward, its first going back.
    fn at_end(&self, direction: Direction) -> bool {
        !self.pages.is_empty()
            && match direction {
                Direction::Next => self.current + 1 == self.pages.len(),
                Direction::Previous => self.current == 0,
            }
    }

    /// A page turn in `direction`. Within the entry it just turns the page. At
    /// that end of a hosted session it pops up what lies beyond instead, and a
    /// second turn the same way opens it. A turn the other way while the popup
    /// is up only closes it.
    fn turn(&mut self, direction: Direction) -> Task<Message> {
        if self.opening.is_some() {
            return Task::none();
        }
        match self.popup {
            Some(shown) if shown == direction => {
                self.open_adjacent(direction);
                return Task::none();
            }
            Some(_) => {
                self.popup = None;
                return Task::none();
            }
            None => {}
        }

        if self.events_socket.is_some() && self.at_end(direction) {
            self.popup = Some(direction);
            return Task::none();
        }

        match direction {
            Direction::Next => self.go_to(self.current + 1),
            Direction::Previous => self.go_to(self.current.saturating_sub(1)),
        }
    }

    /// Asks the host to open the entry on the `direction` side, if it offered
    /// one.
    fn open_adjacent(&mut self, direction: Direction) {
        if self.opening.is_none() && matches!(self.adjacent(direction), Adjacent::Available(_)) {
            self.opening = Some(direction);
            self.events.send(ReaderEvent::OpenAdjacent { direction });
        }
    }

    /// The on-disk path for page `index`, under the session folder.
    fn page_path(&self, index: usize) -> PathBuf {
        self.dir.join(format!("{index:05}.img"))
    }

    /// Moves to `page` (clamped to the valid range). On an actual change it emits
    /// a `ShowingPage` event and kicks off the background loads for the new page
    /// and its neighbours; the handle itself arrives later via `PageLoaded`.
    fn go_to(&mut self, page: usize) -> Task<Message> {
        // While an adjacent entry is opening, a page turn would be reported to
        // the host as a page of the entry it's about to start.
        if self.pages.is_empty() || self.opening.is_some() {
            return Task::none();
        }

        let target = page.min(self.pages.len() - 1);
        if target == self.current {
            return Task::none();
        }

        self.current = target;
        self.cache.touch(target);
        self.events.send(ReaderEvent::ShowingPage { page: target });
        self.ensure_loaded()
    }

    /// Requests the current page plus a few neighbours, so the current one is
    /// being decoded and the likely next/previous flips are already warm.
    fn ensure_loaded(&mut self) -> Task<Message> {
        let tasks: Vec<_> = self
            .prefetch_targets()
            .into_iter()
            .filter_map(|index| self.request(index))
            .collect();
        Task::batch(tasks)
    }

    /// Pages worth decoding around `current`: the page itself (first, so it's
    /// dispatched ahead of the prefetches), a couple ahead, and one behind.
    fn prefetch_targets(&self) -> Vec<usize> {
        let mut targets = vec![self.current];
        for ahead in 1..=PREFETCH_AHEAD {
            targets.push(self.current + ahead);
        }
        if self.current > 0 {
            targets.push(self.current - 1);
        }
        targets
    }

    /// Whether `index` is one of the pages we'd want decoded around the current
    /// position — used to decide if a freshly-ready page is worth loading now.
    fn wants(&self, index: usize) -> bool {
        self.prefetch_targets().contains(&index)
    }

    /// Starts a background load for `index` unless it's out of range, not yet on
    /// disk, already cached, or already in flight. Marks it in flight on dispatch.
    fn request(&mut self, index: usize) -> Option<Task<Message>> {
        if index >= self.pages.len()
            || !self.ready.contains(&index)
            || self.cache.contains(index)
            || self.inflight.contains(&index)
        {
            return None;
        }

        self.inflight.insert(index);
        Some(load_page_task(
            self.session,
            index,
            self.pages[index].clone(),
        ))
    }

    fn view(&self) -> Element<'_, Message> {
        let page: Element<'_, Message> = if let Some(error) = &self.error {
            message_screen(error)
        } else if let Some(handle) = self.cache.peek(self.current) {
            container(
                image(handle.clone())
                    .width(Fill)
                    .height(Fill)
                    .content_fit(self.fit.content_fit())
                    .rotation(Rotation::Solid(Radians(
                        self.rotation_quarters as f32 * std::f32::consts::FRAC_PI_2,
                    )))
                    .scale(self.zoom),
            )
            .center_x(Fill)
            .center_y(Fill)
            .clip(true)
            .into()
        } else if self.pages.is_empty() {
            // Nothing to show: either still waiting on the host's session, or a
            // genuinely empty standalone folder.
            message_screen(if self.awaiting_session {
                "loading…"
            } else {
                "no page"
            })
        } else if self.inflight.contains(&self.current) || !self.ready.contains(&self.current) {
            // In flight, or not downloaded by the host yet — both read as loading.
            message_screen("loading…")
        } else {
            message_screen("could not load this page")
        };

        let mut layers = stack![page, touch_zones()].width(Fill).height(Fill);
        if let Some(chrome) = self.chrome() {
            layers = layers.push(chrome);
        }
        if let Some(popup) = self.popup() {
            layers = layers.push(popup);
        }
        layers.into()
    }

    /// Whether the bars are on screen, fully or partly (mid-slide).
    fn chrome_visible(&self) -> bool {
        self.chrome.value() || self.chrome.is_animating(self.now)
    }

    /// The top bar (title) and bottom bar (status) laid
    /// over the page, each slid off its edge of the window by how far the
    /// chrome is from fully shown. `None` while they're fully hidden, so
    /// nothing covers the page.
    fn chrome(&self) -> Option<Element<'_, Message>> {
        if !self.chrome_visible() {
            return None;
        }

        // 0 = fully hidden, 1 = fully shown.
        let shown: f32 = self.chrome.interpolate(0.0, 1.0, self.now);
        let hidden = 1.0 - shown;

        let mut top = column![].width(Fill);
        if let Some(header) = self.header() {
            top = top.push(header);
        }

        let bottom = column![self.status_bar()].width(Fill);

        // `opaque` keeps a tap on a bar from reaching the page-turn zones
        // underneath it.
        let top = float(opaque(chrome_bar(top)))
            .translate(move |bounds, _| Vector::new(0.0, -bounds.height * hidden));
        let bottom = float(opaque(chrome_bar(bottom)))
            .translate(move |bounds, _| Vector::new(0.0, bounds.height * hidden));

        Some(
            column![top, space().height(Fill), bottom]
                .width(Fill)
                .height(Fill)
                .into(),
        )
    }

    /// The popup shown when the user tried to turn past an end of the entry:
    /// over a dimmed backdrop, a centred note that the entry ended (or starts
    /// here, going back) and the entry beyond it, as a cover to click; or that
    /// there is none. A click on the backdrop closes it. `None` while closed.
    fn popup(&self) -> Option<Element<'_, Message>> {
        let direction = self.popup?;
        let noun = self.kind.map_or("entry", EntryKind::noun);

        let mut card = column![].spacing(14).align_x(iced::Center);
        match self.adjacent(direction) {
            Adjacent::Unknown => {
                card = card
                    .push(text(end_message(direction, noun)).size(20))
                    .push(text("Looking for what's next…").size(14));
            }
            Adjacent::None => {
                let missing = match direction {
                    Direction::Next => format!("This {noun} ended, there is no next {noun}."),
                    Direction::Previous => {
                        format!("This is the first {noun}, there is no previous {noun}.")
                    }
                };
                card = card.push(text(missing).size(20));
            }
            Adjacent::Available(offer) => {
                let heading = match direction {
                    Direction::Next => format!("This {noun} ended, next one:"),
                    Direction::Previous => format!("This {noun} starts here, previous one:"),
                };
                let opening = self.opening == Some(direction);
                let cover = button(offer_cover(offer))
                    .padding(0)
                    .style(|_, _| button::Style::default())
                    .on_press_maybe((!opening).then_some(Message::OpenAdjacent));
                let caption = if opening {
                    format!("Opening {}…", offer.label)
                } else {
                    offer.label.clone()
                };
                card = card
                    .push(text(heading).size(20))
                    .push(cover)
                    .push(text(caption).size(16));
            }
        }

        let card = container(card)
            .padding(24)
            .max_width(480)
            .style(|_| container::Style {
                background: Some(Color::from_rgba(0.08, 0.08, 0.08, 0.9).into()),
                text_color: Some(Color::WHITE),
                border: Border::default().rounded(12),
                ..container::Style::default()
            });

        // `opaque` keeps a click on the card from reaching the backdrop (and
        // closing the popup), and one on the backdrop from turning the page.
        let backdrop =
            container(opaque(card))
                .center(Fill)
                .padding(16)
                .style(|_| container::Style {
                    background: Some(Color::from_rgba(0.0, 0.0, 0.0, 0.6).into()),
                    ..container::Style::default()
                });
        Some(opaque(mouse_area(backdrop).on_press(Message::DismissPopup)))
    }

    /// The title line: `series — entry` once the host has told us what we're
    /// reading, or the folder's name in standalone mode.
    fn header(&self) -> Option<Element<'_, Message>> {
        let title = match (&self.series, &self.entry) {
            (Some(series), Some(entry)) => format!("{series}  —  {entry}"),
            (Some(part), None) | (None, Some(part)) => part.clone(),
            (None, None) => self.dir.file_name()?.to_string_lossy().into_owned(),
        };
        Some(container(text(title).size(14)).padding(8).into())
    }

    /// A thin, non-interactive status line. Navigation is by keyboard (see
    /// [`key_message`]) or the page's click zones (see [`touch_zones`]); this reflects reading position (page and progress)
    /// and view state (zoom, fit).
    fn status_bar(&self) -> Element<'_, Message> {
        let position = if self.pages.is_empty() {
            "0 / 0".to_string()
        } else {
            let progress = (self.current + 1) * 100 / self.pages.len();
            format!("{} / {} ({progress}%)", self.current + 1, self.pages.len())
        };

        let status = format!(
            "{position}  ·  {:.0}%  ·  {}",
            self.zoom * 100.0,
            self.fit.label()
        );

        container(text(status).size(13)).padding(8).into()
    }

    fn subscription(&self) -> Subscription<Message> {
        let keyboard = keyboard::listen().filter_map(key_message);
        // Redraw every frame only while the bars are sliding.
        let keyboard = if self.chrome.is_animating(self.now) {
            Subscription::batch([keyboard, iced::window::frames().map(Message::Frame)])
        } else {
            keyboard
        };

        match &self.events_socket {
            // Hosted: also run the duplex command channel, keyed by the socket
            // name so it's created once and kept alive.
            Some(name) => Subscription::batch([
                keyboard,
                Subscription::run_with(name.clone(), command_stream),
            ]),
            None => keyboard,
        }
    }
}

/// The hosted command channel as a [`Stream`] of [`Message`]s: connects to the
/// host socket, hands the event sender back via [`Message::Connected`], then
/// forwards each incoming [`ReaderCommand`] as [`Message::Command`]. The blocking
/// socket read (inside [`ReaderLink`]) is bridged onto the async executor through
/// a forwarding thread so it never blocks the UI.
fn command_stream(socket_name: &String) -> impl Stream<Item = Message> + use<> {
    let socket_name = socket_name.clone();
    iced::stream::channel::<Message>(
        64,
        move |mut output: iced::futures::channel::mpsc::Sender<Message>| async move {
            let ReaderLink { events, commands } = ReaderLink::connect(&socket_name);
            let _ = output
                .send(Message::Connected(EventHandle::new(events)))
                .await;

            let (mut bridge_tx, mut bridge_rx) =
                iced::futures::channel::mpsc::channel::<ReaderCommand>(64);
            std::thread::spawn(move || {
                while let Ok(command) = commands.recv() {
                    if bridge_tx.try_send(command).is_err() {
                        break;
                    }
                }
            });

            while let Some(command) = bridge_rx.next().await {
                if output.send(Message::Command(command)).await.is_err() {
                    break;
                }
            }
        },
    )
}

/// Click/tap zones laid over the page: the left third turns back a page, the
/// right third forward, and the middle third shows (or hides) the bars.
fn touch_zones<'a>() -> Element<'a, Message> {
    let zone = |message| {
        mouse_area(space().width(Fill).height(Fill))
            .on_press(message)
            .interaction(iced::mouse::Interaction::Pointer)
    };

    row![
        zone(Message::Prev),
        mouse_area(space().width(Fill).height(Fill)).on_press(Message::ToggleChrome),
        zone(Message::Next)
    ]
    .width(Fill)
    .height(Fill)
    .into()
}

/// The look shared by the top and bottom bars: a translucent dark band across
/// the window with light text, so the page still shows through faintly.
fn chrome_bar<'a>(content: impl Into<Element<'a, Message>>) -> Element<'a, Message> {
    container(content)
        .width(Fill)
        .style(|_| container::Style {
            background: Some(Color::from_rgba(0.0, 0.0, 0.0, 0.8).into()),
            text_color: Some(Color::WHITE),
            ..container::Style::default()
        })
        .into()
}

/// The popup's note while what lies beyond the `direction` end of the entry
/// isn't known yet.
fn end_message(direction: Direction, noun: &str) -> String {
    match direction {
        Direction::Next => format!("This {noun} ended"),
        Direction::Previous => format!("This {noun} starts here"),
    }
}

/// An offered entry's cover at thumbnail size, or a plain box with its kind
/// when the host had no cover for it.
fn offer_cover(offer: &Offer) -> Element<'_, Message> {
    fn frame<'a>(content: Element<'a, Message>) -> container::Container<'a, Message> {
        container(content)
            .width(POPUP_COVER_WIDTH)
            .height(POPUP_COVER_HEIGHT)
            .center_x(POPUP_COVER_WIDTH)
            .center_y(POPUP_COVER_HEIGHT)
            .clip(true)
    }
    match &offer.cover {
        Some(cover) => frame(
            image(cover.clone())
                .width(Fill)
                .height(Fill)
                .content_fit(ContentFit::Cover)
                .into(),
        )
        .into(),
        None => frame(text(offer.kind.noun()).size(14).into())
            .style(|_| container::Style {
                background: Some(Color::from_rgba(1.0, 1.0, 1.0, 0.1).into()),
                border: Border::default().rounded(6),
                ..container::Style::default()
            })
            .into(),
    }
}

/// A task that reports [`Message::HideChrome`] for the `shown`th showing of the
/// bars once `after` has passed. Sleeps on its own thread, so it needs no async
/// timer from the executor.
fn hide_chrome_after(after: Duration, shown: u64) -> Task<Message> {
    Task::perform(
        async move {
            let (tx, rx) = iced::futures::channel::oneshot::channel();
            std::thread::spawn(move || {
                std::thread::sleep(after);
                let _ = tx.send(());
            });
            let _ = rx.await;
        },
        move |()| Message::HideChrome(shown),
    )
}

/// Maps an ignored keyboard event to a reader message, if it's a key we handle.
fn key_message(event: keyboard::Event) -> Option<Message> {
    let keyboard::Event::KeyPressed { key, .. } = event else {
        return None;
    };

    match key {
        Key::Named(Named::ArrowRight | Named::Space | Named::PageDown) => Some(Message::Next),
        Key::Named(Named::ArrowLeft | Named::Backspace | Named::PageUp) => Some(Message::Prev),
        Key::Named(Named::Home) => Some(Message::First),
        Key::Named(Named::End) => Some(Message::Last),
        Key::Named(Named::Enter) => Some(Message::OpenAdjacent),
        Key::Named(Named::Escape) => Some(Message::DismissPopup),
        Key::Character(c) => match c.as_str() {
            "+" | "=" => Some(Message::ZoomIn),
            "-" | "_" => Some(Message::ZoomOut),
            "0" => Some(Message::ZoomReset),
            "r" | "R" => Some(Message::Rotate),
            "f" | "F" => Some(Message::CycleFit),
            "m" | "M" => Some(Message::ToggleChrome),
            _ => None,
        },
        _ => None,
    }
}

/// Parsed command-line configuration.
#[derive(Clone)]
struct Config {
    dir: PathBuf,
    start_page: usize,
    events: Option<String>,
}

impl Config {
    fn from_args(args: impl IntoIterator<Item = String>) -> Result<Config, String> {
        let mut dir: Option<PathBuf> = None;
        let mut start_page = 0;
        let mut events = None;

        let mut args = args.into_iter();
        while let Some(arg) = args.next() {
            match arg.as_str() {
                "--page" => {
                    let value = args.next().ok_or("--page needs a value")?;
                    start_page = value
                        .parse()
                        .map_err(|_| format!("invalid --page {value:?}"))?;
                }
                "--events" => {
                    events = Some(args.next().ok_or("--events needs a value")?);
                }
                _ if dir.is_none() => dir = Some(PathBuf::from(arg)),
                _ => return Err(format!("unexpected argument {arg:?}")),
            }
        }

        Ok(Config {
            dir: dir.ok_or("usage: komasabi-reader <dir> [--page N] [--events <name>]")?,
            start_page,
            events,
        })
    }
}

/// A full-viewport centered status line (error, empty, loading, …) shown when
/// there is no page image to display.
fn message_screen(message: &str) -> Element<'_, Message> {
    container(text(message).size(16))
        .center_x(Fill)
        .center_y(Fill)
        .into()
}

/// Builds a task that reads and decodes page `index` on a dedicated thread,
/// reporting the result as [`Message::PageLoaded`]. The read and decode run only
/// when the task is polled, and never on the UI thread, so a page flip doesn't
/// block on disk I/O or image decoding.
fn load_page_task(session: u64, index: usize, path: PathBuf) -> Task<Message> {
    Task::perform(
        async move {
            let (tx, rx) = iced::futures::channel::oneshot::channel();
            std::thread::spawn(move || {
                let _ = tx.send(decode_page(&path));
            });
            rx.await.ok().flatten()
        },
        move |handle| Message::PageLoaded {
            session,
            index,
            handle,
        },
    )
}

/// Reads a page file and decodes it to RGBA. Decodes from the file's *bytes*
/// (format detected from content, not from whatever extension the host gave the
/// file) and hands the renderer finished pixels, so no decoding happens while
/// drawing a frame. Returns `None` (logged) if the file can't be read or
/// decoded; the caller shows a placeholder.
fn decode_page(path: &Path) -> Option<image::Handle> {
    let bytes = match std::fs::read(path) {
        Ok(bytes) => bytes,
        Err(error) => {
            eprintln!("reader: could not read {}: {error}", path.display());
            return None;
        }
    };

    match ::image::load_from_memory(&bytes) {
        Ok(decoded) => {
            let rgba = decoded.to_rgba8();
            let (width, height) = rgba.dimensions();
            Some(image::Handle::from_rgba(width, height, rgba.into_raw()))
        }
        Err(error) => {
            eprintln!("reader: could not decode {}: {error}", path.display());
            None
        }
    }
}

/// A bounded most-recently-used cache of decoded page handles, keyed by page
/// index. Cloning a handle is cheap (it shares the underlying pixels and the
/// renderer's texture), so entries are handed out by clone. `front` is the
/// least-recently-used end, `back` the most recent.
struct PageCache {
    entries: VecDeque<(usize, image::Handle)>,
    capacity: usize,
}

impl PageCache {
    fn new(capacity: usize) -> Self {
        Self {
            entries: VecDeque::new(),
            capacity,
        }
    }

    /// Borrows the handle for `index` without affecting recency — safe to call
    /// from `view`, which runs every frame.
    fn peek(&self, index: usize) -> Option<&image::Handle> {
        self.entries
            .iter()
            .find(|(i, _)| *i == index)
            .map(|(_, h)| h)
    }

    fn contains(&self, index: usize) -> bool {
        self.entries.iter().any(|(i, _)| *i == index)
    }

    /// Marks `index` most-recently used so a prefetch of other pages won't evict
    /// it. No-op if it isn't cached yet.
    fn touch(&mut self, index: usize) {
        if let Some(pos) = self.entries.iter().position(|(i, _)| *i == index) {
            let entry = self.entries.remove(pos).expect("position was just found");
            self.entries.push_back(entry);
        }
    }

    /// Inserts (or refreshes) the handle for `index` as most-recent, evicting
    /// least-recently-used pages past capacity.
    fn insert(&mut self, index: usize, handle: image::Handle) {
        if let Some(pos) = self.entries.iter().position(|(i, _)| *i == index) {
            self.entries.remove(pos);
        }
        self.entries.push_back((index, handle));
        while self.entries.len() > self.capacity {
            self.entries.pop_front();
        }
    }
}

/// Lists the image files in `dir` in filename order. The host writes pages as
/// zero-padded names so lexicographic order is reading order.
fn list_pages(dir: &PathBuf) -> Result<Vec<PathBuf>, String> {
    let entries = std::fs::read_dir(dir)
        .map_err(|error| format!("cannot read {}: {error}", dir.display()))?;

    let mut pages: Vec<PathBuf> = entries
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| path.is_file())
        .collect();

    pages.sort();
    Ok(pages)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A standalone reader over `pages` pages, all already on disk.
    fn reader_with(pages: usize) -> Reader {
        let mut reader = Reader::blank(PathBuf::from("."), 0, None);
        reader.pages = (0..pages)
            .map(|i| PathBuf::from(format!("{i:05}.img")))
            .collect();
        reader.ready = (0..pages).collect();
        reader
    }

    /// A hosted reader awaiting its session (no pages until `OpenSession`).
    fn hosted_reader() -> Reader {
        Reader::hosted(Config {
            dir: PathBuf::from("."),
            start_page: 0,
            events: Some("test-socket".to_string()),
        })
    }

    fn fake_handle() -> image::Handle {
        image::Handle::from_rgba(1, 1, vec![0, 0, 0, 255])
    }

    #[test]
    fn next_and_prev_advance_and_clamp_at_the_ends() {
        let mut reader = reader_with(3);

        let _ = reader.update(Message::Next);
        assert_eq!(reader.current, 1);
        let _ = reader.update(Message::Next);
        let _ = reader.update(Message::Next); // past the end, clamps
        assert_eq!(reader.current, 2);

        let _ = reader.update(Message::Prev);
        assert_eq!(reader.current, 1);
        let _ = reader.update(Message::First);
        assert_eq!(reader.current, 0);
        let _ = reader.update(Message::Prev); // before the start, clamps
        assert_eq!(reader.current, 0);

        let _ = reader.update(Message::Last);
        assert_eq!(reader.current, 2);
    }

    /// Clicks the reader's view at `x` (window 900x600) and returns the
    /// messages it emitted.
    fn click_at(reader: &Reader, x: f32) -> Vec<String> {
        let mut ui = iced_test::Simulator::with_size(
            iced::Settings::default(),
            iced::Size::new(900.0, 600.0),
            reader.view(),
        );
        ui.point_at(iced::Point::new(x, 300.0));
        let _ = ui.simulate(iced_test::simulator::click());
        ui.into_messages()
            .map(|message| format!("{message:?}"))
            .collect()
    }

    #[test]
    fn clicking_the_outer_thirds_turns_the_page() {
        let mut reader = reader_with(3);
        reader.current = 1;

        assert_eq!(click_at(&reader, 100.0), ["Prev"]);
        assert_eq!(click_at(&reader, 800.0), ["Next"]);
        assert_eq!(click_at(&reader, 450.0), ["ToggleChrome"]);
    }

    /// Taps the middle of the page and lets the bars finish sliding in.
    fn show_chrome(reader: &mut Reader) {
        let _ = reader.update(Message::ToggleChrome);
        let _ = reader.update(Message::Frame(Instant::now() + CHROME_SLIDE * 2));
    }

    #[test]
    fn the_bars_stay_hidden_until_the_middle_is_tapped() {
        let mut reader = hosted_on_last_page(3);
        assert!(!shows(&reader, "One Piece  —  Volume 5"));
        assert!(!shows(&reader, "3 / 3 (100%)  ·  100%  ·  Fit"));

        show_chrome(&mut reader);

        assert!(shows(&reader, "One Piece  —  Volume 5"));
        assert!(shows(&reader, "3 / 3 (100%)  ·  100%  ·  Fit"));
    }

    #[test]
    fn the_bars_slide_in_from_their_edges() {
        let mut reader = hosted_on_last_page(3);
        let _ = reader.update(Message::ToggleChrome);

        assert!(reader.chrome_visible(), "on screen while sliding in");
        assert!(reader.chrome.is_animating(reader.now));
        let shown: f32 = reader.chrome.interpolate(0.0, 1.0, reader.now);
        assert_eq!(shown, 0.0, "starts fully off screen");
    }

    #[test]
    fn the_bars_hide_again_when_their_timer_runs_out() {
        let mut reader = hosted_on_last_page(3);
        show_chrome(&mut reader);
        let shown = reader.chrome_shown;

        let _ = reader.update(Message::HideChrome(shown));
        let _ = reader.update(Message::Frame(Instant::now() + CHROME_SLIDE * 2));

        assert!(!reader.chrome_visible());
        assert!(!shows(&reader, "One Piece  —  Volume 5"));
    }

    #[test]
    fn tapping_the_middle_again_hides_the_bars_and_cancels_the_timer() {
        let mut reader = reader_with(3);
        show_chrome(&mut reader);
        let first = reader.chrome_shown;

        let _ = reader.update(Message::ToggleChrome);
        assert!(!reader.chrome.value());

        // Shown again; the first showing's timer must not cut this one short.
        show_chrome(&mut reader);
        let _ = reader.update(Message::HideChrome(first));
        assert!(reader.chrome.value());
    }

    #[test]
    fn a_tap_on_a_bar_does_not_turn_the_page() {
        let mut reader = reader_with(3);
        reader.current = 1;
        show_chrome(&mut reader);

        let mut ui = iced_test::Simulator::with_size(
            iced::Settings::default(),
            iced::Size::new(900.0, 600.0),
            reader.view(),
        );
        // The status bar along the bottom edge, over the "next page" zone.
        ui.point_at(iced::Point::new(800.0, 595.0));
        let _ = ui.simulate(iced_test::simulator::click());
        assert_eq!(ui.into_messages().count(), 0);
    }

    #[test]
    fn navigation_on_an_empty_folder_stays_put() {
        let mut reader = reader_with(0);

        let _ = reader.update(Message::Next);
        let _ = reader.update(Message::Last);

        assert_eq!(reader.current, 0);
    }

    #[test]
    fn zoom_clamps_and_resets() {
        let mut reader = reader_with(1);

        for _ in 0..100 {
            let _ = reader.update(Message::ZoomIn);
        }
        assert!(reader.zoom <= MAX_ZOOM);

        let _ = reader.update(Message::ZoomReset);
        assert_eq!(reader.zoom, 1.0);

        for _ in 0..100 {
            let _ = reader.update(Message::ZoomOut);
        }
        assert!(reader.zoom >= MIN_ZOOM);
    }

    #[test]
    fn rotation_wraps_every_four_quarters() {
        let mut reader = reader_with(1);

        for _ in 0..4 {
            let _ = reader.update(Message::Rotate);
        }

        assert_eq!(reader.rotation_quarters, 0);
    }

    #[test]
    fn list_pages_returns_files_in_filename_order() {
        let dir = std::env::temp_dir().join(format!("komasabi-reader-list-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        // Write out of order; the host names pages zero-padded, so lexicographic
        // order is reading order.
        for name in ["00002.img", "00000.img", "00001.img"] {
            std::fs::write(dir.join(name), b"x").unwrap();
        }

        let pages = list_pages(&dir).unwrap();
        let names: Vec<_> = pages
            .iter()
            .map(|p| p.file_name().unwrap().to_str().unwrap().to_string())
            .collect();

        assert_eq!(names, vec!["00000.img", "00001.img", "00002.img"]);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn cache_evicts_least_recently_used_past_capacity() {
        let mut cache = PageCache::new(2);
        cache.insert(0, fake_handle());
        cache.insert(1, fake_handle());
        cache.insert(2, fake_handle());

        assert!(!cache.contains(0), "oldest page should have been evicted");
        assert!(cache.contains(1));
        assert!(cache.contains(2));
    }

    #[test]
    fn touch_protects_a_page_from_eviction() {
        let mut cache = PageCache::new(2);
        cache.insert(0, fake_handle());
        cache.insert(1, fake_handle());
        cache.touch(0); // 0 is now most-recent, 1 is oldest
        cache.insert(2, fake_handle());

        assert!(cache.contains(0), "touched page should survive");
        assert!(!cache.contains(1), "untouched older page should be evicted");
        assert!(cache.contains(2));
    }

    #[test]
    fn request_skips_out_of_range_cached_and_inflight_pages() {
        let mut reader = reader_with(3);

        assert!(reader.request(5).is_none(), "out of range");

        assert!(reader.request(0).is_some(), "first request dispatches");
        assert!(reader.inflight.contains(&0));
        assert!(reader.request(0).is_none(), "already in flight");

        reader.inflight.remove(&0);
        reader.cache.insert(0, fake_handle());
        assert!(reader.request(0).is_none(), "already cached");
    }

    #[test]
    fn page_loaded_caches_the_handle_and_clears_inflight() {
        let mut reader = reader_with(3);
        reader.inflight.insert(1);

        let _ = reader.update(Message::PageLoaded {
            session: reader.session,
            index: 1,
            handle: Some(fake_handle()),
        });

        assert!(reader.cache.contains(1));
        assert!(!reader.inflight.contains(&1));
    }

    #[test]
    fn page_loaded_failure_clears_inflight_without_caching() {
        let mut reader = reader_with(3);
        reader.inflight.insert(1);

        let _ = reader.update(Message::PageLoaded {
            session: reader.session,
            index: 1,
            handle: None,
        });

        assert!(!reader.cache.contains(1));
        assert!(!reader.inflight.contains(&1));
    }

    #[test]
    fn request_skips_pages_not_yet_on_disk() {
        let mut reader = reader_with(3);
        reader.ready.remove(&1); // host hasn't downloaded page 1 yet

        assert!(reader.request(1).is_none(), "not on disk yet");
        assert!(!reader.inflight.contains(&1));
        assert!(reader.request(2).is_some(), "a ready page still dispatches");
    }

    #[test]
    fn open_session_sizes_the_session_and_sets_metadata() {
        let mut reader = hosted_reader();
        reader.start_page = 2;
        assert!(reader.awaiting_session);

        let _ = reader.update(Message::Command(ReaderCommand::OpenSession {
            series: "One Piece".to_string(),
            entry: "Volume 5".to_string(),
            pages: 5,
            kind: None,
            dir: None,
            start_page: 0,
        }));

        assert_eq!(reader.pages.len(), 5);
        assert_eq!(reader.series.as_deref(), Some("One Piece"));
        assert_eq!(reader.entry.as_deref(), Some("Volume 5"));
        assert_eq!(reader.current, 2, "opens on the clamped start page");
        assert!(!reader.awaiting_session);
    }

    /// A hosted reader with an open `pages`-page session of a volume, sitting
    /// on its last page.
    fn hosted_on_last_page(pages: usize) -> Reader {
        let mut reader = hosted_reader();
        let _ = reader.update(Message::Command(ReaderCommand::OpenSession {
            series: "One Piece".to_string(),
            entry: "Volume 5".to_string(),
            pages,
            kind: Some(EntryKind::Volume),
            dir: None,
            start_page: 0,
        }));
        let _ = reader.update(Message::Last);
        reader
    }

    /// Renders `reader` and reports whether `label` is on screen.
    fn shows(reader: &Reader, label: &str) -> bool {
        let mut ui = iced_test::Simulator::with_size(
            iced::Settings::default(),
            iced::Size::new(900.0, 600.0),
            reader.view(),
        );
        ui.find(label).is_ok()
    }

    fn offer(direction: Direction, label: &str) -> Message {
        Message::Command(ReaderCommand::Adjacent {
            direction,
            entry: Some(AdjacentEntry {
                label: label.to_string(),
                kind: EntryKind::Volume,
                cover: None,
            }),
        })
    }

    #[test]
    fn turning_past_the_last_page_pops_up_the_next_entry() {
        let mut reader = hosted_on_last_page(3);
        assert!(reader.popup().is_none(), "no popup just for being there");

        let _ = reader.update(Message::Next);
        assert_eq!(reader.current, 2);
        assert!(shows(&reader, "This volume ended"));
        assert!(shows(&reader, "Looking for what's next…"));

        let _ = reader.update(offer(Direction::Next, "Volume 6"));
        assert!(shows(&reader, "This volume ended, next one:"));
        assert!(shows(&reader, "Volume 6"));
    }

    #[test]
    fn the_popup_says_when_there_is_no_next_entry() {
        let mut reader = hosted_on_last_page(3);
        let _ = reader.update(Message::Command(ReaderCommand::Adjacent {
            direction: Direction::Next,
            entry: None,
        }));
        let _ = reader.update(Message::Next);

        assert!(shows(
            &reader,
            "This volume ended, there is no next volume."
        ));
    }

    #[test]
    fn turning_back_past_the_first_page_pops_up_the_previous_entry() {
        let mut reader = hosted_on_last_page(3);
        let _ = reader.update(Message::First);
        let _ = reader.update(offer(Direction::Previous, "Volume 4"));

        let _ = reader.update(Message::Prev);
        assert_eq!(reader.popup, Some(Direction::Previous));
        assert!(shows(&reader, "This volume starts here, previous one:"));
        assert!(shows(&reader, "Volume 4"));

        let _ = reader.update(Message::Command(ReaderCommand::Adjacent {
            direction: Direction::Previous,
            entry: None,
        }));
        assert!(shows(
            &reader,
            "This is the first volume, there is no previous volume."
        ));
    }

    #[test]
    fn a_turn_the_other_way_or_escape_closes_the_popup() {
        let mut reader = hosted_on_last_page(3);
        let _ = reader.update(Message::Next);
        let _ = reader.update(Message::Prev);
        assert_eq!(reader.popup, None);
        assert_eq!(reader.current, 2, "closing the popup doesn't turn the page");

        let _ = reader.update(Message::Next);
        let _ = reader.update(Message::DismissPopup);
        assert_eq!(reader.popup, None);
    }

    #[test]
    fn a_click_on_the_backdrop_closes_the_popup_but_one_on_the_cover_opens_it() {
        let mut reader = hosted_on_last_page(3);
        let _ = reader.update(offer(Direction::Next, "Volume 6"));
        let _ = reader.update(Message::Next);

        let mut ui = iced_test::Simulator::with_size(
            iced::Settings::default(),
            iced::Size::new(900.0, 600.0),
            reader.view(),
        );
        // The backdrop, over the "next page" zone.
        ui.point_at(iced::Point::new(880.0, 300.0));
        let _ = ui.simulate(iced_test::simulator::click());
        let _ = ui.click("Volume 6");
        let messages: Vec<_> = ui
            .into_messages()
            .map(|message| format!("{message:?}"))
            .collect();
        assert_eq!(messages, ["DismissPopup"], "the label isn't the button");

        // The middle of the window is the cover.
        let mut ui = iced_test::Simulator::with_size(
            iced::Settings::default(),
            iced::Size::new(900.0, 600.0),
            reader.view(),
        );
        ui.point_at(iced::Point::new(450.0, 300.0));
        let _ = ui.simulate(iced_test::simulator::click());
        let messages: Vec<_> = ui
            .into_messages()
            .map(|message| format!("{message:?}"))
            .collect();
        assert_eq!(messages, ["OpenAdjacent"]);
    }

    #[test]
    fn standalone_reader_has_no_popup_at_either_end() {
        let mut reader = reader_with(2);
        let _ = reader.update(Message::Prev);
        assert_eq!(reader.popup, None);
        let _ = reader.update(Message::Last);
        let _ = reader.update(Message::Next);
        assert_eq!(reader.popup, None);
    }

    #[test]
    fn a_second_turn_opens_only_an_offered_entry() {
        let mut reader = hosted_on_last_page(3);
        let _ = reader.update(Message::Next);
        let _ = reader.update(Message::Next);
        assert_eq!(reader.opening, None, "nothing offered yet");

        let _ = reader.update(offer(Direction::Next, "Volume 6"));
        let _ = reader.update(Message::Next);
        assert_eq!(reader.opening, Some(Direction::Next));

        let _ = reader.update(Message::Prev);
        assert_eq!(reader.current, 2, "no page turns while the next one opens");
        assert_eq!(reader.popup, Some(Direction::Next), "and the popup stays");
        assert!(shows(&reader, "Opening Volume 6…"));

        // The host couldn't open it, and offers it again.
        let _ = reader.update(offer(Direction::Next, "Volume 6"));
        assert_eq!(reader.opening, None);
    }

    #[test]
    fn a_previous_entry_opens_on_its_last_page() {
        let mut reader = hosted_on_last_page(3);
        let _ = reader.update(Message::Command(ReaderCommand::OpenSession {
            series: "One Piece".to_string(),
            entry: "Volume 4".to_string(),
            pages: 6,
            kind: Some(EntryKind::Volume),
            dir: Some(PathBuf::from("previous-volume")),
            start_page: 5,
        }));
        assert_eq!(reader.current, 5);
        assert_eq!(reader.popup, None);
    }

    #[test]
    fn a_new_session_switches_folder_and_starts_over() {
        let mut reader = hosted_on_last_page(3);
        reader.ready.insert(2);
        reader.cache.insert(2, fake_handle());
        let old_session = reader.session;

        let _ = reader.update(Message::Command(ReaderCommand::OpenSession {
            series: "One Piece".to_string(),
            entry: "Volume 6".to_string(),
            pages: 4,
            kind: Some(EntryKind::Volume),
            dir: Some(PathBuf::from("next-volume")),
            start_page: 0,
        }));

        assert_eq!(reader.dir, PathBuf::from("next-volume"));
        assert_eq!(reader.pages[0], PathBuf::from("next-volume/00000.img"));
        assert_eq!(reader.current, 0);
        assert_eq!(reader.entry.as_deref(), Some("Volume 6"));
        assert!(reader.ready.is_empty());
        assert!(!reader.cache.contains(2));
        assert_eq!(reader.next, Adjacent::Unknown);
        assert_eq!(reader.previous, Adjacent::Unknown);

        // A load of the previous entry's page that lands now is dropped.
        let _ = reader.update(Message::PageLoaded {
            session: old_session,
            index: 1,
            handle: Some(fake_handle()),
        });
        assert!(!reader.cache.contains(1));
    }

    #[test]
    fn page_ready_marks_the_page_and_makes_it_loadable() {
        let mut reader = hosted_reader();
        let _ = reader.update(Message::Command(ReaderCommand::OpenSession {
            series: "S".to_string(),
            entry: "E".to_string(),
            pages: 5,
            kind: None,
            dir: None,
            start_page: 0,
        }));
        // Nothing is on disk yet, so the opening page can't load.
        assert!(reader.ready.is_empty());
        assert!(!reader.inflight.contains(&0));

        let _ = reader.update(Message::Command(ReaderCommand::PageReady { index: 0 }));

        assert!(reader.ready.contains(&0));
        // Page 0 is the current page (wanted), so its load was dispatched.
        assert!(reader.inflight.contains(&0));
    }
}
