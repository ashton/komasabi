//! The user's Library: the items added from sources (see
//! [`crate::models::library_item`]).

use std::cmp::Ordering;

use diesel::{ExpressionMethods, QueryDsl, SelectableHelper};
use diesel_async::RunQueryDsl;

use crate::{
    database,
    models::library_item::{LibraryItem, kind_name},
    models::series_entry::EntryKind,
};

/// Every item in the Library.
pub async fn all() -> Result<Vec<LibraryItem>, String> {
    use crate::schema::library_items::dsl::*;

    let mut conn = database::connect()
        .await
        .map_err(crate::error::log_and_stringify)?;
    library_items
        .select(LibraryItem::as_select())
        .load(&mut conn)
        .await
        .map_err(crate::error::log_and_stringify)
}

/// The items added from one series of a source, in reading order.
pub async fn series_items(
    source_id: String,
    series_id: String,
) -> Result<Vec<LibraryItem>, String> {
    use crate::schema::library_items::dsl::*;

    let mut conn = database::connect()
        .await
        .map_err(crate::error::log_and_stringify)?;
    let mut items = library_items
        .filter(integration_id.eq(source_id))
        .filter(series_external_id.eq(series_id))
        .select(LibraryItem::as_select())
        .load(&mut conn)
        .await
        .map_err(crate::error::log_and_stringify)?;

    items.sort_by(reading_order);
    Ok(items)
}

/// Saves `item`, replacing the one already added for the same source entry.
pub async fn save(item: LibraryItem) -> Result<LibraryItem, String> {
    use crate::schema::library_items::dsl::library_items;

    let mut conn = database::connect()
        .await
        .map_err(crate::error::log_and_stringify)?;
    diesel::replace_into(library_items)
        .values(&item)
        .execute(&mut conn)
        .await
        .map_err(crate::error::log_and_stringify)?;

    Ok(item)
}

/// Removes the item with the id `item_id` from the Library.
pub async fn remove(item_id: String) -> Result<(), String> {
    use crate::schema::library_items::dsl::*;

    let mut conn = database::connect()
        .await
        .map_err(crate::error::log_and_stringify)?;
    diesel::delete(library_items.filter(id.eq(item_id)))
        .execute(&mut conn)
        .await
        .map(|_| ())
        .map_err(crate::error::log_and_stringify)
}

/// The ids of the series of a source that have items in the Library.
pub async fn series_ids(source_id: String) -> Result<Vec<String>, String> {
    use crate::schema::library_items::dsl::*;

    let mut conn = database::connect()
        .await
        .map_err(crate::error::log_and_stringify)?;
    library_items
        .filter(integration_id.eq(source_id))
        .select(series_external_id)
        .distinct()
        .load(&mut conn)
        .await
        .map_err(crate::error::log_and_stringify)
}

/// Removes every item of one series of a source from the Library. Returns how
/// many were removed.
pub async fn remove_series(source_id: String, series_id: String) -> Result<usize, String> {
    use crate::schema::library_items::dsl::*;

    let mut conn = database::connect()
        .await
        .map_err(crate::error::log_and_stringify)?;
    diesel::delete(
        library_items
            .filter(integration_id.eq(source_id))
            .filter(series_external_id.eq(series_id)),
    )
    .execute(&mut conn)
    .await
    .map_err(crate::error::log_and_stringify)
}

/// Whether a source entry is in the Library.
pub async fn contains(
    source_id: &str,
    entry_kind: EntryKind,
    entry_id: &str,
) -> Result<bool, String> {
    use crate::schema::library_items::dsl::*;

    let mut conn = database::connect()
        .await
        .map_err(crate::error::log_and_stringify)?;
    library_items
        .filter(integration_id.eq(source_id))
        .filter(kind.eq(kind_name(entry_kind)))
        .filter(external_id.eq(entry_id))
        .count()
        .get_result::<i64>(&mut conn)
        .await
        .map(|count| count > 0)
        .map_err(crate::error::log_and_stringify)
}

/// Records that the first `read` of a source entry's `total` pages were read,
/// if the entry is in the Library. Progress only moves forward, so going back
/// to an earlier page doesn't unread anything. The page count is saved too,
/// as an item added without one only learns it once it's read.
pub async fn record_progress(
    source_id: &str,
    entry_kind: EntryKind,
    entry_id: &str,
    read: i32,
    total: i32,
) -> Result<(), String> {
    use crate::schema::library_items::dsl::*;

    let mut conn = database::connect()
        .await
        .map_err(crate::error::log_and_stringify)?;
    diesel::update(
        library_items
            .filter(integration_id.eq(source_id))
            .filter(kind.eq(kind_name(entry_kind)))
            .filter(external_id.eq(entry_id)),
    )
    .set(pages.eq(Some(total)))
    .execute(&mut conn)
    .await
    .map_err(crate::error::log_and_stringify)?;

    diesel::update(
        library_items
            .filter(integration_id.eq(source_id))
            .filter(kind.eq(kind_name(entry_kind)))
            .filter(external_id.eq(entry_id))
            .filter(pages_read.lt(read)),
    )
    .set(pages_read.eq(read))
    .execute(&mut conn)
    .await
    .map(|_| ())
    .map_err(crate::error::log_and_stringify)
}

/// Sets an item's page count and progress as they are, unlike
/// [`record_progress`], which only moves progress forward: used to follow the
/// progress kept by the item's source (see [`crate::library::sync_progress`]).
pub async fn set_progress(item_id: &str, page_count: Option<i32>, read: i32) -> Result<(), String> {
    use crate::schema::library_items::dsl::*;

    let mut conn = database::connect()
        .await
        .map_err(crate::error::log_and_stringify)?;
    diesel::update(library_items.filter(id.eq(item_id)))
        .set((pages.eq(page_count), pages_read.eq(read)))
        .execute(&mut conn)
        .await
        .map(|_| ())
        .map_err(crate::error::log_and_stringify)
}

/// Volumes before chapters, then by name, comparing the numbers in names by
/// value so "#2" comes before "#10".
fn reading_order(a: &LibraryItem, b: &LibraryItem) -> Ordering {
    b.kind
        .cmp(&a.kind) // "volume" > "chapter"
        .then_with(|| natural_cmp(&a.name, &b.name))
}

pub(crate) fn natural_cmp(a: &str, b: &str) -> Ordering {
    let (mut a, mut b) = (a.chars().peekable(), b.chars().peekable());
    loop {
        match (a.peek().copied(), b.peek().copied()) {
            (None, None) => return Ordering::Equal,
            (None, Some(_)) => return Ordering::Less,
            (Some(_), None) => return Ordering::Greater,
            (Some(x), Some(y)) if x.is_ascii_digit() && y.is_ascii_digit() => {
                let number = |chars: &mut std::iter::Peekable<std::str::Chars>| {
                    let mut digits = String::new();
                    while let Some(c) = chars.peek().copied().filter(char::is_ascii_digit) {
                        digits.push(c);
                        chars.next();
                    }
                    digits.parse::<u64>().unwrap_or(u64::MAX)
                };
                match number(&mut a).cmp(&number(&mut b)) {
                    Ordering::Equal => {}
                    other => return other,
                }
            }
            (Some(x), Some(y)) => {
                match x.to_lowercase().cmp(y.to_lowercase()) {
                    Ordering::Equal => {}
                    other => return other,
                }
                a.next();
                b.next();
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn natural_order_compares_numbers_by_value() {
        let mut names = vec!["#10", "#2", "#1", "Special", "#2.5"];
        names.sort_by(|a, b| natural_cmp(a, b));
        assert_eq!(names, vec!["#1", "#2", "#2.5", "#10", "Special"]);
    }

    #[test]
    fn volumes_come_before_chapters() {
        use crate::models::library_item::tests::item;

        let chapter = item("s", "1", "1", 1, 0);
        let volume = LibraryItem {
            kind: "volume".to_string(),
            name: "Volume 9".to_string(),
            ..item("s", "1", "2", 1, 0)
        };
        assert_eq!(reading_order(&volume, &chapter), Ordering::Less);
    }
}
