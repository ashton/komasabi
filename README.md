![Komasabi — comic & manga reader](docs/komasabi-header.jpg)

# Komasabi

Komasabi is a desktop app for keeping your comic and manga libraries in one place. It aggregates
local folders and external services — [Kavita](https://www.kavitareader.com/),
[Komga](https://komga.org/), [Suwayomi](https://github.com/Suwayomi/Suwayomi-Server) and any
[OPDS](https://opds.io/) catalog — into a single library view, with a built-in page reader.

## Install

Prebuilt binaries for Linux, Windows and macOS (universal) are attached to each
[GitHub release](https://github.com/ashton/komasabi/releases). They aren't code-signed yet, so
Windows SmartScreen warns on first launch, and on macOS you open the app the first time with
right-click → Open.

Or, with a [Rust toolchain](https://www.rust-lang.org/learn/get-started) installed:

```sh
cargo install komasabi
```

This installs a single `komasabi` binary. Your libraries are kept in a SQLite database in your
platform's data directory (e.g. `~/.local/share/komasabi` on Linux), created on first launch.

Building the OPDS integration needs a C++ compiler, since it reads CBR archives through `unrar`.

### Choosing integrations

Each service is a cargo feature — `kavita`, `komga`, `opds` and `suwayomi` — and all of them are on
by default. To build only the ones you use:

```sh
cargo install komasabi --no-default-features --features kavita,opds
```

Local folders are always available, so `--no-default-features` alone builds a local-only reader.

## About

The app is built with Rust and [`iced`](https://iced.rs/) for the UI. The workspace's crates:

- `app` (`komasabi`) — the desktop application: screens, local SQLite persistence (via
  [Diesel](https://diesel.rs/)), background sync jobs, and provider integrations.
- `reader` (`komasabi-reader`) — the page viewer the app opens chapters in.
- `kavita-client`, `komga-client`, `suwayomi-client`, `opds-client` — standalone clients for each
  service, with no dependency on `app`.

## Development

```sh
# Build / run (with mise: https://mise.jdx.dev/)
cargo build
mise run cargo:run
mise run cargo:run-reader <dir>          # the reader alone, on a folder of images

# Tests
cargo test --workspace

# Database migrations (sqlite; the app also runs pending ones on startup)
mise run migration:generate <name>       # scaffold a new migration
mise run migration:apply                 # run pending migrations
```

See `CLAUDE.md` for a deeper look at the codebase's architecture.
