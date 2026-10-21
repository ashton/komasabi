//! Opening a [`SeriesEntry`] in the standalone reader.
//!
//! The reader is a separate process that displays page images from a folder.
//! This module bridges the gap and then *streams* into it: it resolves the
//! entry to a cache folder, launches the reader pointed at that folder straight
//! away (so a window appears without waiting on any download), and hands the
//! session to a background worker. The worker, on its own Tokio runtime, opens a
//! duplex socket to the reader — sending it the session metadata
//! ([`ReaderCommand::OpenSession`]) and a [`ReaderCommand::PageReady`] as each
//! page lands on disk — while downloading pages in the order the reader actually
//! needs them, reprioritising around the page it reports it's showing
//! ([`ReaderEvent::ShowingPage`]).

use std::collections::BTreeSet;
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use interprocess::TryClone;
use interprocess::local_socket::{
    GenericNamespaced, Listener, ListenerOptions, Stream, prelude::*,
};
use uuid::Uuid;

use komasabi_reader::{ReaderCommand, ReaderEvent};

use crate::cache;
use crate::models::series_entry::SeriesEntry;
use crate::providers::{PageRef, Provider, provider_for_library};
use crate::repository::library as library_repository;

/// Everything the background session worker needs to serve one reader. Moved
/// onto its own thread, so it must be `Send + 'static`.
struct SessionSpec {
    library_external_id: String,
    series: String,
    entry: SeriesEntry,
    dir: PathBuf,
}

/// Resolves `entry` to its cache folder, launches the reader on it immediately,
/// and spawns the background worker that streams pages into it. Returns as soon
/// as the reader is launched — downloading happens in the worker, not here.
pub async fn prepare_and_launch(
    library_external_id: String,
    series_name: String,
    entry: SeriesEntry,
) -> Result<(), String> {
    let namespace = cache_namespace_for_library(&library_external_id).await?;

    let dir = cache::pages_dir(&format!("{namespace}-{}", entry.cover_key()))
        .ok_or("no cache directory available for reader pages")?;
    std::fs::create_dir_all(&dir)
        .map_err(|error| format!("could not create {}: {error}", dir.display()))?;

    let socket_name = bind_and_serve(SessionSpec {
        library_external_id,
        series: series_name,
        entry,
        dir: dir.clone(),
    })?;

    spawn_reader(&dir, &socket_name)
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
/// opens the session, and downloads pages into the folder in reader-driven
/// priority order.
async fn run_session(listener: Listener, spec: SessionSpec) -> Result<(), String> {
    let (provider, _integration_id) = provider_for_library(&spec.library_external_id).await?;
    let plan = provider.reader_pages(&spec.entry).await?;
    if plan.is_empty() {
        return Err("this entry has no pages to read".to_string());
    }

    let mut stream = listener
        .accept()
        .map_err(|error| format!("reader never connected: {error}"))?;

    send_command(
        &mut stream,
        &ReaderCommand::OpenSession {
            series: spec.series.clone(),
            entry: spec.entry.label.clone(),
            pages: plan.len(),
        },
    )
    .map_err(|error| format!("could not open the reader session: {error}"))?;

    // The reader reports the page it's showing; track it so downloads can follow
    // the reader instead of blindly marching from page zero.
    let read_half = stream
        .try_clone()
        .map_err(|error| format!("could not split the reader socket: {error}"))?;
    let current = Arc::new(AtomicUsize::new(0));
    std::thread::spawn({
        let current = Arc::clone(&current);
        move || observe_position(read_half, current)
    });

    download_session(&provider, &plan, &spec.dir, &mut stream, &current).await;
    Ok(())
}

/// Downloads every page into `dir`, newest-reader-position first, announcing each
/// with `PageReady`. Pages already on disk (a prior read of the same entry) are
/// reused and announced without re-downloading. Stops early if the reader hangs
/// up (a `PageReady` write fails).
async fn download_session(
    provider: &impl Provider,
    plan: &[PageRef],
    dir: &Path,
    stream: &mut Stream,
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

/// Reads [`ReaderEvent`]s from the reader, updating `current` as it reports the
/// page it's showing. Runs until the reader disconnects. The single seam where
/// future "mark chapter as read" work hooks in.
fn observe_position(read_half: Stream, current: Arc<AtomicUsize>) {
    for line in BufReader::new(read_half).lines() {
        match line {
            Ok(line) if line.trim().is_empty() => {}
            Ok(line) => match serde_json::from_str::<ReaderEvent>(&line) {
                Ok(event) => {
                    tracing::debug!("reader socket ← {event:?}");
                    if let ReaderEvent::ShowingPage { page } = event {
                        current.store(page, Ordering::Relaxed);
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

/// Writes one command to the reader as a line of JSON.
fn send_command(stream: &mut Stream, command: &ReaderCommand) -> std::io::Result<()> {
    tracing::debug!("reader socket → {command:?}");
    let line = serde_json::to_string(command).map_err(std::io::Error::other)?;
    stream.write_all(line.as_bytes())?;
    stream.write_all(b"\n")?;
    stream.flush()
}

/// Launches the reader process pointed at `dir`, telling it to talk to the host
/// on `socket_name`. Fire-and-forget: a slow or long-lived reader must not block
/// the app.
fn spawn_reader(dir: &Path, socket_name: &str) -> Result<(), String> {
    Command::new(reader_binary()?)
        .arg(dir)
        .arg("--events")
        .arg(socket_name)
        .spawn()
        .map(|_child| ())
        .map_err(|error| format!("could not launch the reader: {error}"))
}

/// Path to the reader binary, which ships alongside the app binary in the same
/// target directory.
fn reader_binary() -> Result<PathBuf, String> {
    let exe = std::env::current_exe()
        .map_err(|error| format!("could not locate the running executable: {error}"))?;
    let dir = exe
        .parent()
        .ok_or("the running executable has no parent directory")?;

    let name = if cfg!(windows) {
        "komasabi-reader.exe"
    } else {
        "komasabi-reader"
    };

    Ok(dir.join(name))
}

#[cfg(test)]
mod tests {
    use super::*;

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

    #[test]
    fn reader_binary_sits_next_to_the_current_exe() {
        let path = reader_binary().unwrap();
        let expected = cfg!(windows)
            .then_some("komasabi-reader.exe")
            .unwrap_or("komasabi-reader");
        assert_eq!(path.file_name().unwrap().to_str().unwrap(), expected);
        assert_eq!(path.parent(), std::env::current_exe().unwrap().parent());
    }
}
