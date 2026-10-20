# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## What this is

Sabita is a desktop app, built with Rust and [`iced`](https://iced.rs/) (not Slint — the repo
originated from a Slint template, hence the stale `README.md` and the enabled `slint` plugin in
`.claude/settings.json`, but the UI toolkit actually in use is `iced`). It lets a user aggregate
comic/manga libraries from external services (currently Kavita, Komga and OPDS catalogs) and from local folders into one
place.

The workspace crates:
- `app` (`sabita-app`) — the iced desktop application: screens, local SQLite persistence (Diesel),
  background jobs, and provider integrations.
- `kavita-client` — a standalone, UI-agnostic HTTP client for the [Kavita](https://www.kavitareader.com/)
  API. Has no dependency on `app` and is tested against a mocked server (`wiremock`), not a live Kavita
  instance.
- `komga-client` — a standalone, UI-agnostic HTTP client for the [Komga](https://komga.org/) API,
  tested with `wiremock`.
- `opds-client` — a standalone, UI-agnostic client for OPDS catalogs. Parses both OPDS 2 (JSON) and
  OPDS 1.2 (Atom XML) into one OPDS 2–shaped model. Also tested with `wiremock`.
- `reader` (`sabita-reader`) — the standalone page-viewer binary the app launches and streams pages into.

## Commands

This project uses [`mise`](https://mise.jdx.dev/) to manage tasks and tool versions (see `mise.toml`),
and `diesel_cli` for migrations (installed automatically by mise, sqlite-only).

```sh
# Build / run
cargo build
mise run cargo:run              # equivalent to `cargo run`, from app/

# Tests (iced screens use iced_test::simulator; kavita-client uses wiremock)
cargo test --workspace
cargo test -p sabita-app
cargo test -p kavita-client
cargo test -p komga-client
cargo test -p opds-client
cargo test <test_name>           # run a single test by name substring

# Database (sqlite, DATABASE_URL defaults to app/sabita.sqlite3 via mise env)
mise run migration:setup                 # create db + run all pending migrations
mise run migration:generate <name>       # scaffold a new migration (creates up.sql/down.sql)
mise run migration:apply                 # run pending migrations
```

Diesel migration tasks run with `dir = "app"`, so `DATABASE_URL` (set in `mise.toml` as
`sabita.sqlite3`) resolves relative to `app/`. If running `diesel` manually outside mise, `cd app`
first.

### Regenerating `app/src/schema.rs`

`schema.rs` is Diesel-generated but hand-patched: diesel-cli's codegen is piped through
`app/diesel-schema-patches/schema.patch` (configured in `app/diesel.toml`) to inject the enum-mapping
imports (`IntegrationTypeMapping`, `JobTypeMapping`, `JobStatusMapping`) that `diesel print-schema`
doesn't know to add on its own. After adding a migration that introduces or changes a `diesel_derive_enum`
column, update the patch file to match, or `schema.rs` will revert to plain `Text` columns the next
time it's regenerated.

## Architecture (`app` crate)

**Screens (`src/screen/`).** Each screen (`home`, `library`, `integrations`, `settings`) is a
self-contained iced sub-application following the same shape:
- a `Message` enum for its own events
- an `Action` enum (`None` / `Run(Task<Message>)`) returned from `update`, which the top-level
  `SabitaApp::update` (in `src/main.rs`) matches on to decide whether to run a follow-up `Task`
- `new() -> (Self, Task<Message>)` for initial load (e.g. kicking off a repository fetch)
- a `view_helper` submodule holding the actual iced widget tree, kept separate from `update` logic
- `#[cfg(test)]` tests driven via `iced_test::simulator`, asserting on both rendered widgets
  (`ui.find("label")`) and emitted messages (`ui.click(...)`, `ui.into_messages()`)

`src/main.rs` owns the top-level `Screen` enum/`Message` enum, routes navigation messages
(`OpenHome`, `OpenLibrary`, ...) to swap `self.screen`, and forwards per-screen messages into the
active screen's `update`. `src/menu.rs` is the generic sidebar widget; it tracks which item is
"active" by comparing message *discriminants*, not values — new nav messages must be unit-like
variants (or the single discriminant will match multiple items).

Some UI copy is in Portuguese (e.g. empty-state strings in `library.rs`/`integrations.rs`) — match
the existing language when adding sibling strings in the same view, don't translate unprompted.

**Data flow for an integration (e.g. Kavita):**
1. `screen/integrations.rs` collects connection details (name, URL, API key) through a `Step` state
   machine (`List` → `ChooseType` → `KavitaForm`) and saves via `repository::integration::create`.
2. On successful creation, it fires `jobs::sync_integration_libraries::spawn(integration_id)` —
   this spawns a **dedicated OS thread** with its own single-threaded tokio runtime, deliberately
   decoupled from iced's own async executor, so a slow/hanging sync can't stall the UI.
3. The job (`src/jobs/sync_integration_libraries.rs`) looks up the integration, authenticates a
   `providers::kavita::provider::KavitaProvider` (exchanges the stored API key for a JWT via
   `kavita-client`), fetches remote libraries, and inserts any not already present (matched by
   `external_id`) via `repository::library`. Job status/errors are persisted via
   `repository::job::update_status` so failures are recorded rather than silently dropped.
4. `providers::kavita::adapter` implements the generic `adapter::{ItemAdapter, LibraryAdapter}`
   traits to convert `kavita-client` wire types (`KavitaLibrary`, `Series`) into this app's own
   `models::library::Library` / `models::library_item::Item`. Adding a second provider means
   implementing these same adapter traits plus a `providers::Provider` impl — don't let
   Kavita-specific types leak past the adapter layer into `repository`/`screen` code.

**Persistence (`src/repository/`, `src/models/`, `src/database.rs`, `src/schema.rs`).** Diesel +
`diesel-async`'s `SyncConnectionWrapper` over sqlite; `database::connect()` opens a fresh connection
per call (no pooling) using `DATABASE_URL`. Repository functions all return `Result<T, String>`
(errors stringified via `.map_err(|e| e.to_string())`) since they're consumed directly as iced
`Task::perform` futures, where the error type flows into a `Message` variant. `models::AsyncModel<T, E>`
is the standard wrapper screens use to represent not-loaded/loading/loaded/error state for data
fetched this way.

Enum columns (`IntegrationType`, `JobType`, `JobStatus`) use `diesel_derive_enum::DbEnum`, stored as
`Text` in sqlite — see the schema-regeneration note above when touching these.

**OPDS provider (`providers::opds`).** Added via the integrations screen's "OPDS" type (catalog URL +
optional basic-auth username/password, stored in `integrations.username`/`password`). OPDS has no
numeric ids, so library/item/entry ids are `OpdsId`s: a feed URL, or (feed page URL, index, key) for a
publication, base64url-encoded. `list_libraries` turns each root navigation link into a library, but
expands a "catalog of catalogs" (e.g. Kavita's "All Libraries") into one library per entry. A library's
items are its feed's navigation links + publications; an item's entries are the publications of the
feed it links to. Reading goes only through the OPDS provider and `opds-client`, with no other
provider's code involved. The file is downloaded from the publication's `…/acquisition/open-access`
link (plain `…/acquisition` as a fallback) and fully extracted, CBZ or CBR, by `providers::opds::archive`
into a cached pages folder. The reader is then served from that folder. Page streams (OPDS-PSE) are
not used for reading. Live check: `OPDS_LIVE_URL=<catalog> cargo test -p sabita-app opds_live -- --ignored --nocapture`
(a Kavita catalog is `http://<host>/api/opds/<api-key>`; Kavita only serves OPDS 1.2).

**Komga provider (`providers::komga`).** Added via the integrations screen's "Komga" type (server URL
plus either an API key, stored in `integrations.api_key`, or the account email/password, stored in
`username`/`password`; the API key wins when both are set). Komga's library → series → book maps
directly onto library → item → entry, using Komga's string ids as-is. Every book is a chapter card
labelled with its title, since Komga doesn't say whether a book is a volume or an issue. Reading
uses the page endpoint (`/api/v1/books/{id}/pages/{n}`, 1-based), so nothing is downloaded up front.
Text-only EPUBs and books Komga hasn't analyzed yet are refused. Live check:
`KOMGA_LIVE_URL=<server> KOMGA_LIVE_API_KEY=<key> cargo test -p sabita-app komga_live -- --ignored --nocapture`.

## Architecture (`kavita-client` crate)

A thin `reqwest`-based wrapper over the Kavita REST API. `KavitaClient` (`src/client.rs`) holds the
base URL and an `http_client: reqwest::Client`; endpoint groups (`account`, `chapter`, `library`,
`series`, `volume`) are implemented as `impl KavitaClient` blocks in separate files under
`src/client/`, each just building a request and deserializing the JSON response — no retry/caching
logic. Models mirror Kavita's wire format under `src/models/` (e.g. `series_filter.rs` encodes
Kavita's series-filter query DSL used by `list_series_with_filter`).

Auth is a two-step exchange, not handled by this crate directly: callers call
`authenticate(api_key, plugin_name)` to get a JWT (`User::token`) from a Kavita API key, then build a
*second* `KavitaClient` with that JWT set as a default `Authorization: Bearer` header (this is what
`app`'s `KavitaProvider::authenticate` does — see above).

Tests mock the Kavita HTTP API with `wiremock` (no live server needed) and commonly assert both the
request shape (method/path/query params/JSON body) and response deserialization in the same test.

## Architecture (`komga-client` crate)

`KomgaClient` (`src/client.rs`, endpoints in `src/client/`) takes an `Auth` (API key sent as
`X-API-Key`, or basic auth) at construction, unlike `kavita-client`'s caller-built `reqwest::Client`.
Paging is 0-based (Spring `Page<T>`, `models/page.rs`). Users are served under `/api/v2` (`me()`), and
everything else is under `/api/v1`. The anonymous `/api/v1/claim` serves as the reachability ping.

## Architecture (`opds-client` crate)

`OpdsClient` (`src/client.rs`, endpoints in `src/client/`) fetches feeds with content negotiation that
prefers OPDS 2. `format::parse_feed` picks a parser from the body's first byte, falling back to the
content type, since servers mislabel feeds (Kavita sends Atom as `application/xml`). It then resolves
every href to an absolute URL. `format/atom.rs` maps Atom entries onto the OPDS 2 model: an entry with
an acquisition or page-stream link is a `Publication`, any other entry is a `Navigation`, and
`pse:count` becomes `LinkProperties::page_count`. `cargo run -p opds-client --example browse --
<catalog-url> [href]` prints what a live catalog parses to.
