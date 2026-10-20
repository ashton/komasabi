use iced::{Element, Task};

use crate::models::integration::{Integration as IntegrationModel, IntegrationType};
use crate::providers::opds::provider::OpdsProvider;
use crate::repository::integration as integration_repository;

#[derive(Debug, Default)]
pub struct Opds {
    name: String,
    url: String,
    username: String,
    password: String,
    /// The catalog's title on success, from the last "Test Connection".
    catalog_test: Option<Result<String, String>>,
    url_invalid: bool,
}

fn is_valid_catalog_url(value: &str) -> bool {
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
    CatalogTested(Result<String, String>),
}

#[derive(Debug)]
pub enum Action {
    None,
    Run(Task<Message>),
    BackPressed,
    Created(IntegrationModel),
}

impl Opds {
    pub fn update(&mut self, message: Message) -> Action {
        match message {
            Message::NameChanged(name) => {
                self.name = name;
                Action::None
            }

            Message::UrlChanged(url) => {
                self.url = url;
                self.url_invalid = !is_valid_catalog_url(&self.url);
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
                    IntegrationType::Opds,
                    self.url.trim().to_string(),
                    optional(&self.username),
                    optional(&self.password),
                ),
                Message::Created,
            )),

            Message::Created(Ok(integration)) => Action::Created(integration),
            Message::Created(Err(_)) => Action::None,

            Message::TestConnectionPressed => {
                self.url_invalid = !is_valid_catalog_url(&self.url);
                self.catalog_test = None;

                if self.url_invalid {
                    Action::None
                } else {
                    Action::Run(Task::perform(
                        crate::runtime::on_tokio(OpdsProvider::check_catalog(
                            self.url.trim().to_string(),
                            optional(&self.username),
                            optional(&self.password),
                        )),
                        Message::CatalogTested,
                    ))
                }
            }

            Message::CatalogTested(result) => {
                self.catalog_test = Some(result);
                Action::None
            }
        }
    }

    pub fn view<'a>(&'a self) -> Element<'a, Message> {
        view_helper::form(self)
    }

    fn has_errors(&self) -> bool {
        self.url_invalid || matches!(self.catalog_test, Some(Err(_)))
    }
}

mod view_helper {
    use super::{Message, Opds};
    use iced::{
        Center, Element, Theme,
        widget::{button, column, container, row, text, text_input},
    };

    pub fn form<'a>(opds: &'a Opds) -> Element<'a, Message> {
        container(
            column![
                text_input("Name", &opds.name).on_input(Message::NameChanged),
                row![
                    text_input("Catalog URL", &opds.url)
                        .on_input(Message::UrlChanged)
                        .style(move |theme: &Theme, status| url_input_style(
                            theme,
                            status,
                            opds.url_invalid
                        )),
                    url_error_toast(opds),
                ]
                .spacing(10)
                .align_y(Center),
                text_input("Username (optional)", &opds.username)
                    .on_input(Message::UsernameChanged),
                text_input("Password (optional)", &opds.password)
                    .secure(true)
                    .on_input(Message::PasswordChanged),
                row![
                    button("Test Connection").on_press(Message::TestConnectionPressed),
                    catalog_toast(opds),
                ]
                .spacing(10)
                .align_y(Center),
                row![
                    button("Back").on_press(Message::BackPressed),
                    button("Save")
                        .on_press_maybe((!opds.has_errors()).then_some(Message::Submitted))
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

    fn url_error_toast<'a>(opds: &'a Opds) -> Element<'a, Message> {
        if opds.url_invalid {
            text("The text must be a valid URL")
                .style(text::danger)
                .into()
        } else {
            row![].into()
        }
    }

    fn catalog_toast<'a>(opds: &'a Opds) -> Element<'a, Message> {
        match &opds.catalog_test {
            Some(Ok(title)) if title.is_empty() => {
                text("Catalog found").style(text::success).into()
            }
            Some(Ok(title)) => text(format!("Catalog found: {title}"))
                .style(text::success)
                .into(),
            Some(Err(_)) => text("It wasn't possible to read an OPDS catalog at this URL")
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
            integration_type: IntegrationType::Opds,
            url: Some("http://localhost:5000/opds".to_string()),
            api_key: None,
            username: None,
            password: None,
        }
    }

    fn click_save(opds: &Opds) -> Vec<Message> {
        let mut ui = simulator(opds.view());
        let _ = ui.click("Save").expect("Save button should be found");

        ui.into_messages().collect()
    }

    #[test]
    fn view_renders_inputs_and_buttons() {
        let opds = Opds::default();

        let mut ui = simulator(opds.view());

        assert!(ui.find("Back").is_ok());
        assert!(ui.find("Save").is_ok());
        assert!(ui.find("Test Connection").is_ok());
    }

    #[test]
    fn update_stores_field_values() {
        let mut opds = Opds::default();

        let _ = opds.update(Message::NameChanged("Library".to_string()));
        let _ = opds.update(Message::UsernameChanged("reader".to_string()));
        let _ = opds.update(Message::PasswordChanged("secret".to_string()));

        assert_eq!(opds.name, "Library");
        assert_eq!(opds.username, "reader");
        assert_eq!(opds.password, "secret");
    }

    #[test]
    fn typing_an_invalid_url_flags_it_and_a_valid_one_clears_it() {
        let mut opds = Opds::default();

        let _ = opds.update(Message::UrlChanged("not a url".to_string()));
        assert!(opds.url_invalid);

        let _ = opds.update(Message::UrlChanged(
            "https://opds.example/catalog".to_string(),
        ));
        assert!(!opds.url_invalid);
    }

    #[test]
    fn update_back_pressed_returns_back_pressed_action() {
        let mut opds = Opds::default();

        assert!(matches!(
            opds.update(Message::BackPressed),
            Action::BackPressed
        ));
    }

    #[test]
    fn update_submitted_runs_a_task() {
        let mut opds = Opds {
            name: "Library".to_string(),
            url: "http://localhost:5000/opds".to_string(),
            ..Opds::default()
        };

        assert!(matches!(opds.update(Message::Submitted), Action::Run(_)));
    }

    #[test]
    fn update_created_ok_returns_created_action_and_err_does_nothing() {
        let mut opds = Opds::default();

        assert!(matches!(
            opds.update(Message::Created(Ok(integration_model("Library")))),
            Action::Created(_)
        ));
        assert!(matches!(
            opds.update(Message::Created(Err("boom".to_string()))),
            Action::None
        ));
    }

    #[test]
    fn clicking_test_connection_sends_test_connection_pressed_message() {
        let opds = Opds::default();

        let mut ui = simulator(opds.view());
        let _ = ui
            .click("Test Connection")
            .expect("Test Connection button should be found");

        let messages: Vec<_> = ui.into_messages().collect();

        assert_eq!(messages, vec![Message::TestConnectionPressed]);
    }

    #[test]
    fn test_connection_with_a_valid_url_clears_the_previous_result_and_runs_a_task() {
        let mut opds = Opds {
            url: "http://localhost:5000/opds".to_string(),
            catalog_test: Some(Err("boom".to_string())),
            ..Opds::default()
        };

        let action = opds.update(Message::TestConnectionPressed);

        assert!(matches!(action, Action::Run(_)));
        assert_eq!(opds.catalog_test, None);
    }

    #[test]
    fn test_connection_with_an_invalid_url_does_not_run_a_task() {
        let mut opds = Opds::default();
        let _ = opds.update(Message::UrlChanged("not a url".to_string()));

        assert!(matches!(
            opds.update(Message::TestConnectionPressed),
            Action::None
        ));
    }

    #[test]
    fn view_renders_the_catalog_title_after_a_successful_test() {
        let opds = Opds {
            catalog_test: Some(Ok("Kavita".to_string())),
            ..Opds::default()
        };

        let mut ui = simulator(opds.view());

        assert!(ui.find("Catalog found: Kavita").is_ok());
    }

    #[test]
    fn view_renders_an_error_after_a_failed_test() {
        let opds = Opds {
            catalog_test: Some(Err("not an OPDS document".to_string())),
            ..Opds::default()
        };

        let mut ui = simulator(opds.view());

        assert!(
            ui.find("It wasn't possible to read an OPDS catalog at this URL")
                .is_ok()
        );
    }

    #[test]
    fn save_is_enabled_without_errors_and_disabled_after_a_failed_test() {
        assert_eq!(click_save(&Opds::default()), vec![Message::Submitted]);

        let failed = Opds {
            catalog_test: Some(Err("boom".to_string())),
            ..Opds::default()
        };
        assert_eq!(click_save(&failed), Vec::<Message>::new());
    }

    #[test]
    fn save_is_disabled_when_the_url_is_invalid() {
        let mut opds = Opds::default();
        let _ = opds.update(Message::UrlChanged("not a url".to_string()));

        assert_eq!(click_save(&opds), Vec::<Message>::new());
    }

    #[test]
    fn optional_treats_blank_values_as_absent() {
        assert_eq!(optional("  "), None);
        assert_eq!(optional(" reader "), Some("reader".to_string()));
    }
}
