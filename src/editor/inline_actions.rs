//! Revision-guarded inline interactions, shared hover grace and source geometry.
use super::*;
use crate::{
    components::inline_picker::{
        InlineEvent, InlinePicker, InlineValue, LinkValue, PriorityValue, TagsValue,
    },
    document::DocumentFormat,
};

mod context;
mod edits;
mod geometry;
#[cfg(test)]
mod tests;

use context::{prepare, priority_token};

#[derive(Clone)]
enum Kind {
    Checkbox,
    Priority,
    Tags,
    Link(LinkHit),
}
#[derive(Clone)]
struct Hit {
    range: ByteRange,
    bounds: Bounds<Pixels>,
    pointer: Point<Pixels>,
    source: String,
    kind: Kind,
}
pub(super) struct InlinePopup {
    hit: Hit,
    revision: Revision,
    picker: Entity<InlinePicker>,
    bounds: Option<Bounds<Pixels>>,
    _subscription: Subscription,
    _observation: Subscription,
}

struct InlinePress {
    hit: Hit,
    revision: Revision,
    position: Point<Pixels>,
    selection: Selection,
}
struct InlineCommit {
    revision: Revision,
    checkbox_hover: Option<ByteRange>,
}
#[derive(Clone, Copy)]
enum HighlightKind {
    Tags,
    Token,
    Link,
}
#[derive(Clone, Copy)]
struct Hovered {
    range: ByteRange,
    kind: HighlightKind,
}
impl Hit {
    fn hovered(&self) -> Hovered {
        Hovered {
            range: self.range,
            kind: match self.kind {
                Kind::Tags => HighlightKind::Tags,
                Kind::Checkbox | Kind::Priority => HighlightKind::Token,
                Kind::Link(_) => HighlightKind::Link,
            },
        }
    }
}
#[derive(Default)]
pub(super) struct InlineActions {
    pub popup: Option<InlinePopup>,
    commit: Option<InlineCommit>,
    hovered: Option<Hovered>,
    dismissed: Option<ByteRange>,
    press: Option<InlinePress>,
    pending: Option<Task<()>>,
    closing: Option<Task<()>>,
}

impl InlineActions {
    fn hover_range(&self) -> Option<ByteRange> {
        self.hovered.map(|h| h.range)
    }
    fn highlight(&self) -> Option<Hovered> {
        self.popup
            .as_ref()
            .map(|p| p.hit.hovered())
            .or(self.hovered)
    }
}

impl SemanticEditor {
    pub(super) fn set_inline_language(
        &mut self,
        language: crate::i18n::Language,
        cx: &mut Context<Self>,
    ) {
        if let Some(popup) = &self.inline_actions.popup {
            popup.picker.update(cx, |picker, cx| {
                picker.language = language;
                cx.notify();
            });
        }
    }

    pub(super) fn inline_document_changed(
        &mut self,
        event: &DocumentEvent,
        cx: &mut Context<Self>,
    ) -> bool {
        let commit = self.inline_actions.commit.take().filter(|commit| matches!(event, DocumentEvent::Edited { delta,.. } if delta.after == commit.revision));
        let hover = commit
            .as_ref()
            .and_then(|commit| commit.checkbox_hover)
            .filter(|range| self.inline_actions.hover_range() == Some(*range));
        self.dismiss_inline(cx);
        self.inline_actions = InlineActions::default();
        self.inline_actions.hovered = hover.map(|range| Hovered {
            range,
            kind: HighlightKind::Token,
        });
        commit.is_some()
    }
    pub(super) fn inline_highlight(&self) -> Option<ByteRange> {
        self.inline_actions.highlight().map(|h| h.range)
    }
    pub(super) fn inline_tag_highlight(&self) -> Option<ByteRange> {
        self.inline_actions
            .highlight()
            .filter(|h| matches!(h.kind, HighlightKind::Tags))
            .map(|h| h.range)
    }
    pub(super) fn inline_background_highlight(&self) -> Option<ByteRange> {
        self.inline_actions
            .highlight()
            .filter(|h| matches!(h.kind, HighlightKind::Token))
            .map(|h| h.range)
    }
    pub(super) fn dismiss_inline(&mut self, cx: &mut Context<Self>) {
        self.inline_actions.pending = None;
        self.inline_actions.closing = None;
        self.inline_actions.hovered = None;
        if let Some(popup) = self.inline_actions.popup.take() {
            self.inline_actions.dismissed = Some(popup.hit.range);
            self.autofocus = true;
            cx.notify();
        }
    }
    pub(super) fn inline_hover(&mut self, position: Point<Pixels>, cx: &mut Context<Self>) {
        if let Some(press) = &self.inline_actions.press
            && ((position.x - press.position.x).abs() > px(4.)
                || (position.y - press.position.y).abs() > px(4.))
        {
            // Only become a text selection after actual movement. A simple click
            // must never flash a temporary caret/active line at the checkbox.
            let press = self.inline_actions.press.take().unwrap();
            if press.revision == self.snapshot(cx).revision() {
                self.selection = Selection::caret(self.hit_test(press.position));
                self.is_selecting = true;
                self.drag_position = Some(position);
                self.sync_selection_utf16(&self.snapshot(cx));
                cx.notify();
            }
        }
        if self.inline_actions.press.is_some() {
            return;
        }
        if self.is_selecting || self.minimap.drag.is_some() || self.minimap.resizing.is_some() {
            return;
        }
        if let Some(popup) = &self.inline_actions.popup
            && (popup.picker.read(cx).is_pinned()
                || popup.bounds.is_some_and(|b| b.contains(&position)))
        {
            self.inline_actions.closing = None;
            return;
        }
        let hit = self.inline_hit(position, cx);
        let range = hit.as_ref().map(|h| h.range);
        if range != self.inline_actions.dismissed {
            self.inline_actions.dismissed = None;
        }
        if range != self.inline_actions.hover_range() {
            self.inline_actions.hovered = hit.as_ref().map(Hit::hovered);
            self.inline_actions.pending = None;
            cx.notify();
        }
        if let Some(hit) = hit {
            self.inline_actions.closing = None;
            if self
                .inline_actions
                .popup
                .as_ref()
                .is_some_and(|p| p.hit.range == hit.range)
                || self.inline_actions.dismissed == Some(hit.range)
            {
                return;
            }
            if matches!(hit.kind, Kind::Priority | Kind::Link(_))
                && self.inline_actions.pending.is_none()
            {
                self.schedule_inline(hit, false, cx);
            }
        } else if self.inline_actions.popup.is_some() && self.inline_actions.closing.is_none() {
            let executor = cx.background_executor().clone();
            self.inline_actions.closing = Some(cx.spawn(async move |this, cx| {
                executor.timer(Duration::from_millis(300)).await;
                let _ = this.update(cx, |this, cx| this.dismiss_inline(cx));
            }));
        }
    }
    fn schedule_inline(&mut self, hit: Hit, clicked: bool, cx: &mut Context<Self>) {
        let snapshot = self.snapshot(cx);
        let revision = snapshot.revision();
        let read_only = self.is_read_only(cx);
        let delay = if clicked || self.inline_actions.popup.is_some() {
            0
        } else {
            350
        };
        let executor = cx.background_executor().clone();
        self.inline_actions.pending = Some(cx.spawn(async move |this, cx| {
            executor.timer(Duration::from_millis(delay)).await;
            let background_hit = hit.clone();
            let value = executor
                .spawn(async move { prepare(&background_hit, &snapshot, read_only) })
                .await;
            let _ = this.update(cx, |this, cx| {
                if this.snapshot(cx).revision() != revision
                    || this.is_selecting
                    || (!clicked && this.inline_actions.hover_range() != Some(hit.range))
                {
                    return;
                }
                let Some(value) = value else {
                    return;
                };
                this.dismiss_todo(cx);
                this.dismiss_timestamp(cx);
                this.todo.hover_range = None;
                this.timestamp.hover_range = None;
                let picker = cx.new(|cx| InlinePicker::new(value, this.ui_language, cx));
                let subscription =
                    cx.subscribe(&picker, |this, _, event, cx| this.inline_event(event, cx));
                let observation = cx.observe(&picker, |_, _, cx| cx.notify());
                this.inline_actions.popup = Some(InlinePopup {
                    hit,
                    revision,
                    picker,
                    bounds: None,
                    _subscription: subscription,
                    _observation: observation,
                });
                this.inline_actions.closing = None;
                cx.notify();
            });
        }));
    }
    pub(super) fn inline_mouse_down(
        &mut self,
        event: &MouseDownEvent,
        cx: &mut Context<Self>,
    ) -> bool {
        self.inline_actions.press = None;
        if event.click_count != 1
            || event.modifiers.shift
            || event.modifiers.platform
            || event.modifiers.control
            || event.modifiers.alt
        {
            return false;
        }
        if let Some(hit) = self.inline_hit(event.position, cx)
            && matches!(hit.kind, Kind::Checkbox | Kind::Tags)
        {
            self.inline_actions.hovered = Some(hit.hovered());
            self.inline_actions.press = Some(InlinePress {
                hit,
                position: event.position,
                selection: self.selection,
                revision: self.snapshot(cx).revision(),
            });
            cx.notify();
            return true;
        }
        false
    }
    pub(super) fn inline_mouse_up(&mut self, position: Point<Pixels>, cx: &mut Context<Self>) {
        let Some(press) = self.inline_actions.press.take() else {
            return;
        };
        if (position.x - press.position.x).abs() > px(4.)
            || (position.y - press.position.y).abs() > px(4.)
            || self.snapshot(cx).revision() != press.revision
            || self
                .inline_hit(position, cx)
                .is_none_or(|h| h.range != press.hit.range)
        {
            return;
        }
        self.selection = press.selection;
        self.sync_selection_utf16(&self.snapshot(cx));
        match press.hit.kind {
            Kind::Checkbox => {
                if let Some(edits) = crate::org_syntax::command::checkbox_transaction(
                    &self.snapshot(cx),
                    press.hit.range,
                ) {
                    self.apply_inline_edits(press.revision, edits, Some(press.hit.range), cx);
                }
            }
            Kind::Tags => self.schedule_inline(press.hit, true, cx),
            _ => {}
        }
    }
    pub(super) fn inline_key(
        &mut self,
        event: &gpui::KeyDownEvent,
        cx: &mut Context<Self>,
    ) -> bool {
        let Some(popup) = self.inline_actions.popup.as_ref() else {
            return false;
        };
        // Inputs own IME and Escape; never intercept their composing keystrokes.
        if popup.picker.read(cx).is_pinned() && event.keystroke.key != "escape" {
            return false;
        }
        popup.picker.update(cx, |p, cx| p.handle_key(event, cx))
    }
}
