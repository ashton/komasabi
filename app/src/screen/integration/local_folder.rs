//! The form for adding a local folder as a source: a name and the folder to
//! read from. Each folder is a source of its own.

use std::path::PathBuf;

use iced::{Element, Task};

use crate::models::integration::Integration as IntegrationModel;
use crate::repository::integration as integration_repository;

#[derive(Debug, Default)]
pub struct LocalFolder {
    name: String,
    folder: Option<PathBuf>,
    /// The last save failure, shown under the form.
    error: Option<String>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Message {
    NameChanged(String),
    ChooseFolderPressed,
    FolderPicked(Option<PathBuf>),
    BackPressed,
    Submitted,
    Created(Result<IntegrationModel, String>),
}

#[derive(Debug)]
pub enum Action {
    None,
    Run(Task<Message>),
    BackPressed,
    /// The source was saved.
    Created,
}

impl LocalFolder {
    pub fn update(&mut self, message: Message) -> Action {
        match message {
            Message::NameChanged(name) => {
                self.name = name;
                Action::None
            }

            Message::ChooseFolderPressed => {
                Action::Run(Task::perform(pick_folder(), Message::FolderPicked))
            }

            Message::FolderPicked(None) => Action::None,
            Message::FolderPicked(Some(folder)) => {
                // Default the (still-empty) name to the folder's, so picking a
                // folder is all it takes.
                if self.name.trim().is_empty() {
                    self.name = folder_name(&folder);
                }
                self.folder = Some(folder);
                Action::None
            }

            Message::BackPressed => Action::BackPressed,

            Message::Submitted => {
                let Some(folder) = self.folder.as_ref().filter(|_| self.can_submit()) else {
                    return Action::None;
                };

                self.error = None;
                Action::Run(Task::perform(
                    integration_repository::create_local_folder(
                        self.name.trim().to_string(),
                        folder.to_string_lossy().into_owned(),
                    ),
                    Message::Created,
                ))
            }

            Message::Created(Ok(_)) => Action::Created,
            Message::Created(Err(error)) => {
                self.error = Some(error);
                Action::None
            }
        }
    }

    pub fn view(&self) -> Element<'_, Message> {
        view_helper::form(self)
    }

    /// Whether the form has a name and a folder.
    fn can_submit(&self) -> bool {
        !self.name.trim().is_empty() && self.folder.is_some()
    }
}

/// The folder's own name, falling back to the full path (e.g. for `/`).
fn folder_name(path: &std::path::Path) -> String {
    path.file_name()
        .map(|name| name.to_string_lossy().into_owned())
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
        Background, Border, Center, Element,
        Length::Fill,
        Theme,
        widget::{button, column, container, row, text, text_input},
    };

    use super::{LocalFolder, Message};

    pub fn form<'a>(form: &'a LocalFolder) -> Element<'a, Message> {
        let mut content = column![
            text_input("Name", &form.name).on_input(Message::NameChanged),
            folder_field(form),
            row![
                button("Back").on_press(Message::BackPressed),
                button("Save").on_press_maybe(form.can_submit().then_some(Message::Submitted)),
            ]
            .spacing(10),
        ]
        .spacing(10);

        if let Some(error) = &form.error {
            content = content.push(text(error).style(text::danger));
        }

        container(content).padding(20).into()
    }

    /// A read-only display of the chosen folder next to the button that opens
    /// the system folder picker.
    fn folder_field<'a>(form: &'a LocalFolder) -> Element<'a, Message> {
        let chosen = form.folder.is_some();
        let label = match &form.folder {
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
                    text_color: Some(if chosen {
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
            button("Choose folder…").on_press(Message::ChooseFolderPressed),
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

    fn click_save(form: &LocalFolder) -> Vec<Message> {
        let mut ui = simulator(form.view());
        let _ = ui.click("Save").expect("Save should be found");
        ui.into_messages().collect()
    }

    #[test]
    fn view_renders_the_fields_and_buttons() {
        let form = LocalFolder::default();

        let mut ui = simulator(form.view());

        assert!(ui.find("No folder selected").is_ok());
        assert!(ui.find("Choose folder…").is_ok());
        assert!(ui.find("Back").is_ok());
        assert!(ui.find("Save").is_ok());
    }

    #[test]
    fn choose_folder_runs_the_picker() {
        let mut form = LocalFolder::default();

        assert!(matches!(
            form.update(Message::ChooseFolderPressed),
            Action::Run(_)
        ));
    }

    #[test]
    fn picking_a_folder_stores_it_and_defaults_the_name() {
        let mut form = LocalFolder::default();

        form.update(Message::FolderPicked(Some(PathBuf::from("/comics/Manga"))));

        assert_eq!(form.folder, Some(PathBuf::from("/comics/Manga")));
        assert_eq!(form.name, "Manga");
    }

    #[test]
    fn picking_a_folder_keeps_a_name_already_typed() {
        let mut form = LocalFolder::default();
        form.update(Message::NameChanged("Shelf".to_string()));

        form.update(Message::FolderPicked(Some(PathBuf::from("/comics/Manga"))));

        assert_eq!(form.name, "Shelf");
    }

    #[test]
    fn cancelling_the_picker_changes_nothing() {
        let mut form = LocalFolder::default();

        form.update(Message::FolderPicked(None));

        assert!(form.folder.is_none());
        assert!(form.name.is_empty());
    }

    #[test]
    fn save_needs_a_name_and_a_folder() {
        assert!(click_save(&LocalFolder::default()).is_empty());

        let mut form = LocalFolder::default();
        form.update(Message::FolderPicked(Some(PathBuf::from("/comics"))));
        assert_eq!(click_save(&form), vec![Message::Submitted]);

        form.update(Message::NameChanged("  ".to_string()));
        assert!(click_save(&form).is_empty());
        assert!(matches!(form.update(Message::Submitted), Action::None));
    }

    #[test]
    fn submitting_a_complete_form_saves_the_source() {
        let mut form = LocalFolder::default();
        form.update(Message::FolderPicked(Some(PathBuf::from("/comics"))));

        assert!(matches!(form.update(Message::Submitted), Action::Run(_)));
    }

    #[test]
    fn a_failed_save_is_shown() {
        let mut form = LocalFolder::default();

        let action = form.update(Message::Created(Err("disk full".to_string())));

        assert!(matches!(action, Action::None));
        let mut ui = simulator(form.view());
        assert!(ui.find("disk full").is_ok());
    }

    #[test]
    fn back_pressed_returns_back() {
        let mut form = LocalFolder::default();

        assert!(matches!(
            form.update(Message::BackPressed),
            Action::BackPressed
        ));
    }
}
