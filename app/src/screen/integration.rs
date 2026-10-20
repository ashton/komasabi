pub mod kavita;
pub mod komga;
pub mod opds;

use iced::{Element, Task, widget::container};

use crate::models::{
    AsyncModel,
    integration::{Integration as IntegrationModel, IntegrationType},
};
use crate::repository::integration as integration_repository;
use kavita::Kavita;
use komga::Komga;
use opds::Opds;

#[derive(Debug)]
enum Step {
    List(AsyncModel<Vec<IntegrationModel>, String>),
    ChooseType,
    Kavita(Kavita),
    Komga(Komga),
    Opds(Opds),
}

#[derive(Debug)]
pub struct Integrations {
    step: Step,
}

impl Default for Integrations {
    fn default() -> Self {
        Self {
            step: Step::List(AsyncModel::Loading),
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum Message {
    IntegrationsLoaded(Result<Vec<IntegrationModel>, String>),
    AddIntegrationPressed,
    TypeSelected(IntegrationType),
    Kavita(kavita::Message),
    Komga(komga::Message),
    Opds(opds::Message),
}

#[derive(Debug)]
pub enum Action {
    None,
    Run(Task<Message>),
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

            Message::TypeSelected(IntegrationType::Kavita) => {
                self.step = Step::Kavita(Kavita::default());
                Action::None
            }

            Message::TypeSelected(IntegrationType::Komga) => {
                self.step = Step::Komga(Komga::default());
                Action::None
            }

            Message::TypeSelected(IntegrationType::Opds) => {
                self.step = Step::Opds(Opds::default());
                Action::None
            }

            Message::TypeSelected(_) => Action::None,

            Message::Kavita(msg) => {
                let Step::Kavita(kavita) = &mut self.step else {
                    return Action::None;
                };

                match kavita.update(msg) {
                    kavita::Action::None => Action::None,
                    kavita::Action::Run(task) => Action::Run(task.map(Message::Kavita)),
                    kavita::Action::BackPressed => self.reload_list(),
                    kavita::Action::Created(integration) => self.integration_created(integration),
                }
            }

            Message::Komga(msg) => {
                let Step::Komga(komga) = &mut self.step else {
                    return Action::None;
                };

                match komga.update(msg) {
                    komga::Action::None => Action::None,
                    komga::Action::Run(task) => Action::Run(task.map(Message::Komga)),
                    komga::Action::BackPressed => self.reload_list(),
                    komga::Action::Created(integration) => self.integration_created(integration),
                }
            }

            Message::Opds(msg) => {
                let Step::Opds(opds) = &mut self.step else {
                    return Action::None;
                };

                match opds.update(msg) {
                    opds::Action::None => Action::None,
                    opds::Action::Run(task) => Action::Run(task.map(Message::Opds)),
                    opds::Action::BackPressed => self.reload_list(),
                    opds::Action::Created(integration) => self.integration_created(integration),
                }
            }
        }
    }

    /// Returns to the list once an integration is saved, kicking off the
    /// background job that imports its libraries.
    fn integration_created(&mut self, integration: IntegrationModel) -> Action {
        self.step = Step::List(AsyncModel::Loading);
        Action::Run(Task::future(async move {
            crate::jobs::sync_integration_libraries::spawn(integration.id);
            Message::IntegrationsLoaded(integration_repository::all().await)
        }))
    }

    fn reload_list(&mut self) -> Action {
        self.step = Step::List(AsyncModel::Loading);
        Action::Run(Task::perform(
            integration_repository::all(),
            Message::IntegrationsLoaded,
        ))
    }

    pub fn view<'a>(&'a self) -> Element<'a, Message> {
        match &self.step {
            Step::List(AsyncModel::Loaded(integrations)) => view_helper::list(integrations),
            Step::List(_) => container(iced::widget::row![]).into(),
            Step::ChooseType => view_helper::choose_type(),
            Step::Kavita(kavita) => kavita.view().map(Message::Kavita),
            Step::Komga(komga) => komga.view().map(Message::Komga),
            Step::Opds(opds) => opds.view().map(Message::Opds),
        }
    }
}

mod view_helper {
    use super::{IntegrationModel, IntegrationType, Message};
    use crate::icons;
    use iced::{
        Background, Border, Center, Element,
        Length::Fill,
        Theme,
        widget::{button, column, container, grid, row, svg, text},
    };
    use iced_font_awesome::fa_icon_solid;

    pub fn list<'a>(integrations: &'a [IntegrationModel]) -> Element<'a, Message> {
        if integrations.is_empty() {
            return empty_list();
        }

        let rows = integrations.iter().map(integration_row);

        container(column![add_integration_button(), column(rows).spacing(8)].spacing(20))
            .padding(20)
            .into()
    }

    /// Always available above the integration list, so more integrations can be
    /// added once the first exists.
    fn add_integration_button<'a>() -> Element<'a, Message> {
        button(
            row![
                fa_icon_solid("plus").size(14.0_f32),
                text("Add Integration")
            ]
            .spacing(6)
            .align_y(Center),
        )
        .on_press(Message::AddIntegrationPressed)
        .into()
    }

    fn integration_row<'a>(integration: &'a IntegrationModel) -> Element<'a, Message> {
        container(
            row![
                icon_badge(integration_icon(integration.integration_type)),
                column![
                    text(&integration.name),
                    text(integration.integration_type.label())
                        .size(12)
                        .style(text::secondary),
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
            text(
                "Você não tem nenhuma integração cadastrada, adicione uma integração com algum serviço"
            ),
            button("Add Integration").on_press(Message::AddIntegrationPressed)
        ]
        .spacing(20)
        .max_width(420)
        .align_x(Center);

        container(content).center(Fill).into()
    }

    pub fn choose_type<'a>() -> Element<'a, Message> {
        container(column![
            grid(vec![
                type_square("Kavita", icons::KAVITA, IntegrationType::Kavita),
                type_square("Komga", icons::KOMGA, IntegrationType::Komga),
                type_square("OPDS", icons::OPDS, IntegrationType::Opds),
                type_square("Suwayomi", icons::SUWAYOMI, IntegrationType::Suwayomi)
            ])
            .columns(2)
            .height(256.0)
            .width(256.0)
            .spacing(25),
        ])
        .center(Fill)
        .into()
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
        }
    }

    #[test]
    fn view_renders_empty_container_while_not_loaded() {
        let integrations = Integrations {
            step: Step::List(AsyncModel::Loading),
        };

        let mut ui = simulator(integrations.view());

        assert!(ui.find("Add Integration").is_err());
    }

    #[test]
    fn view_renders_call_to_action_when_loaded_without_integrations() {
        let integrations = Integrations {
            step: Step::List(AsyncModel::Loaded(vec![])),
        };

        let mut ui = simulator(integrations.view());

        assert!(ui.find("Add Integration").is_ok());
    }

    #[test]
    fn view_renders_integration_names_when_loaded() {
        let integrations = Integrations {
            step: Step::List(AsyncModel::Loaded(vec![integration_model("My Kavita")])),
        };

        let mut ui = simulator(integrations.view());

        assert!(ui.find("My Kavita").is_ok());
    }

    #[test]
    fn view_renders_add_integration_button_when_integrations_exist() {
        let integrations = Integrations {
            step: Step::List(AsyncModel::Loaded(vec![integration_model("My Kavita")])),
        };

        let mut ui = simulator(integrations.view());
        let _ = ui
            .click("Add Integration")
            .expect("Add Integration button should be found");

        let messages: Vec<_> = ui.into_messages().collect();

        assert_eq!(messages, vec![Message::AddIntegrationPressed]);
    }

    #[test]
    fn clicking_add_integration_sends_add_integration_pressed_message() {
        let integrations = Integrations {
            step: Step::List(AsyncModel::Loaded(vec![])),
        };

        let mut ui = simulator(integrations.view());
        let _ = ui
            .click("Add Integration")
            .expect("Add Integration button should be found");

        let messages: Vec<_> = ui.into_messages().collect();

        assert_eq!(messages, vec![Message::AddIntegrationPressed]);
    }

    #[test]
    fn update_add_integration_pressed_shows_choose_type_step() {
        let mut integrations = Integrations::default();

        let action = integrations.update(Message::AddIntegrationPressed);

        assert!(matches!(action, Action::None));
        assert!(matches!(integrations.step, Step::ChooseType));
    }

    #[test]
    fn view_choose_type_renders_kavita_option() {
        let integrations = Integrations {
            step: Step::ChooseType,
        };

        let mut ui = simulator(integrations.view());

        assert!(ui.find("Kavita").is_ok());
    }

    #[test]
    fn clicking_kavita_square_sends_type_selected_message() {
        let integrations = Integrations {
            step: Step::ChooseType,
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
    fn update_type_selected_shows_kavita_form_step() {
        let mut integrations = Integrations::default();

        let action = integrations.update(Message::TypeSelected(IntegrationType::Kavita));

        assert!(matches!(action, Action::None));
        assert!(matches!(integrations.step, Step::Kavita(_)));
    }

    #[test]
    fn clicking_opds_square_sends_type_selected_message() {
        let integrations = Integrations {
            step: Step::ChooseType,
        };

        let mut ui = simulator(integrations.view());
        let _ = ui.click("OPDS").expect("OPDS square should be found");

        let messages: Vec<_> = ui.into_messages().collect();

        assert_eq!(messages, vec![Message::TypeSelected(IntegrationType::Opds)]);
    }

    #[test]
    fn update_type_selected_opds_shows_opds_form_step() {
        let mut integrations = Integrations::default();

        let action = integrations.update(Message::TypeSelected(IntegrationType::Opds));

        assert!(matches!(action, Action::None));
        assert!(matches!(integrations.step, Step::Opds(_)));
    }

    #[test]
    fn update_opds_back_pressed_returns_to_list_and_reloads() {
        let mut integrations = Integrations {
            step: Step::Opds(Opds::default()),
        };

        let action = integrations.update(Message::Opds(opds::Message::BackPressed));

        assert!(matches!(action, Action::Run(_)));
        assert!(matches!(integrations.step, Step::List(AsyncModel::Loading)));
    }

    #[test]
    fn update_opds_created_returns_to_list_and_reloads() {
        let mut integrations = Integrations {
            step: Step::Opds(Opds::default()),
        };

        let action = integrations.update(Message::Opds(opds::Message::Created(Ok(
            integration_model("My Catalog"),
        ))));

        assert!(matches!(action, Action::Run(_)));
        assert!(matches!(integrations.step, Step::List(AsyncModel::Loading)));
    }

    #[test]
    fn clicking_komga_square_sends_type_selected_message() {
        let integrations = Integrations {
            step: Step::ChooseType,
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
    fn update_type_selected_komga_shows_komga_form_step() {
        let mut integrations = Integrations::default();

        let action = integrations.update(Message::TypeSelected(IntegrationType::Komga));

        assert!(matches!(action, Action::None));
        assert!(matches!(integrations.step, Step::Komga(_)));
    }

    #[test]
    fn update_komga_created_returns_to_list_and_reloads() {
        let mut integrations = Integrations {
            step: Step::Komga(Komga::default()),
        };

        let action = integrations.update(Message::Komga(komga::Message::Created(Ok(
            integration_model("My Komga"),
        ))));

        assert!(matches!(action, Action::Run(_)));
        assert!(matches!(integrations.step, Step::List(AsyncModel::Loading)));
    }

    #[test]
    fn update_kavita_back_pressed_returns_to_list_and_reloads() {
        let mut integrations = Integrations {
            step: Step::Kavita(Kavita::default()),
        };

        let action = integrations.update(Message::Kavita(kavita::Message::BackPressed));

        assert!(matches!(action, Action::Run(_)));
        assert!(matches!(integrations.step, Step::List(AsyncModel::Loading)));
    }

    #[test]
    fn update_kavita_created_returns_to_list_and_reloads() {
        let mut integrations = Integrations {
            step: Step::Kavita(Kavita::default()),
        };

        let action = integrations.update(Message::Kavita(kavita::Message::Created(Ok(
            integration_model("My Kavita"),
        ))));

        assert!(matches!(action, Action::Run(_)));
        assert!(matches!(integrations.step, Step::List(AsyncModel::Loading)));
    }
}
