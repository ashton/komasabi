![Komasabi — comic & manga reader](docs/komasabi-header.jpg)

# Komasabi

Komasabi is a desktop app for keeping your comic and manga libraries in one place. It aggregates
local folders and external services — currently [Kavita](https://www.kavitareader.com/) — into a
single library view.

## About

The app is built with Rust and [`iced`](https://iced.rs/) for the UI. The workspace has two crates:

- `app` (`komasabi-app`) — the desktop application: screens, local SQLite persistence (via
  [Diesel](https://diesel.rs/)), background sync jobs, and provider integrations.
- `kavita-client` — a standalone HTTP client for the Kavita API, with no dependency on `app`.

## Usage

1. Install Rust by following its [getting-started guide](https://www.rust-lang.org/learn/get-started).
   Once this is done, you should have the `rustc` compiler and the `cargo` build system installed in
   your `PATH`.
2. Install [`mise`](https://mise.jdx.dev/), which manages this project's tasks and the `diesel_cli`
   tool used for migrations.
3. Clone the repository and change into it.
4. Set up the database:
   ```
   mise run migration:setup
   ```
5. Run the application:
   ```
   mise run cargo:run
   ```

## Development

```sh
# Build / run
cargo build
mise run cargo:run

# Tests
cargo test --workspace

# Database migrations (sqlite)
mise run migration:setup                 # create db + run all pending migrations
mise run migration:generate <name>       # scaffold a new migration
mise run migration:apply                 # run pending migrations
```

See `CLAUDE.md` for a deeper look at the codebase's architecture.
