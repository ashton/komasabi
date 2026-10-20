//! OPDS 2 (`application/opds+json`): the models mirror the JSON, so this is a
//! straight serde decode.

use crate::error::Error;
use crate::models::feed::Feed;

pub fn parse_feed(body: &[u8]) -> Result<Feed, Error> {
    Ok(serde_json::from_slice(body)?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::rel;

    /// Trimmed from https://test.opds.io/2.0/home.json, the OPDS 2 reference
    /// test catalog.
    const HOME: &str = r#"{
      "metadata": {"title": "OPDS 2.0 Test Catalog"},
      "links": [
        {"rel": "self", "href": "https://test.opds.io/2.0/home.json", "type": "application/opds+json"}
      ],
      "navigation": [
        {"href": "https://test.opds.io/2.0/navigation.json", "title": "Explore (Navigation Feed)", "type": "application/opds+json"},
        {"href": "https://test.opds.io/2.0/publications.json", "title": "French Books (Publications Feed)", "type": "application/opds+json"}
      ],
      "groups": [
        {
          "metadata": {"title": "French Classics", "numberOfItems": 5},
          "links": [{"rel": "self", "href": "https://test.opds.io/2.0/publications.json", "type": "application/opds+json"}],
          "publications": [
            {
              "metadata": {
                "@type": "http://schema.org/Book",
                "title": {"en": "Title in multiple languages", "fr": "Titre en plusieurs langues"},
                "author": {"name": "Jules Verne", "sortAs": "Verne, Jules"},
                "identifier": "http://fr.feedbooks.com/book/1474/voyage-au-centre-de-la-terre",
                "numberOfPages": 251
              },
              "links": [
                {"rel": "http://opds-spec.org/acquisition/open-access", "href": "https://test.opds.io/assets/centredelaterre/file.epub", "type": "application/epub+zip"}
              ],
              "images": [
                {"href": "https://test.opds.io/assets/centredelaterre/small.jpg", "type": "image/jpeg", "height": 110, "width": 146}
              ]
            }
          ]
        }
      ]
    }"#;

    #[test]
    fn parses_navigation_groups_and_publications() {
        let feed = parse_feed(HOME.as_bytes()).unwrap();

        assert_eq!(feed.metadata.title, "OPDS 2.0 Test Catalog");
        assert_eq!(feed.navigation.len(), 2);
        assert_eq!(
            feed.navigation[1].title(),
            "French Books (Publications Feed)"
        );
        assert_eq!(feed.groups[0].metadata.number_of_items, Some(5));
        assert!(feed.link(rel::SELF).is_some());

        let publication = feed.all_publications().next().unwrap();
        assert_eq!(publication.metadata.title, "Title in multiple languages");
        assert_eq!(publication.page_count(), Some(251));
        assert_eq!(publication.acquisitions().count(), 1);
        assert!(publication.cover().is_some());
    }

    #[test]
    fn tolerates_a_bare_feed() {
        let feed = parse_feed(br#"{"metadata": {"title": "Empty"}}"#).unwrap();

        assert!(feed.is_empty());
    }
}
