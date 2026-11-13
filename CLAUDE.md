# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## What this is

Komasabi is a desktop app, built with Rust and [`iced`](https://iced.rs/) (not Slint — the repo
originated from a Slint template, hence the stale `README.md` and the enabled `slint` plugin in
`.claude/settings.json`, but the UI toolkit actually in use is `iced`). It lets a user aggregate
comic/manga libraries from external services (currently Kavita, Komga, Suwayomi and OPDS catalogs) and from local folders into one
place. Each of those is a **source** (called "Sources" in the UI, `Integration` in code), and a local folder is a
source like any server.

The workspace crates:
- `app` (`komasabi`) — the iced desktop application: screens, local SQLite persistence (Diesel),
  and provider integrations.
- `kavita-client` — a standalone, UI-agnostic HTTP client for the [Kavita](https://www.kavitareader.com/)
  API. Has no dependency on `app` and is tested against a mocked server (`wiremock`), not a live Kavita
  instance.
- `komga-client` — a standalone, UI-agnostic HTTP client for the [Komga](https://komga.org/) API,
  tested with `wiremock`.
- `suwayomi-client` — a standalone, UI-agnostic client for a [Suwayomi](https://github.com/Suwayomi/Suwayomi-Server)
  server's GraphQL API, tested with `wiremock`.
- `opds-client` — a standalone, UI-agnostic client for OPDS catalogs. Parses both OPDS 2 (JSON) and
  OPDS 1.2 (Atom XML) into one OPDS 2–shaped model. Also tested with `wiremock`.
- `reader` (`komasabi-reader`) — the page viewer the app launches and streams pages into. Its whole
  program is the library's `run(args)`: the app re-launches its own executable as `komasabi read <dir>
  --events <socket>` (`reader_launch::READER_SUBCOMMAND`, handled first thing in `main`), so
  `cargo install komasabi` is one self-contained binary. The crate's own `komasabi-reader` binary is a
  one-line wrapper for running the reader on its own (`mise run cargo:run-reader <dir>`).

## Commands

This project uses [`mise`](https://mise.jdx.dev/) to manage tasks and tool versions (see `mise.toml`),
and `diesel_cli` for migrations (installed automatically by mise, sqlite-only).

```sh
# Build / run
cargo build
mise run cargo:run              # equivalent to `cargo run`, from app/

# Tests (iced screens use iced_test::simulator; kavita-client uses wiremock)
cargo test --workspace
cargo test -p komasabi
cargo test -p kavita-client
cargo test -p komga-client
cargo test -p suwayomi-client
cargo test -p opds-client
cargo test <test_name>           # run a single test by name substring

# Database (sqlite, DATABASE_URL defaults to app/komasabi.sqlite3 via mise env)
mise run migration:setup                 # create db + run all pending migrations
mise run migration:generate <name>       # scaffold a new migration (creates up.sql/down.sql)
mise run migration:apply                 # run pending migrations
```

Diesel migration tasks run with `dir = "app"`, so `DATABASE_URL` (set in `mise.toml` as
`komasabi.sqlite3`) resolves relative to `app/`. If running `diesel` manually outside mise, `cd app`
first.

### Publishing

Every crate publishes to crates.io, sharing `[workspace.package]` metadata (one version for all) from
the root `Cargo.toml`. Internal dependencies go through `[workspace.dependencies]`, which pins each
one's path *and* version, so a release bumps both `workspace.package.version` and those versions.
Each crate's `LICENSE` is a symlink to the root one. `cargo publish --workspace --dry-run` packages
and builds every crate in dependency order; drop `--dry-run` to publish.

Binaries come from `.github/workflows/release.yml`: pushing a `v*` tag builds `komasabi` on native
runners (Linux x86_64, Windows x86_64, and a universal macOS binary `lipo`'d from both Mac targets) and
attaches the archives to a GitHub Release; a manual run only builds them. SQLite is bundled
(`libsqlite3-sys` `bundled` feature), since Windows has none to link. Each job also builds that
platform's installers with `cargo packager` (`.deb`/`.AppImage`/pacman, NSIS `-setup.exe`, `.app`/`.dmg`),
configured under `[package.metadata.packager]` in `app/Cargo.toml`; it packages the binary already in
`target/<target>/release` rather than building one. `app/build.rs` embeds `assets/komasabi.ico` as
the Windows executable's icon. Linux also gets an `.rpm` from `cargo generate-rpm -p app`
(`[package.metadata.generate-rpm]`). Every Linux package installs `app/packaging/komasabi.desktop`. The
pacman `PKGBUILD` that cargo-packager writes would install its own source tarball at `/`, so
`app/packaging/patch-pkgbuild.sh` fixes it and points its source at the GitHub release. The Linux job
runs on Ubuntu 22.04 so the binary only needs glibc 2.35. Locally: `cd app && cargo build --release && cargo packager --release
-f deb,appimage`.

### Regenerating `app/src/schema.rs`

`schema.rs` is Diesel-generated but hand-patched: diesel-cli's codegen is piped through
`app/diesel-schema-patches/schema.patch` (configured in `app/diesel.toml`) to inject the enum-mapping
imports (`IntegrationTypeMapping`, `LibraryTypeMapping`) that `diesel print-schema` doesn't know to add on its own. After adding a migration that introduces or changes a `diesel_derive_enum`
column, update the patch file to match, or `schema.rs` will revert to plain `Text` columns the next
time it's regenerated.

## Architecture (`app` crate)

**Screens (`src/screen/`).** Each screen (`home`, `library`, `integration` (sources), `collections`,
`browse`, `series_detail`, `settings`) is a self-contained iced sub-application following the same shape:
- a `Message` enum for its own events
- an `Action` enum (`None` / `Run(Task<Message>)`) returned from `update`, which the top-level
  `KomasabiApp::update` (in `src/main.rs`) matches on to decide whether to run a follow-up `Task`
- `new() -> (Self, Task<Message>)` for initial load (e.g. kicking off a repository fetch)
- a `view_helper` submodule holding the actual iced widget tree, kept separate from `update` logic
- `#[cfg(test)]` tests driven via `iced_test::simulator`, asserting on both rendered widgets
  (`ui.find("label")`) and emitted messages (`ui.click(...)`, `ui.into_messages()`)

`src/main.rs` owns the top-level `Screen` enum/`Message` enum, routes navigation messages
(`OpenHome`, `OpenIntegrations`, ...) and screens' navigation `Action`s to swap `self.screen`, and forwards per-screen messages into the
active screen's `update`. `src/menu.rs` is the generic sidebar widget; it tracks which item is
"active" by comparing message *discriminants*, not values — new nav messages must be unit-like
variants (or the single discriminant will match multiple items).

A screen's top-bar "back" control is `screen::back_button::back_button(msg)`, a chevron with no
text, so tests click it by its container id (`back_button::ID`). The Back buttons beside a form's
Save (`screen/integration/*`) stay labelled buttons.

Wrap every `scrollable` in `touch_scroll::touch_scroll(...)`. iced only touch-scrolls when the content
doesn't capture the finger press, and buttons (every card) do, so without the wrapper a touch screen
can't scroll. The wrapper turns a drag into pixel wheel scrolls, with flick momentum.

Some UI copy is in Portuguese (e.g. empty-state strings in `integration.rs`/`collections.rs`) — match
the existing language when adding sibling strings in the same view, don't translate unprompted.

**Sources, collections and items.** Navigation is Sources → a source's collections → a collection's
series (items) → a series' entries (volumes/chapters) → the reader:
1. `screen/integration.rs` lists the sources and collects a new one's details through a `Step` state
   machine (`List` → `ChooseType` → a per-type form under `screen/integration/`, e.g. `Kavita`,
   `LocalFolder`). Saving stores only the `integrations` row (a local folder's path goes in its
   `folder` column). Nothing is synced or copied: collections aren't stored anywhere.
2. Clicking a source opens `screen/collections.rs`, which resolves the source's provider
   (`providers::provider_for_integration`) and calls `Provider::list_collections` live. A
   **collection** (`models::collection::Collection`) is anything in a source that gathers series: a
   Kavita/Komga library, collection or reading list, a Suwayomi category, an OPDS feed, or a local folder
   (its one collection). The screen groups them by the generic `CollectionKind`.
3. Opening a collection hands a `CollectionRef` (source id + collection) to `screen/browse.rs`, which
   pages through `Provider::list_items(collection_id, …)` as `CollectionItem`s. `series_detail.rs`,
   the reader launcher and the cover/page caches all key off the source's id (the cache namespace).
   A collection with `importable` set (a local folder) also offers "Import items", which goes through
   `Provider::import_files`.
4. Screens only ever see `Collection`, `CollectionItem` and `SeriesEntry`. Each provider adapts its
   client's wire types through `adapter::{CollectionAdapter, ItemAdapter}` and owns its collection id
   format: a remote provider with several kinds of collection encodes the kind in the id (Kavita
   `library:5` / `collection:4` / `reading-list:2`, Komga `library:<id>` / `collection:<id>` /
   `read-list:<id>`, Suwayomi `category:<id>`, OPDS a base64url feed URL) and decodes it back in
   `list_items`. Reading lists hold chapters/books, so their items are the distinct series those come
   from, in list order, paginated client-side (`providers::paginate`). Collections and reading lists
   are listed best-effort: if the server refuses them, the libraries still are. Adding a backend means
   implementing the adapter traits plus a `providers::Provider` impl and a `providers::resolve` arm —
   don't let backend-specific types leak past the adapter layer into `repository`/`screen` code.

**Reading history.** While the reader is open, every `ReaderEvent::ShowingPage` it reports is
forwarded by `reader_launch` to `reading_progress::report_progress`, which runs alongside the page
downloads on the session's runtime. That module is backend-agnostic: it turns each page into a
`providers::ReadingProgress` (the `PageRef` plus its chapter's page count) and calls
`Provider::update_reading_progress`. The trait's default does nothing. A backend that keeps reading
history overrides it, so far only Kavita (`POST /api/Reader/progress`, with the chapter's
volume/series/library ids looked up once per chapter via `/api/Reader/chapter-info`; the last page
is saved as the page count, which marks the chapter read). A backlog of page flips is collapsed to
the last page of each chapter run, and failures are only logged.

**Cargo features.** Each remote provider is a feature of the `app` crate: `kavita`, `komga`, `opds`,
`suwayomi`, all on by default. A feature pulls in its client crate, its `providers::<name>` and
`screen::integration::<name>` modules, its `Step`/`Message`/`ResolvedProvider` variants, its match arm
in `providers::resolve`, and its tests. `IntegrationType` keeps every
variant regardless, so a database written by a fuller build still loads; a left-out type resolves to
`providers::not_built_in`. Local folders (`IntegrationType::LocalFolder`, `providers::file_system`) are always built in. Check new provider code with
`cargo check -p komasabi --tests --no-default-features --features=<name>` as well as the default
build.

**Persistence (`src/repository/`, `src/models/`, `src/database.rs`, `src/schema.rs`).** Diesel +
`diesel-async`'s `SyncConnectionWrapper` over sqlite; `database::connect()` opens a fresh connection
per call (no pooling) using `DATABASE_URL`, or `komasabi.sqlite3` in the platform data dir
(`ProjectDirs::data_dir()`) when that's unset, as in a release install. Migrations are embedded in
the binary (`embed_migrations!`, with `app/build.rs` rebuilding on changes) and run once per process
before the first connection, so new migrations reach users without `diesel`. Repository functions all return `Result<T, String>`
(errors stringified via `.map_err(|e| e.to_string())`) since they're consumed directly as iced
`Task::perform` futures, where the error type flows into a `Message` variant. `models::AsyncModel<T, E>`
is the standard wrapper screens use to represent not-loaded/loading/loaded/error state for data
fetched this way.

The app tables are `integrations` (sources) and `libraries` (the user's own Library, listed by
`screen/library.rs`; sources don't sync anything into it, and `LibraryItem` is its counterpart to
`CollectionItem`). The legacy `servers` table is unused. The enum columns (`IntegrationType`,
`LibraryType`) use `diesel_derive_enum::DbEnum`, stored as `Text` in sqlite — see the schema-regeneration note above when touching these.

**OPDS provider (`providers::opds`).** Added via the integrations screen's "OPDS" type (catalog URL +
optional basic-auth username/password, stored in `integrations.username`/`password`). OPDS has no
numeric ids, so collection/item/entry ids are `OpdsId`s: a feed URL, or (feed page URL, index, key) for a
publication, base64url-encoded. `list_collections` turns each root navigation link into a catalog
collection, but expands a "catalog of catalogs" (e.g. Kavita's "All Libraries") into one library
collection per entry. A collection's items are its feed's navigation links + publications; an item's entries are the publications of the
feed it links to. Reading goes only through the OPDS provider and `opds-client`, with no other
provider's code involved. The file is downloaded from the publication's `…/acquisition/open-access`
link (plain `…/acquisition` as a fallback) and fully extracted, CBZ or CBR, by `providers::opds::archive`
into a cached pages folder. The reader is then served from that folder. Page streams (OPDS-PSE) are
not used for reading. Live check: `OPDS_LIVE_URL=<catalog> cargo test -p komasabi opds_live -- --ignored --nocapture`
(a Kavita catalog is `http://<host>/api/opds/<api-key>`; Kavita only serves OPDS 1.2).

**Komga provider (`providers::komga`).** Added via the integrations screen's "Komga" type (server URL
plus either an API key, stored in `integrations.api_key`, or the account email/password, stored in
`username`/`password`; the API key wins when both are set). Komga's libraries, collections and
read lists are the source's collections; its series → book map onto item → entry, using Komga's string
ids as-is. Every book is a chapter card
labelled with its title, since Komga doesn't say whether a book is a volume or an issue. Reading
uses the page endpoint (`/api/v1/books/{id}/pages/{n}`, 1-based), so nothing is downloaded up front.
Text-only EPUBs and books Komga hasn't analyzed yet are refused. Live check:
`KOMGA_LIVE_URL=<server> KOMGA_LIVE_API_KEY=<key> cargo test -p komasabi komga_live -- --ignored --nocapture`.

**Suwayomi provider (`providers::suwayomi`).** Added via the integrations screen's "Suwayomi" type
(server URL plus optional basic-auth username/password; Suwayomi runs unauthenticated by default).
Categories map to collections (`category:{id}`), mangas to items and chapters to chapter entries.
An empty default category is skipped. A category's
mangas aren't paged by the server, so `list_items` slices the full list. A chapter entry's id is
`{manga_id}-{chapter_id}`. Chapters have no images of their own, so a chapter card shows the manga
cover. Reading runs the `fetchChapterPages` mutation, which makes the server fetch the page list from
the source. The pages are then fetched from `/api/v1/manga/{manga}/chapter/{sourceOrder}/page/{n}`
(0-based), so a `PageRef.chapter_id` is `{manga_id}-{source_order}`. Live check:
`SUWAYOMI_LIVE_URL=<server> cargo test -p komasabi suwayomi_live -- --ignored --nocapture`.

## Architecture (`kavita-client` crate)

A thin `reqwest`-based wrapper over the Kavita REST API. `KavitaClient` (`src/client.rs`) holds the
base URL and an `http_client: reqwest::Client`; endpoint groups (`account`, `chapter`, `collection`, `library`, `reader`, `reading_list`,
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

## Architecture (`suwayomi-client` crate)

`SuwayomiClient` (`src/client.rs`, endpoints in `src/client/`) takes an `Auth` (basic or none). Data
comes from GraphQL (`POST /api/graphql`, its own `Error` type: HTTP or GraphQL `errors`). REST is
deprecated upstream, so only the image paths GraphQL hands out are used: manga thumbnails and chapter
pages. The default category (id `0`) is virtual, since it holds mangas that have no category. Filtering
`mangas` by `categoryId: 0` returns nothing, so `category(id).mangas` is used instead.
Pages are addressed by a chapter's `sourceOrder`, not its server-wide `id`.
`cargo run -p suwayomi-client --example browse -- <server-url> [manga-id [chapter-id]]` prints what a
live server serves.
