//! The sources screen (integrations, in code): lists every source the user has
//! added (servers and local folders) and walks through adding one. Opening a
//! source leads to its collections (see [`super::collections`]).

#[cfg(feature = "kavita")]
pub mod kavita;
#[cfg(feature = "komga")]
pub mod komga;
pub mod local_folder;
#[cfg(feature = "opds")]
pub mod opds;
#[cfg(feature = "suwayomi")]
pub mod suwayomi;

use iced::{Element, Task, widget::container};

use crate::models::{
    AsyncModel,
    integration::{Integration as IntegrationModel, IntegrationType},
};
use crate::repository::integration as integration_repository;
#[cfg(feature = "kavita")]
use kavita::Kavita;
#[cfg(feature = "komga")]
use komga::Komga;
use local_folder::LocalFolder;
#[cfg(feature = "opds")]
use opds::Opds;
#[cfg(feature = "suwayomi")]
use suwayomi::Suwayomi;

#[derive(Debug)]
enum Step {
    List(AsyncModel<Vec<IntegrationModel>, String>),
    ChooseType,
    LocalFolder(LocalFolder),
    #[cfg(feature = "kavita")]
    Kavita(Kavita),
    #[cfg(feature = "komga")]
    Komga(Komga),
    #[cfg(feature = "opds")]
    Opds(Opds),
    #[cfg(feature = "suwayomi")]
    Suwayomi(Suwayomi),
}

#[derive(Debug)]
pub struct Integrations {
    step: Step,
    /// The id of the source whose removal is waiting to be confirmed, if any.
    confirming_removal: Option<String>,
    /// Whether a removal is running, so it isn't asked for twice.
    removing: bool,
    /// The last removal failure, shown above the list until the next attempt.
    error: Option<String>,
}

impl Default for Integrations {
    fn default() -> Self {
        Self {
            step: Step::List(AsyncModel::Loading),
            confirming_removal: None,
            removing: false,
            error: None,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum Message {
    IntegrationsLoaded(Result<Vec<IntegrationModel>, String>),
    AddIntegrationPressed,
    TypeSelected(IntegrationType),
    /// A source in the list was clicked; carries its id.
    SourceSelected(String),
    /// A source's remove button was pressed (by id): ask to confirm.
    RemovePressed(String),
    /// The removal waiting to be confirmed was cancelled.
    RemoveCancelled,
    /// The removal of a source (by id) was confirmed: remove it, and the items
    /// added from it to the Library.
    RemoveConfirmed(String),
    /// A source was removed, or failed to be.
    Removed(Result<(), String>),
    LocalFolder(local_folder::Message),
    #[cfg(feature = "kavita")]
    Kavita(kavita::Message),
    #[cfg(feature = "komga")]
    Komga(komga::Message),
    #[cfg(feature = "opds")]
    Opds(opds::Message),
    #[cfg(feature = "suwayomi")]
    Suwayomi(suwayomi::Message),
}

#[derive(Debug)]
pub enum Action {
    None,
    Run(Task<Message>),
    /// Open the source's collections; carries its id.
    OpenSource(String),
}

impl Integrations {
    pub fn new() -> (Self, Task<Message>) {
        (
            Self::default(),
            Task::perform(integration_repository::all(), Message::IntegrationsLoaded),
        )
    }

    pub fn update(&mut self, message: Message) -> Action {
        match message {
            Message::IntegrationsLoaded(result) => {
                self.step = Step::List(result.into());
                Action::None
            }

            Message::AddIntegrationPressed => {
                self.step = Step::ChooseType;
                Action::None
            }

            Message::SourceSelected(integration_id) => Action::OpenSource(integration_id),

            Message::RemovePressed(integration_id) => {
                self.error = None;
                self.confirming_removal = Some(integration_id);
                Action::None
            }
            Message::RemoveCancelled => {
                self.confirming_removal = None;
                Action::None
            }
            Message::RemoveConfirmed(integration_id) => {
                if self.removing || self.confirming_removal.as_ref() != Some(&integration_id) {
                    return Action::None;
                }
                self.removing = true;

                Action::Run(Task::perform(
                    integration_repository::remove(integration_id),
                    Message::Removed,
                ))
            }
            Message::Removed(result) => {
                self.removing = false;
                self.confirming_removal = None;
                if let Err(error) = result {
                    tracing::debug!("failed to remove the source: {error}");
                    self.error = Some(format!("Could not remove the source: {error}"));
                }
                self.reload_list()
            }

            Message::TypeSelected(IntegrationType::LocalFolder) => {
                self.step = Step::LocalFolder(LocalFolder::default());
                Action::None
            }

            #[cfg(feature = "kavita")]
            Message::TypeSelected(IntegrationType::Kavita) => {
                self.step = Step::Kavita(Kavita::default());
                Action::None
            }

            #[cfg(feature = "komga")]
            Message::TypeSelected(IntegrationType::Komga) => {
                self.step = Step::Komga(Komga::default());
                Action::None
            }

            #[cfg(feature = "opds")]
            Message::TypeSelected(IntegrationType::Opds) => {
                self.step = Step::Opds(Opds::default());
                Action::None
            }

            #[cfg(feature = "suwayomi")]
            Message::TypeSelected(IntegrationType::Suwayomi) => {
                self.step = Step::Suwayomi(Suwayomi::default());
                Action::None
            }
            // Not offered by `choose_type` when its feature is off.
            #[allow(unreachable_patterns)]
            Message::TypeSelected(_) => Action::None,

            Message::LocalFolder(msg) => {
                let Step::LocalFolder(form) = &mut self.step else {
                    return Action::None;
                };

                match form.update(msg) {
                    local_folder::Action::None => Action::None,
                    local_folder::Action::Run(task) => Action::Run(task.map(Message::LocalFolder)),
                    local_folder::Action::BackPressed => self.reload_list(),
                    local_folder::Action::Created => self.reload_list(),
                }
            }

            #[cfg(feature = "kavita")]
            Message::Kavita(msg) => {
                let Step::Kavita(kavita) = &mut self.step else {
                    return Action::None;
                };

                match kavita.update(msg) {
                    kavita::Action::None => Action::None,
                    kavita::Action::Run(task) => Action::Run(task.map(Message::Kavita)),
                    kavita::Action::BackPressed => self.reload_list(),
                    kavita::Action::Created => self.reload_list(),
                }
            }

            #[cfg(feature = "komga")]
            Message::Komga(msg) => {
                let Step::Komga(komga) = &mut self.step else {
                    return Action::None;
                };

                match komga.update(msg) {
                    komga::Action::None => Action::None,
                    komga::Action::Run(task) => Action::Run(task.map(Message::Komga)),
                    komga::Action::BackPressed => self.reload_list(),
                    komga::Action::Created => self.reload_list(),
                }
            }

            #[cfg(feature = "opds")]
            Message::Opds(msg) => {
                let Step::Opds(opds) = &mut self.step else {
                    return Action::None;
                };

                match opds.update(msg) {
                    opds::Action::None => Action::None,
                    opds::Action::Run(task) => Action::Run(task.map(Message::Opds)),
                    opds::Action::BackPressed => self.reload_list(),
                    opds::Action::Created => self.reload_list(),
                }
            }

            #[cfg(feature = "suwayomi")]
            Message::Suwayomi(msg) => {
                let Step::Suwayomi(suwayomi) = &mut self.step else {
                    return Action::None;
                };

                match suwayomi.update(msg) {
                    suwayomi::Action::None => Action::None,
                    suwayomi::Action::Run(task) => Action::Run(task.map(Message::Suwayomi)),
                    suwayomi::Action::BackPressed => self.reload_list(),
                    suwayomi::Action::Created => self.reload_list(),
                }
            }
        }
    }

    /// Returns to the (reloaded) list, after a form is left or a source saved.
    /// Saving a source stores just the source itself: its collections are
    /// listed live from it whenever it's opened.
    fn reload_list(&mut self) -> Action {
        self.step = Step::List(AsyncModel::Loading);
        Action::Run(Task::perform(
            integration_repository::all(),
            Message::IntegrationsLoaded,
        ))
    }

    pub fn view<'a>(&'a self) -> Element<'a, Message> {
        match &self.step {
            Step::List(AsyncModel::Loaded(integrations)) => view_helper::list(
                integrations,
                self.confirming_removal.as_deref(),
                self.removing,
                self.error.as_deref(),
            ),
            Step::List(_) => container(iced::widget::row![]).into(),
            Step::ChooseType => view_helper::choose_type(),
            Step::LocalFolder(form) => form.view().map(Message::LocalFolder),
            #[cfg(feature = "kavita")]
            Step::Kavita(kavita) => kavita.view().map(Message::Kavita),
            #[cfg(feature = "komga")]
            Step::Komga(komga) => komga.view().map(Message::Komga),
            #[cfg(feature = "opds")]
            Step::Opds(opds) => opds.view().map(Message::Opds),
            #[cfg(feature = "suwayomi")]
            Step::Suwayomi(suwayomi) => suwayomi.view().map(Message::Suwayomi),
        }
    }
}

mod view_helper {
    use super::{IntegrationModel, IntegrationType, Message};
    use crate::icons;
    use crate::touch_scroll::touch_scroll;
    use iced::{
        Background, Border, Center, Element,
        Length::Fill,
        Theme,
        widget::{button, column, container, grid, row, scrollable, space, svg, text},
    };
    use iced_font_awesome::fa_icon_solid;

    pub fn list<'a>(
        integrations: &'a [IntegrationModel],
        confirming_removal: Option<&str>,
        removing: bool,
        error: Option<&'a str>,
    ) -> Element<'a, Message> {
        if integrations.is_empty() && error.is_none() {
            return empty_list();
        }

        let rows = integrations.iter().map(|integration| {
            let confirming = confirming_removal == Some(integration.id.as_str());
            row![
                integration_row(integration),
                removal_controls(integration, confirming, removing),
            ]
            .spacing(8)
            .align_y(Center)
            .into()
        });

        let mut content = column![add_integration_button()].spacing(20);
        if let Some(error) = error {
            content = content.push(text(error).size(13));
        }

        container(content.push(touch_scroll(
            scrollable(column(rows).spacing(8)).height(Fill),
        )))
        .width(Fill)
        .height(Fill)
        .padding(20)
        .into()
    }

    /// Always available above the source list, so more sources can be added
    /// once the first exists.
    fn add_integration_button<'a>() -> Element<'a, Message> {
        button(
            row![fa_icon_solid("plus").size(14.0_f32), text("Add Source")]
                .spacing(6)
                .align_y(Center),
        )
        .on_press(Message::AddIntegrationPressed)
        .into()
    }

    /// A source, clickable to open its collections.
    fn integration_row<'a>(integration: &'a IntegrationModel) -> Element<'a, Message> {
        button(
            row![
                icon_badge(integration_icon(integration.integration_type)),
                column![
                    text(&integration.name),
                    text(integration.integration_type.label())
                        .size(12)
                        .style(text::secondary),
                ]
                .spacing(4),
                space::horizontal(),
                fa_icon_solid("chevron-right").size(12.0_f32),
            ]
            .spacing(12)
            .align_y(Center)
            .padding(6),
        )
        .width(Fill)
        .on_press(Message::SourceSelected(integration.id.clone()))
        .style(|theme: &Theme, status| {
            let palette = theme.extended_palette();
            let background = match status {
                button::Status::Hovered | button::Status::Pressed => {
                    palette.background.strong.color
                }
                _ => palette.background.weak.color,
            };

            button::Style {
                background: Some(Background::Color(background)),
                text_color: palette.background.base.text,
                border: Border {
                    radius: 8.0.into(),
                    ..Border::default()
                },
                ..button::Style::default()
            }
        })
        .into()
    }

    /// Beside a source's row: a trash button, or, once pressed, the question
    /// whether to remove the source, with buttons to confirm or cancel.
    fn removal_controls<'a>(
        integration: &IntegrationModel,
        confirming: bool,
        removing: bool,
    ) -> Element<'a, Message> {
        if !confirming {
            return button(
                fa_icon_solid("trash")
                    .size(14.0_f32)
                    .style(|theme: &Theme| text::Style {
                        color: Some(theme.extended_palette().background.base.text),
                    }),
            )
            .on_press(Message::RemovePressed(integration.id.clone()))
            .padding([10, 12])
            .style(button::text)
            .into();
        }

        row![
            text("Remove it and its library items?").size(13),
            button(text(if removing { "Removing…" } else { "Remove" }))
                .on_press_maybe(
                    (!removing).then(|| Message::RemoveConfirmed(integration.id.clone()))
                )
                .style(button::danger),
            button(text("Cancel"))
                .on_press_maybe((!removing).then_some(Message::RemoveCancelled))
                .style(button::secondary),
        ]
        .spacing(8)
        .align_y(Center)
        .into()
    }

    fn icon_badge<'a>(icon: &'static [u8]) -> Element<'a, Message> {
        container(svg(svg::Handle::from_memory(icon)).width(20).height(20))
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

    fn integration_icon(integration_type: IntegrationType) -> &'static [u8] {
        match integration_type {
            IntegrationType::Kavita => icons::KAVITA,
            IntegrationType::Komga => icons::KOMGA,
            IntegrationType::LocalFolder => icons::FOLDER,
            IntegrationType::Opds => icons::OPDS,
            IntegrationType::Suwayomi => icons::SUWAYOMI,
        }
    }

    fn empty_list<'a>() -> Element<'a, Message> {
        let content = column![
            svg(svg::Handle::from_memory(icons::PLUG_CONNECT))
                .height(64)
                .width(64)
                .style(|theme: &Theme, _status| svg::Style {
                    color: Some(theme.extended_palette().background.neutral.color),
                }),
            text("Você não tem nenhuma fonte cadastrada, adicione uma pasta local ou um servidor"),
            button("Add Source").on_press(Message::AddIntegrationPressed)
        ]
        .spacing(20)
        .max_width(420)
        .align_x(Center);

        container(content).center(Fill).into()
    }

    pub fn choose_type<'a>() -> Element<'a, Message> {
        container(column![
            grid(type_squares())
                .columns(3)
                .height(256.0)
                .width(384.0)
                .spacing(25),
        ])
        .center(Fill)
        .into()
    }

    /// One square per source type built into this binary. Local folders always
    /// are.
    fn type_squares<'a>() -> Vec<Element<'a, Message>> {
        let mut squares = vec![type_square(
            "Local folder",
            icons::FOLDER,
            IntegrationType::LocalFolder,
        )];
        #[cfg(feature = "kavita")]
        squares.push(type_square(
            "Kavita",
            icons::KAVITA,
            IntegrationType::Kavita,
        ));
        #[cfg(feature = "komga")]
        squares.push(type_square("Komga", icons::KOMGA, IntegrationType::Komga));
        #[cfg(feature = "opds")]
        squares.push(type_square("OPDS", icons::OPDS, IntegrationType::Opds));
        #[cfg(feature = "suwayomi")]
        squares.push(type_square(
            "Suwayomi",
            icons::SUWAYOMI,
            IntegrationType::Suwayomi,
        ));
        squares
    }

    fn type_square<'a>(
        name: &'a str,
        logo: &'static [u8],
        integration_type: IntegrationType,
    ) -> Element<'a, Message> {
        button(
            container(
                column![
                    svg(svg::Handle::from_memory(logo)).width(48).height(48),
                    text(name)
                ]
                .align_x(Center)
                .spacing(5),
            )
            .center(Fill),
        )
        .width(Fill)
        .height(Fill)
        .on_press(Message::TypeSelected(integration_type))
        .into()
    }
}

#[cfg(test)]
mod tests {
    use iced_test::simulator;

    use super::*;

    fn integration_model(name: &str) -> IntegrationModel {
        IntegrationModel {
            id: "id".to_string(),
            name: name.to_string(),
            integration_type: IntegrationType::Kavita,
            url: Some("http://localhost:5000".to_string()),
            api_key: Some("api-key".to_string()),
            username: None,
            password: None,
            folder: None,
        }
    }

    #[test]
    fn view_renders_empty_container_while_not_loaded() {
        let integrations = Integrations {
            step: Step::List(AsyncModel::Loading),
            ..Integrations::default()
        };

        let mut ui = simulator(integrations.view());

        assert!(ui.find("Add Source").is_err());
    }

    #[test]
    fn view_renders_call_to_action_when_loaded_without_integrations() {
        let integrations = Integrations {
            step: Step::List(AsyncModel::Loaded(vec![])),
            ..Integrations::default()
        };

        let mut ui = simulator(integrations.view());

        assert!(ui.find("Add Source").is_ok());
    }

    #[test]
    fn view_renders_integration_names_when_loaded() {
        let integrations = Integrations {
            step: Step::List(AsyncModel::Loaded(vec![integration_model("My Kavita")])),
            ..Integrations::default()
        };

        let mut ui = simulator(integrations.view());

        assert!(ui.find("My Kavita").is_ok());
    }

    #[test]
    fn integration_list_scrolls_when_it_overflows_the_screen() {
        let integrations = Integrations {
            step: Step::List(AsyncModel::Loaded(
                (0..30)
                    .map(|index| integration_model(&format!("Integration {index}")))
                    .collect(),
            )),
            ..Integrations::default()
        };

        let mut ui = iced_test::Simulator::with_size(
            iced::Settings::default(),
            (800.0, 300.0),
            integrations.view(),
        );
        assert_eq!(ui.find("Integration 29").unwrap().visible_bounds(), None);

        ui.point_at(iced::Point::new(400.0, 200.0));
        let _ = ui.simulate([iced::Event::Mouse(iced::mouse::Event::WheelScrolled {
            delta: iced::mouse::ScrollDelta::Lines { x: 0.0, y: -50.0 },
        })]);

        assert!(
            ui.find("Integration 29")
                .unwrap()
                .visible_bounds()
                .is_some()
        );
    }

    #[test]
    fn view_renders_add_integration_button_when_integrations_exist() {
        let integrations = Integrations {
            step: Step::List(AsyncModel::Loaded(vec![integration_model("My Kavita")])),
            ..Integrations::default()
        };

        let mut ui = simulator(integrations.view());
        let _ = ui
            .click("Add Source")
            .expect("Add Source button should be found");

        let messages: Vec<_> = ui.into_messages().collect();

        assert_eq!(messages, vec![Message::AddIntegrationPressed]);
    }

    #[test]
    fn clicking_add_integration_sends_add_integration_pressed_message() {
        let integrations = Integrations {
            step: Step::List(AsyncModel::Loaded(vec![])),
            ..Integrations::default()
        };

        let mut ui = simulator(integrations.view());
        let _ = ui
            .click("Add Source")
            .expect("Add Source button should be found");

        let messages: Vec<_> = ui.into_messages().collect();

        assert_eq!(messages, vec![Message::AddIntegrationPressed]);
    }

    #[test]
    fn clicking_a_source_sends_source_selected() {
        let integrations = Integrations {
            step: Step::List(AsyncModel::Loaded(vec![integration_model("My Kavita")])),
            ..Integrations::default()
        };

        let mut ui = simulator(integrations.view());
        let _ = ui.click("My Kavita").expect("source row should be found");

        assert_eq!(
            ui.into_messages().collect::<Vec<_>>(),
            vec![Message::SourceSelected("id".to_string())]
        );
    }

    #[test]
    fn removing_a_source_asks_to_confirm_first() {
        let integrations = Integrations {
            step: Step::List(AsyncModel::Loaded(vec![integration_model("My Kavita")])),
            ..Integrations::default()
        };
        let mut ui = simulator(integrations.view());
        assert!(ui.find("Remove").is_err());
        drop(ui);

        let mut integrations = integrations;
        assert!(matches!(
            integrations.update(Message::RemovePressed("id".to_string())),
            Action::None
        ));

        let mut ui = simulator(integrations.view());
        let _ = ui
            .click("Remove")
            .expect("the confirm button should be found");
        let _ = ui
            .click("Cancel")
            .expect("the cancel button should be found");
        assert_eq!(
            ui.into_messages().collect::<Vec<_>>(),
            vec![
                Message::RemoveConfirmed("id".to_string()),
                Message::RemoveCancelled
            ]
        );
    }

    #[test]
    fn cancelling_a_removal_removes_nothing() {
        let mut integrations = Integrations::default();
        let _ = integrations.update(Message::RemovePressed("id".to_string()));

        let _ = integrations.update(Message::RemoveCancelled);

        assert_eq!(integrations.confirming_removal, None);
        // Nothing to confirm any more.
        assert!(matches!(
            integrations.update(Message::RemoveConfirmed("id".to_string())),
            Action::None
        ));
    }

    #[test]
    fn a_confirmed_removal_runs_once_then_reloads_the_list() {
        let mut integrations = Integrations::default();
        let _ = integrations.update(Message::RemovePressed("id".to_string()));

        assert!(matches!(
            integrations.update(Message::RemoveConfirmed("id".to_string())),
            Action::Run(_)
        ));
        assert!(matches!(
            integrations.update(Message::RemoveConfirmed("id".to_string())),
            Action::None
        ));

        assert!(matches!(
            integrations.update(Message::Removed(Ok(()))),
            Action::Run(_)
        ));
        assert!(!integrations.removing);
        assert_eq!(integrations.confirming_removal, None);
        assert!(matches!(integrations.step, Step::List(AsyncModel::Loading)));
    }

    #[test]
    fn a_failed_removal_is_reported() {
        let mut integrations = Integrations::default();
        let _ = integrations.update(Message::RemovePressed("id".to_string()));
        let _ = integrations.update(Message::RemoveConfirmed("id".to_string()));

        let _ = integrations.update(Message::Removed(Err("locked".to_string())));
        let _ = integrations.update(Message::IntegrationsLoaded(Ok(vec![integration_model(
            "My Kavita",
        )])));

        let mut ui = simulator(integrations.view());
        assert!(ui.find("Could not remove the source: locked").is_ok());
    }

    #[test]
    fn selecting_a_source_opens_it() {
        let mut integrations = Integrations::default();

        let action = integrations.update(Message::SourceSelected("id".to_string()));

        assert!(matches!(action, Action::OpenSource(id) if id == "id"));
    }

    #[test]
    fn view_choose_type_always_offers_local_folders() {
        let integrations = Integrations {
            step: Step::ChooseType,
            ..Integrations::default()
        };

        let mut ui = simulator(integrations.view());
        let _ = ui
            .click("Local folder")
            .expect("Local folder square should be found");

        assert_eq!(
            ui.into_messages().collect::<Vec<_>>(),
            vec![Message::TypeSelected(IntegrationType::LocalFolder)]
        );
    }

    #[test]
    fn update_type_selected_local_folder_shows_its_form() {
        let mut integrations = Integrations::default();

        let action = integrations.update(Message::TypeSelected(IntegrationType::LocalFolder));

        assert!(matches!(action, Action::None));
        assert!(matches!(integrations.step, Step::LocalFolder(_)));
    }

    #[test]
    fn update_local_folder_created_returns_to_the_list_and_reloads() {
        let mut integrations = Integrations {
            step: Step::LocalFolder(LocalFolder::default()),
            ..Integrations::default()
        };

        let action = integrations.update(Message::LocalFolder(local_folder::Message::Created(Ok(
            integration_model("Comics"),
        ))));

        assert!(matches!(action, Action::Run(_)));
        assert!(matches!(integrations.step, Step::List(AsyncModel::Loading)));
    }

    #[test]
    fn update_add_integration_pressed_shows_choose_type_step() {
        let mut integrations = Integrations::default();

        let action = integrations.update(Message::AddIntegrationPressed);

        assert!(matches!(action, Action::None));
        assert!(matches!(integrations.step, Step::ChooseType));
    }

    #[test]
    #[cfg(feature = "kavita")]
    fn view_choose_type_renders_kavita_option() {
        let integrations = Integrations {
            step: Step::ChooseType,
            ..Integrations::default()
        };

        let mut ui = simulator(integrations.view());

        assert!(ui.find("Kavita").is_ok());
    }

    #[test]
    #[cfg(feature = "kavita")]
    fn clicking_kavita_square_sends_type_selected_message() {
        let integrations = Integrations {
            step: Step::ChooseType,
            ..Integrations::default()
        };

        let mut ui = simulator(integrations.view());
        let _ = ui.click("Kavita").expect("Kavita square should be found");

        let messages: Vec<_> = ui.into_messages().collect();

        assert_eq!(
            messages,
            vec![Message::TypeSelected(IntegrationType::Kavita)]
        );
    }

    #[test]
    #[cfg(feature = "kavita")]
    fn update_type_selected_shows_kavita_form_step() {
        let mut integrations = Integrations::default();

        let action = integrations.update(Message::TypeSelected(IntegrationType::Kavita));

        assert!(matches!(action, Action::None));
        assert!(matches!(integrations.step, Step::Kavita(_)));
    }

    #[test]
    #[cfg(feature = "opds")]
    fn clicking_opds_square_sends_type_selected_message() {
        let integrations = Integrations {
            step: Step::ChooseType,
            ..Integrations::default()
        };

        let mut ui = simulator(integrations.view());
        let _ = ui.click("OPDS").expect("OPDS square should be found");

        let messages: Vec<_> = ui.into_messages().collect();

        assert_eq!(messages, vec![Message::TypeSelected(IntegrationType::Opds)]);
    }

    #[test]
    #[cfg(feature = "opds")]
    fn update_type_selected_opds_shows_opds_form_step() {
        let mut integrations = Integrations::default();

        let action = integrations.update(Message::TypeSelected(IntegrationType::Opds));

        assert!(matches!(action, Action::None));
        assert!(matches!(integrations.step, Step::Opds(_)));
    }

    #[test]
    #[cfg(feature = "opds")]
    fn update_opds_back_pressed_returns_to_list_and_reloads() {
        let mut integrations = Integrations {
            step: Step::Opds(Opds::default()),
            ..Integrations::default()
        };

        let action = integrations.update(Message::Opds(opds::Message::BackPressed));

        assert!(matches!(action, Action::Run(_)));
        assert!(matches!(integrations.step, Step::List(AsyncModel::Loading)));
    }

    #[test]
    #[cfg(feature = "opds")]
    fn update_opds_created_returns_to_list_and_reloads() {
        let mut integrations = Integrations {
            step: Step::Opds(Opds::default()),
            ..Integrations::default()
        };

        let action = integrations.update(Message::Opds(opds::Message::Created(Ok(
            integration_model("My Catalog"),
        ))));

        assert!(matches!(action, Action::Run(_)));
        assert!(matches!(integrations.step, Step::List(AsyncModel::Loading)));
    }

    #[test]
    #[cfg(feature = "komga")]
    fn clicking_komga_square_sends_type_selected_message() {
        let integrations = Integrations {
            step: Step::ChooseType,
            ..Integrations::default()
        };

        let mut ui = simulator(integrations.view());
        let _ = ui.click("Komga").expect("Komga square should be found");

        let messages: Vec<_> = ui.into_messages().collect();

        assert_eq!(
            messages,
            vec![Message::TypeSelected(IntegrationType::Komga)]
        );
    }

    #[test]
    #[cfg(feature = "komga")]
    fn update_type_selected_komga_shows_komga_form_step() {
        let mut integrations = Integrations::default();

        let action = integrations.update(Message::TypeSelected(IntegrationType::Komga));

        assert!(matches!(action, Action::None));
        assert!(matches!(integrations.step, Step::Komga(_)));
    }

    #[test]
    #[cfg(feature = "komga")]
    fn update_komga_created_returns_to_list_and_reloads() {
        let mut integrations = Integrations {
            step: Step::Komga(Komga::default()),
            ..Integrations::default()
        };

        let action = integrations.update(Message::Komga(komga::Message::Created(Ok(
            integration_model("My Komga"),
        ))));

        assert!(matches!(action, Action::Run(_)));
        assert!(matches!(integrations.step, Step::List(AsyncModel::Loading)));
    }

    #[test]
    #[cfg(feature = "suwayomi")]
    fn clicking_suwayomi_square_sends_type_selected_message() {
        let integrations = Integrations {
            step: Step::ChooseType,
            ..Integrations::default()
        };

        let mut ui = simulator(integrations.view());
        let _ = ui
            .click("Suwayomi")
            .expect("Suwayomi square should be found");

        let messages: Vec<_> = ui.into_messages().collect();

        assert_eq!(
            messages,
            vec![Message::TypeSelected(IntegrationType::Suwayomi)]
        );
    }

    #[test]
    #[cfg(feature = "suwayomi")]
    fn update_type_selected_suwayomi_shows_suwayomi_form_step() {
        let mut integrations = Integrations::default();

        let action = integrations.update(Message::TypeSelected(IntegrationType::Suwayomi));

        assert!(matches!(action, Action::None));
        assert!(matches!(integrations.step, Step::Suwayomi(_)));
    }

    #[test]
    #[cfg(feature = "suwayomi")]
    fn update_suwayomi_created_returns_to_list_and_reloads() {
        let mut integrations = Integrations {
            step: Step::Suwayomi(Suwayomi::default()),
            ..Integrations::default()
        };

        let action = integrations.update(Message::Suwayomi(suwayomi::Message::Created(Ok(
            integration_model("My Suwayomi"),
        ))));

        assert!(matches!(action, Action::Run(_)));
        assert!(matches!(integrations.step, Step::List(AsyncModel::Loading)));
    }

    #[test]
    #[cfg(feature = "kavita")]
    fn update_kavita_back_pressed_returns_to_list_and_reloads() {
        let mut integrations = Integrations {
            step: Step::Kavita(Kavita::default()),
            ..Integrations::default()
        };

        let action = integrations.update(Message::Kavita(kavita::Message::BackPressed));

        assert!(matches!(action, Action::Run(_)));
        assert!(matches!(integrations.step, Step::List(AsyncModel::Loading)));
    }

    #[test]
    #[cfg(feature = "kavita")]
    fn update_kavita_created_returns_to_list_and_reloads() {
        let mut integrations = Integrations {
            step: Step::Kavita(Kavita::default()),
            ..Integrations::default()
        };

        let action = integrations.update(Message::Kavita(kavita::Message::Created(Ok(
            integration_model("My Kavita"),
        ))));

        assert!(matches!(action, Action::Run(_)));
        assert!(matches!(integrations.step, Step::List(AsyncModel::Loading)));
    }
}
