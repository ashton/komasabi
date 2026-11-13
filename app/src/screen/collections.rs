//! A source's collections: everything in it that gathers series (libraries,
//! collections, reading lists, categories, feeds, a local folder), grouped by
//! kind. Opening one browses its series.
//!
//! The list is fetched live from the source's provider every time, and only
//! the provider-agnostic [`Collection`] model reaches this screen.

use iced::{Element, Task};

use crate::models::{
    AsyncModel,
    collection::{Collection, CollectionRef},
    integration::Integration,
};
use crate::providers::{Provider, provider_for_integration};
use crate::repository::integration as integration_repository;

#[derive(Debug)]
pub struct Collections {
    integration_id: String,
    /// The source, for its name in the header, once loaded.
    source: Option<Integration>,
    collections: AsyncModel<Vec<Collection>, String>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Message {
    SourceLoaded(Result<Integration, String>),
    CollectionsLoaded(Result<Vec<Collection>, String>),
    /// A collection was clicked; carries its index in the loaded list.
    CollectionSelected(usize),
    RetryPressed,
    BackPressed,
}

#[derive(Debug)]
pub enum Action {
    None,
    Run(Task<Message>),
    /// Open the collection's series.
    OpenCollection(CollectionRef),
    /// Back to the list of sources.
    Back,
}

impl Collections {
    pub fn new(integration_id: String) -> (Self, Task<Message>) {
        let id = integration_id.clone();
        let task = Task::batch([
            Task::perform(
                async move { integration_repository::find(&id).await },
                Message::SourceLoaded,
            ),
            load(integration_id.clone()),
        ]);

        (
            Self {
                integration_id,
                source: None,
                collections: AsyncModel::Loading,
            },
            task,
        )
    }

    pub fn update(&mut self, message: Message) -> Action {
        match message {
            Message::SourceLoaded(result) => {
                self.source = result.ok();
                Action::None
            }

            Message::CollectionsLoaded(result) => {
                self.collections = result.map(sorted).into();
                Action::None
            }

            Message::CollectionSelected(index) => {
                let AsyncModel::Loaded(collections) = &self.collections else {
                    return Action::None;
                };
                let Some(collection) = collections.get(index) else {
                    return Action::None;
                };

                Action::OpenCollection(CollectionRef {
                    integration_id: self.integration_id.clone(),
                    collection: collection.clone(),
                })
            }

            Message::RetryPressed => {
                self.collections = AsyncModel::Loading;
                Action::Run(load(self.integration_id.clone()))
            }

            Message::BackPressed => Action::Back,
        }
    }

    pub fn view(&self) -> Element<'_, Message> {
        let title = self
            .source
            .as_ref()
            .map(|source| source.name.as_str())
            .unwrap_or_default();

        view_helper::view(title, &self.collections)
    }
}

/// Fetches the source's collections through its provider, on a Tokio runtime
/// (the remote providers need its reactor).
fn load(integration_id: String) -> Task<Message> {
    Task::perform(
        crate::runtime::on_tokio(async move {
            let (provider, _namespace) = provider_for_integration(&integration_id).await?;
            provider.list_collections().await
        }),
        Message::CollectionsLoaded,
    )
}

/// Groups collections by kind (in [`CollectionKind`]'s order), keeping each
/// group in the order the source listed it.
///
/// [`CollectionKind`]: crate::models::collection::CollectionKind
fn sorted(mut collections: Vec<Collection>) -> Vec<Collection> {
    collections.sort_by_key(|collection| collection.kind);
    collections
}

mod view_helper {
    use iced::{
        Background, Border, Center, Element,
        Length::Fill,
        Theme,
        widget::{button, column, container, row, scrollable, space, text},
    };
    use iced_font_awesome::fa_icon_solid;

    use super::Message;
    use crate::models::{
        AsyncModel,
        collection::{Collection, CollectionKind},
    };
    use crate::touch_scroll::touch_scroll;

    pub fn view<'a>(
        title: &'a str,
        collections: &'a AsyncModel<Vec<Collection>, String>,
    ) -> Element<'a, Message> {
        let body = match collections {
            AsyncModel::Loaded(collections) if collections.is_empty() => empty_state(),
            AsyncModel::Loaded(collections) => list(collections),
            AsyncModel::Error(error) => error_state(error),
            AsyncModel::NotLoaded | AsyncModel::Loading => {
                container(text("Loading…").style(text::secondary))
                    .center(Fill)
                    .into()
            }
        };

        column![top_bar(title), body]
            .width(Fill)
            .height(Fill)
            .into()
    }

    fn top_bar<'a>(title: &'a str) -> Element<'a, Message> {
        container(
            row![
                crate::screen::back_button::back_button(Message::BackPressed),
                text(title).size(20),
            ]
            .spacing(10)
            .align_y(Center),
        )
        .padding(20)
        .into()
    }

    /// The collections, under a heading per kind. `collections` is already
    /// grouped (see [`super::sorted`]).
    fn list<'a>(collections: &'a [Collection]) -> Element<'a, Message> {
        let mut content = column![].spacing(8);
        let mut current_kind: Option<CollectionKind> = None;

        for (index, collection) in collections.iter().enumerate() {
            if current_kind != Some(collection.kind) {
                current_kind = Some(collection.kind);
                content = content.push(
                    container(
                        text(collection.kind.group_label())
                            .size(13)
                            .style(text::secondary),
                    )
                    .padding([8, 0]),
                );
            }
            content = content.push(collection_row(index, collection));
        }

        container(touch_scroll(scrollable(content).height(Fill)))
            .width(Fill)
            .height(Fill)
            .padding([0, 20])
            .into()
    }

    fn collection_row<'a>(index: usize, collection: &'a Collection) -> Element<'a, Message> {
        button(
            row![
                fa_icon_solid(kind_icon(collection.kind)).size(16.0_f32),
                text(&collection.name),
                space::horizontal(),
                fa_icon_solid("chevron-right").size(12.0_f32),
            ]
            .spacing(12)
            .align_y(Center)
            .padding(6),
        )
        .width(Fill)
        .on_press(Message::CollectionSelected(index))
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

    fn kind_icon(kind: CollectionKind) -> &'static str {
        match kind {
            CollectionKind::Library => "book",
            CollectionKind::Collection => "layer-group",
            CollectionKind::ReadingList => "list-ol",
            CollectionKind::Category => "tag",
            CollectionKind::Catalog => "rss",
            CollectionKind::Folder => "folder",
        }
    }

    fn empty_state<'a>() -> Element<'a, Message> {
        container(text("Esta fonte não tem nenhuma coleção").style(text::secondary))
            .center(Fill)
            .into()
    }

    fn error_state<'a>(error: &'a str) -> Element<'a, Message> {
        container(
            column![text(error), button("Retry").on_press(Message::RetryPressed)]
                .spacing(12)
                .align_x(Center),
        )
        .center(Fill)
        .into()
    }
}

#[cfg(test)]
mod tests {
    use iced_test::simulator;

    use super::*;
    use crate::models::collection::CollectionKind;

    fn collections(loaded: AsyncModel<Vec<Collection>, String>) -> Collections {
        Collections {
            integration_id: "source-1".to_string(),
            source: None,
            collections: loaded,
        }
    }

    fn sample() -> Vec<Collection> {
        sorted(vec![
            Collection::new("reading-list:1", "Crossover", CollectionKind::ReadingList),
            Collection::new("library:1", "Manga", CollectionKind::Library),
            Collection::new("collection:1", "Favorites", CollectionKind::Collection),
            Collection::new("library:2", "Comics", CollectionKind::Library),
        ])
    }

    #[test]
    fn sorted_groups_by_kind_keeping_the_source_order_within_a_group() {
        let names: Vec<String> = sample().into_iter().map(|c| c.name).collect();

        assert_eq!(names, vec!["Manga", "Comics", "Favorites", "Crossover"]);
    }

    #[test]
    fn view_renders_each_collection_under_its_kind_heading() {
        let screen = collections(AsyncModel::Loaded(sample()));

        let mut ui = simulator(screen.view());

        for label in [
            "Libraries",
            "Manga",
            "Comics",
            "Collections",
            "Favorites",
            "Reading lists",
            "Crossover",
        ] {
            assert!(ui.find(label).is_ok(), "{label} should be shown");
        }
        assert!(ui.find("Categories").is_err());
    }

    #[test]
    fn view_renders_the_source_name() {
        let mut screen = collections(AsyncModel::Loaded(sample()));
        screen.source = Some(Integration {
            id: "source-1".to_string(),
            name: "My Kavita".to_string(),
            integration_type: crate::models::integration::IntegrationType::Kavita,
            url: None,
            api_key: None,
            username: None,
            password: None,
            folder: None,
        });

        let mut ui = simulator(screen.view());

        assert!(ui.find("My Kavita").is_ok());
    }

    #[test]
    fn clicking_a_collection_sends_its_index() {
        let screen = collections(AsyncModel::Loaded(sample()));

        let mut ui = simulator(screen.view());
        let _ = ui.click("Favorites").expect("collection should be found");

        assert_eq!(
            ui.into_messages().collect::<Vec<_>>(),
            vec![Message::CollectionSelected(2)]
        );
    }

    #[test]
    fn selecting_a_collection_opens_it_within_the_source() {
        let mut screen = collections(AsyncModel::Loaded(sample()));

        let action = screen.update(Message::CollectionSelected(2));

        match action {
            Action::OpenCollection(location) => {
                assert_eq!(location.integration_id, "source-1");
                assert_eq!(location.collection.external_id, "collection:1");
            }
            other => panic!("expected OpenCollection, got {other:?}"),
        }
    }

    #[test]
    fn selecting_an_unknown_index_does_nothing() {
        let mut screen = collections(AsyncModel::Loaded(sample()));

        assert!(matches!(
            screen.update(Message::CollectionSelected(9)),
            Action::None
        ));
    }

    #[test]
    fn collections_loaded_are_grouped_by_kind() {
        let mut screen = collections(AsyncModel::Loading);

        screen.update(Message::CollectionsLoaded(Ok(vec![
            Collection::new("reading-list:1", "Crossover", CollectionKind::ReadingList),
            Collection::new("library:1", "Manga", CollectionKind::Library),
        ])));

        let AsyncModel::Loaded(loaded) = &screen.collections else {
            panic!("collections should be loaded");
        };
        assert_eq!(loaded[0].name, "Manga");
    }

    #[test]
    fn an_error_offers_a_retry_that_reloads() {
        let mut screen = collections(AsyncModel::Error("unreachable".to_string()));

        let mut ui = simulator(screen.view());
        assert!(ui.find("unreachable").is_ok());
        let _ = ui.click("Retry").expect("retry should be offered");
        assert_eq!(
            ui.into_messages().collect::<Vec<_>>(),
            vec![Message::RetryPressed]
        );

        let action = screen.update(Message::RetryPressed);
        assert!(matches!(action, Action::Run(_)));
        assert!(matches!(screen.collections, AsyncModel::Loading));
    }

    #[test]
    fn back_pressed_goes_back_to_the_sources() {
        let mut screen = collections(AsyncModel::Loading);

        assert!(matches!(screen.update(Message::BackPressed), Action::Back));
    }
}
