use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use opds_client::models::{feed::Navigation, publication::Publication};

use crate::{
    adapter::{CollectionAdapter, ItemAdapter},
    models::{
        collection::{Collection, CollectionKind},
        collection_item::CollectionItem,
        series_entry::{EntryKind, SeriesEntry},
    },
};

/// What an OPDS-backed collection, item or entry id points at. OPDS has no stable
/// numeric ids — everything is addressed by URL — so ids encode enough to fetch
/// the thing again: a feed's URL, or the URL of the feed page a publication
/// was listed on plus where it sat there.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum OpdsId {
    /// A feed: a collection, or a navigation entry (e.g. a series) within one.
    Feed(String),
    /// A publication, found again by refetching `feed` and taking the
    /// publication at `index` — or, if the feed shifted, the one whose
    /// [`Publication::key`] is `key`.
    Publication {
        feed: String,
        index: usize,
        key: String,
    },
}

impl OpdsId {
    /// Encodes the id as unpadded URL-safe base64, so it is safe as a cache
    /// key / file name segment (no `/`) and round-trips through [`decode`].
    ///
    /// [`decode`]: OpdsId::decode
    pub fn encode(&self) -> String {
        let raw = match self {
            OpdsId::Feed(url) => format!("f\n{url}"),
            OpdsId::Publication { feed, index, key } => format!("p\n{feed}\n{index}\n{key}"),
        };
        URL_SAFE_NO_PAD.encode(raw)
    }

    pub fn decode(id: &str) -> Result<Self, String> {
        let malformed = || format!("malformed OPDS id: {id}");

        let bytes = URL_SAFE_NO_PAD.decode(id).map_err(|_| malformed())?;
        let raw = String::from_utf8(bytes).map_err(|_| malformed())?;

        let mut parts = raw.splitn(4, '\n');
        match (parts.next(), parts.next(), parts.next(), parts.next()) {
            (Some("f"), Some(url), None, None) => Ok(OpdsId::Feed(url.to_string())),
            (Some("p"), Some(feed), Some(index), Some(key)) => Ok(OpdsId::Publication {
                feed: feed.to_string(),
                index: index.parse().map_err(|_| malformed())?,
                key: key.to_string(),
            }),
            _ => Err(malformed()),
        }
    }
}

/// A feed discovered in the catalog that becomes one of the source's
/// collections: a server's library (an entry of a catalog of catalogs, like
/// Kavita's "All Libraries") or any other feed of the root catalog.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OpdsCollection {
    pub title: String,
    pub url: String,
    pub is_library: bool,
}

/// One browsable thing on a catalog page: a link to a deeper feed (typically a
/// series), or a publication listed directly.
#[derive(Clone, Debug, PartialEq)]
pub enum CatalogEntry {
    Navigation(Navigation),
    Publication {
        /// The feed page the publication was listed on.
        feed: String,
        /// Its position among that page's publications.
        index: usize,
        publication: Publication,
    },
}

impl CatalogEntry {
    pub fn id(&self) -> OpdsId {
        match self {
            CatalogEntry::Navigation(navigation) => OpdsId::Feed(navigation.link.href.clone()),
            CatalogEntry::Publication {
                feed,
                index,
                publication,
            } => OpdsId::Publication {
                feed: feed.clone(),
                index: *index,
                key: publication.key(),
            },
        }
    }

    /// The URL of the entry's cover image, when the catalog provides one.
    pub fn cover_href(&self) -> Option<&str> {
        match self {
            CatalogEntry::Navigation(navigation) => navigation.cover(),
            CatalogEntry::Publication { publication, .. } => publication.cover(),
        }
        .map(|link| link.href.as_str())
    }
}

pub struct OpdsCollectionAdapter;

impl CollectionAdapter<OpdsCollection> for OpdsCollectionAdapter {
    fn adapt_collection(source: OpdsCollection) -> Collection {
        let kind = if source.is_library {
            CollectionKind::Library
        } else {
            CollectionKind::Catalog
        };

        Collection::new(OpdsId::Feed(source.url).encode(), source.title, kind)
    }
}

pub struct OpdsItemAdapter;

impl ItemAdapter<CatalogEntry> for OpdsItemAdapter {
    fn adapt_item(source: CatalogEntry) -> CollectionItem {
        let external_id = source.id().encode();

        match source {
            CatalogEntry::Navigation(navigation) => CollectionItem {
                external_id,
                name: navigation.title().to_string(),
                cover: navigation
                    .cover()
                    .map(|link| link.href.clone())
                    .unwrap_or_default(),
                pages: navigation.link.properties.page_count.and_then(to_pages),
            },
            CatalogEntry::Publication { publication, .. } => CollectionItem {
                external_id,
                name: publication.metadata.title.clone(),
                cover: publication
                    .cover()
                    .map(|link| link.href.clone())
                    .unwrap_or_default(),
                pages: publication.page_count().and_then(to_pages),
            },
        }
    }
}

/// A publication as a card in the series-detail grid. OPDS doesn't distinguish
/// volumes from issues, so every publication is a chapter-kind entry labelled
/// with its own title.
pub fn publication_entry(feed: &str, index: usize, publication: &Publication) -> SeriesEntry {
    SeriesEntry {
        external_id: OpdsId::Publication {
            feed: feed.to_string(),
            index,
            key: publication.key(),
        }
        .encode(),
        kind: EntryKind::Chapter,
        label: publication.metadata.title.clone(),
        pages: publication.page_count().and_then(to_pages),
        read_state: Default::default(),
    }
}

fn to_pages(count: u32) -> Option<u16> {
    u16::try_from(count).ok().filter(|pages| *pages > 0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use opds_client::models::{
        link::{Link, LinkProperties},
        publication::PublicationMetadata,
        rel,
    };

    fn publication(identifier: &str, title: &str, pages: u32) -> Publication {
        Publication {
            metadata: PublicationMetadata {
                identifier: Some(identifier.to_string()),
                title: title.to_string(),
                ..PublicationMetadata::default()
            },
            links: vec![Link {
                href: "http://h/image?page={pageNumber}".into(),
                rel: vec![rel::PAGE_STREAM.into()],
                properties: LinkProperties {
                    page_count: Some(pages),
                    ..LinkProperties::default()
                },
                ..Link::default()
            }],
            images: vec![Link {
                href: "http://h/cover.jpg".into(),
                rel: vec![rel::IMAGE.into()],
                ..Link::default()
            }],
        }
    }

    #[test]
    fn ids_round_trip_and_are_safe_as_file_names() {
        let ids = [
            OpdsId::Feed("http://h:5000/api/opds/key/series/4?pageNumber=2".into()),
            OpdsId::Publication {
                feed: "http://h/feed".into(),
                index: 3,
                key: "urn:isbn:978\nodd".into(),
            },
        ];

        for id in ids {
            let encoded = id.encode();
            assert!(!encoded.contains('/') && !encoded.contains('\\'));
            assert_eq!(OpdsId::decode(&encoded).unwrap(), id);
        }
    }

    #[test]
    fn decode_rejects_garbage() {
        assert!(OpdsId::decode("!!!").is_err());
        assert!(OpdsId::decode(&URL_SAFE_NO_PAD.encode("x\nwhat")).is_err());
    }

    #[test]
    fn adapt_collection_encodes_the_feed_url_as_external_id() {
        let library = OpdsCollectionAdapter::adapt_collection(OpdsCollection {
            title: "DC".into(),
            url: "http://h/libraries/1".into(),
            is_library: true,
        });

        assert_eq!(library.name, "DC");
        assert_eq!(library.kind, CollectionKind::Library);
        assert_eq!(
            OpdsId::decode(&library.external_id).unwrap(),
            OpdsId::Feed("http://h/libraries/1".into())
        );

        let feed = OpdsCollectionAdapter::adapt_collection(OpdsCollection {
            title: "On Deck".into(),
            url: "http://h/on-deck".into(),
            is_library: false,
        });
        assert_eq!(feed.kind, CollectionKind::Catalog);
    }

    #[test]
    fn adapt_item_maps_a_navigation_entry_to_a_feed_item() {
        let navigation = Navigation {
            link: Link {
                href: "http://h/series/4".into(),
                title: Some("Action Comics".into()),
                ..Link::default()
            },
            images: vec![Link {
                href: "http://h/series-cover".into(),
                rel: vec![rel::IMAGE.into()],
                ..Link::default()
            }],
        };

        let item = OpdsItemAdapter::adapt_item(CatalogEntry::Navigation(navigation));

        assert_eq!(item.name, "Action Comics");
        assert_eq!(item.cover, "http://h/series-cover");
        assert_eq!(
            OpdsId::decode(&item.external_id).unwrap(),
            OpdsId::Feed("http://h/series/4".into())
        );
    }

    #[test]
    fn adapt_item_maps_a_publication_with_its_page_count() {
        let item = OpdsItemAdapter::adapt_item(CatalogEntry::Publication {
            feed: "http://h/feed".into(),
            index: 2,
            publication: publication("75", "Volume 957", 29),
        });

        assert_eq!(item.name, "Volume 957");
        assert_eq!(item.pages, Some(29));
        assert_eq!(
            OpdsId::decode(&item.external_id).unwrap(),
            OpdsId::Publication {
                feed: "http://h/feed".into(),
                index: 2,
                key: "75".into()
            }
        );
    }

    #[test]
    fn publication_entry_is_a_chapter_card() {
        let entry = publication_entry("http://h/feed", 0, &publication("75", "Volume 957", 0));

        assert_eq!(entry.kind, EntryKind::Chapter);
        assert_eq!(entry.label, "Volume 957");
        assert_eq!(entry.pages, None);
    }
}
