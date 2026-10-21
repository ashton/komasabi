//! Standalone comic reader: given a folder of images, display them one page at
//! a time with navigation, zoom, rotation and fit controls. It has no knowledge
//! of where the images came from — a host process (the Komasabi app) downloads a
//! chapter's pages into a folder and launches this binary pointed at it.
//!
//! Usage: `komasabi-reader <dir> [--page N] [--events <socket-name>]`
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

use iced::futures::{SinkExt, Stream, StreamExt};
use iced::keyboard::{self, Key, key::Named};
use iced::widget::{column, container, image, text};
use iced::{ContentFit, Element, Length::Fill, Radians, Rotation, Subscription, Task};

use komasabi_reader::{EventSender, ReaderCommand, ReaderEvent, ReaderLink};

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
const APP_ICON: &[u8] = include_bytes!("../assets/komasabi-icon.png");

fn main() -> iced::Result {
    iced::application(Reader::new, Reader::update, Reader::view)
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
    /// A background load for page `index` finished; `None` if it couldn't be
    /// read or decoded.
    PageLoaded {
        index: usize,
        handle: Option<image::Handle>,
    },
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
    fn new() -> (Self, Task<Message>) {
        match Config::from_args() {
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
            awaiting_session: false,
            zoom: 1.0,
            rotation_quarters: 0,
            fit: Fit::Contain,
            events: EventHandle::disabled(),
            events_socket,
            error: None,
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
            Message::Next => self.go_to(self.current + 1),
            Message::Prev => self.go_to(self.current.saturating_sub(1)),
            Message::First => self.go_to(0),
            Message::Last => self.go_to(self.pages.len().saturating_sub(1)),
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
            Message::PageLoaded { index, handle } => {
                self.inflight.remove(&index);
                if let Some(handle) = handle {
                    self.cache.insert(index, handle);
                }
                Task::none()
            }
            Message::Connected(events) => {
                self.events = events;
                Task::none()
            }
            Message::Command(command) => self.handle_command(command),
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
            } => {
                self.series = Some(series);
                self.entry = Some(entry);
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
        if self.pages.is_empty() {
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
        Some(load_page_task(index, self.pages[index].clone()))
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

        let mut layout = column![].width(Fill).height(Fill);
        if let Some(header) = self.header() {
            layout = layout.push(header);
        }
        layout.push(page).push(self.status_bar()).into()
    }

    /// The session title line (`series — entry`), shown when the host has told
    /// us what we're reading. Absent in standalone mode.
    fn header(&self) -> Option<Element<'_, Message>> {
        let title = match (&self.series, &self.entry) {
            (Some(series), Some(entry)) => format!("{series}  —  {entry}"),
            (Some(part), None) | (None, Some(part)) => part.clone(),
            (None, None) => return None,
        };
        Some(container(text(title).size(14)).padding(8).into())
    }

    /// A thin, non-interactive status line. Navigation is keyboard-only for now
    /// (see [`key_message`]); this reflects reading position (page and progress)
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
        Key::Character(c) => match c.as_str() {
            "+" | "=" => Some(Message::ZoomIn),
            "-" | "_" => Some(Message::ZoomOut),
            "0" => Some(Message::ZoomReset),
            "r" | "R" => Some(Message::Rotate),
            "f" | "F" => Some(Message::CycleFit),
            _ => None,
        },
        _ => None,
    }
}

/// Parsed command-line configuration.
struct Config {
    dir: PathBuf,
    start_page: usize,
    events: Option<String>,
}

impl Config {
    fn from_args() -> Result<Config, String> {
        let mut dir: Option<PathBuf> = None;
        let mut start_page = 0;
        let mut events = None;

        let mut args = std::env::args().skip(1);
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
fn load_page_task(index: usize, path: PathBuf) -> Task<Message> {
    Task::perform(
        async move {
            let (tx, rx) = iced::futures::channel::oneshot::channel();
            std::thread::spawn(move || {
                let _ = tx.send(decode_page(&path));
            });
            rx.await.ok().flatten()
        },
        move |handle| Message::PageLoaded { index, handle },
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
        }));

        assert_eq!(reader.pages.len(), 5);
        assert_eq!(reader.series.as_deref(), Some("One Piece"));
        assert_eq!(reader.entry.as_deref(), Some("Volume 5"));
        assert_eq!(reader.current, 2, "opens on the clamped start page");
        assert!(!reader.awaiting_session);
    }

    #[test]
    fn page_ready_marks_the_page_and_makes_it_loadable() {
        let mut reader = hosted_reader();
        let _ = reader.update(Message::Command(ReaderCommand::OpenSession {
            series: "S".to_string(),
            entry: "E".to_string(),
            pages: 5,
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
