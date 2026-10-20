pub mod feed;
pub mod link;
pub mod manifest;
pub mod publication;

mod localized;

/// Link relations this crate (and its callers) care about.
pub mod rel {
    pub const SELF: &str = "self";
    pub const NEXT: &str = "next";
    pub const START: &str = "start";
    pub const SUBSECTION: &str = "subsection";
    /// Prefix shared by every acquisition relation (`…/acquisition`,
    /// `…/acquisition/open-access`, `…/acquisition/borrow`, …).
    pub const ACQUISITION: &str = "http://opds-spec.org/acquisition";
    /// A free download of the publication's file.
    pub const OPEN_ACCESS: &str = "http://opds-spec.org/acquisition/open-access";
    pub const IMAGE: &str = "http://opds-spec.org/image";
    pub const THUMBNAIL: &str = "http://opds-spec.org/image/thumbnail";
    /// OPDS Page Streaming Extension: a templated link (`{pageNumber}`) that
    /// serves a publication's pages one image at a time.
    pub const PAGE_STREAM: &str = "http://vaemendis.net/opds-pse/stream";
}

/// Media types used to recognise documents and links.
pub mod media_type {
    pub const OPDS_JSON: &str = "application/opds+json";
    pub const OPDS_PUBLICATION_JSON: &str = "application/opds-publication+json";
    pub const ATOM: &str = "application/atom+xml";
    /// Readium Web Publication / Divina manifests, whose `readingOrder` lists a
    /// publication's resources (for comics: one image per page).
    pub const WEBPUB_JSON: &str = "application/webpub+json";
    pub const DIVINA_JSON: &str = "application/divina+json";
    pub const CBZ: &[&str] = &[
        "application/vnd.comicbook+zip",
        "application/x-cbz",
        "application/zip",
        "application/x-zip-compressed",
    ];
    pub const CBR: &[&str] = &[
        "application/vnd.comicbook-rar",
        "application/x-cbr",
        "application/vnd.rar",
        "application/x-rar-compressed",
    ];
}
