//! The pieces of a cover card shared by the screens that show covers in a
//! grid: the bar under the cover (an icon, the card's name and a "⋮" button)
//! and the menu that button opens over the cover. iced has no context menu, so
//! the screen stacks the open menu over the card's cover itself.

use iced::{
    Background, Border, Center, Element,
    Length::{Fill, Shrink},
    Shadow, Theme, Vector, border,
    widget::{button, column, container, row, text},
};
use iced_font_awesome::fa_icon_solid;

/// Corner radius of a cover, its placeholder box and the shade over it. A
/// card's cover only rounds its top corners: its bar carries on below it.
pub const COVER_RADIUS: f32 = 8.0;

/// The bar under a card's cover: `icon`, then `name` (which should cut a name
/// too long for the card off rather than wrap it), then, when `menu_toggle` is
/// given, the "⋮" button that sends it.
pub fn bar<'a, Message: Clone + 'a>(
    icon: &'static str,
    name: Element<'a, Message>,
    menu_toggle: Option<Message>,
) -> Element<'a, Message> {
    let name = container(name).width(Fill).clip(true);

    let mut bar = row![
        fa_icon_solid(icon)
            .size(14.0_f32)
            .style(|theme: &Theme| text::Style {
                color: Some(
                    theme
                        .extended_palette()
                        .background
                        .weak
                        .text
                        .scale_alpha(0.7)
                ),
            }),
        name,
    ]
    .spacing(10)
    .align_y(Center);

    if let Some(menu_toggle) = menu_toggle {
        bar = bar.push(
            button(
                fa_icon_solid("ellipsis-vertical")
                    .size(14.0_f32)
                    .style(|theme: &Theme| text::Style {
                        color: Some(theme.extended_palette().background.weak.text),
                    }),
            )
            .on_press(menu_toggle)
            .padding([4, 8])
            .style(menu_button_style),
        );
    }

    container(bar)
        .padding([8, 10])
        .width(Fill)
        .style(|theme: &Theme| {
            let palette = theme.extended_palette();

            container::Style {
                background: Some(Background::Color(palette.background.weak.color)),
                text_color: Some(palette.background.weak.text),
                border: Border {
                    radius: border::bottom(COVER_RADIUS),
                    ..Border::default()
                },
                ..container::Style::default()
            }
        })
        .into()
}

/// A card's open menu, holding `items` (see [`menu_item`]), laid over the
/// bottom-right of its cover, just above the bar's "⋮" button.
pub fn menu<'a, Message: 'a>(items: Vec<Element<'a, Message>>) -> Element<'a, Message> {
    // Shrink is set before the items are pushed, which widens it to fit them:
    // set after, it would collapse the menu to nothing.
    let items = items
        .into_iter()
        .fold(column![].width(Shrink), |menu, item| menu.push(item));
    let menu = container(items).padding(4).style(|theme: &Theme| {
        let palette = theme.extended_palette();

        container::Style {
            background: Some(Background::Color(palette.background.base.color)),
            text_color: Some(palette.background.base.text),
            border: Border {
                radius: 6.0.into(),
                width: 1.0,
                color: palette.background.strong.color,
            },
            shadow: Shadow {
                color: iced::Color::BLACK.scale_alpha(0.4),
                offset: Vector::new(0.0, 2.0),
                blur_radius: 8.0,
            },
            ..container::Style::default()
        }
    });

    container(menu)
        .padding(6)
        .width(Fill)
        .height(Fill)
        .align_right(Fill)
        .align_bottom(Fill)
        .into()
}

/// One item of a card's menu: an icon and its label.
pub fn menu_item<'a, Message: Clone + 'a>(
    icon: &'static str,
    label: &'static str,
    on_press: Message,
) -> Element<'a, Message> {
    menu_item_maybe(icon, label, Some(on_press))
}

/// A menu item that, without a message, is shown as a state (e.g. something
/// under way) rather than an action, and can't be chosen.
pub fn menu_item_maybe<'a, Message: Clone + 'a>(
    icon: &'static str,
    label: &'static str,
    on_press: Option<Message>,
) -> Element<'a, Message> {
    button(
        row![
            fa_icon_solid(icon)
                .size(13.0_f32)
                .style(|theme: &Theme| text::Style {
                    color: Some(theme.extended_palette().background.base.text),
                }),
            text(label).size(13),
        ]
        .spacing(8)
        .align_y(Center),
    )
    .on_press_maybe(on_press)
    .padding([6, 12])
    .width(Fill)
    .style(menu_button_style)
    .into()
}

/// A flat button that's only shaded while hovered or pressed.
fn menu_button_style(theme: &Theme, status: button::Status) -> button::Style {
    let palette = theme.extended_palette();

    button::Style {
        background: matches!(status, button::Status::Hovered | button::Status::Pressed)
            .then(|| Background::Color(palette.background.strong.color)),
        text_color: palette.background.base.text,
        border: Border {
            radius: 4.0.into(),
            ..Border::default()
        },
        ..button::Style::default()
    }
}

