//! Bundled SVG icons, embedded in the binary at compile time.
//!
//! Embedding (instead of loading `assets/*.svg` at runtime) keeps rendering
//! independent of the process' working directory and turns a missing or renamed
//! asset into a build error rather than a blank widget.

pub const APP_ICON: &[u8] = include_bytes!("../assets/komasabi-icon.png");

pub const BOOK_OPEN: &[u8] = include_bytes!("../assets/book-open.svg");
pub const PLUG_CONNECT: &[u8] = include_bytes!("../assets/plug-connect.svg");

pub const KAVITA: &[u8] = include_bytes!("../assets/kavita.svg");
pub const KOMGA: &[u8] = include_bytes!("../assets/komga.svg");
pub const SUWAYOMI: &[u8] = include_bytes!("../assets/suwayomi.svg");
pub const OPDS: &[u8] = include_bytes!("../assets/opds.svg");

pub const MANGA: &[u8] = include_bytes!("../assets/manga.svg");
pub const COMIC: &[u8] = include_bytes!("../assets/comic.svg");
pub const EBOOK: &[u8] = include_bytes!("../assets/ebook.svg");
