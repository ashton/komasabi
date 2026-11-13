//! The downloads screen: every volume/chapter downloaded for reading offline
//! (see [`crate::downloads`]), under the series it's from. Opening one reads
//! it from its download, and each card's menu removes the download.

use std::collections::HashMap;

use iced::widget::image;
use iced::{Element, Task};

use crate::downloads::DownloadedEntry;
use crate::models::AsyncModel;

#[derive(Debug, Default)]
pub struct Downloads {
    entries: AsyncModel<Vec<DownloadedEntry>, String>,
    /// The key ([`key`]) of the entry whose card menu is open, if any.
    menu: Option<String>,
    /// Whether an entry is being opened in the reader, so it isn't opened twice.
    launching: bool,
    /// The last failure to read or remove a download, shown until the next
    /// attempt.
    error: Option<String>,
    /// Decoded covers (each download's first page), keyed by [`key`].
    covers: HashMap<String, image::Handle>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Message {
    Loaded(Vec<DownloadedEntry>),
    /// Cover bytes read from the downloads' first pages, keyed by [`key`].
    CoversLoaded(Vec<(String, Vec<u8>)>),
    /// A download's cover was clicked: read it.
    Read(DownloadedEntry),
    /// The reader opened, or failed to.
    ReaderLaunched(Result<(), String>),
    /// The "⋮" button of a download's card (by [`key`]) was pressed: open its
    /// menu, or close it if it's the one open.
    MenuToggled(String),
    /// A click landed outside the open menu, closing it.
    MenuDismissed,
    /// "Remove download" was chosen in a download's menu: delete its files.
    Remove(DownloadedEntry),
    /// A download (by [`key`]) was deleted, or failed to be.
    Removed(String, Result<(), String>),
}

#[derive(Debug)]
pub enum Action {
    None,
    Run(Task<Message>),
}

/// What tells downloads apart: their folder.
fn key(entry: &DownloadedEntry) -> String {
    entry.dir.to_string_lossy().into_owned()
}

impl Downloads {
    pub fn new() -> (Self, Task<Message>) {
        (
            Self {
                entries: AsyncModel::Loading,
                ..Self::default()
            },
            Task::perform(crate::downloads::all(), Message::Loaded),
        )
    }

    pub fn update(&mut self, message: Message) -> Action {
        // Doing anything else on the screen closes an open card menu.
        if matches!(
            message,
            Message::Read(_) | Message::Remove(_) | Message::MenuDismissed
        ) {
            self.menu = None;
        }

        match message {
            Message::Loaded(entries) => {
                let pages: Vec<(String, std::path::PathBuf)> = entries
                    .iter()
                    .filter(|entry| !self.covers.contains_key(&key(entry)))
                    .map(|entry| (key(entry), entry.first_page()))
                    .collect();
                self.entries = AsyncModel::Loaded(entries);

                if pages.is_empty() {
                    Action::None
                } else {
                    Action::Run(Task::perform(read_covers(pages), Message::CoversLoaded))
                }
            }
            Message::CoversLoaded(covers) => {
                for (key, bytes) in covers {
                    self.covers.insert(key, image::Handle::from_bytes(bytes));
                }
                Action::None
            }

            Message::Read(download) => {
                if self.launching {
                    return Action::None;
                }
                self.launching = true;
                self.error = None;

                Action::Run(Task::perform(
                    crate::runtime::on_tokio(crate::reader_launch::prepare_and_launch(
                        download.integration_id,
                        download.series_id,
                        download.series_name,
                        download.entry,
                    )),
                    Message::ReaderLaunched,
                ))
            }
            Message::ReaderLaunched(result) => {
                self.launching = false;
                if let Err(error) = result {
                    tracing::debug!("failed to open the reader: {error}");
                    self.error = Some(error);
                }
                Action::None
            }

            Message::MenuToggled(key) => {
                self.menu = (self.menu.as_ref() != Some(&key)).then_some(key);
                Action::None
            }
            Message::MenuDismissed => Action::None,

            Message::Remove(download) => {
                self.error = None;
                let key = key(&download);

                Action::Run(Task::perform(
                    crate::downloads::remove(download.dir),
                    move |result| Message::Removed(key.clone(), result),
                ))
            }
            Message::Removed(removed, Ok(())) => {
                if let AsyncModel::Loaded(entries) = &mut self.entries {
                    entries.retain(|entry| key(entry) != removed);
                }
                Action::None
            }
            Message::Removed(_, Err(error)) => {
                tracing::debug!("failed to remove the download: {error}");
                self.error = Some(format!("Could not remove the download: {error}"));
                Action::None
            }
        }
    }

    pub fn view<'a>(&'a self) -> Element<'a, Message> {
        view_helper::view(
            &self.entries,
            &self.covers,
            self.menu.as_deref(),
            self.error.as_deref(),
        )
    }
}

/// Reads the given cover files, leaving out any that can't be read. They're
/// read as bytes, not opened as paths: page files are named `.img`, and an
/// image opened by path has its format guessed from its extension, which
/// fails, while bytes have it guessed from their content.
async fn read_covers(pages: Vec<(String, std::path::PathBuf)>) -> Vec<(String, Vec<u8>)> {
    pages
        .into_iter()
        .filter_map(|(key, path)| match std::fs::read(&path) {
            Ok(bytes) => Some((key, bytes)),
            Err(error) => {
                tracing::debug!("no cover at {}: {error}", path.display());
                None
            }
        })
        .collect()
}

mod view_helper {
    use std::collections::HashMap;

    use iced::{
        Background, Border, Center, ContentFit, Element, Font,
        Length::Fill,
        Theme, border,
        widget::{
            button, column, container, grid, image, mouse_area, row, scrollable, stack, text,
        },
    };
    use iced_font_awesome::fa_icon_solid;

    use super::{Message, key};
    use crate::downloads::DownloadedEntry;
    use crate::models::AsyncModel;
    use crate::screen::card::{self, COVER_RADIUS};
    use crate::touch_scroll::touch_scroll;

    pub fn view<'a>(
        entries: &'a AsyncModel<Vec<DownloadedEntry>, String>,
        covers: &'a HashMap<String, image::Handle>,
        open_menu: Option<&str>,
        error: Option<&'a str>,
    ) -> Element<'a, Message> {
        let body: Element<'a, Message> = match entries {
            // Centered on the whole screen, with no title above it.
            AsyncModel::Loaded(entries) if entries.is_empty() && error.is_none() => {
                return empty_state();
            }
            AsyncModel::Loaded(entries) => series_sections(entries, covers, open_menu),
            AsyncModel::Error(error) => container(text(error)).center_x(Fill).padding(40).into(),
            AsyncModel::NotLoaded | AsyncModel::Loading => container(row![]).padding(40).into(),
        };

        let title = text("Downloads").size(28).font(Font {
            weight: iced::font::Weight::Bold,
            ..Font::default()
        });

        let mut content = column![title].spacing(24).padding(24).width(Fill);
        if let Some(error) = error {
            content = content.push(text(error).size(13));
        }

        let screen = touch_scroll(scrollable(content.push(body)).width(Fill).height(Fill));

        // With a card's menu open, a click anywhere the menu (or another
        // button) doesn't take closes it.
        if open_menu.is_some() {
            mouse_area(screen).on_press(Message::MenuDismissed).into()
        } else {
            screen.into()
        }
    }

    /// The downloads under a heading per series. `entries` is already sorted
    /// by series (see [`crate::downloads::all`]).
    fn series_sections<'a>(
        entries: &'a [DownloadedEntry],
        covers: &'a HashMap<String, image::Handle>,
        open_menu: Option<&str>,
    ) -> Element<'a, Message> {
        let mut sections = column![].spacing(32);
        let mut start = 0;
        while start < entries.len() {
            let first = &entries[start];
            let end = entries[start..]
                .iter()
                .position(|entry| {
                    entry.integration_id != first.integration_id
                        || entry.series_id != first.series_id
                })
                .map_or(entries.len(), |offset| start + offset);

            let cards = entries[start..end].iter().map(|entry| {
                let key = key(entry);
                let menu_open = open_menu == Some(key.as_str());
                entry_card(entry, covers.get(&key), menu_open)
            });
            sections = sections.push(
                column![
                    text(&first.series_name).size(20),
                    grid(cards)
                        .fluid(160.0)
                        .spacing(16.0)
                        .height(grid::aspect_ratio(2.0, 3.0)),
                ]
                .spacing(12),
            );
            start = end;
        }
        sections.into()
    }

    /// A download's card: its first page as its cover, which reads it, over a
    /// bar with its name, page count and menu.
    fn entry_card<'a>(
        download: &'a DownloadedEntry,
        cover: Option<&image::Handle>,
        menu_open: bool,
    ) -> Element<'a, Message> {
        // Shown once read (see `super::read_covers`); until then, or without
        // one, the box's background stands in.
        let cover: Element<'a, Message> = match cover {
            Some(handle) => image(handle.clone())
                .width(Fill)
                .height(Fill)
                .content_fit(ContentFit::Cover)
                .border_radius(border::top(COVER_RADIUS))
                .into(),
            None => row![].into(),
        };
        // Only its top corners are rounded: the bar carries on below it.
        let cover = container(cover)
            .width(Fill)
            .height(Fill)
            .clip(true)
            .style(|theme: &Theme| container::Style {
                background: Some(Background::Color(
                    theme.extended_palette().background.strong.color,
                )),
                border: Border {
                    radius: border::top(COVER_RADIUS),
                    ..Border::default()
                },
                ..container::Style::default()
            });
        let cover: Element<'a, Message> = button(cover)
            .on_press(Message::Read(download.clone()))
            .padding(0)
            .width(Fill)
            .height(Fill)
            .style(button::text)
            .into();

        // The open menu sits over the cover, just above its "⋮" button.
        let cover = if menu_open {
            stack![
                cover,
                card::menu(vec![card::menu_item(
                    "trash",
                    "Remove download",
                    Message::Remove(download.clone()),
                )])
            ]
            .into()
        } else {
            cover
        };

        let pages = download.pages.len();
        let name = column![
            text(&download.entry.label)
                .size(13)
                .wrapping(text::Wrapping::None),
            text(format!("{pages} pages"))
                .size(11)
                .style(text::secondary),
        ]
        .spacing(2);

        column![
            cover,
            card::bar(
                "download",
                name.into(),
                Some(Message::MenuToggled(key(download))),
            )
        ]
        .width(Fill)
        .height(Fill)
        .into()
    }

    fn empty_state<'a>() -> Element<'a, Message> {
        let content = column![
            fa_icon_solid("download")
                .size(64.0_f32)
                .style(|theme: &Theme| text::Style {
                    color: Some(theme.extended_palette().background.neutral.color),
                }),
            text("Nenhum item baixado, baixe uma série pelo menu do seu card").center(),
        ]
        .spacing(20)
        .max_width(420)
        .align_x(Center);

        container(content).center(Fill).into()
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use iced_test::simulator;

    use super::*;
    use crate::models::series_entry::{EntryKind, ReadState, SeriesEntry};
    use crate::providers::PageRef;

    fn download(series: &str, id: &str) -> DownloadedEntry {
        DownloadedEntry {
            integration_id: "src".to_string(),
            series_id: series.to_string(),
            series_name: format!("Series {series}"),
            entry: SeriesEntry {
                external_id: id.to_string(),
                kind: EntryKind::Volume,
                label: format!("Volume {id}"),
                pages: Some(2),
                read_state: ReadState::Unread,
            },
            pages: vec![
                PageRef {
                    chapter_id: id.to_string(),
                    page: 0,
                },
                PageRef {
                    chapter_id: id.to_string(),
                    page: 1,
                },
            ],
            dir: PathBuf::from(format!("/downloads/src/{series}/{id}")),
        }
    }

    fn loaded(entries: Vec<DownloadedEntry>) -> Downloads {
        let mut downloads = Downloads::default();
        let _ = downloads.update(Message::Loaded(entries));
        downloads
    }

    #[test]
    fn covers_are_read_as_bytes_whatever_the_page_files_extension() {
        let dir = std::env::temp_dir().join(format!("komasabi-cover-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let page = dir.join("00000.img");
        std::fs::write(&page, [1, 2, 3]).unwrap();

        let covers = tokio::runtime::Builder::new_current_thread()
            .build()
            .unwrap()
            .block_on(read_covers(vec![
                ("a".to_string(), page),
                ("missing".to_string(), dir.join("nope.img")),
            ]));

        assert_eq!(covers, vec![("a".to_string(), vec![1, 2, 3])]);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn loading_downloads_reads_their_covers() {
        let mut downloads = Downloads::default();

        assert!(matches!(
            downloads.update(Message::Loaded(vec![download("Series", "1")])),
            Action::Run(_)
        ));
        let _ = downloads.update(Message::CoversLoaded(vec![(
            key(&download("Series", "1")),
            vec![1, 2, 3],
        )]));
        assert_eq!(downloads.covers.len(), 1);
        // Already read, so loading again doesn't read it twice.
        assert!(matches!(
            downloads.update(Message::Loaded(vec![download("Series", "1")])),
            Action::None
        ));
    }

    #[test]
    fn new_starts_loading() {
        let (downloads, _task) = Downloads::new();
        assert!(matches!(downloads.entries, AsyncModel::Loading));
    }

    #[test]
    fn lists_every_download_under_its_series() {
        let downloads = loaded(vec![
            download("1", "1"),
            download("1", "2"),
            download("2", "3"),
        ]);

        let mut ui = simulator(downloads.view());
        for label in ["Series 1", "Series 2", "Volume 1", "Volume 2", "Volume 3"] {
            assert!(ui.find(label).is_ok(), "{label} should be shown");
        }
        assert!(ui.find("2 pages").is_ok());
    }

    #[test]
    fn without_downloads_it_says_so() {
        let downloads = loaded(vec![]);

        let mut ui = simulator(downloads.view());
        assert!(
            ui.find("Nenhum item baixado, baixe uma série pelo menu do seu card")
                .is_ok()
        );
    }

    #[test]
    fn the_menu_offers_to_remove_the_download() {
        let mut downloads = loaded(vec![download("1", "1")]);
        let mut ui = simulator(downloads.view());
        assert!(ui.find("Remove download").is_err());
        drop(ui);

        let _ = downloads.update(Message::MenuToggled("/downloads/src/1/1".to_string()));
        let mut ui = simulator(downloads.view());
        let _ = ui
            .click("Remove download")
            .expect("the remove item should be found");
        let messages: Vec<_> = ui.into_messages().collect();
        assert!(messages.contains(&Message::Remove(download("1", "1"))));

        assert!(matches!(
            downloads.update(Message::Remove(download("1", "1"))),
            Action::Run(_)
        ));
        // Choosing an item closes the menu.
        assert_eq!(downloads.menu, None);
    }

    #[test]
    fn a_removed_download_leaves_the_list() {
        let mut downloads = loaded(vec![download("1", "1"), download("1", "2")]);

        let _ = downloads.update(Message::Removed("/downloads/src/1/1".to_string(), Ok(())));

        let AsyncModel::Loaded(entries) = &downloads.entries else {
            panic!("the downloads should be loaded");
        };
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].entry.external_id, "2");
    }

    #[test]
    fn a_failed_removal_is_reported() {
        let mut downloads = loaded(vec![download("1", "1")]);

        let _ = downloads.update(Message::Removed(
            "/downloads/src/1/1".to_string(),
            Err("busy".to_string()),
        ));

        let mut ui = simulator(downloads.view());
        assert!(ui.find("Could not remove the download: busy").is_ok());
    }

    #[test]
    fn a_download_is_not_opened_again_while_the_reader_launches() {
        let mut downloads = loaded(vec![download("1", "1")]);
        // As after a `Read`, whose task (it starts right away) would launch
        // a reader from the test binary.
        downloads.launching = true;

        assert!(matches!(
            downloads.update(Message::Read(download("1", "1"))),
            Action::None
        ));

        let _ = downloads.update(Message::ReaderLaunched(Err("no reader".to_string())));
        assert!(!downloads.launching);
        assert_eq!(downloads.error.as_deref(), Some("no reader"));
    }
}
