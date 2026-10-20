//! A small, UI-agnostic client for [OPDS](https://opds.io/) catalogs.
//!
//! Both catalog flavours in the wild are supported behind one model:
//!
//! - **OPDS 2** (`application/opds+json`) is parsed natively — the
//!   [`models`] mirror its JSON shape (feeds with `navigation`,
//!   `publications` and `groups`, publications with `metadata`/`links`/
//!   `images`).
//! - **OPDS 1.2** (Atom XML) is converted into that same model: navigation
//!   entries become [`Navigation`](models::feed::Navigation) links and
//!   acquisition entries become [`Publication`](models::publication::Publication)s.
//!   The OPDS Page Streaming Extension (`pse:count`) is carried over as
//!   [`LinkProperties::page_count`](models::link::LinkProperties::page_count).
//!
//! Which parser runs is decided per response (see [`format::parse_feed`]), so a
//! single catalog may even mix the two. Every `href` in a parsed feed is
//! resolved to an absolute URL against the document it came from, so callers
//! never deal with relative links.

pub mod client;
pub mod error;
pub mod format;
pub mod models;

pub use client::OpdsClient;
pub use error::Error;
