use iced::{Element, Task};

use crate::models::integration::{Integration as IntegrationModel, IntegrationType};
use crate::providers::suwayomi::provider::SuwayomiProvider;
use crate::repository::integration as integration_repository;

/// The form for a Suwayomi server. Suwayomi runs without authentication by
/// default; the username and password are for a server set up with basic auth.
#[derive(Debug, Default)]
pub struct Suwayomi {
    name: String,
    url: String,
    username: String,
    password: String,
    /// The server's name and version on success, from the last "Test
    /// Connection".
    server_test: Option<Result<String, String>>,
    url_invalid: bool,
}

fn is_valid_server_url(value: &str) -> bool {
    url::Url::parse(value)
        .is_ok_and(|url| matches!(url.scheme(), "http" | "https") && url.host().is_some())
}

fn optional(value: &str) -> Option<String> {
    let value = value.trim();
    (!value.is_empty()).then(|| value.to_string())
}

#[derive(Clone, Debug, PartialEq)]
pub enum Message {
    NameChanged(String),
    UrlChanged(String),
    UsernameChanged(String),
    PasswordChanged(String),
    BackPressed,
    Submitted,
    Created(Result<IntegrationModel, String>),
    TestConnectionPressed,
    ServerTested(Result<String, String>),
}

#[derive(Debug)]
pub enum Action {
    None,
    Run(Task<Message>),
    BackPressed,
    Created(IntegrationModel),
}

impl Suwayomi {
    pub fn update(&mut self, message: Message) -> Action {
        match message {
            Message::NameChanged(name) => {
                self.name = name;
                Action::None
            }

            Message::UrlChanged(url) => {
                self.url = url;
                self.url_invalid = !is_valid_server_url(&self.url);
                Action::None
            }

            Message::UsernameChanged(username) => {
                self.username = username;
                Action::None
            }

            Message::PasswordChanged(password) => {
                self.password = password;
                Action::None
            }

            Message::BackPressed => Action::BackPressed,

            Message::Submitted => Action::Run(Task::perform(
                integration_repository::create_with_credentials(
                    self.name.clone(),
                    IntegrationType::Suwayomi,
                    self.url.trim().to_string(),
                    optional(&self.username),
                    optional(&self.password),
                ),
                Message::Created,
            )),

            Message::Created(Ok(integration)) => Action::Created(integration),
            Message::Created(Err(_)) => Action::None,

            Message::TestConnectionPressed => {
                self.url_invalid = !is_valid_server_url(&self.url);
                self.server_test = None;

                if self.url_invalid {
                    Action::None
                } else {
                    Action::Run(Task::perform(
                        crate::runtime::on_tokio(SuwayomiProvider::check_server(
                            self.url.trim().to_string(),
                            optional(&self.username),
                            optional(&self.password),
                        )),
                        Message::ServerTested,
                    ))
                }
            }

            Message::ServerTested(result) => {
                self.server_test = Some(result);
                Action::None
            }
        }
    }

    pub fn view<'a>(&'a self) -> Element<'a, Message> {
        view_helper::form(self)
    }

    fn has_errors(&self) -> bool {
        self.url_invalid || matches!(self.server_test, Some(Err(_)))
    }
}

mod view_helper {
    use super::{Message, Suwayomi};
    use iced::{
        Center, Element, Theme,
        widget::{button, column, container, row, text, text_input},
    };

    pub fn form<'a>(suwayomi: &'a Suwayomi) -> Element<'a, Message> {
        container(
            column![
                text_input("Name", &suwayomi.name).on_input(Message::NameChanged),
                row![
                    text_input("Server URL", &suwayomi.url)
                        .on_input(Message::UrlChanged)
                        .style(move |theme: &Theme, status| url_input_style(
                            theme,
                            status,
                            suwayomi.url_invalid
                        )),
                    url_error_toast(suwayomi),
                ]
                .spacing(10)
                .align_y(Center),
                text_input("Username (optional)", &suwayomi.username)
                    .on_input(Message::UsernameChanged),
                text_input("Password (optional)", &suwayomi.password)
                    .secure(true)
                    .on_input(Message::PasswordChanged),
                row![
                    button("Test Connection").on_press(Message::TestConnectionPressed),
                    server_toast(suwayomi),
                ]
                .spacing(10)
                .align_y(Center),
                row![
                    button("Back").on_press(Message::BackPressed),
                    button("Save")
                        .on_press_maybe((!suwayomi.has_errors()).then_some(Message::Submitted))
                ]
                .spacing(10)
            ]
            .spacing(10),
        )
        .into()
    }

    fn url_input_style(
        theme: &Theme,
        status: text_input::Status,
        url_invalid: bool,
    ) -> text_input::Style {
        let mut style = text_input::default(theme, status);

        if url_invalid {
            style.border.color = theme.extended_palette().danger.base.color;
        }

        style
    }

    fn url_error_toast<'a>(suwayomi: &'a Suwayomi) -> Element<'a, Message> {
        if suwayomi.url_invalid {
            text("The text must be a valid URL")
                .style(text::danger)
                .into()
        } else {
            row![].into()
        }
    }

    fn server_toast<'a>(suwayomi: &'a Suwayomi) -> Element<'a, Message> {
        match &suwayomi.server_test {
            Some(Ok(server)) => text(format!("Server connection successful: {server}"))
                .style(text::success)
                .into(),
            Some(Err(_)) => text("It wasn't possible to connect to a Suwayomi server at this URL")
                .style(text::danger)
                .into(),
            None => row![].into(),
        }
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
            integration_type: IntegrationType::Suwayomi,
            url: Some("http://localhost:4567".to_string()),
            api_key: None,
            username: None,
            password: None,
        }
    }

    fn click_save(suwayomi: &Suwayomi) -> Vec<Message> {
        let mut ui = simulator(suwayomi.view());
        let _ = ui.click("Save").expect("Save button should be found");

        ui.into_messages().collect()
    }

    #[test]
    fn view_renders_inputs_and_buttons() {
        let suwayomi = Suwayomi::default();

        let mut ui = simulator(suwayomi.view());

        assert!(ui.find("Back").is_ok());
        assert!(ui.find("Save").is_ok());
        assert!(ui.find("Test Connection").is_ok());
    }

    #[test]
    fn update_stores_field_values() {
        let mut suwayomi = Suwayomi::default();

        let _ = suwayomi.update(Message::NameChanged("Library".to_string()));
        let _ = suwayomi.update(Message::UsernameChanged("reader".to_string()));
        let _ = suwayomi.update(Message::PasswordChanged("secret".to_string()));

        assert_eq!(suwayomi.name, "Library");
        assert_eq!(suwayomi.username, "reader");
        assert_eq!(suwayomi.password, "secret");
    }

    #[test]
    fn typing_an_invalid_url_flags_it_and_a_valid_one_clears_it() {
        let mut suwayomi = Suwayomi::default();

        let _ = suwayomi.update(Message::UrlChanged("not a url".to_string()));
        assert!(suwayomi.url_invalid);

        let _ = suwayomi.update(Message::UrlChanged("http://localhost:4567".to_string()));
        assert!(!suwayomi.url_invalid);
    }

    #[test]
    fn update_back_pressed_returns_back_pressed_action() {
        let mut suwayomi = Suwayomi::default();

        assert!(matches!(
            suwayomi.update(Message::BackPressed),
            Action::BackPressed
        ));
    }

    #[test]
    fn update_submitted_runs_a_task() {
        let mut suwayomi = Suwayomi {
            name: "Library".to_string(),
            url: "http://localhost:4567".to_string(),
            ..Suwayomi::default()
        };

        assert!(matches!(
            suwayomi.update(Message::Submitted),
            Action::Run(_)
        ));
    }

    #[test]
    fn update_created_ok_returns_created_action_and_err_does_nothing() {
        let mut suwayomi = Suwayomi::default();

        assert!(matches!(
            suwayomi.update(Message::Created(Ok(integration_model("Library")))),
            Action::Created(_)
        ));
        assert!(matches!(
            suwayomi.update(Message::Created(Err("boom".to_string()))),
            Action::None
        ));
    }

    #[test]
    fn clicking_test_connection_sends_test_connection_pressed_message() {
        let suwayomi = Suwayomi::default();

        let mut ui = simulator(suwayomi.view());
        let _ = ui
            .click("Test Connection")
            .expect("Test Connection button should be found");

        let messages: Vec<_> = ui.into_messages().collect();

        assert_eq!(messages, vec![Message::TestConnectionPressed]);
    }

    #[test]
    fn test_connection_with_a_valid_url_clears_the_previous_result_and_runs_a_task() {
        let mut suwayomi = Suwayomi {
            url: "http://localhost:4567".to_string(),
            server_test: Some(Err("boom".to_string())),
            ..Suwayomi::default()
        };

        let action = suwayomi.update(Message::TestConnectionPressed);

        assert!(matches!(action, Action::Run(_)));
        assert_eq!(suwayomi.server_test, None);
    }

    #[test]
    fn test_connection_with_an_invalid_url_does_not_run_a_task() {
        let mut suwayomi = Suwayomi::default();
        let _ = suwayomi.update(Message::UrlChanged("not a url".to_string()));

        assert!(matches!(
            suwayomi.update(Message::TestConnectionPressed),
            Action::None
        ));
    }

    #[test]
    fn view_renders_the_server_version_after_a_successful_test() {
        let suwayomi = Suwayomi {
            server_test: Some(Ok("Suwayomi-Server v2.3.2243".to_string())),
            ..Suwayomi::default()
        };

        let mut ui = simulator(suwayomi.view());

        assert!(
            ui.find("Server connection successful: Suwayomi-Server v2.3.2243")
                .is_ok()
        );
    }

    #[test]
    fn view_renders_an_error_after_a_failed_test() {
        let suwayomi = Suwayomi {
            server_test: Some(Err("connection refused".to_string())),
            ..Suwayomi::default()
        };

        let mut ui = simulator(suwayomi.view());

        assert!(
            ui.find("It wasn't possible to connect to a Suwayomi server at this URL")
                .is_ok()
        );
    }

    #[test]
    fn save_is_enabled_without_errors_and_disabled_after_a_failed_test() {
        assert_eq!(click_save(&Suwayomi::default()), vec![Message::Submitted]);

        let failed = Suwayomi {
            server_test: Some(Err("boom".to_string())),
            ..Suwayomi::default()
        };
        assert_eq!(click_save(&failed), Vec::<Message>::new());
    }

    #[test]
    fn save_is_disabled_when_the_url_is_invalid() {
        let mut suwayomi = Suwayomi::default();
        let _ = suwayomi.update(Message::UrlChanged("not a url".to_string()));

        assert_eq!(click_save(&suwayomi), Vec::<Message>::new());
    }

    #[test]
    fn optional_treats_blank_values_as_absent() {
        assert_eq!(optional("  "), None);
        assert_eq!(optional(" reader "), Some("reader".to_string()));
    }
}
