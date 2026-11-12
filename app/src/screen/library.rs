//! The user's own Library: the libraries stored in the `libraries` table,
//! listed with the source each came from. Sources no longer sync their
//! collections into it (they're browsed live from the Sources screen), so it
//! only shows rows that already exist, as the start of a Library curated by the
//! user.

use iced::{
    Element,
    Length::Fill,
    Task,
    widget::{container, row},
};

use crate::models::{
    AsyncModel, integration::Integration as IntegrationModel, library::Library as LibraryModel,
};
use crate::repository::{integration as integration_repository, library as library_repository};

#[derive(Debug, Default)]
pub struct Library {
    items: AsyncModel<Vec<LibraryModel>, String>,
    integrations: AsyncModel<Vec<IntegrationModel>, String>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Message {
    LibrariesLoaded(Result<Vec<LibraryModel>, String>),
    IntegrationsLoaded(Result<Vec<IntegrationModel>, String>),
}

#[derive(Debug)]
pub enum Action {
    None,
}

impl Library {
    pub fn new() -> (Self, Task<Message>) {
        (
            Self {
                items: AsyncModel::Loading,
                integrations: AsyncModel::Loading,
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
        }
    }

    pub fn view<'a>(&'a self) -> Element<'a, Message> {
        let integrations: &[IntegrationModel] = match &self.integrations {
            AsyncModel::Loaded(integrations) => integrations,
            _ => &[],
        };

        match &self.items {
            AsyncModel::Loaded(libraries) => view_helper::list(libraries, integrations),
            _ => container(row![]).width(Fill).height(Fill).into(),
        }
    }
}

mod view_helper {
    use iced::{
        Background, Border, Center, Element,
        Length::Fill,
        Theme,
        widget::{column, container, row, scrollable, svg, text},
    };

    use crate::icons;
    use crate::models::integration::Integration;
    use crate::models::library::{Library, LibraryType};
    use crate::touch_scroll::touch_scroll;

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

        container(touch_scroll(
            scrollable(column(rows).spacing(8)).height(Fill),
        ))
        .width(Fill)
        .height(Fill)
        .padding(20)
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
                }),
            text!("Você não tem nenhuma biblioteca cadastrada").center(),
        ]
        .spacing(20)
        .max_width(420)
        .align_x(Center);

        container(content).center(Fill).into()
    }
}

#[cfg(test)]
mod tests {
    use iced_test::simulator;

    use super::*;
    use crate::models::integration::IntegrationType;
    use crate::models::library::LibraryType;

    fn library_model(name: &str) -> LibraryModel {
        LibraryModel {
            id: "id".to_string(),
            kind: LibraryType::Comic,
            name: name.to_string(),
            external_id: None,
            folder: None,
            cover: None,
            integration_id: None,
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
            folder: None,
        }
    }

    fn library(
        items: AsyncModel<Vec<LibraryModel>, String>,
        integrations: AsyncModel<Vec<IntegrationModel>, String>,
    ) -> Library {
        Library {
            items,
            integrations,
        }
    }

    #[test]
    fn view_renders_nothing_while_not_loaded() {
        let library = library(AsyncModel::Loading, AsyncModel::Loading);

        let mut ui = simulator(library.view());

        assert!(
            ui.find("Você não tem nenhuma biblioteca cadastrada")
                .is_err()
        );
    }

    #[test]
    fn view_renders_the_empty_state_when_loaded_without_libraries() {
        let library = library(AsyncModel::Loaded(vec![]), AsyncModel::Loaded(vec![]));

        let mut ui = simulator(library.view());

        assert!(
            ui.find("Você não tem nenhuma biblioteca cadastrada")
                .is_ok()
        );
    }

    #[test]
    fn view_renders_library_names_and_kinds_when_loaded() {
        let library = library(
            AsyncModel::Loaded(vec![library_model("My Library")]),
            AsyncModel::Loaded(vec![]),
        );

        let mut ui = simulator(library.view());

        assert!(ui.find("My Library").is_ok());
        assert!(ui.find("Comic").is_ok());
    }

    #[test]
    fn view_renders_the_server_tag_of_a_library_from_a_source() {
        let library = library(
            AsyncModel::Loaded(vec![LibraryModel {
                integration_id: Some("integration-id".to_string()),
                ..library_model("My Library")
            }]),
            AsyncModel::Loaded(vec![integration_model("integration-id", "My Kavita")]),
        );

        let mut ui = simulator(library.view());

        assert!(ui.find("My Kavita").is_ok());
    }

    #[test]
    fn view_renders_no_server_tag_for_a_library_without_a_source() {
        let library = library(
            AsyncModel::Loaded(vec![library_model("My Library")]),
            AsyncModel::Loaded(vec![integration_model("integration-id", "My Kavita")]),
        );

        let mut ui = simulator(library.view());

        assert!(ui.find("My Kavita").is_err());
    }

    #[test]
    fn update_stores_loaded_libraries_and_errors() {
        let mut library = Library::default();

        library.update(Message::LibrariesLoaded(Ok(vec![library_model(
            "My Library",
        )])));
        assert!(matches!(library.items, AsyncModel::Loaded(ref libs) if libs.len() == 1));

        library.update(Message::LibrariesLoaded(Err("boom".to_string())));
        assert!(matches!(library.items, AsyncModel::Error(ref error) if error == "boom"));
    }

    #[test]
    fn update_stores_loaded_integrations() {
        let mut library = Library::default();

        library.update(Message::IntegrationsLoaded(Ok(vec![integration_model(
            "integration-id",
            "My Kavita",
        )])));

        assert!(
            matches!(library.integrations, AsyncModel::Loaded(ref integrations) if integrations.len() == 1)
        );
    }

    #[test]
    fn library_list_scrolls_when_it_overflows_the_screen() {
        let libraries = (0..30)
            .map(|index| library_model(&format!("Library {index}")))
            .collect();
        let library = library(AsyncModel::Loaded(libraries), AsyncModel::Loaded(vec![]));

        let mut ui = iced_test::Simulator::with_size(
            iced::Settings::default(),
            (800.0, 300.0),
            library.view(),
        );
        assert_eq!(ui.find("Library 29").unwrap().visible_bounds(), None);

        ui.point_at(iced::Point::new(400.0, 200.0));
        let _ = ui.simulate([iced::Event::Mouse(iced::mouse::Event::WheelScrolled {
            delta: iced::mouse::ScrollDelta::Lines { x: 0.0, y: -50.0 },
        })]);

        assert!(ui.find("Library 29").unwrap().visible_bounds().is_some());
    }
}
