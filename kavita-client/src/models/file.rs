use chrono::NaiveDateTime;
use serde::Deserialize;
use serde_repr::Deserialize_repr;

#[derive(Deserialize_repr, Debug, Default, PartialEq)]
#[repr(u8)]
pub enum KavitaFileType {
    #[default]
    Archive = 1,
    EPub = 2,
    Pdf = 3,
    Images = 4,
}

#[derive(Deserialize, Debug, Default)]
#[serde(rename_all = "camelCase")]
pub struct File {
    pub id: u16,
    pub file_path: Option<String>,
    pub pages: u16,
    pub format: KavitaFileType,
    pub bytes: u32,
    pub created: NaiveDateTime,
    pub extension: Option<String>,
}
