use serde::Deserialize;

use super::{link::Link, localized};

/// A Readium Web Publication manifest (the Divina profile is what comics use).
/// Only the parts needed to read a publication page by page are kept.
#[derive(Clone, Debug, Default, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Manifest {
    #[serde(default)]
    pub metadata: ManifestMetadata,
    #[serde(default)]
    pub links: Vec<Link>,
    /// The publication's resources in reading order.
    #[serde(default)]
    pub reading_order: Vec<Link>,
}

#[derive(Clone, Debug, Default, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ManifestMetadata {
    #[serde(default, deserialize_with = "localized::string")]
    pub title: String,
}

impl Manifest {
    /// The reading order's image resources: a comic's pages.
    pub fn pages(&self) -> impl Iterator<Item = &Link> {
        self.reading_order.iter().filter(|link| {
            link.essence()
                .is_none_or(|essence| essence.starts_with("image/"))
        })
    }
}
