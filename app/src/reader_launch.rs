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
//! for the entry after it ([`Provider::next_entry`]) and tells the reader
//! ([`ReaderCommand::NextEntry`]). If the user takes it up
//! ([`ReaderEvent::OpenNext`]), the same reader window moves on to that entry:
//! the worker opens a new session in it, with its own page folder, and streams
//! its pages the same way.

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

use komasabi_reader::{ReaderCommand, ReaderEvent};

use crate::cache;
use crate::models::series_entry::SeriesEntry;
use crate::providers::{PageRef, Provider, provider_for_library};
use crate::reading_progress;
use crate::repository::library as library_repository;

/// The first argument that makes `main` run the reader (`komasabi read <dir> …`)
/// instead of the library window. Shipping the reader inside the app binary
/// keeps `cargo install komasabi` a single, self-contained install.
pub const READER_SUBCOMMAND: &str = "read";

/// Everything the background session worker needs to serve one reader. Moved
/// onto its own thread, so it must be `Send + 'static`.
struct SessionSpec {
    library_external_id: String,
    /// The series' provider-side id, to look up the entry after the one read.
    series_id: String,
    series: String,
    entry: SeriesEntry,
    dir: PathBuf,
}

/// Resolves `entry` to its cache folder, launches the reader on it immediately,
/// and spawns the background worker that streams pages into it. Returns as soon
/// as the reader is launched — downloading happens in the worker, not here.
pub async fn prepare_and_launch(
    library_external_id: String,
    series_external_id: String,
    series_name: String,
    entry: SeriesEntry,
) -> Result<(), String> {
    let namespace = cache_namespace_for_library(&library_external_id).await?;
    let dir = entry_dir(&namespace, &entry)?;

    let socket_name = bind_and_serve(SessionSpec {
        library_external_id,
        series_id: series_external_id,
        series: series_name,
        entry,
        dir: dir.clone(),
    })?;

    spawn_reader(&dir, &socket_name)
}

/// Creates (if need be) and returns the cache folder `entry`'s pages are
/// downloaded into, under the library's cache `namespace`.
fn entry_dir(namespace: &str, entry: &SeriesEntry) -> Result<PathBuf, String> {
    let dir = cache::pages_dir(&format!("{namespace}-{}", entry.cover_key()))
        .ok_or("no cache directory available for reader pages")?;
    std::fs::create_dir_all(&dir)
        .map_err(|error| format!("could not create {}: {error}", dir.display()))?;
    Ok(dir)
}

/// The cache namespace for a library's reader pages, resolved without
/// authenticating a provider — just enough to compute the page cache folder up
/// front. A library's integration id namespaces remote pages; a local library
/// (no integration) falls back to its own id. The provider itself is resolved
/// later, on the worker's own runtime.
async fn cache_namespace_for_library(library_external_id: &str) -> Result<String, String> {
    let library = library_repository::find_by_external_id(library_external_id).await?;
    Ok(library.integration_id.unwrap_or(library.id))
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

/// The worker thread: runs the session to completion on a dedicated
/// single-threaded Tokio runtime (so its `reqwest`-based downloads have a
/// reactor and never touch iced's executor). Errors just end the session; the
/// reader keeps whatever pages already arrived.
fn serve_session(listener: Listener, spec: SessionSpec) {
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
}

/// Authenticates the provider, resolves the entry's pages, accepts the reader,
/// and serves it the entry (see [`serve_entry`]). When the user moves on to the
/// next entry, the same reader is handed a new session for it, until the reader
/// closes.
async fn run_session(listener: Listener, spec: SessionSpec) -> Result<(), String> {
    let (provider, namespace) = provider_for_library(&spec.library_external_id).await?;
    let mut entry = spec.entry;
    let mut dir = spec.dir;
    let mut plan = provider.reader_pages(&entry).await?;
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

    // The folder to announce with the next `OpenSession`: `None` for the first
    // entry, whose folder the reader was launched on, then each next entry's.
    let mut open_session = Some(None);
    loop {
        if let Some(new_dir) = open_session.take() {
            send_command(
                &stream,
                &ReaderCommand::OpenSession {
                    series: spec.series.clone(),
                    entry: entry.label.clone(),
                    pages: plan.len(),
                    dir: new_dir,
                },
            )
            .map_err(|error| format!("could not open the reader session: {error}"))?;
        }

        let session = EntrySession {
            series_id: &spec.series_id,
            entry: &entry,
            plan: &plan,
            dir: &dir,
            stream: &stream,
        };
        let Some(next) = serve_entry(&provider, session, &mut events).await else {
            return Ok(());
        };

        match prepare_entry(&provider, &namespace, &next).await {
            Ok((next_dir, next_plan)) => {
                open_session = Some(Some(next_dir.clone()));
                entry = next;
                dir = next_dir;
                plan = next_plan;
            }
            Err(error) => {
                // Stay on the current entry, and offer the next one again so
                // the user can retry.
                tracing::warn!("could not open {}: {error}", next.label);
                send_command(
                    &stream,
                    &ReaderCommand::NextEntry {
                        label: Some(next.label),
                    },
                )
                .map_err(|error| format!("reader went away: {error}"))?;
            }
        }
    }
}

/// The page folder and pages of `entry`, ready for a new reader session.
async fn prepare_entry(
    provider: &impl Provider,
    namespace: &str,
    entry: &SeriesEntry,
) -> Result<(PathBuf, Vec<PageRef>), String> {
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
    entry: &'a SeriesEntry,
    plan: &'a [PageRef],
    dir: &'a Path,
    stream: &'a RefCell<Stream>,
}

/// Serves one entry to the reader, already told about it with `OpenSession`:
/// downloads its pages in reader-driven priority order, reports the pages the
/// reader shows to the provider's reading history, and, once the reader reaches
/// the last page, looks up the entry after it and offers it to the reader.
///
/// Returns that next entry if the user asked to read it, or `None` once the
/// reader has closed.
async fn serve_entry(
    provider: &impl Provider,
    session: EntrySession<'_>,
    events: &mut UnboundedReceiver<ReaderEvent>,
) -> Option<SeriesEntry> {
    let current = AtomicUsize::new(0);
    let (shown_pages, shown_pages_rx) = mpsc::unbounded_channel();

    let dispatch = async {
        // Moved in, so it's dropped (ending the progress reporting) as soon as
        // this entry's session is over.
        let shown_pages = shown_pages;
        // What follows the entry, once looked up: `Some(None)` when nothing does.
        let mut next: Option<Option<SeriesEntry>> = None;

        while let Some(event) = events.recv().await {
            match event {
                ReaderEvent::ShowingPage { page } => {
                    current.store(page, Ordering::Relaxed);
                    let _ = shown_pages.send(page);

                    if page + 1 >= session.plan.len() && next.is_none() {
                        next = look_up_next(provider, &session).await;
                    }
                }
                ReaderEvent::OpenNext => {
                    if let Some(Some(entry)) = &next {
                        return Some(entry.clone());
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

/// Asks the provider what follows the session's entry and tells the reader.
/// Returns the answer to remember, or `None` if the lookup failed, so it's
/// tried again the next time the reader lands on the last page.
async fn look_up_next(
    provider: &impl Provider,
    session: &EntrySession<'_>,
) -> Option<Option<SeriesEntry>> {
    let next = match provider.next_entry(session.series_id, session.entry).await {
        Ok(next) => Some(next),
        Err(error) => {
            tracing::warn!(
                "could not look up what follows {}: {error}",
                session.entry.label
            );
            None
        }
    };

    let label = next
        .as_ref()
        .and_then(|next| next.as_ref())
        .map(|entry| entry.label.clone());
    if let Err(error) = send_command(session.stream, &ReaderCommand::NextEntry { label }) {
        tracing::debug!("could not offer the next entry: {error}");
    }
    next
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
fn page_file(index: usize) -> String {
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
    use crate::models::library_item::LibraryItem;
    use crate::models::series_entry::EntryKind;

    /// A provider serving one-byte pages, with `next` after any entry.
    struct FakeProvider {
        next: Option<SeriesEntry>,
    }

    impl Provider for FakeProvider {
        async fn list_items(
            &self,
            _: &str,
            _: Option<u32>,
            _: Option<u32>,
        ) -> Result<Vec<LibraryItem>, String> {
            unimplemented!()
        }
        async fn list_series_entries(&self, _: &str) -> Result<Vec<SeriesEntry>, String> {
            unimplemented!()
        }
        async fn cover(&self, _: &str, _: &str) -> Result<Vec<u8>, String> {
            unimplemented!()
        }
        async fn volume_cover(&self, _: &str, _: &str) -> Result<Vec<u8>, String> {
            unimplemented!()
        }
        async fn chapter_cover(&self, _: &str, _: &str) -> Result<Vec<u8>, String> {
            unimplemented!()
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
    ) -> (Option<SeriesEntry>, Vec<ReaderCommand>) {
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
        };
        let events = vec![
            ReaderEvent::ShowingPage { page: 0 },
            ReaderEvent::ShowingPage { page: 1 },
            ReaderEvent::OpenNext,
        ];

        let (outcome, commands) = serve(provider, events, false).await;

        assert_eq!(outcome, Some(volume("2")));
        assert!(commands.contains(&ReaderCommand::NextEntry {
            label: Some("Volume 2".to_string())
        }));
    }

    #[tokio::test]
    async fn the_last_entry_is_offered_nothing_and_ends_with_the_reader() {
        let provider = FakeProvider { next: None };
        let events = vec![
            ReaderEvent::ShowingPage { page: 1 },
            // Nothing was offered, so there's nothing to open.
            ReaderEvent::OpenNext,
        ];

        let (outcome, commands) = serve(provider, events, true).await;

        assert_eq!(outcome, None);
        assert!(commands.contains(&ReaderCommand::NextEntry { label: None }));
    }

    #[tokio::test]
    async fn the_next_entry_is_only_looked_up_on_the_last_page() {
        let provider = FakeProvider {
            next: Some(volume("2")),
        };
        let events = vec![ReaderEvent::ShowingPage { page: 0 }, ReaderEvent::OpenNext];

        let (outcome, commands) = serve(provider, events, true).await;

        assert_eq!(outcome, None);
        assert!(
            !commands
                .iter()
                .any(|command| matches!(command, ReaderCommand::NextEntry { .. }))
        );
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
    fn page_file_is_zero_padded() {
        assert_eq!(page_file(0), "00000.img");
        assert_eq!(page_file(42), "00042.img");
    }
}
