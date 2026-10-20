//! A small, UI-agnostic client for the [Komga](https://komga.org/) REST API
//! (`/api/v1`, and `/api/v2` for users).
//!
//! Komga's hierarchy is library → series → book, where a book is one file
//! (a CBZ/CBR/PDF/EPUB) whose pages are served as individual images. Every
//! endpoint is authenticated, either with an API key (`X-API-Key` header) or
//! with HTTP basic auth — see [`client::Auth`].

pub mod client;
pub mod models;
