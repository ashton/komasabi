# Changelog

All notable changes to Komasabi are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and this project
adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [0.1.3] - 2026-11-02

### Added

- Reading progress syncs to Kavita. Each page the reader shows is reported to the provider, and the
  last page of a chapter marks it read. Other backends keep ignoring progress for now.
- Touch and click zones in the reader: tap the left third of the page to go back, the right third
  to go forward.

### Fixed

- Touch-drag scrolling now works when the drag starts on a series/chapter card or a list row, with
  flick momentum.
- Opening a volume or chapter on Windows failed because cache file names contained `:`. Cache names
  are now escaped so every platform accepts them.

## [0.1.2] - 2026-10-21

### Added

- `.rpm` and Arch (pacman) packages for Linux releases. Every Linux package installs a `.desktop`
  entry.

### Changed

- Linux release binaries are built on Ubuntu 22.04, so they only need glibc 2.35.

### Fixed

- The release build cache is keyed by runner image.

## [0.1.1] - 2026-10-21

### Added

- Installers for every platform: `.deb` and `.AppImage` on Linux, an NSIS setup `.exe` on Windows,
  and a `.dmg` (plus a zipped `.app`) on macOS.
- The Windows executable carries the Komasabi icon.

## [0.1.0] - 2026-10-21

First release.

### Added

- Aggregate comic/manga libraries from Kavita, Komga, Suwayomi, OPDS catalogs (OPDS 2 and 1.2) and
  local folders into one place.
- Browse libraries and series with cover thumbnails and infinite scrolling, and open a series to
  see its volumes and chapters.
- A streaming page reader, shipped inside the `komasabi` binary.
- Each remote provider is a cargo feature (`kavita`, `komga`, `opds`, `suwayomi`), all on by
  default.
- A self-contained SQLite database in the platform data directory, with migrations embedded in the
  binary.
- Release binaries for Linux, Windows and a universal macOS build.
- Standalone client crates: `kavita-client`, `komga-client`, `opds-client`, `suwayomi-client`.

[0.1.3]: https://github.com/ashton/komasabi/compare/v0.1.2...v0.1.3
[0.1.2]: https://github.com/ashton/komasabi/compare/v0.1.1...v0.1.2
[0.1.1]: https://github.com/ashton/komasabi/compare/v0.1.0...v0.1.1
[0.1.0]: https://github.com/ashton/komasabi/releases/tag/v0.1.0
