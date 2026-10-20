use iced::{Element, Task};

use crate::models::integration::{Integration as IntegrationModel, IntegrationType};
use crate::providers::kavita::provider::KavitaProvider;
use crate::repository::integration as integration_repository;

#[derive(Debug, Default)]
pub struct Kavita {
    name: String,
    url: String,
    api_key: String,
    connection_test: Option<Result<(), String>>,
    authentication_test: Option<Result<(), String>>,
    url_invalid: bool,
}

fn is_valid_server_url(value: &str) -> bool {
    url::Url::parse(value)
        .is_ok_and(|url| matches!(url.scheme(), "http" | "https") && url.host().is_some())
}

#[derive(Clone, Debug, PartialEq)]
pub enum Message {
    NameChanged(String),
    UrlChanged(String),
    ApiKeyChanged(String),
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

impl Kavita {
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

            Message::BackPressed => Action::BackPressed,

            Message::Submitted => Action::Run(Task::perform(
                integration_repository::create(
                    self.name.clone(),
                    IntegrationType::Kavita,
                    Some(self.url.clone()),
                    Some(self.api_key.clone()),
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
                    let connection_task = Task::perform(
                        crate::runtime::on_tokio(KavitaProvider::ping(self.url.clone())),
                        Message::ConnectionTested,
                    );

                    let authentication_task = Task::perform(
                        crate::runtime::on_tokio(KavitaProvider::check_authentication(
                            self.url.clone(),
                            self.api_key.clone(),
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
    use super::{Kavita, Message};
    use iced::{
        Center, Element, Theme,
        widget::{button, column, container, row, text, text_input},
    };

    pub fn form<'a>(kavita: &'a Kavita) -> Element<'a, Message> {
        container(
            column![
                text_input("Name", &kavita.name).on_input(Message::NameChanged),
                row![
                    text_input("Server URL", &kavita.url)
                        .on_input(Message::UrlChanged)
                        .style(move |theme: &Theme, status| url_input_style(
                            theme,
                            status,
                            kavita.url_invalid
                        )),
                    url_error_toast(kavita),
                ]
                .spacing(10)
                .align_y(Center),
                text_input("API Key", &kavita.api_key).on_input(Message::ApiKeyChanged),
                row![
                    button("Test Connection").on_press(Message::TestConnectionPressed),
                    connection_toast(kavita),
                    authentication_toast(kavita),
                ]
                .spacing(10)
                .align_y(Center),
                row![
                    button("Back").on_press(Message::BackPressed),
                    button("Save")
                        .on_press_maybe((!kavita.has_errors()).then_some(Message::Submitted))
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

    fn url_error_toast<'a>(kavita: &'a Kavita) -> Element<'a, Message> {
        if kavita.url_invalid {
            text("The text must be a valid URL")
                .style(text::danger)
                .into()
        } else {
            row![].into()
        }
    }

    fn connection_toast<'a>(kavita: &'a Kavita) -> Element<'a, Message> {
        match &kavita.connection_test {
            Some(Ok(())) => text("Server connection successful")
                .style(text::success)
                .into(),
            Some(Err(_)) => text("It wasn't possible to connect to the server")
                .style(text::danger)
                .into(),
            None => row![].into(),
        }
    }

    fn authentication_toast<'a>(kavita: &'a Kavita) -> Element<'a, Message> {
        match &kavita.authentication_test {
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
            integration_type: IntegrationType::Kavita,
            url: Some("http://localhost:5000".to_string()),
            api_key: Some("api-key".to_string()),
            username: None,
            password: None,
        }
    }

    #[test]
    fn view_renders_inputs_and_buttons() {
        let kavita = Kavita::default();

        let mut ui = simulator(kavita.view());

        assert!(ui.find("Back").is_ok());
        assert!(ui.find("Save").is_ok());
    }

    #[test]
    fn update_name_changed_stores_value() {
        let mut kavita = Kavita::default();

        let _ = kavita.update(Message::NameChanged("My Kavita".to_string()));

        assert_eq!(kavita.name, "My Kavita");
    }

    #[test]
    fn update_back_pressed_returns_back_pressed_action() {
        let mut kavita = Kavita::default();

        let action = kavita.update(Message::BackPressed);

        assert!(matches!(action, Action::BackPressed));
    }

    #[test]
    fn update_submitted_runs_a_task() {
        let mut kavita = Kavita {
            name: "My Kavita".to_string(),
            url: "http://localhost:5000".to_string(),
            api_key: "api-key".to_string(),
            connection_test: None,
            authentication_test: None,
            url_invalid: false,
        };

        let action = kavita.update(Message::Submitted);

        assert!(matches!(action, Action::Run(_)));
    }

    #[test]
    fn update_created_ok_returns_created_action() {
        let mut kavita = Kavita::default();

        let action = kavita.update(Message::Created(Ok(integration_model("My Kavita"))));

        assert!(matches!(action, Action::Created(_)));
    }

    #[test]
    fn update_created_err_returns_no_action() {
        let mut kavita = Kavita::default();

        let action = kavita.update(Message::Created(Err("boom".to_string())));

        assert!(matches!(action, Action::None));
    }

    #[test]
    fn view_renders_test_connection_button() {
        let kavita = Kavita::default();

        let mut ui = simulator(kavita.view());

        assert!(ui.find("Test Connection").is_ok());
    }

    #[test]
    fn clicking_test_connection_sends_test_connection_pressed_message() {
        let kavita = Kavita::default();

        let mut ui = simulator(kavita.view());
        let _ = ui
            .click("Test Connection")
            .expect("Test Connection button should be found");

        let messages: Vec<_> = ui.into_messages().collect();

        assert_eq!(messages, vec![Message::TestConnectionPressed]);
    }

    #[test]
    fn update_test_connection_pressed_clears_previous_results_and_runs_a_task() {
        let mut kavita = Kavita {
            url: "http://localhost:5000".to_string(),
            connection_test: Some(Ok(())),
            authentication_test: Some(Ok(())),
            ..Kavita::default()
        };

        let action = kavita.update(Message::TestConnectionPressed);

        assert!(matches!(action, Action::Run(_)));
        assert_eq!(kavita.connection_test, None);
        assert_eq!(kavita.authentication_test, None);
    }

    #[test]
    fn update_test_connection_pressed_without_api_key_still_checks_authentication() {
        let mut kavita = Kavita {
            url: "http://localhost:5000".to_string(),
            authentication_test: Some(Ok(())),
            ..Kavita::default()
        };

        assert_eq!(kavita.api_key, "");

        let action = kavita.update(Message::TestConnectionPressed);

        assert!(matches!(action, Action::Run(_)));
        assert_eq!(kavita.authentication_test, None);
    }

    #[test]
    fn update_connection_tested_ok_stores_success() {
        let mut kavita = Kavita::default();

        let action = kavita.update(Message::ConnectionTested(Ok(())));

        assert!(matches!(action, Action::None));
        assert_eq!(kavita.connection_test, Some(Ok(())));
    }

    #[test]
    fn update_connection_tested_err_stores_error() {
        let mut kavita = Kavita::default();

        let action = kavita.update(Message::ConnectionTested(Err("boom".to_string())));

        assert!(matches!(action, Action::None));
        assert_eq!(kavita.connection_test, Some(Err("boom".to_string())));
    }

    #[test]
    fn update_authentication_tested_ok_stores_success() {
        let mut kavita = Kavita::default();

        let action = kavita.update(Message::AuthenticationTested(Ok(())));

        assert!(matches!(action, Action::None));
        assert_eq!(kavita.authentication_test, Some(Ok(())));
    }

    #[test]
    fn update_authentication_tested_err_stores_error() {
        let mut kavita = Kavita::default();

        let action = kavita.update(Message::AuthenticationTested(Err("boom".to_string())));

        assert!(matches!(action, Action::None));
        assert_eq!(kavita.authentication_test, Some(Err("boom".to_string())));
    }

    #[test]
    fn view_renders_success_message_after_successful_connection_test() {
        let kavita = Kavita {
            connection_test: Some(Ok(())),
            ..Kavita::default()
        };

        let mut ui = simulator(kavita.view());

        assert!(ui.find("Server connection successful").is_ok());
    }

    #[test]
    fn view_renders_generic_error_message_after_failed_connection_test() {
        let kavita = Kavita {
            connection_test: Some(Err("connection refused".to_string())),
            ..Kavita::default()
        };

        let mut ui = simulator(kavita.view());

        assert!(
            ui.find("It wasn't possible to connect to the server")
                .is_ok()
        );
    }

    #[test]
    fn view_renders_success_message_after_successful_authentication_test() {
        let kavita = Kavita {
            authentication_test: Some(Ok(())),
            ..Kavita::default()
        };

        let mut ui = simulator(kavita.view());

        assert!(ui.find("Authentication successful").is_ok());
    }

    #[test]
    fn view_renders_generic_error_message_after_failed_authentication_test() {
        let kavita = Kavita {
            authentication_test: Some(Err("unauthorized".to_string())),
            ..Kavita::default()
        };

        let mut ui = simulator(kavita.view());

        assert!(ui.find("It wasn't possible to authenticate").is_ok());
    }

    #[test]
    fn view_renders_both_toasts_at_once() {
        let kavita = Kavita {
            connection_test: Some(Ok(())),
            authentication_test: Some(Err("unauthorized".to_string())),
            ..Kavita::default()
        };

        let mut ui = simulator(kavita.view());

        assert!(ui.find("Server connection successful").is_ok());
        assert!(ui.find("It wasn't possible to authenticate").is_ok());
    }

    #[test]
    fn is_valid_server_url_accepts_http_and_https_urls_with_a_host() {
        assert!(is_valid_server_url("http://localhost:5000"));
        assert!(is_valid_server_url("https://kavita.example.com"));
    }

    #[test]
    fn is_valid_server_url_rejects_malformed_or_hostless_values() {
        assert!(!is_valid_server_url(""));
        assert!(!is_valid_server_url("not a url"));
        assert!(!is_valid_server_url("file:///etc/passwd"));
    }

    #[test]
    fn typing_an_invalid_url_flags_it_immediately() {
        let mut kavita = Kavita::default();

        let _ = kavita.update(Message::UrlChanged("not a url".to_string()));

        assert!(kavita.url_invalid);
    }

    #[test]
    fn typing_a_valid_url_clears_the_flag() {
        let mut kavita = Kavita::default();
        let _ = kavita.update(Message::UrlChanged("not a url".to_string()));

        let _ = kavita.update(Message::UrlChanged("http://localhost:5000".to_string()));

        assert!(!kavita.url_invalid);
    }

    #[test]
    fn view_renders_error_toast_and_red_border_for_invalid_url() {
        let mut kavita = Kavita::default();
        let _ = kavita.update(Message::UrlChanged("not a url".to_string()));

        let mut ui = simulator(kavita.view());

        assert!(ui.find("The text must be a valid URL").is_ok());
    }

    #[test]
    fn view_does_not_render_error_toast_for_untouched_url_field() {
        let kavita = Kavita::default();

        let mut ui = simulator(kavita.view());

        assert!(ui.find("The text must be a valid URL").is_err());
    }

    #[test]
    fn test_connection_pressed_with_invalid_url_does_not_run_a_task() {
        let mut kavita = Kavita::default();
        let _ = kavita.update(Message::UrlChanged("not a url".to_string()));

        let action = kavita.update(Message::TestConnectionPressed);

        assert!(matches!(action, Action::None));
        assert!(kavita.url_invalid);
    }

    #[test]
    fn view_renders_error_toast_after_pressing_test_connection_with_invalid_url() {
        let mut kavita = Kavita::default();
        let _ = kavita.update(Message::UrlChanged("not a url".to_string()));
        let _ = kavita.update(Message::TestConnectionPressed);

        let mut ui = simulator(kavita.view());

        assert!(ui.find("The text must be a valid URL").is_ok());
    }

    #[test]
    fn test_connection_pressed_with_valid_url_runs_a_task() {
        let mut kavita = Kavita::default();
        let _ = kavita.update(Message::UrlChanged("http://localhost:5000".to_string()));

        let action = kavita.update(Message::TestConnectionPressed);

        assert!(matches!(action, Action::Run(_)));
        assert!(!kavita.url_invalid);
    }

    fn click_save(kavita: &Kavita) -> Vec<Message> {
        let mut ui = simulator(kavita.view());
        let _ = ui.click("Save").expect("Save button should be found");

        ui.into_messages().collect()
    }

    #[test]
    fn save_is_enabled_when_there_are_no_errors() {
        let kavita = Kavita::default();

        assert_eq!(click_save(&kavita), vec![Message::Submitted]);
    }

    #[test]
    fn save_is_enabled_when_tests_passed() {
        let kavita = Kavita {
            connection_test: Some(Ok(())),
            authentication_test: Some(Ok(())),
            ..Kavita::default()
        };

        assert_eq!(click_save(&kavita), vec![Message::Submitted]);
    }

    #[test]
    fn save_is_disabled_when_the_url_is_invalid() {
        let mut kavita = Kavita::default();
        let _ = kavita.update(Message::UrlChanged("not a url".to_string()));

        assert_eq!(click_save(&kavita), Vec::<Message>::new());
    }

    #[test]
    fn save_is_disabled_when_the_connection_test_failed() {
        let kavita = Kavita {
            connection_test: Some(Err("boom".to_string())),
            ..Kavita::default()
        };

        assert_eq!(click_save(&kavita), Vec::<Message>::new());
    }

    #[test]
    fn save_is_disabled_when_the_authentication_test_failed() {
        let kavita = Kavita {
            authentication_test: Some(Err("boom".to_string())),
            ..Kavita::default()
        };

        assert_eq!(click_save(&kavita), Vec::<Message>::new());
    }
}
