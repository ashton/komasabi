//! OPDS 2 allows several string fields (titles, descriptions, …) to be either a
//! plain string or a language map (`{"en": "…", "fr": "…"}`). This app only
//! displays one, so a map collapses to its English value, or its first one.

use serde::{Deserialize, Deserializer};
use std::collections::BTreeMap;

#[derive(Deserialize)]
#[serde(untagged)]
enum Localized {
    Plain(String),
    Map(BTreeMap<String, String>),
}

impl Localized {
    fn into_string(self) -> Option<String> {
        match self {
            Localized::Plain(value) => Some(value),
            Localized::Map(mut map) => map.remove("en").or_else(|| map.into_values().next()),
        }
    }
}

pub fn string<'de, D: Deserializer<'de>>(deserializer: D) -> Result<String, D::Error> {
    Ok(option(deserializer)?.unwrap_or_default())
}

pub fn option<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Option<String>, D::Error> {
    Ok(Option::<Localized>::deserialize(deserializer)?.and_then(Localized::into_string))
}

/// `rel` on an OPDS 2 link is either one relation or an array of them.
pub fn one_or_many<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Vec<String>, D::Error> {
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum OneOrMany {
        One(String),
        Many(Vec<String>),
    }

    Ok(match Option::<OneOrMany>::deserialize(deserializer)? {
        None => Vec::new(),
        Some(OneOrMany::One(value)) => vec![value],
        Some(OneOrMany::Many(values)) => values,
    })
}

/// An OPDS 2 identifier is usually a string, but some servers emit numbers.
pub fn identifier<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Option<String>, D::Error> {
    Ok(
        match Option::<serde_json::Value>::deserialize(deserializer)? {
            Some(serde_json::Value::String(value)) => Some(value),
            Some(serde_json::Value::Number(value)) => Some(value.to_string()),
            _ => None,
        },
    )
}
