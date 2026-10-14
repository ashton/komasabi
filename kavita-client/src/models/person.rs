use serde::Deserialize;
use serde_repr::Deserialize_repr;

#[derive(Deserialize_repr, Debug)]
#[repr(u8)]
pub enum PersonRole {
    Writer = 3,
    Penciller = 4,
    Inker = 5,
    Colorist = 6,
    Letterer = 7,
    CoverArtist = 8,
    Editor = 9,
    Publisher = 10,
    Character = 11,
    Translator = 12,
    Imprint = 13,
    Team = 14,
    Location = 15,
}

#[derive(Deserialize, Debug, Default)]
#[serde(rename_all = "camelCase")]
pub struct Person {
    pub id: u16,
    pub name: Option<String>,
    pub cover_image: Option<String>,
    pub description: Option<String>,
    pub roles: Vec<PersonRole>,
}
