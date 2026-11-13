//! The shade laid over a hovered cover, holding the actions on what it shows
//! (a series, or one of its volumes/chapters). Shared by the screens that show
//! covers in a grid.

use iced::{
    Background, Border, Center, Element,
    Length::Fill,
    Theme, border,
    widget::{button, column, container, row, text},
};
use iced_font_awesome::fa_icon_solid;

/// Lays `actions` side by side on a dark shade filling the cover, whose
/// corners are rounded by `radius` to match it.
pub fn shade<'a, Message: 'a>(
    actions: Vec<Element<'a, Message>>,
    radius: border::Radius,
) -> Element<'a, Message> {
    container(row(actions).spacing(8).width(Fill).align_y(Center))
        .center(Fill)
        .padding(8)
        .style(move |_theme: &Theme| container::Style {
            background: Some(Background::Color(iced::Color::BLACK.scale_alpha(0.55))),
            border: Border {
                radius,
                ..Border::default()
            },
            ..container::Style::default()
        })
        .into()
}

/// One action on a hovered cover: an icon with a small caption under it.
/// Without a message it's shown as a (dimmed) state, not a button to press.
pub fn action<'a, Message: Clone + 'a>(
    icon: &'static str,
    label: &'static str,
    on_press: Option<Message>,
) -> Element<'a, Message> {
    let enabled = on_press.is_some();
    let color = if enabled {
        iced::Color::WHITE
    } else {
        iced::Color::WHITE.scale_alpha(0.7)
    };

    let content = column![
        fa_icon_solid(icon).size(22.0_f32).color(color),
        // Captions wrap, so each action takes the same share of the row.
        text(label)
            .size(11)
            .color(color)
            .center()
            .width(Fill)
            .wrapping(text::Wrapping::WordOrGlyph),
    ]
    .spacing(6)
    .width(Fill)
    .align_x(Center);

    button(content)
        .on_press_maybe(on_press)
        .padding([8, 6])
        .width(Fill)
        .style(|_theme: &Theme, status| button::Style {
            background: matches!(status, button::Status::Hovered | button::Status::Pressed)
                .then(|| Background::Color(iced::Color::WHITE.scale_alpha(0.15))),
            text_color: iced::Color::WHITE,
            border: Border {
                radius: 6.0.into(),
                ..Border::default()
            },
            ..button::Style::default()
        })
        .into()
}
