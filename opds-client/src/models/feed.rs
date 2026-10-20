use serde::Deserialize;

use super::{link::Link, localized, publication::Publication, rel};

/// An OPDS feed: a catalog page of navigation links and/or publications.
#[derive(Clone, Debug, Default, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Feed {
    #[serde(default)]
    pub metadata: FeedMetadata,
    #[serde(default)]
    pub links: Vec<Link>,
    #[serde(default)]
    pub navigation: Vec<Navigation>,
    #[serde(default)]
    pub publications: Vec<Publication>,
    #[serde(default)]
    pub groups: Vec<Group>,
}

#[derive(Clone, Debug, Default, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FeedMetadata {
    #[serde(default, deserialize_with = "localized::string")]
    pub title: String,
    #[serde(default)]
    pub number_of_items: Option<u32>,
    #[serde(default)]
    pub items_per_page: Option<u32>,
    #[serde(default)]
    pub current_page: Option<u32>,
}

/// A navigation entry: a link to another feed. OPDS 2 navigation links carry
/// no artwork, but Atom navigation entries often do (e.g. a series' cover), so
/// they're kept alongside the link.
#[derive(Clone, Debug, Default, PartialEq, Deserialize)]
#[serde(from = "Link")]
pub struct Navigation {
    pub link: Link,
    pub images: Vec<Link>,
}

impl From<Link> for Navigation {
    fn from(link: Link) -> Self {
        Navigation {
            link,
            images: Vec::new(),
        }
    }
}

impl Navigation {
    pub fn title(&self) -> &str {
        self.link.title.as_deref().unwrap_or_default()
    }

    pub fn cover(&self) -> Option<&Link> {
        self.images
            .iter()
            .find(|image| !image.has_rel(rel::THUMBNAIL))
            .or_else(|| self.images.first())
    }
}

/// An OPDS 2 group: a titled sub-collection embedded in a feed.
#[derive(Clone, Debug, Default, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Group {
    #[serde(default)]
    pub metadata: FeedMetadata,
    #[serde(default)]
    pub links: Vec<Link>,
    #[serde(default)]
    pub navigation: Vec<Navigation>,
    #[serde(default)]
    pub publications: Vec<Publication>,
}

impl Feed {
    pub fn link(&self, rel: &str) -> Option<&Link> {
        self.links.iter().find(|link| link.has_rel(rel))
    }

    /// The next page of this feed, when it's paginated.
    pub fn next(&self) -> Option<&Link> {
        self.link(rel::NEXT)
    }

    /// Feed-level artwork (Atom catalogs often attach the series cover to the
    /// feed listing its publications).
    pub fn cover(&self) -> Option<&Link> {
        self.link(rel::IMAGE).or_else(|| self.link(rel::THUMBNAIL))
    }

    /// Every navigation entry: the feed's own, then each group's.
    pub fn all_navigation(&self) -> impl Iterator<Item = &Navigation> {
        self.navigation
            .iter()
            .chain(self.groups.iter().flat_map(|group| group.navigation.iter()))
    }

    /// Every publication: the feed's own, then each group's.
    pub fn all_publications(&self) -> impl Iterator<Item = &Publication> {
        self.publications.iter().chain(
            self.groups
                .iter()
                .flat_map(|group| group.publications.iter()),
        )
    }

    /// Whether this feed only points at other feeds.
    pub fn is_navigation_only(&self) -> bool {
        self.all_publications().next().is_none() && self.all_navigation().next().is_some()
    }

    pub fn is_empty(&self) -> bool {
        self.all_publications().next().is_none() && self.all_navigation().next().is_none()
    }
}
