//! Shared trackpad swipe recognition; hosts own sidebar visibility and motion.
use gpui::{Axis, Modifiers, ScrollDelta, ScrollWheelEvent, TouchPhase};

const AXIS_LOCK_PX: f32 = 8.;
const THRESHOLD_PX: f32 = 48.;

#[derive(Default)]
pub(crate) struct SidebarSwipe {
    contact: Option<Contact>,
}

#[derive(Default)]
struct Contact {
    x: f32,
    y: f32,
    axis: Option<Axis>,
    triggered: bool,
}

#[derive(Default, Debug, PartialEq)]
pub(crate) struct SwipeOutcome {
    pub consumed: bool,
    pub visibility: Option<bool>,
}

impl SidebarSwipe {
    pub(crate) fn update(&mut self, event: &ScrollWheelEvent, visible: bool) -> SwipeOutcome {
        let ScrollDelta::Pixels(delta) = event.delta else {
            self.contact = None;
            return SwipeOutcome::default();
        };
        let (x, y) = (f32::from(delta.x), f32::from(delta.y));
        if event.modifiers != Modifiers::default() || !x.is_finite() || !y.is_finite() {
            self.contact = None;
            return SwipeOutcome::default();
        }
        if event.touch_phase == TouchPhase::Started {
            self.contact = Some(Contact::default());
        }
        let Some(contact) = self.contact.as_mut() else {
            // Momentum and mouse wheels do not start a new finger contact.
            return SwipeOutcome::default();
        };
        if event.touch_phase == TouchPhase::Cancelled {
            self.contact = None;
            return SwipeOutcome::default();
        }
        contact.x += x;
        contact.y += y;
        if contact.axis.is_none() && contact.x.abs().max(contact.y.abs()) >= AXIS_LOCK_PX {
            contact.axis = Some(if contact.x.abs() > contact.y.abs() * 1.2 {
                Axis::Horizontal
            } else {
                Axis::Vertical
            });
        }
        let mut outcome = SwipeOutcome {
            consumed: contact.axis == Some(Axis::Horizontal),
            visibility: None,
        };
        if outcome.consumed && !contact.triggered {
            let target = if contact.x >= THRESHOLD_PX {
                Some(true)
            } else if contact.x <= -THRESHOLD_PX {
                Some(false)
            } else {
                None
            };
            if let Some(target) = target.filter(|target| *target != visible) {
                contact.triggered = true;
                outcome.visibility = Some(target);
            }
        }
        if event.touch_phase == TouchPhase::Ended {
            self.contact = None;
        }
        outcome
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::{point, px};

    fn event(x: f32, y: f32, phase: TouchPhase) -> ScrollWheelEvent {
        ScrollWheelEvent {
            delta: ScrollDelta::Pixels(point(px(x), px(y))),
            touch_phase: phase,
            ..Default::default()
        }
    }

    #[test]
    fn horizontal_contacts_change_visibility_once_and_ignore_momentum() {
        let mut swipe = SidebarSwipe::default();
        for (visible, sign) in [(false, 1.), (true, -1.)] {
            let started = swipe.update(&event(sign * 10., 1., TouchPhase::Started), visible);
            assert!(started.consumed);
            assert_eq!(started.visibility, None);
            assert_eq!(
                swipe
                    .update(&event(sign * 40., 1., TouchPhase::Moved), visible)
                    .visibility,
                Some(!visible)
            );
            for phase in [TouchPhase::Moved, TouchPhase::Ended, TouchPhase::Moved] {
                assert_eq!(
                    swipe
                        .update(&event(-sign * 200., 0., phase), !visible)
                        .visibility,
                    None
                );
            }
        }
    }

    #[test]
    fn vertical_contacts_stay_vertical_and_short_or_cancelled_swipes_do_nothing() {
        let mut swipe = SidebarSwipe::default();
        for event in [
            event(2., 10., TouchPhase::Started),
            event(200., 0., TouchPhase::Moved),
            event(0., 0., TouchPhase::Ended),
            event(7., 0., TouchPhase::Started),
            event(0., 0., TouchPhase::Ended),
            event(10., 0., TouchPhase::Started),
            event(100., 0., TouchPhase::Cancelled),
            event(100., 0., TouchPhase::Moved),
        ] {
            assert_eq!(swipe.update(&event, false).visibility, None);
        }
    }

    #[test]
    fn mouse_wheels_modifiers_and_invalid_deltas_do_not_claim_a_swipe() {
        let mut swipe = SidebarSwipe::default();
        for mut event in [
            ScrollWheelEvent {
                delta: ScrollDelta::Lines(point(100., 0.)),
                touch_phase: TouchPhase::Started,
                ..Default::default()
            },
            ScrollWheelEvent {
                modifiers: Modifiers {
                    shift: true,
                    ..Default::default()
                },
                ..event(100., 0., TouchPhase::Started)
            },
            event(f32::NAN, 0., TouchPhase::Started),
            event(0., f32::INFINITY, TouchPhase::Started),
        ] {
            assert_eq!(swipe.update(&event, false), SwipeOutcome::default());
            event.touch_phase = TouchPhase::Moved;
            assert_eq!(swipe.update(&event, false), SwipeOutcome::default());
        }
    }
}
