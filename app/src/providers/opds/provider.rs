use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::{LazyLock, Mutex};

use opds_client::OpdsClient;
use opds_client::models::{feed::Feed, link::Link, media_type, publication::Publication};

use crate::{
    adapter::{ItemAdapter, LibraryAdapter},
    models::{library::Library, library_item::LibraryItem, series_entry::SeriesEntry},
    providers::{
        PageRef, Provider,
        opds::adapter::{
            CatalogEntry, OpdsId, OpdsItemAdapter, OpdsLibrary, OpdsLibraryAdapter,
            publication_entry,
        },
        opds::archive,
    },
};

/// Upper bound on feed pages followed through `next` links in one walk, so a
/// server whose pagination loops can't hang a request forever.
const MAX_FEED_PAGES: usize = 200;

/// How many navigation children of a feed are explored when looking for its
/// publications (a series whose feed nests one more level of navigation).
const MAX_NESTED_FEEDS: usize = 30;

/// A [`Provider`] over an OPDS catalog, speaking OPDS 2 (JSON) or OPDS 1.2
/// (Atom) — whichever the server answers with (see `opds-client`).
///
/// The app's library → series → entry hierarchy maps onto the catalog as:
/// - a **library** is a catalog feed (see [`OpdsProvider::list_libraries`]);
/// - its **items** are that feed's entries: navigation links (typically
///   series) and publications listed directly;
/// - an item's **entries** are the publications of the feed it links to (or,
///   for a publication item, the publication itself);
/// - a publication is read by downloading its file through its open-access
///   acquisition link and extracting the pages (CBZ or CBR) up front.
pub struct OpdsProvider {
    client: OpdsClient,
}

/// One fetched page of a paginated feed: enough to skip over it without
/// refetching when a later browse page is requested.
#[derive(Clone)]
struct FeedPage {
    len: usize,
    next: Option<String>,
}

/// Pages already walked, keyed by the URL of their feed's first page. OPDS only
/// paginates by `next` links, so serving browse page N would otherwise mean
/// refetching every page before it. Process-wide because a provider is
/// resolved afresh for each request.
static FEED_PAGES: LazyLock<Mutex<HashMap<String, Vec<FeedPage>>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

/// Cover URLs of items and entries already listed, keyed by their encoded id.
/// The [`Provider`] cover methods only get an id, and the cover URL was in the
/// feed that listed it; remembering it saves fetching that feed again.
static COVER_HREFS: LazyLock<Mutex<HashMap<String, String>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

impl OpdsProvider {
    /// Builds a provider for the catalog rooted at `url`, authenticating with
    /// basic auth when a username is given.
    pub fn connect(url: String, username: Option<String>, password: Option<String>) -> Self {
        let client = OpdsClient::new(url, reqwest::Client::new());
        let client = match username.filter(|username| !username.is_empty()) {
            Some(username) => client.with_basic_auth(username, password),
            None => client,
        };

        Self { client }
    }

    /// Fetches the catalog root, returning its title: proves the URL serves an
    /// OPDS catalog and that the credentials (if any) are accepted.
    pub async fn check_catalog(
        url: String,
        username: Option<String>,
        password: Option<String>,
    ) -> Result<String, String> {
        let catalog = Self::connect(url, username, password)
            .client
            .catalog()
            .await
            .map_err(crate::error::log_and_stringify)?;

        Ok(catalog.metadata.title)
    }

    /// Discovers the catalog feeds to import as libraries.
    ///
    /// Each navigation link of the root catalog is a candidate. Most become a
    /// library as-is ("On Deck", "Recently Added", a publications feed, …), but
    /// a *catalog of catalogs* — a navigation feed whose entries are themselves
    /// navigation feeds, like Kavita's "All Libraries" or Komga's "Libraries" —
    /// is expanded into one library per entry instead, since those entries are
    /// what users think of as libraries. Empty feeds are skipped. A root that
    /// lists publications directly (and nothing else) is itself the library.
    pub async fn list_libraries(&self, integration_id: &str) -> Result<Vec<Library>, String> {
        let root = self
            .client
            .catalog()
            .await
            .map_err(crate::error::log_and_stringify)?;
        let root_url = self
            .client
            .url("")
            .map_err(crate::error::log_and_stringify)?
            .to_string();

        let mut seen: HashSet<String> = HashSet::from([root_url.clone()]);
        if let Some(link) = root.link(opds_client::models::rel::SELF) {
            seen.insert(link.href.clone());
        }

        let mut libraries = Vec::new();

        for candidate in root.all_navigation() {
            let href = candidate.link.href.clone();
            if !seen.insert(href.clone()) {
                continue;
            }

            let feed = match self.client.feed(&href).await {
                Ok(feed) => feed,
                Err(error) => {
                    tracing::debug!("skipping unreadable catalog feed {href}: {error}");
                    continue;
                }
            };

            if feed.is_empty() {
                continue;
            }

            if self.is_catalog_of_catalogs(&feed).await {
                for library in self.navigation_of(feed, &href).await {
                    if seen.insert(library.link.href.clone()) {
                        libraries.push(OpdsLibrary {
                            title: library.title().to_string(),
                            url: library.link.href,
                        });
                    }
                }
            } else {
                libraries.push(OpdsLibrary {
                    title: candidate.title().to_string(),
                    url: href,
                });
            }
        }

        if libraries.is_empty() && root.all_publications().next().is_some() {
            libraries.push(OpdsLibrary {
                title: root.metadata.title.clone(),
                url: root_url,
            });
        }

        Ok(libraries
            .into_iter()
            .map(|library| OpdsLibraryAdapter::adapt_library(library, integration_id))
            .collect())
    }

    /// Whether `feed` only links to other navigation-only feeds (judged by its
    /// first entry).
    async fn is_catalog_of_catalogs(&self, feed: &Feed) -> bool {
        if !feed.is_navigation_only() {
            return false;
        }

        let Some(first) = feed.all_navigation().next() else {
            return false;
        };

        self.client
            .feed(&first.link.href)
            .await
            .is_ok_and(|child| child.is_navigation_only())
    }

    /// Every navigation entry of a feed, across all of its pages.
    async fn navigation_of(
        &self,
        first_page: Feed,
        url: &str,
    ) -> Vec<opds_client::models::feed::Navigation> {
        let mut navigation = Vec::new();
        let mut page = Some(first_page);
        let mut visited = HashSet::from([url.to_string()]);

        while let Some(feed) = page.take() {
            navigation.extend(feed.all_navigation().cloned());

            let Some(next) = feed.next().map(|link| link.href.clone()) else {
                break;
            };
            if !visited.insert(next.clone()) || visited.len() > MAX_FEED_PAGES {
                break;
            }
            page = self.client.feed(&next).await.ok();
        }

        navigation
    }

    /// The browsable entries of one fetched feed page, in display order:
    /// navigation links first, then publications.
    fn catalog_entries(feed: &Feed, url: &str) -> Vec<CatalogEntry> {
        feed.all_navigation()
            .cloned()
            .map(CatalogEntry::Navigation)
            .chain(
                feed.all_publications()
                    .enumerate()
                    .map(|(index, publication)| CatalogEntry::Publication {
                        feed: url.to_string(),
                        index,
                        publication: publication.clone(),
                    }),
            )
            .collect()
    }

    /// Lists the items on browse page `page` (1-based) of `page_size` items of
    /// the feed at `url`. The browse screen's fixed-size pages don't line up
    /// with the server's, so the feed's own pages are walked through `next`
    /// links, skipping already-measured ones (see [`FEED_PAGES`]) and fetching
    /// only those overlapping the requested window.
    async fn list_feed_items(
        &self,
        url: &str,
        page: Option<u32>,
        page_size: Option<u32>,
    ) -> Result<Vec<LibraryItem>, String> {
        let page_size = page_size.map_or(usize::MAX, |size| size as usize);
        let start = (page.unwrap_or(1).max(1) as usize - 1).saturating_mul(page_size);
        let end = start.saturating_add(page_size);

        let mut items = Vec::new();
        let mut offset = 0;
        let mut current = Some(url.to_string());
        let mut index = 0;

        while let Some(page_url) = current.take() {
            if offset >= end || index >= MAX_FEED_PAGES {
                break;
            }

            let known = FEED_PAGES
                .lock()
                .ok()
                .and_then(|pages| pages.get(url).and_then(|pages| pages.get(index).cloned()));

            if let Some(known) = known.filter(|known| offset + known.len <= start) {
                offset += known.len;
                current = known.next;
                index += 1;
                continue;
            }

            let feed = self
                .client
                .feed(&page_url)
                .await
                .map_err(crate::error::log_and_stringify)?;
            let entries = Self::catalog_entries(&feed, &page_url);
            let next = feed
                .next()
                .map(|link| link.href.clone())
                .filter(|next| *next != page_url);

            remember_page(
                url,
                index,
                FeedPage {
                    len: entries.len(),
                    next: next.clone(),
                },
            );

            for (position, entry) in (offset..).zip(entries.iter()) {
                if (start..end).contains(&position) {
                    remember_cover(&entry.id().encode(), entry.cover_href());
                    items.push(OpdsItemAdapter::adapt_item(entry.clone()));
                }
            }

            if entries.is_empty() {
                break;
            }

            offset += entries.len();
            current = next;
            index += 1;
        }

        Ok(items)
    }

    /// Every publication of the feed at `url` (all pages), each with the URL of
    /// the page it was on and its position there.
    async fn feed_publications(
        &self,
        url: &str,
    ) -> Result<Vec<(String, usize, Publication)>, String> {
        let mut publications = Vec::new();
        let mut current = Some(url.to_string());
        let mut visited = HashSet::new();
        let mut nested = Vec::new();

        while let Some(page_url) = current.take() {
            if !visited.insert(page_url.clone()) || visited.len() > MAX_FEED_PAGES {
                break;
            }

            let feed = self
                .client
                .feed(&page_url)
                .await
                .map_err(crate::error::log_and_stringify)?;

            publications.extend(
                feed.all_publications()
                    .enumerate()
                    .map(|(index, publication)| (page_url.clone(), index, publication.clone())),
            );
            nested.extend(
                feed.all_navigation()
                    .map(|navigation| navigation.link.href.clone()),
            );

            current = feed.next().map(|link| link.href.clone());
        }

        // A feed of sub-feeds (e.g. a series split into story arcs): gather the
        // publications one level down instead of showing nothing.
        if publications.is_empty() {
            for href in nested.into_iter().take(MAX_NESTED_FEEDS) {
                match self.client.feed(&href).await {
                    Ok(feed) => publications.extend(
                        feed.all_publications()
                            .enumerate()
                            .map(|(index, publication)| (href.clone(), index, publication.clone())),
                    ),
                    Err(error) => tracing::debug!("skipping unreadable feed {href}: {error}"),
                }
            }
        }

        Ok(publications)
    }

    /// Finds a publication again from its id: refetches the page it was listed
    /// on and takes the one at its position, falling back to a key match if the
    /// feed has shifted since.
    async fn find_publication(
        &self,
        feed: &str,
        index: usize,
        key: &str,
    ) -> Result<Publication, String> {
        let page = self
            .client
            .feed(feed)
            .await
            .map_err(crate::error::log_and_stringify)?;

        page.all_publications()
            .nth(index)
            .filter(|publication| publication.key() == key)
            .or_else(|| {
                page.all_publications()
                    .find(|publication| publication.key() == key)
            })
            .cloned()
            .ok_or_else(|| "the publication is no longer in the catalog".to_string())
    }

    /// The cover URL for an item or entry id: remembered from the listing that
    /// produced it, else looked up from the catalog (a publication's own cover;
    /// a feed's artwork, or that of its first publication).
    async fn cover_href(&self, id: &str) -> Result<String, String> {
        if let Some(href) = COVER_HREFS
            .lock()
            .ok()
            .and_then(|covers| covers.get(id).cloned())
        {
            return Ok(href);
        }

        let href = match OpdsId::decode(id)? {
            OpdsId::Publication { feed, index, key } => self
                .find_publication(&feed, index, &key)
                .await?
                .cover()
                .map(|link| link.href.clone()),
            OpdsId::Feed(url) => {
                let feed = self
                    .client
                    .feed(&url)
                    .await
                    .map_err(crate::error::log_and_stringify)?;
                feed.cover()
                    .or_else(|| feed.all_publications().find_map(Publication::cover))
                    .map(|link| link.href.clone())
            }
        };

        let href = href.ok_or_else(|| "no cover image available".to_string())?;
        remember_cover(id, Some(&href));
        Ok(href)
    }

    /// Returns the cover for `id`, from the on-disk cache when present (the
    /// same cache the other providers use), otherwise downloading and caching
    /// it.
    async fn cached_cover(&self, key: String, id: &str) -> Result<Vec<u8>, String> {
        if let Some(bytes) = crate::cache::read_cover(&key) {
            return Ok(bytes);
        }

        let href = self.cover_href(id).await?;
        let bytes = self
            .client
            .bytes(&href)
            .await
            .map_err(crate::error::log_and_stringify)?;

        if let Err(error) = crate::cache::write_cover(&key, &bytes) {
            tracing::debug!("failed to cache cover for {key}: {error}");
        }

        Ok(bytes)
    }

    /// Prepares a publication for the reader: downloads its file through the
    /// catalog's open-access acquisition link and extracts its pages (see
    /// [`archive`]). A prepared publication is kept, so reopening it doesn't
    /// download it again. Returns the prepared folder's key and page count.
    async fn prepare(&self, publication: &Publication) -> Result<(String, usize), String> {
        let link = readable_download(publication)?;
        let key = OpdsId::Feed(link.href.clone()).encode();
        let pages_dir = prepared_dir(&key)?;

        if pages_dir.is_dir() {
            return Ok((key, archive::page_count(&pages_dir)));
        }

        if let Some(parent) = pages_dir.parent() {
            std::fs::create_dir_all(parent).map_err(crate::error::log_and_stringify)?;
        }

        // Downloaded and extracted aside, then moved into place, so a folder
        // that exists is always a complete one.
        let download = pages_dir.with_extension("download");
        let staging = pages_dir.with_extension("partial");

        let result = async {
            self.client
                .download(&link.href, &download)
                .await
                .map_err(crate::error::log_and_stringify)?;

            let _ = std::fs::remove_dir_all(&staging);
            std::fs::create_dir_all(&staging).map_err(crate::error::log_and_stringify)?;
            let count = archive::extract_pages(&download, &staging)?;
            std::fs::rename(&staging, &pages_dir).map_err(crate::error::log_and_stringify)?;

            Ok::<_, String>(count)
        }
        .await;

        let _ = std::fs::remove_file(&download);
        if result.is_err() {
            let _ = std::fs::remove_dir_all(&staging);
        }

        result.map(|count| (key, count))
    }
}

/// The open-access download to read a publication from: a comic archive
/// (CBZ/CBR) when the catalog labels its files, else an unlabelled one (the
/// downloaded content is checked anyway).
fn readable_download(publication: &Publication) -> Result<&Link, String> {
    let downloads = publication.open_access();

    if downloads.is_empty() {
        return Err(
            "this publication can't be read: the catalog offers no open-access download for it"
                .to_string(),
        );
    }

    let is_comic_archive = |link: &Link| {
        link.essence().is_some_and(|essence| {
            media_type::CBZ.contains(&essence.as_str())
                || media_type::CBR.contains(&essence.as_str())
        })
    };

    downloads
        .iter()
        .find(|link| is_comic_archive(link))
        .or_else(|| downloads.iter().find(|link| link.media_type.is_none()))
        .copied()
        .ok_or_else(|| {
            let formats: Vec<String> = downloads.iter().filter_map(|link| link.essence()).collect();
            format!(
                "this publication can't be read yet: it's only offered as {}",
                formats.join(", ")
            )
        })
}

/// Where a publication's extracted pages live, by the key [`OpdsProvider::prepare`]
/// returns.
fn prepared_dir(key: &str) -> Result<PathBuf, String> {
    crate::cache::download_path(&format!("opds-{key}"))
        .ok_or_else(|| "no cache directory available for downloads".to_string())
}

fn remember_page(feed: &str, index: usize, page: FeedPage) {
    let Ok(mut pages) = FEED_PAGES.lock() else {
        return;
    };
    let pages = pages.entry(feed.to_string()).or_default();
    if index < pages.len() {
        pages[index] = page;
    } else if index == pages.len() {
        pages.push(page);
    }
}

fn remember_cover(id: &str, href: Option<&str>) {
    if let (Some(href), Ok(mut covers)) = (href, COVER_HREFS.lock()) {
        covers.insert(id.to_string(), href.to_string());
    }
}

impl Provider for OpdsProvider {
    async fn list_items(
        &self,
        library_external_id: &str,
        page: Option<u32>,
        page_size: Option<u32>,
    ) -> Result<Vec<LibraryItem>, String> {
        let OpdsId::Feed(url) = OpdsId::decode(library_external_id)? else {
            return Err("an OPDS library must be a feed".to_string());
        };

        self.list_feed_items(&url, page, page_size).await
    }

    async fn list_series_entries(&self, series_id: &str) -> Result<Vec<SeriesEntry>, String> {
        let entries: Vec<SeriesEntry> = match OpdsId::decode(series_id)? {
            OpdsId::Feed(url) => self
                .feed_publications(&url)
                .await?
                .iter()
                .map(|(feed, index, publication)| {
                    let entry = publication_entry(feed, *index, publication);
                    remember_cover(
                        &entry.external_id,
                        publication.cover().map(|link| link.href.as_str()),
                    );
                    entry
                })
                .collect(),
            OpdsId::Publication { feed, index, key } => {
                let publication = self.find_publication(&feed, index, &key).await?;
                let entry = publication_entry(&feed, index, &publication);
                remember_cover(
                    &entry.external_id,
                    publication.cover().map(|link| link.href.as_str()),
                );
                vec![entry]
            }
        };

        Ok(entries)
    }

    async fn cover(&self, integration_id: &str, series_id: &str) -> Result<Vec<u8>, String> {
        self.cached_cover(format!("{integration_id}-{series_id}"), series_id)
            .await
    }

    async fn volume_cover(&self, integration_id: &str, volume_id: &str) -> Result<Vec<u8>, String> {
        self.cached_cover(format!("{integration_id}-vol-{volume_id}"), volume_id)
            .await
    }

    async fn chapter_cover(
        &self,
        integration_id: &str,
        chapter_id: &str,
    ) -> Result<Vec<u8>, String> {
        self.cached_cover(format!("{integration_id}-ch-{chapter_id}"), chapter_id)
            .await
    }

    async fn reader_pages(&self, entry: &SeriesEntry) -> Result<Vec<PageRef>, String> {
        let OpdsId::Publication { feed, index, key } = OpdsId::decode(&entry.external_id)? else {
            return Err("only a publication can be read".to_string());
        };

        let publication = self.find_publication(&feed, index, &key).await?;
        let (prepared, count) = self.prepare(&publication).await?;

        Ok((0..count.min(usize::from(u16::MAX)) as u16)
            .map(|page| PageRef {
                chapter_id: prepared.clone(),
                page,
            })
            .collect())
    }

    async fn reader_image(&self, chapter_id: &str, page: u16) -> Result<Vec<u8>, String> {
        let path = archive::page_path(&prepared_dir(chapter_id)?, usize::from(page));
        std::fs::read(&path).map_err(|error| {
            tracing::debug!("could not read prepared page {}: {error}", path.display());
            format!("page {page} of this publication hasn't been prepared")
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use wiremock::matchers::{method, path, query_param};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    async fn serve_json(server: &MockServer, route: &str, body: String) {
        Mock::given(method("GET"))
            .and(path(route))
            .respond_with(ResponseTemplate::new(200).set_body_raw(body, "application/opds+json"))
            .mount(server)
            .await;
    }

    async fn serve_atom(server: &MockServer, route: &str, entries: &str) {
        Mock::given(method("GET"))
            .and(path(route))
            .respond_with(ResponseTemplate::new(200).set_body_raw(
                format!(
                    r#"<feed xmlns="http://www.w3.org/2005/Atom"><title>t</title>{entries}</feed>"#
                ),
                "application/xml",
            ))
            .mount(server)
            .await;
    }

    fn atom_nav(title: &str, href: &str) -> String {
        format!(
            r#"<entry><title>{title}</title><link rel="subsection" type="application/atom+xml;profile=opds-catalog" href="{href}"/></entry>"#
        )
    }

    fn atom_publication(id: &str, pages: u32) -> String {
        format!(
            r#"<entry><id>{id}</id><title>Issue {id}</title>
               <link rel="http://opds-spec.org/acquisition/open-access" type="application/x-cbz" href="/download/{id}.cbz" pse:count="{pages}" xmlns:pse="http://vaemendis.net/opds-pse/ns"/>
               <link rel="http://vaemendis.net/opds-pse/stream" type="image/jpeg" href="/image?id={id}&amp;page={{pageNumber}}" pse:count="{pages}" xmlns:pse="http://vaemendis.net/opds-pse/ns"/></entry>"#
        )
    }

    fn names(libraries: &[Library]) -> Vec<&str> {
        libraries
            .iter()
            .map(|library| library.name.as_str())
            .collect()
    }

    #[tokio::test]
    async fn list_libraries_expands_catalogs_of_catalogs_and_skips_empty_and_self_links() {
        let server = MockServer::start().await;
        serve_atom(
            &server,
            "/opds",
            &[
                atom_nav("On Deck", "/opds/on-deck"),
                atom_nav("Home again", "/opds"),
                atom_nav("Want to Read", "/opds/want"),
                atom_nav("All Libraries", "/opds/libraries"),
            ]
            .concat(),
        )
        .await;
        serve_atom(
            &server,
            "/opds/on-deck",
            &atom_nav("A series", "/opds/series/1"),
        )
        .await;
        serve_atom(&server, "/opds/series/1", &atom_publication("1", 3)).await;
        serve_atom(&server, "/opds/want", "").await;
        serve_atom(
            &server,
            "/opds/libraries",
            &[
                atom_nav("Manga", "/opds/libraries/1"),
                atom_nav("Comics", "/opds/libraries/2"),
            ]
            .concat(),
        )
        .await;
        serve_atom(
            &server,
            "/opds/libraries/1",
            &atom_nav("A series", "/opds/series/1"),
        )
        .await;

        let provider = OpdsProvider::connect(format!("{}/opds", server.uri()), None, None);
        let libraries = provider.list_libraries("integration").await.unwrap();

        assert_eq!(names(&libraries), vec!["On Deck", "Manga", "Comics"]);
        assert_eq!(
            OpdsId::decode(libraries[1].external_id.as_deref().unwrap()).unwrap(),
            OpdsId::Feed(format!("{}/opds/libraries/1", server.uri()))
        );
    }

    #[tokio::test]
    async fn list_libraries_uses_an_opds2_root_that_only_lists_publications() {
        let server = MockServer::start().await;
        serve_json(
            &server,
            "/catalog.json",
            r#"{"metadata": {"title": "Shelf"},
                "publications": [{"metadata": {"title": "Book"}, "links": []}]}"#
                .to_string(),
        )
        .await;

        let provider = OpdsProvider::connect(format!("{}/catalog.json", server.uri()), None, None);
        let libraries = provider.list_libraries("integration").await.unwrap();

        assert_eq!(names(&libraries), vec!["Shelf"]);
    }

    #[tokio::test]
    async fn list_items_maps_browse_pages_onto_the_servers_pages() {
        let server = MockServer::start().await;
        let feed_page = |ids: std::ops::Range<u32>, next: Option<&str>| {
            let navigation: Vec<String> = ids
                .map(|id| format!(r#"{{"href": "/series/{id}", "title": "S{id}"}}"#))
                .collect();
            let next = next
                .map(|href| format!(r#"{{"rel": "next", "href": "{href}"}}"#))
                .unwrap_or_default();
            format!(
                r#"{{"metadata": {{"title": "L"}}, "links": [{next}], "navigation": [{}]}}"#,
                navigation.join(",")
            )
        };
        // The server pages by 3; the browse screen asks for pages of 2.
        Mock::given(method("GET"))
            .and(path("/library"))
            .and(query_param("page", "2"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_raw(feed_page(4..7, None), "application/opds+json"),
            )
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/library"))
            .respond_with(ResponseTemplate::new(200).set_body_raw(
                feed_page(1..4, Some("/library?page=2")),
                "application/opds+json",
            ))
            .mount(&server)
            .await;

        let provider = OpdsProvider::connect(server.uri(), None, None);
        let library = OpdsId::Feed(format!("{}/library", server.uri())).encode();
        let page = |n| Provider::list_items(&provider, &library, Some(n), Some(2));

        let names =
            |items: Vec<LibraryItem>| items.into_iter().map(|item| item.name).collect::<Vec<_>>();
        assert_eq!(names(page(1).await.unwrap()), vec!["S1", "S2"]);
        assert_eq!(names(page(2).await.unwrap()), vec!["S3", "S4"]);
        assert_eq!(names(page(3).await.unwrap()), vec!["S5", "S6"]);
        assert!(page(4).await.unwrap().is_empty());
    }

    /// A CBZ holding `pages` one-byte images (byte = page number, 1-based),
    /// stored out of name order, plus a non-image entry.
    fn cbz(pages: u8) -> Vec<u8> {
        use std::io::Write;

        let mut archive = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
        let options = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Stored);
        archive.start_file("ComicInfo.xml", options).unwrap();
        archive.write_all(b"<ComicInfo/>").unwrap();
        for page in (1..=pages).rev() {
            archive
                .start_file(format!("{page:03}.jpg"), options)
                .unwrap();
            archive.write_all(&[page]).unwrap();
        }
        archive.finish().unwrap().into_inner()
    }

    async fn serve_download(server: &MockServer, route: &str, body: Vec<u8>, times: u64) {
        Mock::given(method("GET"))
            .and(path(route))
            .respond_with(ResponseTemplate::new(200).set_body_raw(body, "application/x-cbz"))
            .expect(times)
            .mount(server)
            .await;
    }

    fn remove_prepared(pages: &[PageRef]) {
        if let Some(page) = pages.first()
            && let Ok(dir) = prepared_dir(&page.chapter_id)
        {
            let _ = std::fs::remove_dir_all(dir);
        }
    }

    #[tokio::test]
    async fn a_series_is_read_by_downloading_its_open_access_file_not_streaming_pages() {
        let server = MockServer::start().await;
        serve_atom(
            &server,
            "/series/4",
            &[atom_publication("75", 2), atom_publication("76", 3)].concat(),
        )
        .await;
        serve_download(&server, "/download/76.cbz", cbz(3), 1).await;
        // The page-stream endpoint must never be used for reading.
        Mock::given(method("GET"))
            .and(path("/image"))
            .respond_with(ResponseTemplate::new(200))
            .expect(0)
            .mount(&server)
            .await;

        let provider = OpdsProvider::connect(server.uri(), None, None);
        let series = OpdsId::Feed(format!("{}/series/4", server.uri())).encode();

        let entries = Provider::list_series_entries(&provider, &series)
            .await
            .unwrap();
        assert_eq!(
            entries
                .iter()
                .map(|entry| entry.label.as_str())
                .collect::<Vec<_>>(),
            vec!["Issue 75", "Issue 76"]
        );
        assert_eq!(entries[1].pages, Some(3));

        let pages = match Provider::reader_pages(&provider, &entries[1]).await {
            Ok(pages) => pages,
            // No cache directory in this environment: nothing to prepare into.
            Err(error) if error.contains("cache directory") => return,
            Err(error) => panic!("{error}"),
        };
        assert_eq!(pages.len(), 3);

        let mut read = Vec::new();
        for page in &pages {
            read.extend(
                Provider::reader_image(&provider, &page.chapter_id, page.page)
                    .await
                    .unwrap(),
            );
        }
        assert_eq!(read, vec![1, 2, 3]);

        remove_prepared(&pages);
    }

    #[tokio::test]
    async fn a_prepared_publication_is_not_downloaded_again() {
        let server = MockServer::start().await;
        serve_atom(&server, "/feed", &atom_publication("9", 2)).await;
        serve_download(&server, "/download/9.cbz", cbz(2), 1).await;

        let provider = OpdsProvider::connect(server.uri(), None, None);
        let feed = OpdsId::Feed(format!("{}/feed", server.uri())).encode();
        let entries = Provider::list_series_entries(&provider, &feed)
            .await
            .unwrap();

        let first = match Provider::reader_pages(&provider, &entries[0]).await {
            Ok(pages) => pages,
            Err(error) if error.contains("cache directory") => return,
            Err(error) => panic!("{error}"),
        };
        let second = Provider::reader_pages(&provider, &entries[0])
            .await
            .unwrap();

        assert_eq!(first, second);
        remove_prepared(&first);
    }

    #[tokio::test]
    async fn an_opds2_publication_is_read_from_its_plain_acquisition_link() {
        let server = MockServer::start().await;
        serve_json(
            &server,
            "/feed",
            r#"{"metadata": {"title": "F"}, "publications": [{
                "metadata": {"title": "Comic", "identifier": "urn:comic"},
                "links": [
                    {"rel": "http://opds-spec.org/acquisition/buy", "href": "/buy", "type": "application/x-cbz"},
                    {"rel": "http://opds-spec.org/acquisition", "href": "/comic.cbz",
                     "type": "application/vnd.comicbook+zip"}]}]}"#
                .to_string(),
        )
        .await;
        serve_download(&server, "/comic.cbz", cbz(2), 1).await;

        let provider = OpdsProvider::connect(server.uri(), None, None);
        let feed = OpdsId::Feed(format!("{}/feed", server.uri())).encode();
        let entries = Provider::list_series_entries(&provider, &feed)
            .await
            .unwrap();

        let pages = match Provider::reader_pages(&provider, &entries[0]).await {
            Ok(pages) => pages,
            Err(error) if error.contains("cache directory") => return,
            Err(error) => panic!("{error}"),
        };
        assert_eq!(pages.len(), 2);
        remove_prepared(&pages);
    }

    #[tokio::test]
    async fn a_failed_download_leaves_nothing_prepared() {
        let server = MockServer::start().await;
        serve_atom(&server, "/feed", &atom_publication("5", 2)).await;
        Mock::given(method("GET"))
            .and(path("/download/5.cbz"))
            .respond_with(ResponseTemplate::new(200).set_body_raw("not a zip", "application/x-cbz"))
            .mount(&server)
            .await;

        let provider = OpdsProvider::connect(server.uri(), None, None);
        let feed = OpdsId::Feed(format!("{}/feed", server.uri())).encode();
        let entries = Provider::list_series_entries(&provider, &feed)
            .await
            .unwrap();

        let error = Provider::reader_pages(&provider, &entries[0])
            .await
            .unwrap_err();
        assert!(!error.is_empty());

        let key = OpdsId::Feed(format!("{}/download/5.cbz", server.uri())).encode();
        if let Ok(dir) = prepared_dir(&key) {
            assert!(!dir.exists());
            assert!(!dir.with_extension("partial").exists());
            assert!(!dir.with_extension("download").exists());
        }
    }

    #[tokio::test]
    async fn an_unreadable_publication_names_the_formats_on_offer() {
        let server = MockServer::start().await;
        serve_atom(
            &server,
            "/feed",
            r#"<entry><id>1</id><title>Novel</title>
               <link rel="http://opds-spec.org/acquisition/open-access" type="application/epub+zip" href="/n.epub"/></entry>
               <entry><id>2</id><title>Paid</title>
               <link rel="http://opds-spec.org/acquisition/buy" type="application/x-cbz" href="/p.cbz"/></entry>"#,
        )
        .await;

        let provider = OpdsProvider::connect(server.uri(), None, None);
        let feed = OpdsId::Feed(format!("{}/feed", server.uri())).encode();
        let entries = Provider::list_series_entries(&provider, &feed)
            .await
            .unwrap();

        let novel = Provider::reader_pages(&provider, &entries[0])
            .await
            .unwrap_err();
        assert!(novel.contains("application/epub+zip"), "{novel}");

        let paid = Provider::reader_pages(&provider, &entries[1])
            .await
            .unwrap_err();
        assert!(paid.contains("no open-access download"), "{paid}");
    }

    /// End-to-end walk of a real catalog, from library discovery down to the
    /// bytes of a page: `OPDS_LIVE_URL=<catalog url> cargo test -p sabita-app
    /// opds_live -- --ignored --nocapture`.
    #[test]
    #[ignore = "needs a live OPDS catalog in OPDS_LIVE_URL"]
    fn opds_live_walks_a_catalog_down_to_a_page() {
        let Ok(url) = std::env::var("OPDS_LIVE_URL") else {
            return;
        };

        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();

        runtime.block_on(async {
            let provider = OpdsProvider::connect(url, None, None);

            let libraries = provider.list_libraries("live").await.unwrap();
            println!(
                "libraries: {:?}",
                libraries.iter().map(|l| &l.name).collect::<Vec<_>>()
            );
            assert!(!libraries.is_empty());

            for library in &libraries {
                let library_id = library.external_id.as_deref().unwrap();
                let items = Provider::list_items(&provider, library_id, Some(1), Some(20))
                    .await
                    .unwrap();
                println!("{}: {} items on page 1", library.name, items.len());

                let Some(item) = items.first() else { continue };
                let entries = Provider::list_series_entries(&provider, &item.external_id)
                    .await
                    .unwrap();
                println!("  {}: {} entries", item.name, entries.len());

                let Some(entry) = entries.first() else {
                    continue;
                };
                let pages = match Provider::reader_pages(&provider, entry).await {
                    Ok(pages) => pages,
                    Err(error) => {
                        println!("    {}: not readable ({error})", entry.label);
                        continue;
                    }
                };
                let page = &pages[0];
                let bytes = Provider::reader_image(&provider, &page.chapter_id, page.page)
                    .await
                    .unwrap();
                println!(
                    "    {}: {} pages, first is {} bytes",
                    entry.label,
                    pages.len(),
                    bytes.len()
                );
                assert!(!bytes.is_empty());
                return;
            }

            panic!("no readable publication found in any library");
        });
    }
}
