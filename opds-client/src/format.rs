//! Turning response bodies into [`Feed`]s, whichever OPDS version they speak.

mod atom;
mod json;

use url::Url;

use crate::error::Error;
use crate::models::{feed::Feed, link::Link, manifest::Manifest};

/// The two wire formats a catalog can answer in.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Format {
    /// OPDS 2: `application/opds+json`.
    Json,
    /// OPDS 1.x: Atom XML.
    Atom,
}

/// Decides how to parse a body. The body itself is authoritative — servers
/// routinely mislabel feeds (Kavita sends Atom as `application/xml`, others
/// send OPDS 2 as plain `application/json`) — so its first significant byte
/// wins and the content type only breaks a tie.
pub fn detect(body: &[u8], content_type: Option<&str>) -> Option<Format> {
    let first = body
        .iter()
        .copied()
        // Skip whitespace and a UTF-8 byte order mark.
        .find(|byte| !byte.is_ascii_whitespace() && !matches!(byte, 0xEF | 0xBB | 0xBF));

    match first {
        Some(b'{') => Some(Format::Json),
        Some(b'<') => Some(Format::Atom),
        _ => {
            let essence = crate::models::link::essence(content_type?);
            if essence.ends_with("json") {
                Some(Format::Json)
            } else if essence.ends_with("xml") {
                Some(Format::Atom)
            } else {
                None
            }
        }
    }
}

/// Parses a feed fetched from `url`, resolving every href against it.
pub fn parse_feed(body: &[u8], content_type: Option<&str>, url: &Url) -> Result<Feed, Error> {
    let mut feed = match detect(body, content_type) {
        Some(Format::Json) => json::parse_feed(body)?,
        Some(Format::Atom) => atom::parse_feed(body)?,
        None => {
            return Err(Error::UnsupportedFormat(
                content_type.unwrap_or("unknown").to_string(),
            ));
        }
    };

    resolve_feed(&mut feed, url);
    Ok(feed)
}

/// Parses a Readium Web Publication / Divina manifest fetched from `url`.
pub fn parse_manifest(body: &[u8], url: &Url) -> Result<Manifest, Error> {
    let mut manifest: Manifest = serde_json::from_slice(body)?;

    // A manifest's own `self` link is the base its resources resolve against,
    // when present; otherwise the URL it was fetched from.
    let base = manifest
        .links
        .iter()
        .find(|link| link.has_rel(crate::models::rel::SELF))
        .and_then(|link| url.join(&link.href).ok())
        .unwrap_or_else(|| url.clone());

    for link in manifest.links.iter_mut().chain(&mut manifest.reading_order) {
        resolve_link(link, &base);
    }

    Ok(manifest)
}

fn resolve_feed(feed: &mut Feed, base: &Url) {
    for link in &mut feed.links {
        resolve_link(link, base);
    }

    let navigation = feed.navigation.iter_mut().chain(
        feed.groups
            .iter_mut()
            .flat_map(|group| group.navigation.iter_mut()),
    );
    for entry in navigation {
        resolve_link(&mut entry.link, base);
        for image in &mut entry.images {
            resolve_link(image, base);
        }
    }

    let publications = feed.publications.iter_mut().chain(
        feed.groups
            .iter_mut()
            .flat_map(|group| group.publications.iter_mut()),
    );
    for publication in publications {
        for link in publication.links.iter_mut().chain(&mut publication.images) {
            resolve_link(link, base);
        }
    }

    for group in &mut feed.groups {
        for link in &mut group.links {
            resolve_link(link, base);
        }
    }
}

fn resolve_link(link: &mut Link, base: &Url) {
    if let Ok(resolved) = base.join(&link.href) {
        link.href = resolved.to_string();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detect_prefers_the_body_over_a_mislabelled_content_type() {
        assert_eq!(
            detect(
                b"  <?xml version=\"1.0\"?><feed/>",
                Some("application/json")
            ),
            Some(Format::Atom)
        );
        assert_eq!(
            detect(b"\n{\"metadata\":{}}", Some("text/xml")),
            Some(Format::Json)
        );
    }

    #[test]
    fn detect_falls_back_to_the_content_type() {
        assert_eq!(
            detect(b"", Some("application/opds+json")),
            Some(Format::Json)
        );
        assert_eq!(
            detect(b"", Some("application/atom+xml;profile=opds-catalog")),
            Some(Format::Atom)
        );
        assert_eq!(detect(b"hello", Some("text/plain")), None);
    }

    #[test]
    fn parse_feed_resolves_relative_hrefs_against_the_feed_url() {
        let url = Url::parse("http://server/opds/catalog").unwrap();
        let body = br#"{
            "metadata": {"title": "Root"},
            "links": [{"rel": "next", "href": "catalog?page=2"}],
            "navigation": [{"href": "/opds/series", "title": "Series"}],
            "publications": [{
                "metadata": {"title": "Book"},
                "links": [{"rel": "http://opds-spec.org/acquisition", "href": "book.cbz"}],
                "images": [{"href": "//cdn.example/cover.jpg"}]
            }]
        }"#;

        let feed = parse_feed(body, None, &url).unwrap();

        assert_eq!(
            feed.next().unwrap().href,
            "http://server/opds/catalog?page=2"
        );
        assert_eq!(feed.navigation[0].link.href, "http://server/opds/series");
        assert_eq!(
            feed.publications[0].links[0].href,
            "http://server/opds/book.cbz"
        );
        assert_eq!(
            feed.publications[0].images[0].href,
            "http://cdn.example/cover.jpg"
        );
    }

    #[test]
    fn parse_manifest_resolves_reading_order_and_keeps_only_images_as_pages() {
        let url = Url::parse("http://server/pub/1/manifest.json").unwrap();
        let body = br#"{
            "metadata": {"title": "Comic"},
            "readingOrder": [
                {"href": "p1.jpg", "type": "image/jpeg"},
                {"href": "p2.png", "type": "image/png"},
                {"href": "notes.html", "type": "text/html"}
            ]
        }"#;

        let manifest = parse_manifest(body, &url).unwrap();
        let pages: Vec<&str> = manifest.pages().map(|link| link.href.as_str()).collect();

        assert_eq!(
            pages,
            vec!["http://server/pub/1/p1.jpg", "http://server/pub/1/p2.png"]
        );
    }

    #[test]
    fn parse_feed_rejects_non_opds_bodies() {
        let url = Url::parse("http://server/").unwrap();

        assert!(matches!(
            parse_feed(b"hello", Some("text/plain"), &url),
            Err(Error::UnsupportedFormat(_))
        ));
    }
}
