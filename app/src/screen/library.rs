use std::path::PathBuf;

use iced::{
    Element,
    Length::Fill,
    Task,
    widget::{container, row},
};

use crate::models::{
    AsyncModel,
    integration::Integration as IntegrationModel,
    library::{Library as LibraryModel, LibraryType},
};
use crate::repository::{integration as integration_repository, library as library_repository};

#[derive(Debug, Default)]
pub struct Library {
    items: AsyncModel<Vec<LibraryModel>, String>,
    integrations: AsyncModel<Vec<IntegrationModel>, String>,
    /// The "Import library from" modal, present only while it's open.
    import: Option<ImportModal>,
}

/// The state of the import modal: the library name and type being entered, plus
/// the file or folder the user picked as its source.
#[derive(Debug)]
struct ImportModal {
    name: String,
    kind: LibraryType,
    source: Option<PathBuf>,
}

impl Default for ImportModal {
    fn default() -> Self {
        Self {
            name: String::new(),
            kind: LibraryType::Comic,
            source: None,
        }
    }
}

impl ImportModal {
    /// Whether the modal has enough to create a library: a name and a source.
    fn can_submit(&self) -> bool {
        !self.name.trim().is_empty() && self.source.is_some()
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum Message {
    LibrariesLoaded(Result<Vec<LibraryModel>, String>),
    IntegrationsLoaded(Result<Vec<IntegrationModel>, String>),
    ImportPressed,
    ImportNameChanged(String),
    ImportTypeSelected(LibraryType),
    ImportChooseFolderPressed,
    ImportSourcePicked(Option<PathBuf>),
    ImportCancelled,
    ImportSubmitted,
    LibraryCreated(Result<LibraryModel, String>),
    BrowseRemoteLibrary(String),
    BrowseLibrary(String),
}

#[derive(Debug)]
pub enum Action {
    None,
    Run(Task<Message>),
    BrowseRemoteLibrary(String),
    BrowseLibrary(String),
}

impl Library {
    pub fn new() -> (Self, Task<Message>) {
        (
            Self {
                items: AsyncModel::Loading,
                integrations: AsyncModel::Loading,
                import: None,
            },
            Task::batch([
                Task::perform(library_repository::all(), Message::LibrariesLoaded),
                Task::perform(integration_repository::all(), Message::IntegrationsLoaded),
            ]),
        )
    }

    pub fn update(&mut self, message: Message) -> Action {
        match message {
            Message::LibrariesLoaded(result) => {
                self.items = result.into();
                Action::None
            }

            Message::IntegrationsLoaded(result) => {
                self.integrations = result.into();
                Action::None
            }

            Message::ImportPressed => {
                self.import = Some(ImportModal::default());
                Action::None
            }

            Message::ImportNameChanged(name) => {
                if let Some(import) = &mut self.import {
                    import.name = name;
                }
                Action::None
            }

            Message::ImportTypeSelected(kind) => {
                if let Some(import) = &mut self.import {
                    import.kind = kind;
                }
                Action::None
            }

            Message::ImportChooseFolderPressed => {
                Action::Run(Task::perform(pick_folder(), Message::ImportSourcePicked))
            }

            Message::ImportSourcePicked(source) => {
                if let (Some(import), Some(source)) = (&mut self.import, source) {
                    // Default the (still-empty) name to the chosen item's name,
                    // so a one-field import is possible.
                    if import.name.trim().is_empty() {
                        import.name = source_display_name(&source);
                    }
                    import.source = Some(source);
                }
                Action::None
            }

            Message::ImportCancelled => {
                self.import = None;
                Action::None
            }

            Message::ImportSubmitted => {
                let Some(import) = &self.import else {
                    return Action::None;
                };
                let Some(source) = &import.source else {
                    return Action::None;
                };
                if import.name.trim().is_empty() {
                    return Action::None;
                }

                Action::Run(Task::perform(
                    library_repository::create_local(
                        import.name.trim().to_string(),
                        import.kind,
                        source.to_string_lossy().into_owned(),
                    ),
                    Message::LibraryCreated,
                ))
            }

            Message::LibraryCreated(Ok(_)) => {
                self.import = None;
                Action::Run(Task::perform(
                    library_repository::all(),
                    Message::LibrariesLoaded,
                ))
            }
            Message::LibraryCreated(Err(_)) => Action::None,

            Message::BrowseRemoteLibrary(external_id) => Action::BrowseRemoteLibrary(external_id),
            Message::BrowseLibrary(library_id) => Action::BrowseLibrary(library_id),
        }
    }

    pub fn view<'a>(&'a self) -> Element<'a, Message> {
        let integrations: &[IntegrationModel] = match &self.integrations {
            AsyncModel::Loaded(integrations) => integrations,
            _ => &[],
        };

        let base: Element<'a, Message> = match &self.items {
            AsyncModel::Loaded(libraries) => view_helper::list(libraries, integrations),
            // Fill even while loading so the import modal, which overlays this
            // via a `stack`, dims the whole screen rather than a collapsed strip.
            _ => container(row![]).width(Fill).height(Fill).into(),
        };

        match &self.import {
            Some(import) => view_helper::with_modal(base, import),
            None => base,
        }
    }
}

/// A display name for a picked source: the file stem for a file, the folder name
/// for a folder, falling back to the full path.
fn source_display_name(path: &std::path::Path) -> String {
    let name = if path.is_dir() {
        path.file_name()
    } else {
        path.file_stem()
    };

    name.map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.to_string_lossy().into_owned())
}

async fn pick_folder() -> Option<PathBuf> {
    let home = std::env::var("HOME").unwrap_or_else(|_| "/".to_string());

    rfd::AsyncFileDialog::new()
        .set_directory(home)
        .pick_folder()
        .await
        .map(|handle| handle.path().to_path_buf())
}

mod view_helper {
    use iced::{
        Background, Border, Center, Color, Element,
        Length::Fill,
        Theme,
        widget::{
            button, column, container, mouse_area, opaque, row, space, stack, svg, text,
            text_input,
        },
    };
    use iced_font_awesome::fa_icon_solid;

    use super::ImportModal;
    use crate::icons;
    use crate::models::integration::Integration;
    use crate::models::library::{Library, LibraryType};

    pub fn list<'a>(
        libraries: &'a [Library],
        integrations: &'a [Integration],
    ) -> Element<'a, super::Message> {
        if libraries.is_empty() {
            return empty_list();
        }

        let rows = libraries
            .iter()
            .map(|library| library_row(library, server_name(library, integrations)));

        // Fill the whole screen area: the import modal overlays this view via a
        // `stack`, whose height follows this (its first child). A `Shrink` height
        // here would confine the dimmed backdrop to just the list's rows.
        container(column![import_button(), column(rows).spacing(8)].spacing(20))
            .width(Fill)
            .height(Fill)
            .padding(20)
            .into()
    }

    /// The single entry point for adding local content, always available above
    /// the library list (and the only control in the empty state).
    fn import_button<'a>() -> Element<'a, super::Message> {
        button(
            row![
                fa_icon_solid("plus").size(14.0_f32),
                text("Import library from")
            ]
            .spacing(6)
            .align_y(Center),
        )
        .on_press(super::Message::ImportPressed)
        .into()
    }

    fn server_name<'a>(library: &Library, integrations: &'a [Integration]) -> Option<&'a str> {
        let integration_id = library.integration_id.as_ref()?;

        integrations
            .iter()
            .find(|integration| &integration.id == integration_id)
            .map(|integration| integration.name.as_str())
    }

    fn library_row<'a>(
        library: &'a Library,
        server_name: Option<&'a str>,
    ) -> Element<'a, super::Message> {
        container(
            row![
                icon_badge(kind_icon(&library.kind)),
                column![
                    row![
                        text(&library.name),
                        text(library.kind.label()).size(12).style(text::secondary),
                    ]
                    .spacing(6)
                    .align_y(Center),
                    server_tag(server_name),
                ]
                .spacing(4),
                space::horizontal(),
                button(
                    row![fa_icon_solid("folder-open").size(14.0_f32), text("Browse")]
                        .spacing(6)
                        .align_y(Center)
                )
                .on_press(browse_message(library))
            ]
            .spacing(12)
            .align_y(Center)
            .padding(12),
        )
        .width(Fill)
        .style(|theme: &Theme| container::Style {
            background: Some(Background::Color(
                theme.extended_palette().background.weak.color,
            )),
            border: Border {
                radius: 8.0.into(),
                ..Border::default()
            },
            ..container::Style::default()
        })
        .into()
    }

    /// Which browse message opening `library` sends. A library tied to an
    /// integration browses as a remote one; a local (filesystem) library browses
    /// as a local one, which additionally offers importing more items into it.
    /// Both resolve through the library's `external_id` (a local library's is its
    /// own id — see `repository::library::create_local`).
    fn browse_message(library: &Library) -> super::Message {
        let external_id = library
            .external_id
            .clone()
            .unwrap_or_else(|| library.id.clone());

        if library.integration_id.is_some() {
            super::Message::BrowseRemoteLibrary(external_id)
        } else {
            super::Message::BrowseLibrary(external_id)
        }
    }

    fn icon_badge<'a>(icon: &'static [u8]) -> Element<'a, super::Message> {
        container(
            svg(svg::Handle::from_memory(icon))
                .width(20)
                .height(20)
                .style(|theme: &Theme, _status| svg::Style {
                    color: Some(theme.palette().text),
                }),
        )
        .center_x(40)
        .center_y(40)
        .style(|theme: &Theme| container::Style {
            background: Some(Background::Color(
                theme.extended_palette().background.strong.color,
            )),
            border: Border {
                radius: 8.0.into(),
                ..Border::default()
            },
            ..container::Style::default()
        })
        .into()
    }

    fn server_tag<'a>(server_name: Option<&'a str>) -> Element<'a, super::Message> {
        match server_name {
            Some(name) => container(text(name).size(12))
                .padding([2, 8])
                .style(|theme: &Theme| {
                    let palette = theme.extended_palette();

                    container::Style {
                        background: Some(Background::Color(palette.secondary.weak.color)),
                        text_color: Some(palette.secondary.weak.text),
                        border: Border {
                            radius: 999.0.into(),
                            ..Border::default()
                        },
                        ..container::Style::default()
                    }
                })
                .into(),
            None => row![].into(),
        }
    }

    fn kind_icon(kind: &LibraryType) -> &'static [u8] {
        match kind {
            LibraryType::Manga => icons::MANGA,
            LibraryType::Comic => icons::COMIC,
            LibraryType::Ebook => icons::EBOOK,
        }
    }

    fn empty_list<'a>() -> Element<'a, super::Message> {
        let content = column![
            svg(svg::Handle::from_memory(icons::BOOK_OPEN))
                .height(64)
                .width(64)
                .style(|theme: &Theme, _status| svg::Style {
                        color: Some(theme.extended_palette().background.neutral.color),
                    }
                ),
            text!(
                "Você não tem nenhuma biblioteca cadastrada, adicione uma pasta ou uma integração com algum serviço"
            )
            .center(),
            import_button(),
        ]
        .spacing(20)
        .max_width(420)
        .align_x(Center);

        container(content).center(Fill).into()
    }

    /// Overlays the import modal on top of `base`, dimming the background. A
    /// click outside the card dismisses the modal.
    pub fn with_modal<'a>(
        base: Element<'a, super::Message>,
        import: &'a ImportModal,
    ) -> Element<'a, super::Message> {
        let dimmed = mouse_area(
            container(opaque(import_card(import)))
                .center(Fill)
                .style(|_theme: &Theme| container::Style {
                    background: Some(Background::Color(Color {
                        a: 0.75,
                        ..Color::BLACK
                    })),
                    ..container::Style::default()
                }),
        )
        .on_press(super::Message::ImportCancelled);

        stack![base, opaque(dimmed)].into()
    }

    fn import_card<'a>(import: &'a ImportModal) -> Element<'a, super::Message> {
        let submit = import
            .can_submit()
            .then_some(super::Message::ImportSubmitted);

        let card = column![
            text("Import Library").size(20),
            text_input("Library name", &import.name).on_input(super::Message::ImportNameChanged),
            text("Type").size(13).style(text::secondary),
            type_selector(import.kind),
            text("Source").size(13).style(text::secondary),
            source_field(import),
            row![
                space::horizontal(),
                button("Cancel").on_press(super::Message::ImportCancelled),
                button("Import").on_press_maybe(submit),
            ]
            .spacing(10),
        ]
        .spacing(14)
        .width(Fill);

        // `max_width` must live on the card container, not the inner column: the
        // column reports `Fill` width (it holds a `Fill` text input), and
        // `container` inherits that width from its content — so without an
        // explicit cap here the styled background stretches edge to edge while
        // the column sits left-aligned inside it.
        container(card)
            .width(Fill)
            .max_width(460)
            .padding(24)
            .style(|theme: &Theme| container::Style {
                background: Some(Background::Color(theme.palette().background)),
                border: Border {
                    radius: 12.0.into(),
                    ..Border::default()
                },
                ..container::Style::default()
            })
            .into()
    }

    fn type_selector<'a>(selected: LibraryType) -> Element<'a, super::Message> {
        let buttons = LibraryType::ALL
            .into_iter()
            .map(|kind| type_button(kind, selected));

        row(buttons).spacing(8).into()
    }

    fn type_button<'a>(kind: LibraryType, selected: LibraryType) -> Element<'a, super::Message> {
        let is_active = kind == selected;

        button(text(kind.label()).size(14))
            .on_press(super::Message::ImportTypeSelected(kind))
            .padding([6, 12])
            .style(move |theme: &Theme, status| {
                let palette = theme.extended_palette();
                let mut style = button::text(theme, status);

                if is_active {
                    style.background = Some(Background::Color(palette.primary.strong.color));
                    style.text_color = palette.primary.strong.text;
                    style.border = Border {
                        radius: 6.0.into(),
                        ..Border::default()
                    };
                }

                style
            })
            .into()
    }

    /// The source field: a read-only display of the chosen folder next to a
    /// single button that opens the system folder picker. (A single native
    /// dialog can't offer files and folders at once, so a library imports from a
    /// folder; individual files are added later via a library's "Import items".)
    fn source_field<'a>(import: &'a ImportModal) -> Element<'a, super::Message> {
        let label = match &import.source {
            Some(path) => path.to_string_lossy().into_owned(),
            None => "No folder selected".to_string(),
        };

        let display = container(text(label).size(13))
            .padding([8, 10])
            .width(Fill)
            .style(move |theme: &Theme| {
                let palette = theme.extended_palette();
                container::Style {
                    background: Some(Background::Color(palette.background.weak.color)),
                    text_color: Some(if import.source.is_some() {
                        palette.background.base.text
                    } else {
                        palette.background.strong.color
                    }),
                    border: Border {
                        width: 1.0,
                        color: palette.background.strong.color,
                        radius: 6.0.into(),
                    },
                    ..container::Style::default()
                }
            });

        row![
            display,
            button("Choose folder…").on_press(super::Message::ImportChooseFolderPressed),
        ]
        .spacing(8)
        .align_y(Center)
        .into()
    }
}

#[cfg(test)]
mod tests {
    use iced_test::simulator;

    use super::*;
    use crate::models::integration::IntegrationType;

    fn library_model(name: &str) -> LibraryModel {
        LibraryModel {
            id: "id".to_string(),
            kind: crate::models::library::LibraryType::Comic,
            name: name.to_string(),
            external_id: None,
            folder: None,
            cover: None,
            integration_id: None,
        }
    }

    fn library_model_with_integration(name: &str, integration_id: &str) -> LibraryModel {
        LibraryModel {
            integration_id: Some(integration_id.to_string()),
            ..library_model(name)
        }
    }

    fn integration_model(id: &str, name: &str) -> IntegrationModel {
        IntegrationModel {
            id: id.to_string(),
            name: name.to_string(),
            integration_type: IntegrationType::Kavita,
            url: None,
            api_key: None,
            username: None,
            password: None,
        }
    }

    /// A `Library` with no import modal open, for the view/update tests that
    /// don't exercise importing.
    fn library(
        items: AsyncModel<Vec<LibraryModel>, String>,
        integrations: AsyncModel<Vec<IntegrationModel>, String>,
    ) -> Library {
        Library {
            items,
            integrations,
            import: None,
        }
    }

    #[test]
    fn view_renders_empty_container_while_not_loaded() {
        let library = library(AsyncModel::Loading, AsyncModel::Loading);

        let mut ui = simulator(library.view());

        assert!(ui.find("Import library from").is_err());
    }

    #[test]
    fn view_renders_call_to_action_when_loaded_without_libraries() {
        let library = library(AsyncModel::Loaded(vec![]), AsyncModel::Loaded(vec![]));

        let mut ui = simulator(library.view());

        assert!(ui.find("Import library from").is_ok());
    }

    #[test]
    fn view_renders_import_button_when_libraries_exist() {
        let library = library(
            AsyncModel::Loaded(vec![library_model("My Library")]),
            AsyncModel::Loaded(vec![]),
        );

        let mut ui = simulator(library.view());

        assert!(ui.find("Import library from").is_ok());
    }

    #[test]
    fn view_renders_library_names_when_loaded() {
        let library = library(
            AsyncModel::Loaded(vec![library_model("My Library")]),
            AsyncModel::Loaded(vec![]),
        );

        let mut ui = simulator(library.view());

        assert!(ui.find("My Library").is_ok());
    }

    #[test]
    fn view_renders_library_kind_label_when_loaded() {
        let library = library(
            AsyncModel::Loaded(vec![library_model("My Library")]),
            AsyncModel::Loaded(vec![]),
        );

        let mut ui = simulator(library.view());

        assert!(ui.find("Comic").is_ok());
    }

    #[test]
    fn view_renders_browse_button_when_loaded() {
        let library = library(
            AsyncModel::Loaded(vec![library_model("My Library")]),
            AsyncModel::Loaded(vec![]),
        );

        let mut ui = simulator(library.view());

        assert!(ui.find("Browse").is_ok());
    }

    #[test]
    fn view_renders_server_tag_for_library_from_an_integration() {
        let library = library(
            AsyncModel::Loaded(vec![library_model_with_integration(
                "My Library",
                "integration-id",
            )]),
            AsyncModel::Loaded(vec![integration_model("integration-id", "My Kavita")]),
        );

        let mut ui = simulator(library.view());

        assert!(ui.find("My Kavita").is_ok());
    }

    #[test]
    fn view_renders_no_server_tag_for_local_library() {
        let library = library(
            AsyncModel::Loaded(vec![library_model("My Library")]),
            AsyncModel::Loaded(vec![integration_model("integration-id", "My Kavita")]),
        );

        let mut ui = simulator(library.view());

        assert!(ui.find("My Kavita").is_err());
    }

    #[test]
    fn clicking_import_sends_import_pressed_message() {
        let library = library(AsyncModel::Loaded(vec![]), AsyncModel::Loaded(vec![]));

        let mut ui = simulator(library.view());
        let _ = ui
            .click("Import library from")
            .expect("Import Items button should be found");

        let messages: Vec<_> = ui.into_messages().collect();

        assert_eq!(messages, vec![Message::ImportPressed]);
    }

    #[test]
    fn import_pressed_opens_the_modal() {
        let mut library = Library::default();

        let action = library.update(Message::ImportPressed);

        assert!(matches!(action, Action::None));
        assert!(library.import.is_some());
    }

    #[test]
    fn view_renders_the_import_modal_when_open() {
        let mut library = library(AsyncModel::Loaded(vec![]), AsyncModel::Loaded(vec![]));
        library.import = Some(ImportModal::default());

        let mut ui = simulator(library.view());

        assert!(ui.find("Library name").is_ok());
        assert!(ui.find("Type").is_ok());
        assert!(ui.find("Source").is_ok());
        assert!(ui.find("Choose folder…").is_ok());
        assert!(ui.find("Import").is_ok());
        assert!(ui.find("Cancel").is_ok());
    }

    #[test]
    fn import_name_changed_stores_the_value() {
        let mut library = Library::default();
        library.import = Some(ImportModal::default());

        library.update(Message::ImportNameChanged("My Comics".to_string()));

        assert_eq!(library.import.as_ref().unwrap().name, "My Comics");
    }

    #[test]
    fn import_type_selected_stores_the_kind() {
        let mut library = Library::default();
        library.import = Some(ImportModal::default());

        library.update(Message::ImportTypeSelected(LibraryType::Manga));

        assert_eq!(library.import.as_ref().unwrap().kind, LibraryType::Manga);
    }

    #[test]
    fn import_choose_folder_runs_a_picker_task() {
        let mut library = Library::default();
        library.import = Some(ImportModal::default());

        let action = library.update(Message::ImportChooseFolderPressed);

        assert!(matches!(action, Action::Run(_)));
    }

    #[test]
    fn clicking_choose_folder_sends_the_pick_message() {
        let mut library = library(AsyncModel::Loaded(vec![]), AsyncModel::Loaded(vec![]));
        library.import = Some(ImportModal::default());

        let mut ui = simulator(library.view());
        let _ = ui
            .click("Choose folder…")
            .expect("Choose folder button should be found");

        let messages: Vec<_> = ui.into_messages().collect();

        assert_eq!(messages, vec![Message::ImportChooseFolderPressed]);
    }

    #[test]
    fn import_source_picked_stores_the_path_and_defaults_the_name() {
        let mut library = Library::default();
        library.import = Some(ImportModal::default());

        library.update(Message::ImportSourcePicked(Some(PathBuf::from(
            "/comics/One Piece",
        ))));

        let import = library.import.as_ref().unwrap();
        assert_eq!(import.source, Some(PathBuf::from("/comics/One Piece")));
        assert_eq!(import.name, "One Piece");
    }

    #[test]
    fn import_source_picked_does_not_overwrite_an_existing_name() {
        let mut library = Library::default();
        library.import = Some(ImportModal {
            name: "Chosen".to_string(),
            ..ImportModal::default()
        });

        library.update(Message::ImportSourcePicked(Some(PathBuf::from(
            "/comics/One Piece",
        ))));

        assert_eq!(library.import.as_ref().unwrap().name, "Chosen");
    }

    #[test]
    fn import_source_picked_none_leaves_the_source_empty() {
        let mut library = Library::default();
        library.import = Some(ImportModal::default());

        library.update(Message::ImportSourcePicked(None));

        assert!(library.import.as_ref().unwrap().source.is_none());
    }

    #[test]
    fn import_cancelled_closes_the_modal() {
        let mut library = Library::default();
        library.import = Some(ImportModal::default());

        let action = library.update(Message::ImportCancelled);

        assert!(matches!(action, Action::None));
        assert!(library.import.is_none());
    }

    #[test]
    fn import_submitted_with_name_and_source_runs_a_task() {
        let mut library = Library::default();
        library.import = Some(ImportModal {
            name: "My Comics".to_string(),
            kind: LibraryType::Comic,
            source: Some(PathBuf::from("/comics")),
        });

        let action = library.update(Message::ImportSubmitted);

        assert!(matches!(action, Action::Run(_)));
    }

    #[test]
    fn import_submitted_without_a_source_does_nothing() {
        let mut library = Library::default();
        library.import = Some(ImportModal {
            name: "My Comics".to_string(),
            kind: LibraryType::Comic,
            source: None,
        });

        let action = library.update(Message::ImportSubmitted);

        assert!(matches!(action, Action::None));
    }

    #[test]
    fn import_is_disabled_until_a_name_and_source_are_set() {
        let mut library = library(AsyncModel::Loaded(vec![]), AsyncModel::Loaded(vec![]));
        library.import = Some(ImportModal::default());

        let mut ui = simulator(library.view());
        let _ = ui.click("Import").expect("Import button should be found");

        // Disabled button emits nothing.
        assert!(ui.into_messages().next().is_none());
    }

    #[test]
    fn import_is_enabled_with_a_name_and_source() {
        let mut library = library(AsyncModel::Loaded(vec![]), AsyncModel::Loaded(vec![]));
        library.import = Some(ImportModal {
            name: "My Comics".to_string(),
            kind: LibraryType::Comic,
            source: Some(PathBuf::from("/comics")),
        });

        let mut ui = simulator(library.view());
        let _ = ui.click("Import").expect("Import button should be found");

        let messages: Vec<_> = ui.into_messages().collect();

        assert_eq!(messages, vec![Message::ImportSubmitted]);
    }

    #[test]
    fn update_libraries_loaded_stores_result_and_returns_no_action() {
        let mut library = Library::default();

        let action = library.update(Message::LibrariesLoaded(Ok(vec![library_model(
            "My Library",
        )])));

        assert!(matches!(action, Action::None));
        assert!(matches!(library.items, AsyncModel::Loaded(ref libs) if libs.len() == 1));
    }

    #[test]
    fn update_libraries_loaded_stores_error_and_returns_no_action() {
        let mut library = Library::default();

        let action = library.update(Message::LibrariesLoaded(Err("boom".to_string())));

        assert!(matches!(action, Action::None));
        assert!(matches!(library.items, AsyncModel::Error(ref error) if error == "boom"));
    }

    #[test]
    fn update_integrations_loaded_stores_result_and_returns_no_action() {
        let mut library = Library::default();

        let action = library.update(Message::IntegrationsLoaded(Ok(vec![integration_model(
            "integration-id",
            "My Kavita",
        )])));

        assert!(matches!(action, Action::None));
        assert!(
            matches!(library.integrations, AsyncModel::Loaded(ref integrations) if integrations.len() == 1)
        );
    }

    #[test]
    fn update_library_created_ok_closes_the_modal_and_reloads_libraries() {
        let mut library = Library::default();
        library.import = Some(ImportModal::default());

        let action = library.update(Message::LibraryCreated(Ok(library_model("My Library"))));

        assert!(matches!(action, Action::Run(_)));
        assert!(library.import.is_none());
    }

    #[test]
    fn update_library_created_err_returns_no_action() {
        let mut library = Library::default();

        let action = library.update(Message::LibraryCreated(Err("boom".to_string())));

        assert!(matches!(action, Action::None));
    }

    #[test]
    fn update_browse_remote_library_returns_browse_remote_library_action() {
        let mut library = Library::default();

        let action = library.update(Message::BrowseRemoteLibrary("external-id".to_string()));

        assert!(matches!(action, Action::BrowseRemoteLibrary(id) if id == "external-id"));
    }

    #[test]
    fn update_browse_library_returns_browse_library_action() {
        let mut library = Library::default();

        let action = library.update(Message::BrowseLibrary("library-id".to_string()));

        assert!(matches!(action, Action::BrowseLibrary(id) if id == "library-id"));
    }

    #[test]
    fn clicking_browse_on_a_remote_library_sends_browse_remote_library_message() {
        // A remote library has both an external id and an integration.
        let remote = LibraryModel {
            external_id: Some("external-id".to_string()),
            integration_id: Some("integration-id".to_string()),
            ..library_model("My Library")
        };
        let library = library(
            AsyncModel::Loaded(vec![remote]),
            AsyncModel::Loaded(vec![]),
        );

        let mut ui = simulator(library.view());
        let _ = ui.click("Browse").expect("Browse button should be found");

        let messages: Vec<_> = ui.into_messages().collect();

        assert_eq!(
            messages,
            vec![Message::BrowseRemoteLibrary("external-id".to_string())]
        );
    }

    #[test]
    fn clicking_browse_on_a_local_library_with_an_external_id_browses_locally() {
        // A local (filesystem) library has an external id equal to its own id but
        // no integration — it must still browse as local so items can be imported.
        let local = LibraryModel {
            id: "lib-id".to_string(),
            external_id: Some("lib-id".to_string()),
            integration_id: None,
            folder: Some("/comics".to_string()),
            ..library_model("My Comics")
        };
        let library = library(
            AsyncModel::Loaded(vec![local]),
            AsyncModel::Loaded(vec![]),
        );

        let mut ui = simulator(library.view());
        let _ = ui.click("Browse").expect("Browse button should be found");

        let messages: Vec<_> = ui.into_messages().collect();

        assert_eq!(messages, vec![Message::BrowseLibrary("lib-id".to_string())]);
    }

    #[test]
    fn clicking_browse_on_a_local_library_sends_browse_library_message() {
        let library = library(
            AsyncModel::Loaded(vec![library_model("My Library")]),
            AsyncModel::Loaded(vec![]),
        );

        let mut ui = simulator(library.view());
        let _ = ui.click("Browse").expect("Browse button should be found");

        let messages: Vec<_> = ui.into_messages().collect();

        assert_eq!(messages, vec![Message::BrowseLibrary("id".to_string())]);
    }
}
