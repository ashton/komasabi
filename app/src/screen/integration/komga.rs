use iced::{Element, Task};

use crate::models::integration::{Integration as IntegrationModel, IntegrationType};
use crate::providers::komga::provider::KomgaProvider;
use crate::repository::integration as integration_repository;

/// The form for a Komga server. Komga authenticates either with an API key
/// (created under the user's account settings) or with the account's email
/// and password; the API key wins when both are filled in.
#[derive(Debug, Default)]
pub struct Komga {
    name: String,
    url: String,
    api_key: String,
    username: String,
    password: String,
    connection_test: Option<Result<(), String>>,
    authentication_test: Option<Result<(), String>>,
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
    ApiKeyChanged(String),
    UsernameChanged(String),
    PasswordChanged(String),
    BackPressed,
    Submitted,
    Created(Result<IntegrationModel, String>),
    TestConnectionPressed,
    ConnectionTested(Result<(), String>),
    AuthenticationTested(Result<(), String>),
}

#[derive(Debug)]
pub enum Action {
    None,
    Run(Task<Message>),
    BackPressed,
    Created(IntegrationModel),
}

impl Komga {
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

            Message::ApiKeyChanged(api_key) => {
                self.api_key = api_key;
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
                integration_repository::create_with_auth(
                    self.name.clone(),
                    IntegrationType::Komga,
                    self.url.trim().to_string(),
                    optional(&self.api_key),
                    optional(&self.username),
                    optional(&self.password),
                ),
                Message::Created,
            )),

            Message::Created(Ok(integration)) => Action::Created(integration),
            Message::Created(Err(_)) => Action::None,

            Message::TestConnectionPressed => {
                self.url_invalid = !is_valid_server_url(&self.url);
                self.connection_test = None;
                self.authentication_test = None;

                if self.url_invalid {
                    Action::None
                } else {
                    let url = self.url.trim().to_string();

                    let connection_task = Task::perform(
                        crate::runtime::on_tokio(KomgaProvider::ping(url.clone())),
                        Message::ConnectionTested,
                    );

                    let authentication_task = Task::perform(
                        crate::runtime::on_tokio(KomgaProvider::check_authentication(
                            url,
                            optional(&self.api_key),
                            optional(&self.username),
                            optional(&self.password),
                        )),
                        Message::AuthenticationTested,
                    );

                    Action::Run(Task::batch([connection_task, authentication_task]))
                }
            }

            Message::ConnectionTested(result) => {
                self.connection_test = Some(result);
                Action::None
            }

            Message::AuthenticationTested(result) => {
                self.authentication_test = Some(result);
                Action::None
            }
        }
    }

    pub fn view<'a>(&'a self) -> Element<'a, Message> {
        view_helper::form(self)
    }

    fn has_errors(&self) -> bool {
        self.url_invalid
            || matches!(self.connection_test, Some(Err(_)))
            || matches!(self.authentication_test, Some(Err(_)))
    }
}

mod view_helper {
    use super::{Komga, Message};
    use iced::{
        Center, Element, Theme,
        widget::{button, column, container, row, text, text_input},
    };

    pub fn form<'a>(komga: &'a Komga) -> Element<'a, Message> {
        container(
            column![
                text_input("Name", &komga.name).on_input(Message::NameChanged),
                row![
                    text_input("Server URL", &komga.url)
                        .on_input(Message::UrlChanged)
                        .style(move |theme: &Theme, status| url_input_style(
                            theme,
                            status,
                            komga.url_invalid
                        )),
                    url_error_toast(komga),
                ]
                .spacing(10)
                .align_y(Center),
                text_input("API Key", &komga.api_key).on_input(Message::ApiKeyChanged),
                text("Or sign in with your Komga account")
                    .size(12)
                    .style(text::secondary),
                text_input("Email", &komga.username).on_input(Message::UsernameChanged),
                text_input("Password", &komga.password)
                    .secure(true)
                    .on_input(Message::PasswordChanged),
                row![
                    button("Test Connection").on_press(Message::TestConnectionPressed),
                    connection_toast(komga),
                    authentication_toast(komga),
                ]
                .spacing(10)
                .align_y(Center),
                row![
                    button("Back").on_press(Message::BackPressed),
                    button("Save")
                        .on_press_maybe((!komga.has_errors()).then_some(Message::Submitted))
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

    fn url_error_toast<'a>(komga: &'a Komga) -> Element<'a, Message> {
        if komga.url_invalid {
            text("The text must be a valid URL")
                .style(text::danger)
                .into()
        } else {
            row![].into()
        }
    }

    fn connection_toast<'a>(komga: &'a Komga) -> Element<'a, Message> {
        match &komga.connection_test {
            Some(Ok(())) => text("Server connection successful")
                .style(text::success)
                .into(),
            Some(Err(_)) => text("It wasn't possible to connect to the server")
                .style(text::danger)
                .into(),
            None => row![].into(),
        }
    }

    fn authentication_toast<'a>(komga: &'a Komga) -> Element<'a, Message> {
        match &komga.authentication_test {
            Some(Ok(())) => text("Authentication successful")
                .style(text::success)
                .into(),
            Some(Err(_)) => text("It wasn't possible to authenticate")
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
            integration_type: IntegrationType::Komga,
            url: Some("http://localhost:25600".to_string()),
            api_key: Some("api-key".to_string()),
            username: None,
            password: None,
        }
    }

    fn click_save(komga: &Komga) -> Vec<Message> {
        let mut ui = simulator(komga.view());
        let _ = ui.click("Save").expect("Save button should be found");

        ui.into_messages().collect()
    }

    #[test]
    fn view_renders_inputs_and_buttons() {
        let komga = Komga::default();

        let mut ui = simulator(komga.view());

        assert!(ui.find("Back").is_ok());
        assert!(ui.find("Save").is_ok());
        assert!(ui.find("Test Connection").is_ok());
        assert!(ui.find("Or sign in with your Komga account").is_ok());
    }

    #[test]
    fn update_stores_field_values() {
        let mut komga = Komga::default();

        let _ = komga.update(Message::NameChanged("My Komga".to_string()));
        let _ = komga.update(Message::ApiKeyChanged("key".to_string()));
        let _ = komga.update(Message::UsernameChanged("me@example.com".to_string()));
        let _ = komga.update(Message::PasswordChanged("secret".to_string()));

        assert_eq!(komga.name, "My Komga");
        assert_eq!(komga.api_key, "key");
        assert_eq!(komga.username, "me@example.com");
        assert_eq!(komga.password, "secret");
    }

    #[test]
    fn typing_an_invalid_url_flags_it_and_a_valid_one_clears_it() {
        let mut komga = Komga::default();

        let _ = komga.update(Message::UrlChanged("not a url".to_string()));
        assert!(komga.url_invalid);

        let _ = komga.update(Message::UrlChanged("http://localhost:25600".to_string()));
        assert!(!komga.url_invalid);
    }

    #[test]
    fn update_back_pressed_returns_back_pressed_action() {
        let mut komga = Komga::default();

        assert!(matches!(
            komga.update(Message::BackPressed),
            Action::BackPressed
        ));
    }

    #[test]
    fn update_submitted_runs_a_task() {
        let mut komga = Komga {
            name: "My Komga".to_string(),
            url: "http://localhost:25600".to_string(),
            api_key: "key".to_string(),
            ..Komga::default()
        };

        assert!(matches!(komga.update(Message::Submitted), Action::Run(_)));
    }

    #[test]
    fn update_created_ok_returns_created_action_and_err_does_nothing() {
        let mut komga = Komga::default();

        assert!(matches!(
            komga.update(Message::Created(Ok(integration_model("My Komga")))),
            Action::Created(_)
        ));
        assert!(matches!(
            komga.update(Message::Created(Err("boom".to_string()))),
            Action::None
        ));
    }

    #[test]
    fn clicking_test_connection_sends_test_connection_pressed_message() {
        let komga = Komga::default();

        let mut ui = simulator(komga.view());
        let _ = ui
            .click("Test Connection")
            .expect("Test Connection button should be found");

        let messages: Vec<_> = ui.into_messages().collect();

        assert_eq!(messages, vec![Message::TestConnectionPressed]);
    }

    #[test]
    fn test_connection_with_a_valid_url_clears_previous_results_and_runs_a_task() {
        let mut komga = Komga {
            url: "http://localhost:25600".to_string(),
            connection_test: Some(Ok(())),
            authentication_test: Some(Err("boom".to_string())),
            ..Komga::default()
        };

        let action = komga.update(Message::TestConnectionPressed);

        assert!(matches!(action, Action::Run(_)));
        assert_eq!(komga.connection_test, None);
        assert_eq!(komga.authentication_test, None);
    }

    #[test]
    fn test_connection_with_an_invalid_url_does_not_run_a_task() {
        let mut komga = Komga::default();
        let _ = komga.update(Message::UrlChanged("not a url".to_string()));

        assert!(matches!(
            komga.update(Message::TestConnectionPressed),
            Action::None
        ));
    }

    #[test]
    fn update_test_results_are_stored() {
        let mut komga = Komga::default();

        let _ = komga.update(Message::ConnectionTested(Ok(())));
        let _ = komga.update(Message::AuthenticationTested(Err("boom".to_string())));

        assert_eq!(komga.connection_test, Some(Ok(())));
        assert_eq!(komga.authentication_test, Some(Err("boom".to_string())));
    }

    #[test]
    fn view_renders_test_results() {
        let komga = Komga {
            connection_test: Some(Ok(())),
            authentication_test: Some(Err("unauthorized".to_string())),
            ..Komga::default()
        };

        let mut ui = simulator(komga.view());

        assert!(ui.find("Server connection successful").is_ok());
        assert!(ui.find("It wasn't possible to authenticate").is_ok());
    }

    #[test]
    fn save_is_enabled_without_errors_and_disabled_after_a_failed_test() {
        assert_eq!(click_save(&Komga::default()), vec![Message::Submitted]);

        let failed = Komga {
            authentication_test: Some(Err("boom".to_string())),
            ..Komga::default()
        };
        assert_eq!(click_save(&failed), Vec::<Message>::new());
    }

    #[test]
    fn save_is_disabled_when_the_url_is_invalid() {
        let mut komga = Komga::default();
        let _ = komga.update(Message::UrlChanged("not a url".to_string()));

        assert_eq!(click_save(&komga), Vec::<Message>::new());
    }
}
