use diesel::{Insertable, Selectable, deserialize::Queryable};
use diesel_derive_enum::DbEnum;

use crate::schema::libraries;

#[derive(Clone, Copy, Debug, DbEnum, PartialEq, Eq)]
pub enum LibraryType {
    Manga,
    Comic,
    Ebook,
}

impl LibraryType {
    pub fn label(&self) -> &'static str {
        match self {
            LibraryType::Manga => "Manga",
            LibraryType::Comic => "Comic",
            LibraryType::Ebook => "Ebook",
        }
    }

    /// Every library type, in display order — for pickers that offer one of each.
    pub const ALL: [LibraryType; 3] = [LibraryType::Manga, LibraryType::Comic, LibraryType::Ebook];
}

#[derive(Clone, Debug, Queryable, Selectable, Insertable, PartialEq)]
#[diesel(table_name = libraries)]
pub struct Library {
    pub id: String,
    pub kind: LibraryType,
    pub name: String,
    pub external_id: Option<String>,
    pub folder: Option<String>,
    pub cover: Option<String>,
    pub integration_id: Option<String>,
}
