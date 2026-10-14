use serde::Deserialize;

use crate::models::chapter::Chapter;

/// Kavita's sentinel volume number for the "loose leaf" bucket: a pseudo-volume
/// that holds chapters/issues which aren't grouped into a numbered volume. A
/// *numbered* volume (`min_number > 0`) represents a bound book and is read as a
/// unit; the loose-leaf volume's chapters are the individual issues. Kavita uses
/// the same value as the chapter number for a numbered volume's single "default"
/// chapter (the one standing in for the whole volume).
pub const LOOSE_LEAF_VOLUME_NUMBER: f32 = -100_000.0;

/// Kavita's sentinel volume number for the specials bucket.
pub const SPECIAL_VOLUME_NUMBER: f32 = -200_000.0;

#[derive(Deserialize, Debug, Default)]
#[serde(rename_all = "camelCase")]
pub struct Volume {
    pub id: u16,
    pub name: String,
    pub min_number: f32,
    pub max_number: f32,
    pub cover_image: String,
    pub pages: u16,
    pub pages_read: u16,
    pub series_id: u16,
    pub min_hours_to_read: u8,
    pub max_hours_to_read: u8,
    pub avg_hours_to_read: f32,
    pub chapters: Vec<Chapter>,
}
