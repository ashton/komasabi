//! OPDS 1.x (Atom XML), converted into the OPDS 2 model.
//!
//! OPDS 1 has no separate navigation/publication collections: every `<entry>`
//! is one or the other depending on its links. An entry with an acquisition or
//! page-stream link is a [`Publication`]; any other entry linking somewhere is
//! a [`Navigation`] entry. Image links (`…/image`, `…/image/thumbnail`) become
//! the entry's images in either case.
//!
//! Elements are matched by local name only, so namespace prefixes (`pse:`,
//! `p5:`, `opensearch:`, `dc:`) don't matter.

use quick_xml::Reader;
use quick_xml::events::{BytesStart, Event};

use crate::error::Error;
use crate::models::{
    feed::{Feed, Navigation},
    link::{Link, LinkProperties},
    publication::{Publication, PublicationMetadata},
    rel,
};

#[derive(Default)]
struct Entry {
    id: Option<String>,
    title: String,
    summary: Option<String>,
    content: Option<String>,
    links: Vec<Link>,
}

pub fn parse_feed(body: &[u8]) -> Result<Feed, Error> {
    let mut reader = Reader::from_reader(body);
    let mut buffer = Vec::new();

    let mut feed = Feed::default();
    let mut entry: Option<Entry> = None;
    // Open element local names, so an element's parent is known when it closes.
    let mut stack: Vec<String> = Vec::new();
    let mut text = String::new();
    let mut saw_feed = false;

    loop {
        match reader.read_event_into(&mut buffer)? {
            Event::Start(element) => {
                let name = local_name(&element);
                text.clear();

                match name.as_str() {
                    "feed" => saw_feed = true,
                    "entry" => entry = Some(Entry::default()),
                    "link" => push_link(&mut feed, &mut entry, parse_link(&element)?),
                    _ => {}
                }

                stack.push(name);
            }
            Event::Empty(element) => {
                if local_name(&element) == "link" {
                    push_link(&mut feed, &mut entry, parse_link(&element)?);
                }
            }
            Event::Text(content) => {
                text.push_str(&content.decode().map_err(|e| Error::Xml(e.to_string()))?);
            }
            Event::CData(content) => {
                text.push_str(&content.decode().map_err(|e| Error::Xml(e.to_string()))?);
            }
            Event::GeneralRef(reference) => {
                if let Some(character) = reference.resolve_char_ref()? {
                    text.push(character);
                } else {
                    let name = reference.decode().map_err(|e| Error::Xml(e.to_string()))?;
                    match quick_xml::escape::resolve_xml_entity(&name) {
                        Some(resolved) => text.push_str(resolved),
                        None => {
                            text.push('&');
                            text.push_str(&name);
                            text.push(';');
                        }
                    }
                }
            }
            Event::End(_) => {
                let Some(name) = stack.pop() else {
                    continue;
                };
                let parent = stack.last().map(String::as_str);
                let value = text.trim().to_string();

                match (parent, name.as_str()) {
                    (_, "entry") => {
                        if let Some(finished) = entry.take() {
                            push_entry(&mut feed, finished);
                        }
                    }
                    (Some("entry"), field) => {
                        if let Some(current) = entry.as_mut() {
                            match field {
                                "id" => current.id = Some(value),
                                "title" => current.title = value,
                                "summary" => current.summary = Some(value),
                                "content" => current.content = Some(value),
                                _ => {}
                            }
                        }
                    }
                    (Some("feed"), "title") => feed.metadata.title = value,
                    (Some("feed"), "totalResults") => {
                        feed.metadata.number_of_items = value.parse().ok();
                    }
                    (Some("feed"), "itemsPerPage") => {
                        feed.metadata.items_per_page = value.parse().ok();
                    }
                    _ => {}
                }

                text.clear();
            }
            Event::Eof => break,
            _ => {}
        }

        buffer.clear();
    }

    if !saw_feed {
        return Err(Error::Xml(
            "document has no Atom <feed> element".to_string(),
        ));
    }

    Ok(feed)
}

fn local_name(element: &BytesStart) -> String {
    String::from_utf8_lossy(element.local_name().as_ref()).into_owned()
}

fn parse_link(element: &BytesStart) -> Result<Link, Error> {
    let mut link = Link {
        properties: LinkProperties::default(),
        ..Link::default()
    };

    for attribute in element.attributes() {
        let attribute = attribute.map_err(|e| Error::Xml(e.to_string()))?;
        let value = attribute
            .normalized_value(quick_xml::XmlVersion::Implicit1_0)
            .map_err(|e| Error::Xml(e.to_string()))?
            .into_owned();

        match attribute.key.local_name().as_ref() {
            b"href" => link.href = value,
            b"rel" => link.rel = vec![value],
            b"type" => link.media_type = Some(value),
            b"title" => link.title = Some(value),
            // OPDS-PSE `pse:count`, whatever prefix the server bound it to.
            b"count" => link.properties.page_count = value.parse().ok(),
            _ => {}
        }
    }

    // Atom's default relation.
    if link.rel.is_empty() {
        link.rel = vec!["alternate".to_string()];
    }

    link.templated = link.href.contains("{pageNumber}");

    Ok(link)
}

fn push_link(feed: &mut Feed, entry: &mut Option<Entry>, link: Link) {
    match entry {
        Some(entry) => entry.links.push(link),
        None => feed.links.push(link),
    }
}

fn is_image(link: &Link) -> bool {
    link.has_rel(rel::IMAGE) || link.has_rel(rel::THUMBNAIL)
}

fn push_entry(feed: &mut Feed, entry: Entry) {
    let (mut images, links): (Vec<Link>, Vec<Link>) = entry.links.into_iter().partition(is_image);
    // Full-size artwork before thumbnails.
    images.sort_by_key(|image| image.has_rel(rel::THUMBNAIL));

    let is_publication = links
        .iter()
        .any(|link| link.has_rel_prefix(rel::ACQUISITION) || link.has_rel(rel::PAGE_STREAM));

    if is_publication {
        feed.publications.push(Publication {
            metadata: PublicationMetadata {
                identifier: entry.id,
                title: entry.title,
                subtitle: None,
                // Some servers (Kavita) put the file's media type in
                // `<content>`, so it's only a fallback description.
                description: entry.summary.or(entry
                    .content
                    .filter(|content| !looks_like_media_type(content))),
                number_of_pages: None,
            },
            links,
            images,
        });
        return;
    }

    let target = links
        .iter()
        .find(|link| link.has_rel(rel::SUBSECTION) || link.is_type(crate::models::media_type::ATOM))
        .or_else(|| links.first())
        .cloned();

    if let Some(mut link) = target {
        link.title = Some(entry.title);
        feed.navigation.push(Navigation { link, images });
    }
}

fn looks_like_media_type(value: &str) -> bool {
    !value.contains(char::is_whitespace) && value.contains('/')
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Trimmed from a live Kavita (OPDS 1.2) navigation feed.
    const NAVIGATION: &str = r#"<?xml version="1.0" encoding="utf-8"?>
<feed xmlns="http://www.w3.org/2005/Atom">
  <id>libraries</id>
  <title>All Libraries</title>
  <author><name>Kavita</name></author>
  <link rel="self" type="application/atom+xml;profile=opds-catalog;kind=acquisition" href="/api/opds/key/libraries" />
  <link rel="start" type="application/atom+xml;profile=opds-catalog;kind=navigation" href="/api/opds/key" />
  <entry>
    <id>1</id>
    <title>DC &amp; Friends</title>
    <link rel="subsection" type="application/atom+xml;profile=opds-catalog;kind=navigation" href="/api/opds/key/libraries/1" />
    <link rel="http://opds-spec.org/image/thumbnail" type="image/jpeg" href="/thumb?libraryId=1&amp;apiKey=key" />
    <link rel="http://opds-spec.org/image" type="image/jpeg" href="/cover?libraryId=1&amp;apiKey=key" />
  </entry>
</feed>"#;

    /// Trimmed from a live Kavita series feed: publications with an
    /// acquisition link and an OPDS-PSE page stream.
    const ACQUISITION: &str = r#"<?xml version="1.0" encoding="utf-8"?>
<feed xmlns="http://www.w3.org/2005/Atom">
  <id>series-4</id>
  <title>Action Comics - Storyline</title>
  <totalResults xmlns="http://a9.com/-/spec/opensearch/1.1/">29</totalResults>
  <itemsPerPage xmlns="http://a9.com/-/spec/opensearch/1.1/">20</itemsPerPage>
  <link rel="next" type="application/atom+xml;profile=opds-catalog;kind=navigation" href="/api/opds/key/series/4?pageNumber=2" />
  <link rel="http://opds-spec.org/image" type="image/jpeg" href="/api/image/series-cover?seriesId=4" />
  <entry>
    <id>75</id>
    <title>Action Comics - Volume 957</title>
    <summary>File Type: x-cbr - 43.07 MB</summary>
    <content type="text">application/x-cbr</content>
    <link rel="http://opds-spec.org/image" type="image/jpeg" href="/api/image/chapter-cover?chapterId=75" />
    <link rel="http://opds-spec.org/acquisition/open-access" type="application/x-cbr" href="/download/v957.cbr" p5:count="29" xmlns:p5="http://vaemendis.net/opds-pse/ns" />
    <link rel="http://vaemendis.net/opds-pse/stream" type="image/jpeg" href="/api/opds/key/image?chapterId=75&amp;pageNumber={pageNumber}" p5:count="29" xmlns:p5="http://vaemendis.net/opds-pse/ns" />
  </entry>
</feed>"#;

    #[test]
    fn navigation_entries_become_navigation_links_with_images() {
        let feed = parse_feed(NAVIGATION.as_bytes()).unwrap();

        assert_eq!(feed.metadata.title, "All Libraries");
        assert!(feed.link(rel::START).is_some());
        assert!(feed.publications.is_empty());

        let entry = &feed.navigation[0];
        assert_eq!(entry.title(), "DC & Friends");
        assert_eq!(entry.link.href, "/api/opds/key/libraries/1");
        assert!(entry.link.has_rel(rel::SUBSECTION));
        // The full-size image sorts ahead of the thumbnail.
        assert_eq!(entry.cover().unwrap().href, "/cover?libraryId=1&apiKey=key");
        assert_eq!(entry.images.len(), 2);
    }

    #[test]
    fn acquisition_entries_become_publications_with_page_streams() {
        let feed = parse_feed(ACQUISITION.as_bytes()).unwrap();

        assert_eq!(feed.metadata.number_of_items, Some(29));
        assert_eq!(feed.metadata.items_per_page, Some(20));
        assert_eq!(
            feed.next().unwrap().href,
            "/api/opds/key/series/4?pageNumber=2"
        );
        assert_eq!(
            feed.cover().unwrap().href,
            "/api/image/series-cover?seriesId=4"
        );
        assert!(feed.navigation.is_empty());

        let publication = &feed.publications[0];
        assert_eq!(publication.metadata.identifier.as_deref(), Some("75"));
        assert_eq!(publication.metadata.title, "Action Comics - Volume 957");
        assert_eq!(
            publication.metadata.description.as_deref(),
            Some("File Type: x-cbr - 43.07 MB")
        );
        assert_eq!(publication.page_count(), Some(29));

        let stream = publication.page_stream().unwrap();
        assert!(stream.templated);
        assert_eq!(
            stream.expand_page(4),
            "/api/opds/key/image?chapterId=75&pageNumber=4"
        );
        assert_eq!(
            publication.cover().unwrap().href,
            "/api/image/chapter-cover?chapterId=75"
        );
    }

    #[test]
    fn content_is_used_as_description_unless_it_is_a_media_type() {
        let feed = parse_feed(
            br#"<feed xmlns="http://www.w3.org/2005/Atom">
              <entry><title>A</title><content>A fine book</content>
                <link rel="http://opds-spec.org/acquisition" href="/a.cbz"/></entry>
              <entry><title>B</title><content>application/x-cbz</content>
                <link rel="http://opds-spec.org/acquisition" href="/b.cbz"/></entry>
            </feed>"#,
        )
        .unwrap();

        assert_eq!(
            feed.publications[0].metadata.description.as_deref(),
            Some("A fine book")
        );
        assert_eq!(feed.publications[1].metadata.description, None);
    }

    #[test]
    fn rejects_xml_that_is_not_an_atom_feed() {
        assert!(parse_feed(b"<html><body>nope</body></html>").is_err());
    }
}
