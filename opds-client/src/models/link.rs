use serde::Deserialize;

use super::localized;

/// An OPDS 2 link object. Atom `<link>` elements map onto it one-to-one.
#[derive(Clone, Debug, Default, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Link {
    /// Always absolute once the link has come out of a parsed feed.
    pub href: String,
    #[serde(default, deserialize_with = "localized::one_or_many")]
    pub rel: Vec<String>,
    #[serde(rename = "type", default)]
    pub media_type: Option<String>,
    #[serde(default, deserialize_with = "localized::option")]
    pub title: Option<String>,
    #[serde(default)]
    pub templated: bool,
    #[serde(default)]
    pub properties: LinkProperties,
}

#[derive(Clone, Debug, Default, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LinkProperties {
    /// How many items the linked feed holds, when the server says.
    #[serde(default)]
    pub number_of_items: Option<u32>,
    /// The publication's page count, from the OPDS Page Streaming Extension's
    /// `pse:count` attribute (Atom catalogs only — OPDS 2 has no equivalent).
    #[serde(skip)]
    pub page_count: Option<u32>,
}

impl Link {
    pub fn has_rel(&self, rel: &str) -> bool {
        self.rel.iter().any(|candidate| candidate == rel)
    }

    pub fn has_rel_prefix(&self, prefix: &str) -> bool {
        self.rel
            .iter()
            .any(|candidate| candidate.starts_with(prefix))
    }

    /// The media type without parameters (`application/atom+xml;profile=…` →
    /// `application/atom+xml`), lowercased.
    pub fn essence(&self) -> Option<String> {
        self.media_type.as_deref().map(essence)
    }

    pub fn is_type(&self, media_type: &str) -> bool {
        self.essence().as_deref() == Some(media_type)
    }

    /// Expands an OPDS-PSE stream template for the 0-based `page`. The template
    /// variable may have been percent-encoded when the href was resolved (it
    /// is in a path segment), so both spellings are substituted; the optional
    /// `{maxWidth}` variable is filled with a generous width.
    pub fn expand_page(&self, page: u32) -> String {
        let page = page.to_string();
        self.href
            .replace("{pageNumber}", &page)
            .replace("%7BpageNumber%7D", &page)
            .replace("{maxWidth}", "4096")
            .replace("%7BmaxWidth%7D", "4096")
    }
}

/// The essence of a media type string: everything before the first `;`,
/// trimmed and lowercased.
pub fn essence(media_type: &str) -> String {
    media_type
        .split(';')
        .next()
        .unwrap_or_default()
        .trim()
        .to_ascii_lowercase()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rel_accepts_a_single_string_or_an_array() {
        let single: Link = serde_json::from_str(r#"{"href": "/a", "rel": "self"}"#).unwrap();
        let many: Link =
            serde_json::from_str(r#"{"href": "/a", "rel": ["self", "start"]}"#).unwrap();

        assert_eq!(single.rel, vec!["self"]);
        assert!(many.has_rel("start"));
    }

    #[test]
    fn essence_strips_parameters() {
        let link = Link {
            media_type: Some("application/atom+xml;profile=opds-catalog;kind=navigation".into()),
            ..Link::default()
        };

        assert!(link.is_type("application/atom+xml"));
    }

    #[test]
    fn expand_page_substitutes_raw_and_percent_encoded_templates() {
        let raw = Link {
            href: "http://h/image?page={pageNumber}".into(),
            ..Link::default()
        };
        let encoded = Link {
            href: "http://h/books/1/pages/%7BpageNumber%7D?w=%7BmaxWidth%7D".into(),
            ..Link::default()
        };

        assert_eq!(raw.expand_page(3), "http://h/image?page=3");
        assert_eq!(encoded.expand_page(0), "http://h/books/1/pages/0?w=4096");
    }
}
