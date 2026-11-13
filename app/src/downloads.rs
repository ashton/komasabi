//! Downloading a series' volumes/chapters for reading offline.
//!
//! Each downloaded entry is a folder of page images in the app's data folder:
//! `<data dir>/downloads/<source>/<series>/<entry>/`, its pages named the way
//! the reader expects ([`reader_launch::page_file`]), so the reader is pointed
//! straight at it. A `manifest.json` written once every page is in place says
//! what the entry is and lists its [`PageRef`]s, so a downloaded entry is read
//! without asking its source for anything (see [`local_entry`]). Until the
//! manifest exists, the folder is only a `.part` being filled, never read.
//!
//! A series whose every entry was downloaded also gets a `series.json`, which
//! is what tells a series card it's downloaded ([`downloaded_series`]).
//! Removing any of its entries takes the mark away again.
//!
//! Nothing here is in the database: the folders are the record.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::cache::file_name;
use crate::models::series_entry::{EntryKind, ReadState, SeriesEntry, cover_key};
use crate::providers::{PageRef, Provider, provider_for_integration};
use crate::reader_launch::page_file;

const MANIFEST: &str = "manifest.json";
const SERIES_MARK: &str = "series.json";

/// A downloaded volume/chapter.
#[derive(Clone, Debug, PartialEq)]
pub struct DownloadedEntry {
    /// The source it was downloaded from.
    pub integration_id: String,
    pub series_id: String,
    pub series_name: String,
    /// The entry, as its source listed it (unread: read state isn't kept here).
    pub entry: SeriesEntry,
    /// Its pages, as the source resolved them, in reading order.
    pub pages: Vec<PageRef>,
    /// The folder holding its page images.
    pub dir: PathBuf,
}

impl DownloadedEntry {
    /// The image of its first page, which stands in for its cover offline.
    pub fn first_page(&self) -> PathBuf {
        self.dir.join(page_file(0))
    }
}

#[derive(Serialize, Deserialize)]
struct Manifest {
    integration_id: String,
    series_id: String,
    series_name: String,
    external_id: String,
    kind: String,
    label: String,
    pages: Vec<ManifestPage>,
    downloaded_at: String,
}

#[derive(Serialize, Deserialize)]
struct ManifestPage {
    chapter_id: String,
    page: u16,
}

#[derive(Serialize, Deserialize)]
struct SeriesMark {
    integration_id: String,
    series_id: String,
    series_name: String,
}

/// Where downloads are kept, or `None` if this platform has no data folder.
pub fn root() -> Option<PathBuf> {
    Some(
        directories::ProjectDirs::from("", "", "komasabi")?
            .data_dir()
            .join("downloads"),
    )
}

fn series_dir(root: &Path, integration_id: &str, series_id: &str) -> PathBuf {
    root.join(file_name(integration_id))
        .join(file_name(series_id))
}

fn entry_dir(root: &Path, integration_id: &str, series_id: &str, entry: &SeriesEntry) -> PathBuf {
    series_dir(root, integration_id, series_id).join(file_name(&entry.cover_key()))
}

fn kind_name(kind: EntryKind) -> &'static str {
    match kind {
        EntryKind::Volume => "volume",
        EntryKind::Chapter => "chapter",
    }
}

/// `entry` of the series `series_id`, if it's downloaded.
pub fn local_entry(
    integration_id: &str,
    series_id: &str,
    entry: &SeriesEntry,
) -> Option<DownloadedEntry> {
    let dir = entry_dir(&root()?, integration_id, series_id, entry);
    read_entry(&dir)
}

/// The downloaded entry in `dir`, if its download finished.
fn read_entry(dir: &Path) -> Option<DownloadedEntry> {
    let manifest: Manifest =
        serde_json::from_slice(&std::fs::read(dir.join(MANIFEST)).ok()?).ok()?;
    let kind = match manifest.kind.as_str() {
        "volume" => EntryKind::Volume,
        "chapter" => EntryKind::Chapter,
        _ => return None,
    };
    let pages: Vec<PageRef> = manifest
        .pages
        .into_iter()
        .map(|page| PageRef {
            chapter_id: page.chapter_id,
            page: page.page,
        })
        .collect();

    Some(DownloadedEntry {
        integration_id: manifest.integration_id,
        series_id: manifest.series_id,
        series_name: manifest.series_name,
        entry: SeriesEntry {
            external_id: manifest.external_id,
            kind,
            label: manifest.label,
            pages: u16::try_from(pages.len()).ok(),
            read_state: ReadState::Unread,
        },
        pages,
        dir: dir.to_path_buf(),
    })
}

/// Every downloaded entry, by series name then in reading order.
pub async fn all() -> Vec<DownloadedEntry> {
    match root() {
        Some(root) => all_in(&root),
        None => Vec::new(),
    }
}

fn all_in(root: &Path) -> Vec<DownloadedEntry> {
    let mut entries: Vec<DownloadedEntry> = subdirs(root)
        .flat_map(|source| subdirs(&source).collect::<Vec<_>>())
        .flat_map(|series| subdirs(&series).collect::<Vec<_>>())
        .filter_map(|dir| read_entry(&dir))
        .collect();

    entries.sort_by(|a, b| {
        a.series_name
            .to_lowercase()
            .cmp(&b.series_name.to_lowercase())
            .then_with(|| a.series_id.cmp(&b.series_id))
            // Volumes before chapters, then by label.
            .then_with(|| kind_name(b.entry.kind).cmp(kind_name(a.entry.kind)))
            .then_with(|| crate::repository::library::natural_cmp(&a.entry.label, &b.entry.label))
    });
    entries
}

/// The folders directly inside `dir` (none if it can't be read).
fn subdirs(dir: &Path) -> impl Iterator<Item = PathBuf> {
    std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| path.is_dir())
}

/// The ids of the series of the source `integration_id` downloaded in full.
pub async fn downloaded_series(integration_id: String) -> HashSet<String> {
    match root() {
        Some(root) => downloaded_series_in(&root, &integration_id),
        None => HashSet::new(),
    }
}

fn downloaded_series_in(root: &Path, integration_id: &str) -> HashSet<String> {
    subdirs(&root.join(file_name(integration_id)))
        .filter_map(|series| read_mark(&series))
        .map(|mark| mark.series_id)
        .collect()
}

/// Every series downloaded in full, from any source, as `(source id, series
/// id)`s.
pub async fn all_downloaded_series() -> HashSet<(String, String)> {
    match root() {
        Some(root) => subdirs(&root)
            .flat_map(|source| subdirs(&source).collect::<Vec<_>>())
            .filter_map(|series| read_mark(&series))
            .map(|mark| (mark.integration_id, mark.series_id))
            .collect(),
        None => HashSet::new(),
    }
}

fn read_mark(series: &Path) -> Option<SeriesMark> {
    serde_json::from_slice(&std::fs::read(series.join(SERIES_MARK)).ok()?).ok()
}

/// Downloads every entry of the series `series_id` of the source
/// `integration_id` not downloaded yet, then marks the series downloaded.
/// Returns how many entries were downloaded. Entries that fail are left out
/// (and the series isn't marked), so downloading again picks up where it
/// stopped.
pub async fn download_series(
    integration_id: String,
    series_id: String,
    series_name: String,
) -> Result<usize, String> {
    let root = root().ok_or("no data folder to download into")?;
    let (provider, _namespace) = provider_for_integration(&integration_id).await?;
    download_series_into(&provider, &root, &integration_id, &series_id, &series_name).await
}

async fn download_series_into(
    provider: &impl Provider,
    root: &Path,
    integration_id: &str,
    series_id: &str,
    series_name: &str,
) -> Result<usize, String> {
    let entries = provider.list_series_entries(series_id).await?;
    if entries.is_empty() {
        return Err(format!("{series_name} has no volumes or chapters"));
    }

    let mut downloaded = 0;
    let mut failed = Vec::new();
    for entry in &entries {
        let dir = entry_dir(root, integration_id, series_id, entry);
        if read_entry(&dir).is_some() {
            continue;
        }
        match download_entry(
            provider,
            &dir,
            integration_id,
            series_id,
            series_name,
            entry,
        )
        .await
        {
            Ok(()) => downloaded += 1,
            Err(error) => {
                tracing::debug!("could not download {}: {error}", entry.label);
                failed.push(format!("{}: {error}", entry.label));
            }
        }
    }

    if !failed.is_empty() {
        return Err(format!(
            "{} of {} could not be downloaded ({})",
            failed.len(),
            entries.len(),
            failed.join("; ")
        ));
    }

    let mark = SeriesMark {
        integration_id: integration_id.to_string(),
        series_id: series_id.to_string(),
        series_name: series_name.to_string(),
    };
    write_json(
        &series_dir(root, integration_id, series_id).join(SERIES_MARK),
        &mark,
    )?;
    Ok(downloaded)
}

/// The cover keys ([`SeriesEntry::cover_key`]) of the downloaded entries of
/// the series `series_id` of the source `integration_id`.
pub async fn downloaded_entries(integration_id: String, series_id: String) -> HashSet<String> {
    match root() {
        Some(root) => downloaded_entries_in(&root, &integration_id, &series_id),
        None => HashSet::new(),
    }
}

fn downloaded_entries_in(root: &Path, integration_id: &str, series_id: &str) -> HashSet<String> {
    subdirs(&series_dir(root, integration_id, series_id))
        .filter_map(|dir| read_entry(&dir))
        .map(|downloaded| cover_key(downloaded.entry.kind, &downloaded.entry.external_id))
        .collect()
}

/// Downloads one `entry` of the series `series_id` of the source
/// `integration_id`, unless it's downloaded already. Once every entry the
/// source lists for the series is downloaded, the series is marked
/// downloaded too, as if it had been downloaded whole.
pub async fn download_one(
    integration_id: String,
    series_id: String,
    series_name: String,
    entry: SeriesEntry,
) -> Result<(), String> {
    let root = root().ok_or("no data folder to download into")?;
    let (provider, _namespace) = provider_for_integration(&integration_id).await?;
    download_one_into(
        &provider,
        &root,
        &integration_id,
        &series_id,
        &series_name,
        &entry,
    )
    .await
}

async fn download_one_into(
    provider: &impl Provider,
    root: &Path,
    integration_id: &str,
    series_id: &str,
    series_name: &str,
    entry: &SeriesEntry,
) -> Result<(), String> {
    let dir = entry_dir(root, integration_id, series_id, entry);
    if read_entry(&dir).is_none() {
        download_entry(
            provider,
            &dir,
            integration_id,
            series_id,
            series_name,
            entry,
        )
        .await?;
    }

    // Marking the series is a bonus: the entry is downloaded either way.
    match provider.list_series_entries(series_id).await {
        Ok(entries)
            if entries.iter().all(|entry| {
                read_entry(&entry_dir(root, integration_id, series_id, entry)).is_some()
            }) =>
        {
            let mark = SeriesMark {
                integration_id: integration_id.to_string(),
                series_id: series_id.to_string(),
                series_name: series_name.to_string(),
            };
            if let Err(error) = write_json(
                &series_dir(root, integration_id, series_id).join(SERIES_MARK),
                &mark,
            ) {
                tracing::debug!("could not mark {series_name} downloaded: {error}");
            }
        }
        Ok(_) => {}
        Err(error) => tracing::debug!("could not list the entries of {series_name}: {error}"),
    }
    Ok(())
}

/// Downloads `entry`'s pages into a `.part` folder next to `dir`, writes its
/// manifest, and only then moves it to `dir`, so `dir` only ever holds a whole
/// download.
async fn download_entry(
    provider: &impl Provider,
    dir: &Path,
    integration_id: &str,
    series_id: &str,
    series_name: &str,
    entry: &SeriesEntry,
) -> Result<(), String> {
    let pages = provider.reader_pages(entry).await?;
    if pages.is_empty() {
        return Err("it has no pages".to_string());
    }

    let mut part = dir.as_os_str().to_owned();
    part.push(".part");
    let part = PathBuf::from(part);
    let _ = std::fs::remove_dir_all(&part);
    std::fs::create_dir_all(&part).map_err(|error| io_error(&part, error))?;

    for (index, page) in pages.iter().enumerate() {
        let bytes = provider.reader_image(&page.chapter_id, page.page).await?;
        let path = part.join(page_file(index));
        std::fs::write(&path, bytes).map_err(|error| io_error(&path, error))?;
    }

    let manifest = Manifest {
        integration_id: integration_id.to_string(),
        series_id: series_id.to_string(),
        series_name: series_name.to_string(),
        external_id: entry.external_id.clone(),
        kind: kind_name(entry.kind).to_string(),
        label: entry.label.clone(),
        pages: pages
            .iter()
            .map(|page| ManifestPage {
                chapter_id: page.chapter_id.clone(),
                page: page.page,
            })
            .collect(),
        downloaded_at: chrono::Utc::now().to_rfc3339(),
    };
    write_json(&part.join(MANIFEST), &manifest)?;

    let _ = std::fs::remove_dir_all(dir);
    std::fs::rename(&part, dir).map_err(|error| io_error(dir, error))
}

fn write_json(path: &Path, value: &impl Serialize) -> Result<(), String> {
    let json = serde_json::to_vec_pretty(value).map_err(|error| error.to_string())?;
    std::fs::write(path, json).map_err(|error| io_error(path, error))
}

fn io_error(path: &Path, error: std::io::Error) -> String {
    format!("{}: {error}", path.display())
}

/// Deletes the downloaded entry in `dir`. Its series is no longer downloaded
/// in full, so it loses its mark, and its folder goes once it's empty.
pub async fn remove(dir: PathBuf) -> Result<(), String> {
    std::fs::remove_dir_all(&dir).map_err(|error| io_error(&dir, error))?;
    if let Some(series) = dir.parent() {
        let _ = std::fs::remove_file(series.join(SERIES_MARK));
        if subdirs(series).next().is_none() {
            let _ = std::fs::remove_dir_all(series);
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::collection::Collection;
    use crate::models::collection_item::CollectionItem;

    /// A provider with two volumes of two pages each, whose pages are the
    /// bytes `[volume, page]`. `broken` makes one volume's pages fail.
    struct FakeProvider {
        broken: Option<&'static str>,
    }

    fn volume(id: &str) -> SeriesEntry {
        SeriesEntry {
            external_id: id.to_string(),
            kind: EntryKind::Volume,
            label: format!("Volume {id}"),
            pages: Some(2),
            read_state: ReadState::Unread,
        }
    }

    impl Provider for FakeProvider {
        async fn list_collections(&self) -> Result<Vec<Collection>, String> {
            unimplemented!()
        }
        async fn list_items(
            &self,
            _: &str,
            _: Option<u32>,
            _: Option<u32>,
        ) -> Result<Vec<CollectionItem>, String> {
            unimplemented!()
        }
        async fn list_series_entries(&self, _: &str) -> Result<Vec<SeriesEntry>, String> {
            Ok(vec![volume("1"), volume("2")])
        }
        async fn cover(&self, _: &str, _: &str) -> Result<Vec<u8>, String> {
            unimplemented!()
        }
        async fn volume_cover(&self, _: &str, _: &str) -> Result<Vec<u8>, String> {
            unimplemented!()
        }
        async fn chapter_cover(&self, _: &str, _: &str) -> Result<Vec<u8>, String> {
            unimplemented!()
        }
        async fn reader_pages(&self, entry: &SeriesEntry) -> Result<Vec<PageRef>, String> {
            Ok((0..2)
                .map(|page| PageRef {
                    chapter_id: entry.external_id.clone(),
                    page,
                })
                .collect())
        }
        async fn reader_image(&self, chapter_id: &str, page: u16) -> Result<Vec<u8>, String> {
            if self.broken == Some(chapter_id) {
                return Err("offline".to_string());
            }
            Ok(vec![chapter_id.parse().unwrap(), page as u8])
        }
        async fn next_entry(
            &self,
            _: &str,
            _: &SeriesEntry,
        ) -> Result<Option<SeriesEntry>, String> {
            unimplemented!()
        }
        async fn previous_entry(
            &self,
            _: &str,
            _: &SeriesEntry,
        ) -> Result<Option<SeriesEntry>, String> {
            unimplemented!()
        }
    }

    /// A fresh, empty downloads folder for one test.
    fn temp_root() -> PathBuf {
        std::env::temp_dir().join(format!("komasabi-downloads-{}", uuid::Uuid::new_v4()))
    }

    fn block_on<F: std::future::Future>(future: F) -> F::Output {
        tokio::runtime::Builder::new_current_thread()
            .build()
            .unwrap()
            .block_on(future)
    }

    #[test]
    fn downloading_a_series_saves_every_entry_and_marks_the_series() {
        let root = temp_root();
        let provider = FakeProvider { broken: None };

        let downloaded = block_on(download_series_into(
            &provider, &root, "src", "42", "Series",
        ));

        assert_eq!(downloaded, Ok(2));
        assert_eq!(
            downloaded_series_in(&root, "src"),
            HashSet::from(["42".to_string()])
        );
        let entries = all_in(&root);
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].entry.label, "Volume 1");
        assert_eq!(entries[0].series_name, "Series");
        assert_eq!(entries[0].pages.len(), 2);
        assert_eq!(
            std::fs::read(entries[1].dir.join(page_file(1))).unwrap(),
            vec![2, 1]
        );

        // Downloading again has nothing left to fetch.
        assert_eq!(
            block_on(download_series_into(
                &provider, &root, "src", "42", "Series"
            )),
            Ok(0)
        );
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn a_failed_entry_leaves_no_partial_download_and_no_series_mark() {
        let root = temp_root();

        let result = block_on(download_series_into(
            &FakeProvider { broken: Some("2") },
            &root,
            "src",
            "42",
            "Series",
        ));

        assert!(result.unwrap_err().contains("1 of 2"));
        assert!(downloaded_series_in(&root, "src").is_empty());
        let entries = all_in(&root);
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].entry.external_id, "1");

        // Retrying once the source is back finishes the series.
        assert_eq!(
            block_on(download_series_into(
                &FakeProvider { broken: None },
                &root,
                "src",
                "42",
                "Series",
            )),
            Ok(1)
        );
        assert_eq!(downloaded_series_in(&root, "src").len(), 1);
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn downloading_one_entry_saves_only_it_until_the_series_is_complete() {
        let root = temp_root();
        let provider = FakeProvider { broken: None };

        block_on(download_one_into(
            &provider,
            &root,
            "src",
            "42",
            "Series",
            &volume("1"),
        ))
        .unwrap();
        assert_eq!(
            downloaded_entries_in(&root, "src", "42"),
            HashSet::from([volume("1").cover_key()])
        );
        assert!(downloaded_series_in(&root, "src").is_empty());

        // The last entry missing completes the series.
        block_on(download_one_into(
            &provider,
            &root,
            "src",
            "42",
            "Series",
            &volume("2"),
        ))
        .unwrap();
        assert_eq!(downloaded_entries_in(&root, "src", "42").len(), 2);
        assert_eq!(
            downloaded_series_in(&root, "src"),
            HashSet::from(["42".to_string()])
        );
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn a_failed_entry_download_is_reported_and_leaves_nothing() {
        let root = temp_root();

        let result = block_on(download_one_into(
            &FakeProvider { broken: Some("1") },
            &root,
            "src",
            "42",
            "Series",
            &volume("1"),
        ));

        assert_eq!(result, Err("offline".to_string()));
        assert!(downloaded_entries_in(&root, "src", "42").is_empty());
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn removing_an_entry_unmarks_its_series_and_drops_the_empty_folder() {
        let root = temp_root();
        let provider = FakeProvider { broken: None };
        block_on(download_series_into(
            &provider, &root, "src", "42", "Series",
        ))
        .unwrap();
        let entries = all_in(&root);

        block_on(remove(entries[0].dir.clone())).unwrap();
        assert!(downloaded_series_in(&root, "src").is_empty());
        assert_eq!(all_in(&root).len(), 1);

        block_on(remove(entries[1].dir.clone())).unwrap();
        assert!(all_in(&root).is_empty());
        assert!(!series_dir(&root, "src", "42").exists());
        let _ = std::fs::remove_dir_all(root);
    }
}
