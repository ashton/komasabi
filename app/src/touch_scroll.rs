//! Drag-to-scroll for touch screens.
//!
//! iced's `scrollable` only starts a touch drag when its content doesn't
//! capture the `FingerPressed`. A grid of cards is a grid of buttons, and a
//! button captures every press it gets, so a drag that starts on a card never
//! scrolls. [`touch_scroll`] wraps a scrollable (or anything holding one) and
//! takes over a finger once it moves past [`DRAG_THRESHOLD`]: the content gets a
//! `FingerLost`, which cancels the pressed button without clicking it, and every
//! later movement is fed to the content as a pixel `WheelScrolled` at the point
//! the finger went down. That reaches the innermost scrollable under the finger,
//! the same as a touchpad. A tap that doesn't move still reaches the button.
//! Lifting the finger mid-flick keeps scrolling, slowing down as it goes.

use std::time::{Duration, Instant};

use iced::advanced::layout::{self, Layout};
use iced::advanced::widget::{Operation, Tree, tree};
use iced::advanced::{Clipboard, Shell, Widget, overlay, renderer};
use iced::{Element, Event, Length, Point, Rectangle, Size, Vector, mouse, touch, window};

/// How far (in logical pixels) a finger moves before a press becomes a drag.
const DRAG_THRESHOLD: f32 = 10.0;

/// The fraction of its speed a flick keeps after a second of coasting.
const FRICTION_PER_SECOND: f32 = 0.05;

/// A flick slower than this (pixels per second) stops coasting.
const MIN_FLICK_SPEED: f32 = 30.0;

/// Movements older than this, at the moment the finger lifts, don't count
/// towards the flick's speed (a finger that stopped before lifting shouldn't
/// coast).
const VELOCITY_WINDOW: Duration = Duration::from_millis(100);

/// Wraps `content` so a finger dragged anywhere over it scrolls it.
pub fn touch_scroll<'a, Message, Theme, Renderer>(
    content: impl Into<Element<'a, Message, Theme, Renderer>>,
) -> TouchScroll<'a, Message, Theme, Renderer> {
    TouchScroll {
        content: content.into(),
    }
}

pub struct TouchScroll<'a, Message, Theme, Renderer> {
    content: Element<'a, Message, Theme, Renderer>,
}

#[derive(Debug, Default)]
enum State {
    #[default]
    Idle,
    /// A finger is down but hasn't moved far enough to be a drag; the content
    /// has seen the press and may still treat it as a tap.
    Pressed {
        finger: touch::Finger,
        origin: Point,
    },
    /// The finger is dragging the content.
    Dragging {
        finger: touch::Finger,
        origin: Point,
        last: Point,
        /// Recent movements, for the flick speed on release.
        samples: Vec<(Instant, Vector)>,
    },
    /// The finger lifted mid-flick; the content keeps scrolling at `velocity`
    /// (pixels per second), which decays each frame.
    Coasting {
        origin: Point,
        velocity: Vector,
        last_frame: Option<Instant>,
    },
}

impl<Message, Theme, Renderer> Widget<Message, Theme, Renderer>
    for TouchScroll<'_, Message, Theme, Renderer>
where
    Renderer: iced::advanced::Renderer,
{
    fn tag(&self) -> tree::Tag {
        tree::Tag::of::<State>()
    }

    fn state(&self) -> tree::State {
        tree::State::new(State::default())
    }

    fn children(&self) -> Vec<Tree> {
        vec![Tree::new(&self.content)]
    }

    fn diff(&self, tree: &mut Tree) {
        tree.diff_children(std::slice::from_ref(&self.content));
    }

    fn size(&self) -> Size<Length> {
        self.content.as_widget().size()
    }

    fn size_hint(&self) -> Size<Length> {
        self.content.as_widget().size_hint()
    }

    fn layout(
        &mut self,
        tree: &mut Tree,
        renderer: &Renderer,
        limits: &layout::Limits,
    ) -> layout::Node {
        self.content
            .as_widget_mut()
            .layout(&mut tree.children[0], renderer, limits)
    }

    fn draw(
        &self,
        tree: &Tree,
        renderer: &mut Renderer,
        theme: &Theme,
        style: &renderer::Style,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        viewport: &Rectangle,
    ) {
        self.content.as_widget().draw(
            &tree.children[0],
            renderer,
            theme,
            style,
            layout,
            cursor,
            viewport,
        );
    }

    fn operate(
        &mut self,
        tree: &mut Tree,
        layout: Layout<'_>,
        renderer: &Renderer,
        operation: &mut dyn Operation,
    ) {
        self.content
            .as_widget_mut()
            .operate(&mut tree.children[0], layout, renderer, operation);
    }

    fn update(
        &mut self,
        tree: &mut Tree,
        event: &Event,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        renderer: &Renderer,
        clipboard: &mut dyn Clipboard,
        shell: &mut Shell<'_, Message>,
        viewport: &Rectangle,
    ) {
        let (state, children) = (tree.state.downcast_mut::<State>(), &mut tree.children);
        let content = &mut self.content;

        // Feeds an event to the content as if the cursor were at `at`.
        let mut forward = |event: &Event, at: mouse::Cursor, shell: &mut Shell<'_, Message>| {
            content.as_widget_mut().update(
                &mut children[0],
                event,
                layout,
                at,
                renderer,
                clipboard,
                shell,
                viewport,
            );
        };

        match event {
            Event::Touch(touch::Event::FingerPressed { id, position }) => {
                // A new finger stops a coasting flick, like on a phone.
                if matches!(state, State::Idle | State::Coasting { .. })
                    && layout.bounds().contains(*position)
                {
                    *state = State::Pressed {
                        finger: *id,
                        origin: *position,
                    };
                }
            }
            Event::Touch(touch::Event::FingerMoved { id, position }) => match state {
                State::Pressed { finger, origin } if finger == id => {
                    if position.distance(*origin) < DRAG_THRESHOLD {
                        // Not a drag yet; keep the content from scrolling a
                        // few pixels on its own before we take over.
                        shell.capture_event();
                        return;
                    }

                    let origin = *origin;
                    let lost = Event::Touch(touch::Event::FingerLost {
                        id: *id,
                        position: *position,
                    });
                    forward(&lost, cursor, shell);

                    *state = State::Dragging {
                        finger: *id,
                        origin,
                        last: origin,
                        samples: Vec::new(),
                    };
                    drag_to(state, *position, &mut forward, shell);
                    shell.capture_event();
                    return;
                }
                State::Dragging { finger, .. } if finger == id => {
                    drag_to(state, *position, &mut forward, shell);
                    shell.capture_event();
                    return;
                }
                _ => {}
            },
            Event::Touch(
                touch::Event::FingerLifted { id, .. } | touch::Event::FingerLost { id, .. },
            ) => match state {
                State::Pressed { finger, .. } if finger == id => {
                    *state = State::Idle;
                }
                State::Dragging {
                    finger,
                    origin,
                    samples,
                    ..
                } if finger == id => {
                    let velocity = flick_velocity(samples, Instant::now());
                    *state = if velocity_magnitude(velocity) >= MIN_FLICK_SPEED {
                        shell.request_redraw();
                        State::Coasting {
                            origin: *origin,
                            velocity,
                            last_frame: None,
                        }
                    } else {
                        State::Idle
                    };
                    // The content already had its `FingerLost`.
                    shell.capture_event();
                    return;
                }
                _ => {}
            },
            Event::Window(window::Event::RedrawRequested(now)) => {
                if let State::Coasting {
                    origin,
                    velocity,
                    last_frame,
                } = state
                {
                    if let Some(last_frame) = *last_frame {
                        let dt = now.saturating_duration_since(last_frame).as_secs_f32();
                        let step = *velocity * dt;
                        *velocity = *velocity * FRICTION_PER_SECOND.powf(dt);
                        scroll_by(*origin, step, &mut forward, shell);
                    }
                    *last_frame = Some(*now);

                    if velocity_magnitude(*velocity) < MIN_FLICK_SPEED {
                        *state = State::Idle;
                    } else {
                        shell.request_redraw();
                    }
                }
            }
            // Any mouse or keyboard input stops a coasting flick.
            Event::Mouse(mouse::Event::ButtonPressed(_) | mouse::Event::WheelScrolled { .. })
            | Event::Keyboard(_) => {
                if matches!(state, State::Coasting { .. }) {
                    *state = State::Idle;
                }
            }
            _ => {}
        }

        forward(event, cursor, shell);
    }

    fn mouse_interaction(
        &self,
        tree: &Tree,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        viewport: &Rectangle,
        renderer: &Renderer,
    ) -> mouse::Interaction {
        self.content.as_widget().mouse_interaction(
            &tree.children[0],
            layout,
            cursor,
            viewport,
            renderer,
        )
    }

    fn overlay<'b>(
        &'b mut self,
        tree: &'b mut Tree,
        layout: Layout<'b>,
        renderer: &Renderer,
        viewport: &Rectangle,
        translation: Vector,
    ) -> Option<overlay::Element<'b, Message, Theme, Renderer>> {
        self.content.as_widget_mut().overlay(
            &mut tree.children[0],
            layout,
            renderer,
            viewport,
            translation,
        )
    }
}

/// Moves a dragging finger to `position`, scrolling the content with it.
fn drag_to<Message>(
    state: &mut State,
    position: Point,
    forward: &mut impl FnMut(&Event, mouse::Cursor, &mut Shell<'_, Message>),
    shell: &mut Shell<'_, Message>,
) {
    let State::Dragging {
        origin,
        last,
        samples,
        ..
    } = state
    else {
        return;
    };

    let delta = position - *last;
    *last = position;

    let now = Instant::now();
    samples.retain(|(at, _)| now.saturating_duration_since(*at) <= VELOCITY_WINDOW);
    samples.push((now, delta));

    scroll_by(*origin, delta, forward, shell);
}

/// Scrolls the content under `origin` so it follows a finger that moved by
/// `delta`: a pixel wheel scroll of `delta` moves the content the same way.
fn scroll_by<Message>(
    origin: Point,
    delta: Vector,
    forward: &mut impl FnMut(&Event, mouse::Cursor, &mut Shell<'_, Message>),
    shell: &mut Shell<'_, Message>,
) {
    let wheel = Event::Mouse(mouse::Event::WheelScrolled {
        delta: mouse::ScrollDelta::Pixels {
            x: delta.x,
            y: delta.y,
        },
    });
    forward(&wheel, mouse::Cursor::Available(origin), shell);
}

/// The finger's speed (pixels per second) over the movements still inside
/// [`VELOCITY_WINDOW`] at `now`.
fn flick_velocity(samples: &[(Instant, Vector)], now: Instant) -> Vector {
    let recent: Vec<_> = samples
        .iter()
        .filter(|(at, _)| now.saturating_duration_since(*at) <= VELOCITY_WINDOW)
        .collect();

    let Some((first, _)) = recent.first() else {
        return Vector::ZERO;
    };

    let total = recent
        .iter()
        .fold(Vector::ZERO, |sum, (_, delta)| sum + *delta);
    // At least one frame's worth, so a single quick sample isn't infinitely fast.
    let elapsed = now
        .saturating_duration_since(*first)
        .max(Duration::from_millis(16))
        .as_secs_f32();

    total * (1.0 / elapsed)
}

fn velocity_magnitude(velocity: Vector) -> f32 {
    velocity.x.hypot(velocity.y)
}

impl<'a, Message, Theme, Renderer> From<TouchScroll<'a, Message, Theme, Renderer>>
    for Element<'a, Message, Theme, Renderer>
where
    Message: 'a,
    Theme: 'a,
    Renderer: iced::advanced::Renderer + 'a,
{
    fn from(widget: TouchScroll<'a, Message, Theme, Renderer>) -> Self {
        Element::new(widget)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use iced::widget::{button, column, scrollable, text};
    use iced::{Fill, Theme};
    use iced_test::simulator;

    #[derive(Debug, Clone, PartialEq)]
    enum Message {
        Tapped(usize),
        Scrolled(f32),
    }

    /// Twenty 100px-tall buttons, far taller than the simulator's window.
    fn view() -> Element<'static, Message, Theme, iced::Renderer> {
        let cards = (0..20).map(|i| {
            button(text(format!("card {i}")))
                .on_press(Message::Tapped(i))
                .width(Fill)
                .height(100)
                .into()
        });

        touch_scroll(
            scrollable(column(cards))
                .height(Fill)
                .on_scroll(|viewport| Message::Scrolled(viewport.absolute_offset().y)),
        )
        .into()
    }

    fn finger(event: fn(touch::Finger, Point) -> touch::Event, x: f32, y: f32) -> Event {
        Event::Touch(event(touch::Finger(1), Point::new(x, y)))
    }

    fn pressed(id: touch::Finger, position: Point) -> touch::Event {
        touch::Event::FingerPressed { id, position }
    }

    fn moved(id: touch::Finger, position: Point) -> touch::Event {
        touch::Event::FingerMoved { id, position }
    }

    fn lifted(id: touch::Finger, position: Point) -> touch::Event {
        touch::Event::FingerLifted { id, position }
    }

    fn last_offset(messages: &[Message]) -> Option<f32> {
        messages.iter().rev().find_map(|message| match message {
            Message::Scrolled(offset) => Some(*offset),
            Message::Tapped(_) => None,
        })
    }

    #[test]
    fn dragging_from_a_button_scrolls_without_pressing_it() {
        let mut ui = simulator(view());
        ui.point_at(Point::new(100.0, 250.0));

        ui.simulate([
            finger(pressed, 100.0, 250.0),
            finger(moved, 100.0, 200.0),
            finger(moved, 100.0, 150.0),
            finger(lifted, 100.0, 150.0),
        ]);

        let messages: Vec<_> = ui.into_messages().collect();
        assert!(!messages.iter().any(|m| matches!(m, Message::Tapped(_))));
        assert_eq!(last_offset(&messages), Some(100.0));
    }

    #[test]
    fn dragging_down_at_the_top_stays_put() {
        let mut ui = simulator(view());
        ui.point_at(Point::new(100.0, 250.0));

        ui.simulate([
            finger(pressed, 100.0, 250.0),
            finger(moved, 100.0, 400.0),
            finger(lifted, 100.0, 400.0),
        ]);

        let messages: Vec<_> = ui.into_messages().collect();
        assert!(!messages.iter().any(|m| matches!(m, Message::Tapped(_))));
        assert!(last_offset(&messages).is_none_or(|offset| offset == 0.0));
    }

    #[test]
    fn a_tap_presses_the_button() {
        let mut ui = simulator(view());
        ui.point_at(Point::new(100.0, 250.0));

        ui.simulate([finger(pressed, 100.0, 250.0), finger(lifted, 100.0, 250.0)]);

        let messages: Vec<_> = ui.into_messages().collect();
        assert_eq!(messages, vec![Message::Tapped(2)]);
    }

    #[test]
    fn a_tap_that_wobbles_under_the_threshold_still_presses_the_button() {
        let mut ui = simulator(view());
        ui.point_at(Point::new(100.0, 250.0));

        ui.simulate([
            finger(pressed, 100.0, 250.0),
            finger(moved, 103.0, 254.0),
            finger(lifted, 103.0, 254.0),
        ]);

        let messages: Vec<_> = ui.into_messages().collect();
        assert_eq!(messages, vec![Message::Tapped(2)]);
    }

    #[test]
    fn a_flick_keeps_scrolling_after_the_finger_lifts() {
        let mut ui = simulator(view());
        ui.point_at(Point::new(100.0, 400.0));

        ui.simulate([
            finger(pressed, 100.0, 400.0),
            finger(moved, 100.0, 300.0),
            finger(moved, 100.0, 200.0),
            finger(lifted, 100.0, 200.0),
        ]);
        let start = Instant::now();
        ui.simulate([
            Event::Window(window::Event::RedrawRequested(start)),
            Event::Window(window::Event::RedrawRequested(start + Duration::from_millis(16))),
            Event::Window(window::Event::RedrawRequested(start + Duration::from_millis(32))),
        ]);

        let messages: Vec<_> = ui.into_messages().collect();
        let offset = last_offset(&messages).expect("scrolled");
        assert!(offset > 200.0, "coasted past the drag: {offset}");
    }

    #[test]
    fn a_finger_that_stopped_before_lifting_does_not_coast() {
        let now = Instant::now();
        let samples = [(now - Duration::from_millis(500), Vector::new(0.0, -100.0))];

        assert_eq!(flick_velocity(&samples, now), Vector::ZERO);
    }
}
