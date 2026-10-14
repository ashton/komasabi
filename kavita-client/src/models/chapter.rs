use chrono::NaiveDateTime;
use serde::Deserialize;
use serde_repr::Deserialize_repr;

use crate::models::{file::File, genre::GenreTag, person::Person};

#[derive(Deserialize_repr, Debug, Default)]
#[repr(u8)]
pub enum Format {
    Image = 0,
    Archive = 1,
    #[default]
    Unknown = 2,
    Epub = 3,
    Pdf = 4,
}

#[derive(Deserialize_repr, Debug, Default)]
#[repr(u8)]
pub enum PublicationStatus {
    #[default]
    OnGoing = 0,
    Hiatus = 1,
    Completed = 2,
    Cancelled = 3,
    Ended = 4,
}

#[derive(Deserialize_repr, Debug, Default)]
#[repr(i8)]
pub enum AgeRating {
    #[default]
    Unknown = 0,
    RatingPending = 1,
    EarlyChildhood = 2,
    Everyone = 3,
    G = 4,
    EveryoneTenPlus = 5,
    PG = 6,
    KidsToAdults = 7,
    Teen = 8,
    MatureFifteenPlus = 9,
    MatureSeventeenPlus = 10,
    Mature = 11,
    REighteenPlus = 12,
    AdultsOnly = 13,
    XEighteenPlus = 14,
    NotApplicable = -1,
}

#[derive(Deserialize, Debug, Default)]
#[serde(rename_all = "camelCase")]
pub struct Chapter {
    pub id: u16,
    pub range: String,
    pub min_number: f32,
    pub max_number: f32,
    pub sort_order: f32,
    pub pages: u16,
    pub is_special: bool,
    pub title: Option<String>,
    pub pages_read: u16,
    pub total_reads: u16,
    pub last_reading_progress_utc: NaiveDateTime,
    pub last_reading_progress: NaiveDateTime,
    pub volume_id: u16,
    pub created_utc: NaiveDateTime,
    pub last_modified_utc: NaiveDateTime,
    pub created: NaiveDateTime,
    pub release_date: NaiveDateTime,
    pub title_name: Option<String>,
    pub summary: Option<String>,
    pub volume_title: Option<String>,
    pub isbn: Option<String>,
    pub language: Option<String>,
    pub cover_image: String,
    pub files: Vec<File>,
    pub age_rating: AgeRating,
    pub writers: Vec<Person>,
    pub cover_artists: Vec<Person>,
    pub publishers: Vec<Person>,
    pub characters: Vec<Person>,
    pub pencillers: Vec<Person>,
    pub inkers: Vec<Person>,
    pub imprints: Vec<Person>,
    pub colorists: Vec<Person>,
    pub letterers: Vec<Person>,
    pub editors: Vec<Person>,
    pub translators: Vec<Person>,
    pub teams: Vec<Person>,
    pub locations: Vec<Person>,
    pub genres: Vec<GenreTag>,
    pub publication_status: PublicationStatus,
    pub format: Format,
}
