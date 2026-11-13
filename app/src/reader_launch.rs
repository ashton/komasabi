//! Opening a [`SeriesEntry`] in the standalone reader.
//!
//! The reader is a separate process that displays page images from a folder:
//! this same executable, re-launched with the [`READER_SUBCOMMAND`].
//! This module bridges the gap and then *streams* into it: it resolves the
//! entry to a cache folder, launches the reader pointed at that folder straight
//! away (so a window appears without waiting on any download), and hands the
//! session to a background worker. The worker, on its own Tokio runtime, opens a
//! duplex socket to the reader — sending it the session metadata
//! ([`ReaderCommand::OpenSession`]) and a [`ReaderCommand::PageReady`] as each
//! page lands on disk — while downloading pages in the order the reader actually
//! needs them, reprioritising around the page it reports it's showing
//! ([`ReaderEvent::ShowingPage`]). Every page the reader shows is also passed
//! on to [`reading_progress`], which saves it to the provider's reading history.
//!
//! When the reader reaches the entry's last page, the worker asks the provider
//! for the entry after it ([`Provider::next_entry`]), and on its first page for
//! the one before it ([`Provider::previous_entry`]), and tells the reader
//! ([`ReaderCommand::Adjacent`]) along with a file holding that entry's cover.
//! The reader shows it when the user turns past that end. If the user takes it
//! up ([`ReaderEvent::OpenAdjacent`]), the same reader window moves on to that
//! entry: the worker opens a new session in it, with its own page folder (from
//! its last page, going back), and streams its pages the same way.
//!
//! A downloaded entry (see [`downloads`]) isn't streamed: the reader is pointed
//! at its download folder, whose pages are all there already, and its pages are
//! the ones saved with it, so the source isn't asked for any. It's read even
//! when the source can't be reached, only without the source's reading history
//! and adjacent entries.

use std::cell::RefCell;
use std::collections::BTreeSet;
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};

use interprocess::TryClone;
use interprocess::local_socket::{
    GenericNamespaced, Listener, ListenerOptions, Stream, prelude::*,
};
use tokio::sync::mpsc::{self, UnboundedReceiver, UnboundedSender};
use uuid::Uuid;

use komasabi_reader::{AdjacentEntry, Direction, ReaderCommand, ReaderEvent};

use crate::cache;
use crate::downloads::{self, DownloadedEntry};
use crate::models::library_item::LibraryItem;
use crate::models::series_entry::{EntryKind, SeriesEntry};
use crate::providers::{PageRef, Provider, provider_for_integration};
use crate::reading_progress;
use crate::repository::library as library_repository;

/// The first argument that makes `main` run the reader (`komasabi read <dir> …`)
/// instead of the library window. Shipping the reader inside the app binary
/// keeps `cargo install komasabi` a single, self-contained install.
pub const READER_SUBCOMMAND: &str = "read";

/// Everything the background session worker needs to serve one reader. Moved
/// onto its own thread, so it must be `Send + 'static`.
struct SessionSpec {
    /// The source (integration) the entry is read from.
    integration_id: String,
    /// The series' provider-side id, to look up the entry after the one read.
    series_id: String,
    series: String,
    entry: SeriesEntry,
    dir: PathBuf,
    /// The page (0-based) to open the entry on.
    start_page: usize,
    /// The entry's download, when it's downloaded: `dir` is then its folder.
    local: Option<DownloadedEntry>,
    /// Told once the reader has closed, when someone wants to know.
    closed: Option<tokio::sync::oneshot::Sender<()>>,
}

/// Resolves `entry` to its cache folder, launches the reader on it immediately,
/// and spawns the background worker that streams pages into it. Returns as soon
/// as the reader is launched — downloading happens in the worker, not here.
pub async fn prepare_and_launch(
    integration_id: String,
    series_external_id: String,
    series_name: String,
    entry: SeriesEntry,
) -> Result<(), String> {
    let local = downloads::local_entry(&integration_id, &series_external_id, &entry);
    let dir = reading_dir(&integration_id, &entry, local.as_ref())?;

    let socket_name = bind_and_serve(SessionSpec {
        integration_id,
        series_id: series_external_id,
        series: series_name,
        entry,
        dir: dir.clone(),
        start_page: 0,
        local,
        closed: None,
    })?;

    spawn_reader(&dir, &socket_name)
}

/// Opens an item of the user's Library in the reader. Library items aren't
/// downloaded, so it's read from its source like any of the source's entries
/// (whose pages it shows are then saved as the item's progress), but starting
/// where the user left off. The returned receiver resolves once the reader
/// has closed, so the progress shown can be reloaded.
pub fn launch_library_item(
    item: LibraryItem,
) -> Result<tokio::sync::oneshot::Receiver<()>, String> {
    let entry = item.as_entry();
    let local = downloads::local_entry(&item.integration_id, &item.series_external_id, &entry);
    let dir = reading_dir(&item.integration_id, &entry, local.as_ref())?;
    let (done, closed) = tokio::sync::oneshot::channel();

    let socket_name = bind_and_serve(SessionSpec {
        integration_id: item.integration_id.clone(),
        series_id: item.series_external_id.clone(),
        series: item.series_name.clone(),
        start_page: resume_page(item.pages_read, item.pages),
        entry,
        dir: dir.clone(),
        local,
        closed: Some(done),
    })?;

    spawn_reader(&dir, &socket_name)?;
    Ok(closed)
}

/// The folder the reader shows `entry`'s pages from: its download's, when
/// it's downloaded, otherwise the cache folder they're streamed into.
fn reading_dir(
    namespace: &str,
    entry: &SeriesEntry,
    local: Option<&DownloadedEntry>,
) -> Result<PathBuf, String> {
    match local {
        Some(local) => Ok(local.dir.clone()),
        // A source's pages are cached under its id (see
        // `providers::provider_for_integration`).
        None => entry_dir(namespace, entry),
    }
}

/// Creates (if need be) and returns the cache folder `entry`'s pages are
/// downloaded into, under the source's cache `namespace`.
fn entry_dir(namespace: &str, entry: &SeriesEntry) -> Result<PathBuf, String> {
    let dir = cache::pages_dir(&format!("{namespace}-{}", entry.cover_key()))
        .ok_or("no cache directory available for reader pages")?;
    std::fs::create_dir_all(&dir)
        .map_err(|error| format!("could not create {}: {error}", dir.display()))?;
    Ok(dir)
}

/// Binds a uniquely-named local socket and spawns the worker thread that accepts
/// the reader's connection and serves the session over it. Returns the socket
/// name to hand to the reader. Bound before the reader is launched so the reader
/// can't race the listener.
fn bind_and_serve(spec: SessionSpec) -> Result<String, String> {
    let socket_name = format!("komasabi-reader-{}.sock", Uuid::new_v4());
    let name = socket_name
        .as_str()
        .to_ns_name::<GenericNamespaced>()
        .map_err(|error| format!("invalid event socket name: {error}"))?;

    let listener = ListenerOptions::new()
        .name(name)
        .create_sync()
        .map_err(|error| format!("could not bind event socket: {error}"))?;

    std::thread::spawn(move || serve_session(listener, spec));

    Ok(socket_name)
}

/// The page to reopen a Library item on: the last one read, or the first once
/// it's read through (or not started). Without a page count, a read-through
/// item can't be told apart, so it starts over too.
fn resume_page(pages_read: i32, pages: Option<i32>) -> usize {
    match pages {
        Some(pages) if pages_read > 0 && pages_read < pages => (pages_read - 1) as usize,
        _ => 0,
    }
}

/// Saves that page `page` (0-based) of an entry's `pages` was shown, if the
/// entry is in the Library. Failures are only logged.
async fn record_library_progress(
    integration_id: &str,
    kind: EntryKind,
    entry_id: &str,
    page: usize,
    pages: usize,
) {
    let read = i32::try_from(page + 1).unwrap_or(i32::MAX);
    let total = i32::try_from(pages).unwrap_or(i32::MAX);
    if let Err(error) =
        library_repository::record_progress(integration_id, kind, entry_id, read, total).await
    {
        tracing::debug!("could not save library progress: {error}");
    }
}

/// The worker thread: runs the session to completion on a dedicated
/// single-threaded Tokio runtime (so its `reqwest`-based downloads have a
/// reactor and never touch iced's executor). Errors just end the session; the
/// reader keeps whatever pages already arrived.
fn serve_session(listener: Listener, mut spec: SessionSpec) {
    // Dropped (or sent) whichever way the session ends, which tells the
    // receiver the reader has closed.
    let closed = spec.closed.take();
    let runtime = match tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
    {
        Ok(runtime) => runtime,
        Err(error) => {
            tracing::debug!("could not build reader session runtime: {error}");
            return;
        }
    };

    if let Err(error) = runtime.block_on(run_session(listener, spec)) {
        tracing::debug!("reader session ended: {error}");
    }
    if let Some(closed) = closed {
        let _ = closed.send(());
    }
}

/// Authenticates the provider, resolves the entry's pages, accepts the reader,
/// and serves it the entry (see [`serve_entry`]). When the user moves on to the
/// next entry, the same reader is handed a new session for it, until the reader
/// closes.
async fn run_session(listener: Listener, spec: SessionSpec) -> Result<(), String> {
    match provider_for_integration(&spec.integration_id).await {
        Ok((provider, namespace)) => run_session_with(&provider, namespace, listener, spec).await,
        // A downloaded entry needs nothing from its source to be read.
        Err(error) if spec.local.is_some() => {
            tracing::warn!("reading a download offline, as its source failed: {error}");
            let namespace = spec.integration_id.clone();
            run_session_with(&Offline(error), namespace, listener, spec).await
        }
        Err(error) => Err(error),
    }
}

/// [`run_session`], with the source's `provider` resolved (or standing in for
/// it, see [`Offline`]).
async fn run_session_with(
    provider: &impl Provider,
    namespace: String,
    listener: Listener,
    spec: SessionSpec,
) -> Result<(), String> {
    let mut entry = spec.entry;
    let mut dir = spec.dir;
    let mut plan = match spec.local {
        Some(local) => local.pages,
        None => provider.reader_pages(&entry).await?,
    };
    if plan.is_empty() {
        return Err("this entry has no pages to read".to_string());
    }

    let stream = listener
        .accept()
        .map_err(|error| format!("reader never connected: {error}"))?;
    let read_half = stream
        .try_clone()
        .map_err(|error| format!("could not split the reader socket: {error}"))?;
    let (events_tx, mut events) = mpsc::unbounded_channel();
    std::thread::spawn(move || forward_events(read_half, events_tx));
    let stream = RefCell::new(stream);

    // How to announce the next `OpenSession`: no folder for the first entry,
    // whose folder the reader was launched on, then each adjacent entry's
    // folder and the page to start it on.
    let mut open_session = Some((None, spec.start_page.min(plan.len() - 1)));
    loop {
        if let Some((new_dir, start_page)) = open_session.take() {
            send_command(
                &stream,
                &ReaderCommand::OpenSession {
                    series: spec.series.clone(),
                    entry: entry.label.clone(),
                    pages: plan.len(),
                    kind: Some(reader_kind(entry.kind)),
                    dir: new_dir,
                    start_page,
                },
            )
            .map_err(|error| format!("could not open the reader session: {error}"))?;
        }

        let in_library = library_repository::contains(&namespace, entry.kind, &entry.external_id)
            .await
            .unwrap_or(false);
        let session = EntrySession {
            series_id: &spec.series_id,
            namespace: &namespace,
            in_library,
            entry: &entry,
            plan: &plan,
            dir: &dir,
            stream: &stream,
        };
        let Some(chosen) = serve_entry(provider, session, &mut events).await else {
            return Ok(());
        };

        match prepare_entry(provider, &namespace, &spec.series_id, &chosen.entry).await {
            Ok((new_dir, new_plan)) => {
                // Going back, carry on from the end of the entry before.
                let start_page = match chosen.direction {
                    Direction::Next => 0,
                    Direction::Previous => new_plan.len() - 1,
                };
                open_session = Some((Some(new_dir.clone()), start_page));
                entry = chosen.entry;
                dir = new_dir;
                plan = new_plan;
            }
            Err(error) => {
                // Stay on the current entry, and offer that one again so the
                // user can retry.
                tracing::warn!("could not open {}: {error}", chosen.entry.label);
                send_command(
                    &stream,
                    &ReaderCommand::Adjacent {
                        direction: chosen.direction,
                        entry: Some(chosen.offer),
                    },
                )
                .map_err(|error| format!("reader went away: {error}"))?;
            }
        }
    }
}

/// Stands in for a source that can't be reached while a downloaded entry is
/// read: every request fails with why the source couldn't be resolved, so the
/// reader just isn't offered adjacent entries, and no reading history is sent.
struct Offline(String);

impl Provider for Offline {
    async fn list_collections(&self) -> Result<Vec<crate::models::collection::Collection>, String> {
        Err(self.0.clone())
    }
    async fn list_items(
        &self,
        _: &str,
        _: Option<u32>,
        _: Option<u32>,
    ) -> Result<Vec<crate::models::collection_item::CollectionItem>, String> {
        Err(self.0.clone())
    }
    async fn list_series_entries(&self, _: &str) -> Result<Vec<SeriesEntry>, String> {
        Err(self.0.clone())
    }
    async fn cover(&self, _: &str, _: &str) -> Result<Vec<u8>, String> {
        Err(self.0.clone())
    }
    async fn volume_cover(&self, _: &str, _: &str) -> Result<Vec<u8>, String> {
        Err(self.0.clone())
    }
    async fn chapter_cover(&self, _: &str, _: &str) -> Result<Vec<u8>, String> {
        Err(self.0.clone())
    }
    async fn reader_pages(&self, _: &SeriesEntry) -> Result<Vec<PageRef>, String> {
        Err(self.0.clone())
    }
    async fn reader_image(&self, _: &str, _: u16) -> Result<Vec<u8>, String> {
        Err(self.0.clone())
    }
    async fn next_entry(&self, _: &str, _: &SeriesEntry) -> Result<Option<SeriesEntry>, String> {
        Err(self.0.clone())
    }
    async fn previous_entry(
        &self,
        _: &str,
        _: &SeriesEntry,
    ) -> Result<Option<SeriesEntry>, String> {
        Err(self.0.clone())
    }
}

/// The reader's name for an entry's kind.
fn reader_kind(kind: EntryKind) -> komasabi_reader::EntryKind {
    match kind {
        EntryKind::Volume => komasabi_reader::EntryKind::Volume,
        EntryKind::Chapter => komasabi_reader::EntryKind::Chapter,
    }
}

/// The page folder and pages of `entry`, of the series `series_id`, ready for
/// a new reader session: its download's, when it's downloaded.
async fn prepare_entry(
    provider: &impl Provider,
    namespace: &str,
    series_id: &str,
    entry: &SeriesEntry,
) -> Result<(PathBuf, Vec<PageRef>), String> {
    if let Some(local) = downloads::local_entry(namespace, series_id, entry) {
        return Ok((local.dir, local.pages));
    }
    let dir = entry_dir(namespace, entry)?;
    let plan = provider.reader_pages(entry).await?;
    if plan.is_empty() {
        return Err("this entry has no pages to read".to_string());
    }
    Ok((dir, plan))
}

/// The entry a reader session is showing, and where its pages go.
struct EntrySession<'a> {
    series_id: &'a str,
    /// The provider's cache namespace, to fetch adjacent entries' covers with.
    /// It's the source's id.
    namespace: &'a str,
    /// Whether the entry is also in the user's Library, whose progress then
    /// follows the pages read here.
    in_library: bool,
    entry: &'a SeriesEntry,
    plan: &'a [PageRef],
    dir: &'a Path,
    stream: &'a RefCell<Stream>,
}

/// An adjacent entry the user chose to read next.
#[derive(Debug, PartialEq)]
struct Chosen {
    direction: Direction,
    entry: SeriesEntry,
    /// How it was offered to the reader, to offer again if it can't be opened.
    offer: AdjacentEntry,
}

/// What lies on each side of the entry being served, once looked up:
/// `Some(None)` when nothing does.
#[derive(Default)]
struct Adjacents {
    next: Option<Option<(SeriesEntry, AdjacentEntry)>>,
    previous: Option<Option<(SeriesEntry, AdjacentEntry)>>,
}

impl Adjacents {
    fn get_mut(
        &mut self,
        direction: Direction,
    ) -> &mut Option<Option<(SeriesEntry, AdjacentEntry)>> {
        match direction {
            Direction::Next => &mut self.next,
            Direction::Previous => &mut self.previous,
        }
    }
}

/// Serves one entry to the reader, already told about it with `OpenSession`:
/// downloads its pages in reader-driven priority order, reports the pages the
/// reader shows to the provider's reading history, and, once the reader reaches
/// the last (or first) page, looks up the entry after (or before) it and offers
/// it to the reader.
///
/// Returns the adjacent entry the user asked to read, or `None` once the reader
/// has closed.
async fn serve_entry(
    provider: &impl Provider,
    session: EntrySession<'_>,
    events: &mut UnboundedReceiver<ReaderEvent>,
) -> Option<Chosen> {
    let current = AtomicUsize::new(0);
    let (shown_pages, shown_pages_rx) = mpsc::unbounded_channel();

    let dispatch = async {
        // Moved in, so it's dropped (ending the progress reporting) as soon as
        // this entry's session is over.
        let shown_pages = shown_pages;
        let mut adjacents = Adjacents::default();

        while let Some(event) = events.recv().await {
            match event {
                ReaderEvent::ShowingPage { page } => {
                    current.store(page, Ordering::Relaxed);
                    let _ = shown_pages.send(page);
                    // Reading the source's copy also moves the Library's on.
                    if session.in_library {
                        record_library_progress(
                            session.namespace,
                            session.entry.kind,
                            &session.entry.external_id,
                            page,
                            session.plan.len(),
                        )
                        .await;
                    }

                    let mut ends = Vec::new();
                    if page == 0 {
                        ends.push(Direction::Previous);
                    }
                    if page + 1 >= session.plan.len() {
                        ends.push(Direction::Next);
                    }
                    for direction in ends {
                        let adjacent = adjacents.get_mut(direction);
                        if adjacent.is_none() {
                            *adjacent = look_up_adjacent(provider, &session, direction).await;
                        }
                    }
                }
                ReaderEvent::OpenAdjacent { direction } => {
                    if let Some(Some((entry, offer))) = adjacents.get_mut(direction) {
                        return Some(Chosen {
                            direction,
                            entry: entry.clone(),
                            offer: offer.clone(),
                        });
                    }
                }
                ReaderEvent::ReadAllFiles { .. } => {}
            }
        }
        None
    };

    let downloads = async {
        download_session(
            provider,
            session.plan,
            session.dir,
            session.stream,
            &current,
        )
        .await;
        // Done downloading; the session still runs until the reader moves on
        // or closes.
        std::future::pending::<()>().await
    };

    let ((), next) = tokio::join!(
        reading_progress::report_progress(provider, session.plan, shown_pages_rx),
        async {
            tokio::select! {
                next = dispatch => next,
                () = downloads => None,
            }
        },
    );
    next
}

/// Asks the provider what lies on the `direction` side of the session's entry,
/// saves that entry's cover for the reader to show, and tells the reader.
/// Returns the answer to remember, or `None` if the lookup failed, so it's
/// tried again the next time the reader lands on that end.
async fn look_up_adjacent(
    provider: &impl Provider,
    session: &EntrySession<'_>,
    direction: Direction,
) -> Option<Option<(SeriesEntry, AdjacentEntry)>> {
    let found = match direction {
        Direction::Next => provider.next_entry(session.series_id, session.entry).await,
        Direction::Previous => {
            provider
                .previous_entry(session.series_id, session.entry)
                .await
        }
    };
    let adjacent = match found {
        Ok(Some(entry)) => {
            let offer = AdjacentEntry {
                label: entry.label.clone(),
                kind: reader_kind(entry.kind),
                cover: cover_file(provider, session.namespace, &entry).await,
            };
            Some(Some((entry, offer)))
        }
        Ok(None) => Some(None),
        Err(error) => {
            tracing::warn!(
                "could not look up the entry {direction:?} of {}: {error}",
                session.entry.label
            );
            None
        }
    };

    let offer = adjacent
        .as_ref()
        .and_then(|adjacent| adjacent.as_ref())
        .map(|(_, offer)| offer.clone());
    if let Err(error) = send_command(
        session.stream,
        &ReaderCommand::Adjacent {
            direction,
            entry: offer,
        },
    ) {
        tracing::debug!("could not offer the adjacent entry: {error}");
    }
    adjacent
}

/// Fetches `entry`'s cover and saves it in the cover cache, returning the file
/// for the reader to load, or `None` when there's no cover to be had (the reader
/// then shows a placeholder).
async fn cover_file(
    provider: &impl Provider,
    namespace: &str,
    entry: &SeriesEntry,
) -> Option<PathBuf> {
    let bytes = match entry.kind {
        EntryKind::Volume => provider.volume_cover(namespace, &entry.external_id).await,
        EntryKind::Chapter => provider.chapter_cover(namespace, &entry.external_id).await,
    };
    let bytes = match bytes {
        Ok(bytes) => bytes,
        Err(error) => {
            tracing::debug!("no cover for {}: {error}", entry.label);
            return None;
        }
    };

    let key = format!("reader-{namespace}-{}", entry.cover_key());
    match cache::write_cover(&key, &bytes) {
        Ok(()) => cache::cover_path(&key),
        Err(error) => {
            tracing::debug!("could not save the cover of {}: {error}", entry.label);
            None
        }
    }
}

/// Downloads every page into `dir`, newest-reader-position first, announcing each
/// with `PageReady`. Pages already on disk (a prior read of the same entry) are
/// reused and announced without re-downloading. Stops early if the reader hangs
/// up (a `PageReady` write fails).
async fn download_session(
    provider: &impl Provider,
    plan: &[PageRef],
    dir: &Path,
    stream: &RefCell<Stream>,
    current: &AtomicUsize,
) {
    let mut pending: BTreeSet<usize> = (0..plan.len()).collect();

    // Reuse a previous (possibly partial) download: announce complete files and
    // drop them from the work set.
    for index in 0..plan.len() {
        if dir.join(page_file(index)).exists() {
            pending.remove(&index);
            if send_command(stream, &ReaderCommand::PageReady { index }).is_err() {
                return;
            }
        }
    }

    while let Some(index) = next_page(&pending, current.load(Ordering::Relaxed)) {
        pending.remove(&index);

        let page = &plan[index];
        match provider.reader_image(&page.chapter_id, page.page).await {
            Ok(bytes) => match write_page(dir, index, &bytes) {
                Ok(()) => {
                    if send_command(stream, &ReaderCommand::PageReady { index }).is_err() {
                        return;
                    }
                }
                Err(error) => tracing::debug!("could not write reader page {index}: {error}"),
            },
            Err(error) => tracing::debug!("could not download reader page {index}: {error}"),
        }
    }
}

/// The next page to fetch given the reader is on `current`: the nearest pending
/// page at or after it (reading moves forward), otherwise the last remaining one
/// behind it. `None` once nothing is pending.
fn next_page(pending: &BTreeSet<usize>, current: usize) -> Option<usize> {
    pending
        .range(current..)
        .next()
        .or_else(|| pending.iter().next_back())
        .copied()
}

/// Reads [`ReaderEvent`]s from the reader and passes each on to `events`, for
/// the session worker. Runs until the reader disconnects, which drops `events`
/// and so tells the worker the reader is gone.
fn forward_events(read_half: Stream, events: UnboundedSender<ReaderEvent>) {
    for line in BufReader::new(read_half).lines() {
        match line {
            Ok(line) if line.trim().is_empty() => {}
            Ok(line) => match serde_json::from_str::<ReaderEvent>(&line) {
                Ok(event) => {
                    tracing::debug!("reader socket ← {event:?}");
                    if events.send(event).is_err() {
                        break;
                    }
                }
                Err(error) => tracing::debug!("unparsable reader event {line:?}: {error}"),
            },
            Err(error) => {
                tracing::debug!("reader event stream ended: {error}");
                break;
            }
        }
    }
}

/// Writes page `index`'s bytes into `dir`, via a `.part` temp renamed into place
/// so a half-written file is never seen as a complete, reusable page.
fn write_page(dir: &Path, index: usize, bytes: &[u8]) -> std::io::Result<()> {
    let final_path = dir.join(page_file(index));
    let temp_path = dir.join(format!("{index:05}.img.part"));
    std::fs::write(&temp_path, bytes)?;
    std::fs::rename(&temp_path, &final_path)
}

/// The file name for page `index`: zero-padded so it matches what the reader
/// derives from the index.
pub(crate) fn page_file(index: usize) -> String {
    format!("{index:05}.img")
}

/// Writes one command to the reader as a line of JSON. The stream is shared by
/// the session's concurrent tasks (downloads, next-entry offers); each write is
/// synchronous, so one command's line is never split by another's.
fn send_command(stream: &RefCell<Stream>, command: &ReaderCommand) -> std::io::Result<()> {
    tracing::debug!("reader socket → {command:?}");
    let line = serde_json::to_string(command).map_err(std::io::Error::other)?;
    let mut stream = stream.borrow_mut();
    stream.write_all(line.as_bytes())?;
    stream.write_all(b"\n")?;
    stream.flush()
}

/// Launches the reader process pointed at `dir`, telling it to talk to the host
/// on `socket_name`. Fire-and-forget: a slow or long-lived reader must not block
/// the app.
fn spawn_reader(dir: &Path, socket_name: &str) -> Result<(), String> {
    let exe = std::env::current_exe()
        .map_err(|error| format!("could not locate the running executable: {error}"))?;

    Command::new(exe)
        .arg(READER_SUBCOMMAND)
        .arg(dir)
        .arg("--events")
        .arg(socket_name)
        .spawn()
        .map(|_child| ())
        .map_err(|error| format!("could not launch the reader: {error}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::collection_item::CollectionItem;
    use crate::models::series_entry::EntryKind;

    /// A provider serving one-byte pages, with `next` after any entry and
    /// `previous` before it. It has no covers.
    #[derive(Default)]
    struct FakeProvider {
        next: Option<SeriesEntry>,
        previous: Option<SeriesEntry>,
    }

    impl Provider for FakeProvider {
        async fn list_collections(
            &self,
        ) -> Result<Vec<crate::models::collection::Collection>, String> {
            unimplemented!()
        }
        async fn list_items(
            &self,
            _: &str,
            _: Option<u32>,
            _: Option<u32>,
        ) -> Result<Vec<CollectionItem>, String> {
            unimplemented!()
        }
        async fn list_series_entries(&self, _: &str) -> Result<Vec<SeriesEntry>, String> {
            unimplemented!()
        }
        async fn cover(&self, _: &str, _: &str) -> Result<Vec<u8>, String> {
            unimplemented!()
        }
        async fn volume_cover(&self, _: &str, _: &str) -> Result<Vec<u8>, String> {
            Err("no cover".to_string())
        }
        async fn chapter_cover(&self, _: &str, _: &str) -> Result<Vec<u8>, String> {
            Err("no cover".to_string())
        }
        async fn reader_pages(&self, _: &SeriesEntry) -> Result<Vec<PageRef>, String> {
            unimplemented!()
        }
        async fn reader_image(&self, _: &str, _: u16) -> Result<Vec<u8>, String> {
            Ok(vec![0])
        }
        async fn next_entry(
            &self,
            _: &str,
            _: &SeriesEntry,
        ) -> Result<Option<SeriesEntry>, String> {
            Ok(self.next.clone())
        }
        async fn previous_entry(
            &self,
            _: &str,
            _: &SeriesEntry,
        ) -> Result<Option<SeriesEntry>, String> {
            Ok(self.previous.clone())
        }
    }

    /// How `entry` is offered to the reader (without a cover, as the fake
    /// provider has none).
    fn offered(direction: Direction, entry: Option<&SeriesEntry>) -> ReaderCommand {
        ReaderCommand::Adjacent {
            direction,
            entry: entry.map(|entry| AdjacentEntry {
                label: entry.label.clone(),
                kind: komasabi_reader::EntryKind::Volume,
                cover: None,
            }),
        }
    }

    fn volume(id: &str) -> SeriesEntry {
        SeriesEntry {
            external_id: id.to_string(),
            kind: EntryKind::Volume,
            label: format!("Volume {id}"),
            pages: None,
            read_state: Default::default(),
        }
    }

    /// A connected host/reader socket pair: the host's end, and a receiver of
    /// the commands arriving at the reader's end.
    fn socket_pair() -> (Stream, std::sync::mpsc::Receiver<ReaderCommand>) {
        let name = format!("komasabi-launch-test-{}.sock", Uuid::new_v4());
        let listener = ListenerOptions::new()
            .name(name.as_str().to_ns_name::<GenericNamespaced>().unwrap())
            .create_sync()
            .unwrap();
        let reader_end =
            Stream::connect(name.as_str().to_ns_name::<GenericNamespaced>().unwrap()).unwrap();
        let host_end = listener.accept().unwrap();

        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            for line in BufReader::new(reader_end).lines() {
                let Ok(line) = line else { break };
                if tx.send(serde_json::from_str(&line).unwrap()).is_err() {
                    break;
                }
            }
        });
        (host_end, rx)
    }

    /// Runs [`serve_entry`] for a two-page volume over `events`, all queued
    /// before it starts, and returns its outcome plus the commands sent.
    async fn serve(
        provider: FakeProvider,
        events: Vec<ReaderEvent>,
        close: bool,
    ) -> (Option<Chosen>, Vec<ReaderCommand>) {
        let dir = std::env::temp_dir().join(format!("komasabi-launch-{}", Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let (host_end, commands) = socket_pair();
        let stream = RefCell::new(host_end);
        let plan: Vec<PageRef> = (0..2)
            .map(|page| PageRef {
                chapter_id: "c".to_string(),
                page,
            })
            .collect();

        let (events_tx, mut events_rx) = mpsc::unbounded_channel();
        for event in events {
            events_tx.send(event).unwrap();
        }
        if close {
            drop(events_tx);
        }

        let entry = volume("1");
        let session = EntrySession {
            series_id: "series",
            namespace: "integration",
            in_library: false,
            entry: &entry,
            plan: &plan,
            dir: &dir,
            stream: &stream,
        };
        let outcome = serve_entry(&provider, session, &mut events_rx).await;

        drop(stream);
        let _ = std::fs::remove_dir_all(&dir);
        (outcome, commands.iter().collect())
    }

    #[tokio::test]
    async fn the_last_page_offers_the_next_entry_and_opens_it_on_request() {
        let provider = FakeProvider {
            next: Some(volume("2")),
            ..FakeProvider::default()
        };
        let events = vec![
            ReaderEvent::ShowingPage { page: 0 },
            ReaderEvent::ShowingPage { page: 1 },
            ReaderEvent::OpenAdjacent {
                direction: Direction::Next,
            },
        ];

        let (outcome, commands) = serve(provider, events, false).await;

        let chosen = outcome.expect("the next entry was chosen");
        assert_eq!(chosen.direction, Direction::Next);
        assert_eq!(chosen.entry, volume("2"));
        assert!(commands.contains(&offered(Direction::Next, Some(&volume("2")))));
    }

    #[tokio::test]
    async fn the_first_page_offers_the_previous_entry_and_opens_it_on_request() {
        let provider = FakeProvider {
            previous: Some(volume("0")),
            ..FakeProvider::default()
        };
        let events = vec![
            ReaderEvent::ShowingPage { page: 0 },
            ReaderEvent::OpenAdjacent {
                direction: Direction::Previous,
            },
        ];

        let (outcome, commands) = serve(provider, events, false).await;

        let chosen = outcome.expect("the previous entry was chosen");
        assert_eq!(chosen.direction, Direction::Previous);
        assert_eq!(chosen.entry, volume("0"));
        assert!(commands.contains(&offered(Direction::Previous, Some(&volume("0")))));
        assert!(
            !commands.iter().any(|command| matches!(
                command,
                ReaderCommand::Adjacent {
                    direction: Direction::Next,
                    ..
                }
            )),
            "the next entry is only looked up on the last page"
        );
    }

    #[tokio::test]
    async fn the_last_entry_is_offered_nothing_and_ends_with_the_reader() {
        let provider = FakeProvider::default();
        let events = vec![
            ReaderEvent::ShowingPage { page: 1 },
            // Nothing was offered, so there's nothing to open.
            ReaderEvent::OpenAdjacent {
                direction: Direction::Next,
            },
        ];

        let (outcome, commands) = serve(provider, events, true).await;

        assert_eq!(outcome, None);
        assert!(commands.contains(&offered(Direction::Next, None)));
    }

    #[tokio::test]
    async fn the_next_entry_is_only_looked_up_on_the_last_page() {
        let provider = FakeProvider {
            next: Some(volume("2")),
            ..FakeProvider::default()
        };
        let events = vec![
            ReaderEvent::ShowingPage { page: 0 },
            ReaderEvent::OpenAdjacent {
                direction: Direction::Next,
            },
        ];

        let (outcome, commands) = serve(provider, events, true).await;

        assert_eq!(outcome, None);
        assert_eq!(
            commands
                .iter()
                .filter(|command| matches!(command, ReaderCommand::Adjacent { .. }))
                .count(),
            1
        );
        assert!(commands.contains(&offered(Direction::Previous, None)));
    }

    #[test]
    fn next_page_prefers_forward_then_mops_up_behind() {
        let pending: BTreeSet<usize> = [0, 1, 2, 5, 6].into_iter().collect();
        // At page 2, the nearest pending page at-or-after wins.
        assert_eq!(next_page(&pending, 2), Some(2));
        assert_eq!(next_page(&pending, 3), Some(5));

        // Nothing at or after the current page: fall back to the last one behind.
        let behind: BTreeSet<usize> = [0, 1].into_iter().collect();
        assert_eq!(next_page(&behind, 5), Some(1));

        assert_eq!(next_page(&BTreeSet::new(), 0), None);
    }

    #[test]
    fn library_items_reopen_on_the_last_page_read() {
        assert_eq!(resume_page(0, Some(20)), 0);
        assert_eq!(resume_page(5, Some(20)), 4);
        // Read through: start over.
        assert_eq!(resume_page(20, Some(20)), 0);
        // No page count yet.
        assert_eq!(resume_page(5, None), 0);
    }

    #[test]
    fn page_file_is_zero_padded() {
        assert_eq!(page_file(0), "00000.img");
        assert_eq!(page_file(42), "00042.img");
    }
}
