use serde::Deserialize;

use super::{link::Link, localized, media_type, rel};

#[derive(Clone, Debug, Default, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Publication {
    #[serde(default)]
    pub metadata: PublicationMetadata,
    #[serde(default)]
    pub links: Vec<Link>,
    /// Cover images, largest first where the server distinguishes sizes.
    #[serde(default)]
    pub images: Vec<Link>,
}

#[derive(Clone, Debug, Default, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PublicationMetadata {
    #[serde(default, deserialize_with = "localized::identifier")]
    pub identifier: Option<String>,
    #[serde(default, deserialize_with = "localized::string")]
    pub title: String,
    #[serde(default, deserialize_with = "localized::option")]
    pub subtitle: Option<String>,
    #[serde(default, deserialize_with = "localized::option")]
    pub description: Option<String>,
    #[serde(default)]
    pub number_of_pages: Option<u32>,
}

impl Publication {
    /// Links that acquire the publication (download, borrow, buy, …).
    pub fn acquisitions(&self) -> impl Iterator<Item = &Link> {
        self.links
            .iter()
            .filter(|link| link.has_rel_prefix(rel::ACQUISITION))
    }

    /// The OPDS-PSE page stream link, if the server streams pages.
    pub fn page_stream(&self) -> Option<&Link> {
        self.links
            .iter()
            .find(|link| link.has_rel(rel::PAGE_STREAM))
    }

    /// A link to a Readium Web Publication / Divina manifest for this
    /// publication, whose `readingOrder` lists its pages.
    pub fn manifest_link(&self) -> Option<&Link> {
        self.links.iter().find(|link| {
            link.is_type(media_type::DIVINA_JSON) || link.is_type(media_type::WEBPUB_JSON)
        })
    }

    /// Links that download the publication's file for free: the
    /// `…/acquisition/open-access` ones, or — for catalogs that don't
    /// distinguish (most OPDS 2 servers) — plain `…/acquisition` ones. Borrow,
    /// buy, sample and subscribe acquisitions are never included.
    pub fn open_access(&self) -> Vec<&Link> {
        let open_access: Vec<&Link> = self
            .links
            .iter()
            .filter(|link| link.has_rel(rel::OPEN_ACCESS))
            .collect();

        if !open_access.is_empty() {
            return open_access;
        }

        self.links
            .iter()
            .filter(|link| link.has_rel(rel::ACQUISITION))
            .collect()
    }

    /// The best cover image: the first full-size image, else any image.
    pub fn cover(&self) -> Option<&Link> {
        self.images
            .iter()
            .find(|image| !image.has_rel(rel::THUMBNAIL))
            .or_else(|| self.images.first())
    }

    /// The page count, from the page stream (`pse:count`) or the metadata.
    pub fn page_count(&self) -> Option<u32> {
        self.page_stream()
            .and_then(|link| link.properties.page_count)
            .or_else(|| {
                self.acquisitions()
                    .find_map(|link| link.properties.page_count)
            })
            .or(self.metadata.number_of_pages)
    }

    /// A stable key for finding this publication again in its feed: its
    /// identifier, else its self link, else its title.
    pub fn key(&self) -> String {
        self.metadata
            .identifier
            .clone()
            .or_else(|| {
                self.links
                    .iter()
                    .find(|link| link.has_rel(rel::SELF))
                    .map(|link| link.href.clone())
            })
            .unwrap_or_else(|| self.metadata.title.clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn link(rel: &str, href: &str) -> Link {
        Link {
            href: href.into(),
            rel: vec![rel.into()],
            ..Link::default()
        }
    }

    #[test]
    fn open_access_prefers_open_access_links() {
        let publication = Publication {
            links: vec![
                link(rel::ACQUISITION, "/plain.cbz"),
                link("http://opds-spec.org/acquisition/borrow", "/borrow"),
                link(rel::OPEN_ACCESS, "/free.cbr"),
            ],
            ..Publication::default()
        };

        let hrefs: Vec<&str> = publication
            .open_access()
            .iter()
            .map(|l| l.href.as_str())
            .collect();

        assert_eq!(hrefs, vec!["/free.cbr"]);
    }

    #[test]
    fn open_access_falls_back_to_plain_acquisitions_but_never_paid_ones() {
        let publication = Publication {
            links: vec![
                link("http://opds-spec.org/acquisition/buy", "/buy"),
                link(rel::ACQUISITION, "/plain.cbz"),
            ],
            ..Publication::default()
        };

        let hrefs: Vec<&str> = publication
            .open_access()
            .iter()
            .map(|l| l.href.as_str())
            .collect();

        assert_eq!(hrefs, vec!["/plain.cbz"]);
    }
}
