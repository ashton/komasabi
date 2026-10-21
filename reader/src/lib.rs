//! The standalone comic reader.
//!
//! The reader runs as its own process and reads images from a folder; it knows
//! nothing about where those images came from. [`run`] is the whole program:
//! the `komasabi-reader` binary calls it, and so does the Komasabi app when it
//! re-launches itself as a reader (`komasabi read …`), which keeps the app a
//! single installable binary. Its tie back to a host is a duplex channel over an
//! `interprocess` local socket: the reader emits [`ReaderEvent`]s about what's
//! on screen, and the host sends [`ReaderCommand`]s driving the session
//! (metadata, page availability).

use std::io::{BufRead, BufReader, Write};
use std::sync::mpsc;

use interprocess::TryClone;
use interprocess::local_socket::{GenericNamespaced, Stream, prelude::*};
use serde::{Deserialize, Serialize};

mod viewer;

pub use viewer::run;

/// An event the reader emits about what the user is reading. Serialized as one
/// JSON object per line over the socket, in the reader→host direction.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "event")]
pub enum ReaderEvent {
    /// The set of image files the reader found in the opened folder, emitted
    /// once at startup after enumeration (standalone runs only; in a hosted
    /// session the host already knows the page count).
    ReadAllFiles { path: String, pages: usize },
    /// The user is now viewing this page (0-based), emitted on every page change.
    /// The host uses it to prioritise which pages it downloads next.
    ShowingPage { page: usize },
}

/// A command the host sends to the reader, one JSON object per line over the
/// same socket, in the host→reader direction. The reverse of [`ReaderEvent`]:
/// the host drives the session, the reader reports what it's showing.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "command")]
pub enum ReaderCommand {
    /// The reading session's fixed metadata, sent once right after the host
    /// accepts the connection. `pages` is the total page count, which sizes the
    /// session before any page bytes exist on disk.
    OpenSession {
        series: String,
        entry: String,
        pages: usize,
    },
    /// Page `index`'s bytes have been written to the reader's folder and can now
    /// be loaded.
    PageReady { index: usize },
}

/// A handle the reader uses to emit [`ReaderEvent`]s. When connected, events are
/// handed to a background writer thread over a channel so socket I/O never
/// blocks the UI thread; when disabled (standalone runs, or a failed connect),
/// [`send`](Self::send) is a no-op.
pub struct EventSender {
    tx: Option<mpsc::Sender<ReaderEvent>>,
}

impl EventSender {
    /// A sender that drops every event — used when the reader runs standalone
    /// with no host socket.
    pub fn disabled() -> Self {
        Self { tx: None }
    }

    /// Spawns the background writer thread feeding events to `stream`.
    fn spawn(stream: Stream) -> Self {
        let (tx, rx) = mpsc::channel::<ReaderEvent>();
        std::thread::spawn(move || write_events(stream, rx));
        Self { tx: Some(tx) }
    }

    /// Emits an event, best-effort. Does nothing when disabled or if the writer
    /// thread has gone away.
    pub fn send(&self, event: ReaderEvent) {
        if let Some(tx) = &self.tx {
            let _ = tx.send(event);
        }
    }
}

/// The reader's end of the duplex channel to its host: an [`EventSender`] for
/// the events it emits, and a receiver for the [`ReaderCommand`]s the host
/// pushes. Both ride one socket connection, cloned into a write half (events
/// out) and a read half (commands in). With no host behind it — standalone runs
/// or a failed connect — events are dropped and the command receiver never
/// yields. The receiver must be kept alive to keep receiving.
pub struct ReaderLink {
    pub events: EventSender,
    pub commands: mpsc::Receiver<ReaderCommand>,
}

impl ReaderLink {
    /// A link with no host behind it: events go nowhere and no commands arrive.
    pub fn disabled() -> Self {
        let (_tx, commands) = mpsc::channel();
        Self {
            events: EventSender::disabled(),
            commands,
        }
    }

    /// Connects to the host's socket named `socket_name` (an `interprocess`
    /// namespaced name), splitting the connection into a write half feeding the
    /// event-writer thread and a read half feeding the command receiver. Any
    /// failure to connect or clone degrades to [`disabled`](Self::disabled)
    /// (logged to stderr) rather than taking the reader down.
    pub fn connect(socket_name: &str) -> Self {
        let name = match socket_name.to_ns_name::<GenericNamespaced>() {
            Ok(name) => name,
            Err(error) => {
                eprintln!("reader: invalid event socket name {socket_name:?}: {error}");
                return Self::disabled();
            }
        };

        let stream = match Stream::connect(name) {
            Ok(stream) => stream,
            Err(error) => {
                eprintln!("reader: could not connect to event socket: {error}");
                return Self::disabled();
            }
        };

        let read_half = match stream.try_clone() {
            Ok(clone) => clone,
            Err(error) => {
                eprintln!("reader: could not split event socket: {error}");
                return Self::disabled();
            }
        };

        let (tx, commands) = mpsc::channel::<ReaderCommand>();
        std::thread::spawn(move || read_commands(read_half, tx));

        Self {
            events: EventSender::spawn(stream),
            commands,
        }
    }
}

/// Drains `rx`, writing each event as a line of JSON to `stream`, until the
/// channel closes or a write fails.
fn write_events(mut stream: Stream, rx: mpsc::Receiver<ReaderEvent>) {
    for event in rx {
        eprintln!("reader: socket → {event:?}");
        let line = match serde_json::to_string(&event) {
            Ok(line) => line,
            Err(error) => {
                eprintln!("reader: could not encode event: {error}");
                continue;
            }
        };

        if let Err(error) = stream.write_all(line.as_bytes()).and_then(|()| {
            stream.write_all(b"\n")?;
            stream.flush()
        }) {
            eprintln!("reader: event write failed, stopping: {error}");
            break;
        }
    }
}

/// Reads newline-delimited [`ReaderCommand`] JSON from `stream`, forwarding each
/// parsed command to `tx` until the host hangs up or the receiver is dropped.
fn read_commands(stream: Stream, tx: mpsc::Sender<ReaderCommand>) {
    for line in BufReader::new(stream).lines() {
        match line {
            Ok(line) if line.trim().is_empty() => {}
            Ok(line) => match serde_json::from_str::<ReaderCommand>(&line) {
                Ok(command) => {
                    eprintln!("reader: socket ← {command:?}");
                    if tx.send(command).is_err() {
                        break;
                    }
                }
                Err(error) => eprintln!("reader: unparsable command {line:?}: {error}"),
            },
            Err(error) => {
                eprintln!("reader: command stream ended: {error}");
                break;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn showing_page_round_trips_through_json() {
        let event = ReaderEvent::ShowingPage { page: 7 };
        let json = serde_json::to_string(&event).unwrap();

        assert_eq!(json, r#"{"event":"showing_page","page":7}"#);
        assert_eq!(serde_json::from_str::<ReaderEvent>(&json).unwrap(), event);
    }

    #[test]
    fn read_all_files_round_trips_through_json() {
        let event = ReaderEvent::ReadAllFiles {
            path: "/tmp/chapter".to_string(),
            pages: 20,
        };
        let json = serde_json::to_string(&event).unwrap();

        assert_eq!(serde_json::from_str::<ReaderEvent>(&json).unwrap(), event);
    }

    #[test]
    fn open_session_command_round_trips_through_json() {
        let command = ReaderCommand::OpenSession {
            series: "One Piece".to_string(),
            entry: "Volume 5".to_string(),
            pages: 190,
        };
        let json = serde_json::to_string(&command).unwrap();

        assert_eq!(
            serde_json::from_str::<ReaderCommand>(&json).unwrap(),
            command
        );
    }

    #[test]
    fn page_ready_command_round_trips_through_json() {
        let command = ReaderCommand::PageReady { index: 3 };
        let json = serde_json::to_string(&command).unwrap();

        assert_eq!(json, r#"{"command":"page_ready","index":3}"#);
        assert_eq!(
            serde_json::from_str::<ReaderCommand>(&json).unwrap(),
            command
        );
    }

    #[test]
    fn disabled_sender_silently_drops_events() {
        let sender = EventSender::disabled();
        // Must not panic; there's just nowhere for it to go.
        sender.send(ReaderEvent::ShowingPage { page: 0 });
    }

    #[test]
    fn disabled_link_drops_events_and_yields_no_commands() {
        let link = ReaderLink::disabled();
        link.events.send(ReaderEvent::ShowingPage { page: 0 });
        assert!(link.commands.try_recv().is_err());
    }
}
