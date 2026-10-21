// A build with no integration features only reads local folders, leaving the
// integration plumbing (sync job, repository writes, …) unused.
#![cfg_attr(
    not(any(
        feature = "kavita",
        feature = "komga",
        feature = "opds",
        feature = "suwayomi"
    )),
    allow(dead_code, unused)
)]

mod adapter;
mod cache;
pub mod database;
mod error;
mod icons;
mod jobs;
mod menu;
mod models;
mod providers;
mod reader_launch;
mod repository;
mod runtime;
mod schema;
mod screen;

use crate::{
    menu::{Menu, MenuItem},
    screen::{Home, browse, home, integration, library, series_detail, settings},
};
use iced::{
    Element,
    Length::{self, Fill},
    Subscription, Task, keyboard,
    widget::{Container, column, container, row},
};
use screen::Screen;

#[derive(Debug, Clone, PartialEq)]
enum Message {
    ExpandMenu,
    CollapseMenu,
    AnimateMenu(iced_anim::Event<f32>),
    TabPressed { shift: bool },
    OpenSettings,
    OpenHome,
    OpenLibrary,
    OpenIntegrations,
    Settings(settings::Message),
    Home(home::Message),
    Library(library::Message),
    Browse(browse::Message),
    SeriesDetail(series_detail::Message),
    Integrations(integration::Message),
}

struct KomasabiApp {
    screen: Screen,
    menu: Menu<Message>,
}

impl KomasabiApp {
    pub fn new() -> (Self, Task<Message>) {
        (
            Self {
                screen: Screen::Home(Home {}),
                menu: Menu::new(
                    vec![
                        MenuItem::new("bars", "", false, Some(Message::ExpandMenu)),
                        MenuItem::new("house", "Home", true, Some(Message::OpenHome)),
                        MenuItem::new("book_open", "Library", false, Some(Message::OpenLibrary)),
                        MenuItem::new("magnifying_glass", "Search", false, None),
                        MenuItem::new("download", "Downloads", false, None),
                        MenuItem::new(
                            "cloud",
                            "Integrations",
                            false,
                            Some(Message::OpenIntegrations),
                        ),
                    ],
                    vec![MenuItem::new(
                        "gear",
                        "Settings",
                        false,
                        Some(Message::OpenSettings),
                    )],
                ),
            },
            Task::none(),
        )
    }

    pub fn update(&mut self, message: Message) -> Task<Message> {
        match message {
            Message::Settings(msg) => {
                let Screen::Settings(settings) = &mut self.screen else {
                    return Task::none();
                };

                match settings.update(msg) {
                    settings::Action::None => Task::none(),
                    settings::Action::Run(task) => task.map(Message::Settings),
                }
            }

            Message::Home(msg) => {
                let Screen::Home(home) = &mut self.screen else {
                    return Task::none();
                };

                match home.update(msg) {}
            }

            Message::Library(msg) => {
                let Screen::Library(library) = &mut self.screen else {
                    return Task::none();
                };

                match library.update(msg) {
                    library::Action::None => Task::none(),
                    library::Action::Run(task) => task.map(Message::Library),
                    library::Action::BrowseRemoteLibrary(external_id) => {
                        self.open_browse_remote(external_id)
                    }
                    library::Action::BrowseLibrary(library_id) => {
                        self.open_browse_local(library_id)
                    }
                }
            }

            Message::Browse(msg) => {
                let Screen::Browse(browse) = &mut self.screen else {
                    return Task::none();
                };

                match browse.update(msg) {
                    browse::Action::None => Task::none(),
                    browse::Action::Run(task) => task.map(Message::Browse),
                    browse::Action::BackPressed => self.open_library(),
                    browse::Action::OpenSeries {
                        series_external_id,
                        series_name,
                        series_pages,
                        library_external_id,
                    } => self.open_series_detail(
                        series_external_id,
                        series_name,
                        series_pages,
                        library_external_id,
                    ),
                }
            }

            Message::SeriesDetail(msg) => {
                let Screen::SeriesDetail(series_detail) = &mut self.screen else {
                    return Task::none();
                };

                match series_detail.update(msg) {
                    series_detail::Action::None => Task::none(),
                    series_detail::Action::Run(task) => task.map(Message::SeriesDetail),
                    series_detail::Action::Back(library_external_id) => {
                        self.open_browse_remote(library_external_id)
                    }
                }
            }

            Message::Integrations(msg) => {
                let Screen::Integrations(integrations) = &mut self.screen else {
                    return Task::none();
                };

                match integrations.update(msg) {
                    integration::Action::None => Task::none(),
                    integration::Action::Run(task) => task.map(Message::Integrations),
                }
            }

            Message::ExpandMenu => {
                self.menu.set_collapsed(false);

                if let Some(toggle) = self.menu.top_items.first_mut() {
                    toggle.set_message(Some(Message::CollapseMenu));
                }

                Task::none()
            }

            Message::CollapseMenu => {
                self.menu.set_collapsed(true);

                if let Some(toggle) = self.menu.top_items.first_mut() {
                    toggle.set_message(Some(Message::ExpandMenu));
                }

                Task::none()
            }

            Message::AnimateMenu(event) => {
                self.menu.animate(event);
                Task::none()
            }

            Message::TabPressed { shift } => {
                if shift {
                    iced::widget::operation::focus_previous()
                } else {
                    iced::widget::operation::focus_next()
                }
            }

            Message::OpenSettings => self.open_settings(),
            Message::OpenHome => self.open_home(),
            Message::OpenLibrary => self.open_library(),
            Message::OpenIntegrations => self.open_integrations(),
        }
    }

    pub fn view(&self) -> Element<'_, Message> {
        let screen = match &self.screen {
            Screen::Home(home) => home.view().map(Message::Home),
            Screen::Library(library) => library.view().map(Message::Library),
            Screen::Browse(browse) => browse.view().map(Message::Browse),
            Screen::SeriesDetail(series_detail) => series_detail.view().map(Message::SeriesDetail),
            Screen::Settings(settings) => settings.view().map(Message::Settings),
            Screen::Integrations(integrations) => integrations.view().map(Message::Integrations),
        };

        container(
            container(
                row![
                    self.menu.view(Message::AnimateMenu),
                    column![topbar(), screen].height(Fill)
                ]
                .height(Fill),
            )
            .height(Fill),
        )
        .width(Length::Fill)
        .height(Length::Fill)
        .into()
    }

    pub fn open_settings(&mut self) -> Task<Message> {
        let (settings, task) = screen::Settings::new();
        self.screen = Screen::Settings(settings);
        self.menu.set_active(&Message::OpenSettings);
        task.map(Message::Settings)
    }

    fn open_home(&mut self) -> Task<Message> {
        let (params, task) = screen::Home::new();
        self.screen = Screen::Home(params);
        self.menu.set_active(&Message::OpenHome);

        task.map(Message::Home)
    }

    fn open_library(&mut self) -> Task<Message> {
        let (params, task) = screen::Library::new();
        self.screen = Screen::Library(params);
        self.menu.set_active(&Message::OpenLibrary);

        task.map(Message::Library)
    }

    fn open_browse_remote(&mut self, external_id: String) -> Task<Message> {
        let (params, task) = screen::Browse::new_remote(external_id);
        self.screen = Screen::Browse(params);

        task.map(Message::Browse)
    }

    fn open_browse_local(&mut self, library_id: String) -> Task<Message> {
        let (params, task) = screen::Browse::new_local(library_id);
        self.screen = Screen::Browse(params);

        task.map(Message::Browse)
    }

    fn open_series_detail(
        &mut self,
        series_external_id: String,
        series_name: String,
        series_pages: Option<u16>,
        library_external_id: String,
    ) -> Task<Message> {
        let (params, task) = screen::SeriesDetail::new(
            series_external_id,
            series_name,
            series_pages,
            library_external_id,
        );
        self.screen = Screen::SeriesDetail(params);

        task.map(Message::SeriesDetail)
    }

    fn open_integrations(&mut self) -> Task<Message> {
        let (params, task) = screen::Integrations::new();
        self.screen = Screen::Integrations(params);
        self.menu.set_active(&Message::OpenIntegrations);

        task.map(Message::Integrations)
    }

    pub fn subscription(&self) -> Subscription<Message> {
        keyboard::listen().filter_map(tab_pressed_message)
    }
}

fn tab_pressed_message(event: keyboard::Event) -> Option<Message> {
    match event {
        keyboard::Event::KeyPressed {
            key: keyboard::Key::Named(keyboard::key::Named::Tab),
            modifiers,
            ..
        } => Some(Message::TabPressed {
            shift: modifiers.shift(),
        }),
        _ => None,
    }
}

fn topbar<'a>() -> Container<'a, Message> {
    container(column![])
}

fn main() -> iced::Result {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| {
                tracing_subscriber::EnvFilter::new("komasabi_app=debug,kavita_client=debug")
            }),
        )
        .init();

    iced::application(KomasabiApp::new, KomasabiApp::update, KomasabiApp::view)
        .subscription(KomasabiApp::subscription)
        .window(iced::window::Settings {
            icon: iced::window::icon::from_file_data(icons::APP_ICON, None).ok(),
            ..Default::default()
        })
        .run()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn app() -> KomasabiApp {
        let (app, _task) = KomasabiApp::new();
        app
    }

    #[test]
    fn menu_starts_collapsed_with_an_expand_toggle() {
        let app = app();

        assert!(app.menu.is_collapsed());
        assert!(!app.menu.is_animating());
        assert_eq!(app.menu.top_items[0].message(), Some(&Message::ExpandMenu));
    }

    #[test]
    fn expanding_the_menu_starts_the_transition_and_flips_the_toggle_to_collapse() {
        let mut app = app();
        let collapsed_width = app.menu.width();

        let _ = app.update(Message::ExpandMenu);

        assert!(!app.menu.is_collapsed());
        assert!(app.menu.is_animating());
        // The width only grows as the animation ticks, not on the click itself.
        assert_eq!(app.menu.width(), collapsed_width);
        assert_eq!(
            app.menu.top_items[0].message(),
            Some(&Message::CollapseMenu)
        );
    }

    #[test]
    fn collapsing_the_menu_starts_the_transition_and_flips_the_toggle_back_to_expand() {
        let mut app = app();

        let _ = app.update(Message::ExpandMenu);
        let _ = app.update(Message::AnimateMenu(iced_anim::Event::Settle));
        let expanded_width = app.menu.width();

        let _ = app.update(Message::CollapseMenu);
        let _ = app.update(Message::AnimateMenu(iced_anim::Event::Settle));

        assert!(app.menu.is_collapsed());
        assert!(app.menu.width() < expanded_width);
        assert_eq!(app.menu.top_items[0].message(), Some(&Message::ExpandMenu));
    }

    #[test]
    fn animate_menu_events_drive_the_width_to_the_expanded_target() {
        let mut app = app();
        let collapsed_width = app.menu.width();

        let _ = app.update(Message::ExpandMenu);
        let _ = app.update(Message::AnimateMenu(iced_anim::Event::Settle));

        assert!(app.menu.width() > collapsed_width);
        assert!(!app.menu.is_animating());
    }

    fn tab_key_press(modifiers: keyboard::Modifiers) -> keyboard::Event {
        keyboard::Event::KeyPressed {
            key: keyboard::Key::Named(keyboard::key::Named::Tab),
            modified_key: keyboard::Key::Named(keyboard::key::Named::Tab),
            physical_key: keyboard::key::Physical::Code(keyboard::key::Code::Tab),
            location: keyboard::Location::Standard,
            modifiers,
            text: None,
            repeat: false,
        }
    }

    #[test]
    fn tab_key_press_sends_tab_pressed_without_shift() {
        let message = tab_pressed_message(tab_key_press(keyboard::Modifiers::empty()));

        assert_eq!(message, Some(Message::TabPressed { shift: false }));
    }

    #[test]
    fn shift_tab_key_press_sends_tab_pressed_with_shift() {
        let message = tab_pressed_message(tab_key_press(keyboard::Modifiers::SHIFT));

        assert_eq!(message, Some(Message::TabPressed { shift: true }));
    }

    #[test]
    fn other_key_presses_are_ignored() {
        let event = keyboard::Event::KeyPressed {
            key: keyboard::Key::Named(keyboard::key::Named::Enter),
            modified_key: keyboard::Key::Named(keyboard::key::Named::Enter),
            physical_key: keyboard::key::Physical::Code(keyboard::key::Code::Enter),
            location: keyboard::Location::Standard,
            modifiers: keyboard::Modifiers::empty(),
            text: None,
            repeat: false,
        };

        assert_eq!(tab_pressed_message(event), None);
    }
}
