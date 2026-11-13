//! The "back" control in a screen's top bar: a chevron, not a labelled button.

use iced::{
    Background, Border, Element, Theme,
    widget::{Id, button, container},
};
use iced_font_awesome::fa_icon_solid;

/// The id of the container around the back chevron, so tests can find (and
/// click) it: it has no text to find it by.
pub const ID: &str = "back";

/// A chevron that sends `on_press`, shaded while hovered.
pub fn back_button<'a, Message: Clone + 'a>(on_press: Message) -> Element<'a, Message> {
    container(
        button(
            fa_icon_solid("chevron-left")
                .size(18.0_f32)
                .style(|theme: &Theme| iced::widget::text::Style {
                    color: Some(theme.extended_palette().background.base.text),
                }),
        )
        .on_press(on_press)
        .padding([6, 10])
        .style(|theme: &Theme, status| {
            let palette = theme.extended_palette();

            button::Style {
                background: matches!(status, button::Status::Hovered | button::Status::Pressed)
                    .then(|| Background::Color(palette.background.strong.color)),
                text_color: palette.background.base.text,
                border: Border {
                    radius: 6.0.into(),
                    ..Border::default()
                },
                ..button::Style::default()
            }
        }),
    )
    .id(Id::new(ID))
    .into()
}
