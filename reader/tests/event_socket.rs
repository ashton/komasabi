//! End-to-end check of the reader's duplex channel over a real `interprocess`
//! local socket: the reader emits [`ReaderEvent`]s that reach the host in order,
//! and [`ReaderCommand`]s the host writes reach the reader's command receiver.
//! This mirrors exactly what the app does when it launches the reader, so it
//! verifies the transport in both directions without needing a display.

use std::io::{BufRead, BufReader, Write};

use interprocess::local_socket::{GenericNamespaced, ListenerOptions, prelude::*};
use sabita_reader::{ReaderCommand, ReaderEvent, ReaderLink};

fn bind(tag: &str) -> (String, interprocess::local_socket::Listener) {
    let socket_name = format!("sabita-reader-test-{tag}-{}.sock", std::process::id());
    let name = socket_name
        .as_str()
        .to_ns_name::<GenericNamespaced>()
        .expect("valid namespaced name");
    let listener = ListenerOptions::new()
        .name(name)
        .create_sync()
        .expect("bind listener");
    (socket_name, listener)
}

#[test]
fn events_from_the_reader_arrive_in_order() {
    let (socket_name, listener) = bind("events");

    // Reader side: connect and emit the events it would during a session.
    let link = ReaderLink::connect(&socket_name);
    link.events.send(ReaderEvent::ReadAllFiles {
        path: "/tmp/chapter".to_string(),
        pages: 3,
    });
    link.events.send(ReaderEvent::ShowingPage { page: 0 });
    link.events.send(ReaderEvent::ShowingPage { page: 1 });

    // Host side: read exactly the three events we sent (not until EOF — the
    // reader's read half keeps the connection open, so there's no hang-up).
    let stream = listener.accept().expect("accept reader connection");
    let events: Vec<ReaderEvent> = BufReader::new(stream)
        .lines()
        .take(3)
        .map(|line| serde_json::from_str(&line.expect("read line")).expect("parse event"))
        .collect();

    assert_eq!(
        events,
        vec![
            ReaderEvent::ReadAllFiles {
                path: "/tmp/chapter".to_string(),
                pages: 3,
            },
            ReaderEvent::ShowingPage { page: 0 },
            ReaderEvent::ShowingPage { page: 1 },
        ]
    );
}

#[test]
fn commands_from_the_host_reach_the_reader() {
    let (socket_name, listener) = bind("commands");

    let link = ReaderLink::connect(&socket_name);

    // Host side: accept, then push the commands it would during a session.
    let mut stream = listener.accept().expect("accept reader connection");
    for command in [
        ReaderCommand::OpenSession {
            series: "One Piece".to_string(),
            entry: "Volume 5".to_string(),
            pages: 3,
        },
        ReaderCommand::PageReady { index: 0 },
        ReaderCommand::PageReady { index: 1 },
    ] {
        let line = serde_json::to_string(&command).expect("encode command");
        stream.write_all(line.as_bytes()).expect("write command");
        stream.write_all(b"\n").expect("write newline");
    }
    stream.flush().expect("flush commands");

    // Reader side: the command receiver yields them in order.
    let received: Vec<ReaderCommand> = (0..3)
        .map(|_| {
            link.commands
                .recv_timeout(std::time::Duration::from_secs(5))
                .expect("command received")
        })
        .collect();

    assert_eq!(
        received,
        vec![
            ReaderCommand::OpenSession {
                series: "One Piece".to_string(),
                entry: "Volume 5".to_string(),
                pages: 3,
            },
            ReaderCommand::PageReady { index: 0 },
            ReaderCommand::PageReady { index: 1 },
        ]
    );
}
