//! A small, UI-agnostic client for the [Suwayomi](https://github.com/Suwayomi/Suwayomi-Server)
//! server API.
//!
//! Data is queried through Suwayomi's GraphQL endpoint (`/api/graphql`), its
//! primary API. Images (manga thumbnails and chapter pages) are served from the
//! `/api/v1` paths the GraphQL API itself hands out. Suwayomi's library is
//! organised in categories, each holding mangas, whose chapters are read page
//! by page. A server runs without authentication by default, or behind HTTP
//! basic auth — see [`client::Auth`].

pub mod client;
pub mod models;
