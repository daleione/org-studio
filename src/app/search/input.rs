use super::notice::Notice;
use super::session::Step;
use crate::{
    app::{
        ContentRoute, WorkspaceWindow,
        native_input::{InputConfig, InputEvent, NativeInput},
    },
    search::Completion,
};
use gpui::{AppContext, Context, KeyDownEvent, Modifiers, Window};

impl WorkspaceWindow {
    pub(super) fn create_search_input(
        &mut self,
        replacement: bool,
        cx: &mut Context<Self>,
    ) -> (gpui::Entity<NativeInput>, gpui::Subscription) {
        let config = InputConfig {
            id: if replacement {
                "document-replacement-input"
            } else {
                "document-search-input"
            },
            placeholder: self
                .language
                .text(if replacement {
                    "search.replacement_placeholder"
                } else {
                    "search.query_placeholder"
                })
                .into(),
            outlined: true,
            preserve_newlines: true,
            font_size: 13.,
            command: |key, m, append_only| {
                matches!(key, "escape" | "enter" | "up" | "down")
                    || (m.control && matches!(key, "s" | "r" | "g"))
                    || (m.platform && key == "f")
                    || (append_only && key == "backspace")
            },
        };
        let input = cx.new(|cx| NativeInput::new(config, cx));
        let subscription = cx.subscribe(&input, |w, input, event, cx| match event {
            InputEvent::Changed(value) => {
                w.search_input_changed(input.entity_id(), value.clone(), cx)
            }
            InputEvent::Command { key, modifiers } => {
                w.search_input_key(input.entity_id(), key, *modifiers, cx)
            }
            InputEvent::MetricsChanged => cx.notify(),
        });
        (input, subscription)
    }

    pub(crate) fn search_capture(
        &mut self,
        event: &KeyDownEvent,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.content_route != ContentRoute::Document {
            return;
        }
        let m = event.keystroke.modifiers;
        let key = event.keystroke.key.as_str();
        if let Some(s) = &self.search.session {
            if s.input.read(cx).is_composing() || s.replacement_input.read(cx).is_composing() {
                return;
            }
            if s.mode.confirming() {
                match key {
                    "y" | "space" => self.search_replace(false, cx),
                    "n" => self.search_navigate(false, cx),
                    "!" => self.search_replace(true, cx),
                    "q" | "escape" => self.close_search(false, cx),
                    _ => {}
                }
                cx.stop_propagation();
                return;
            }
            // Native input owns text/IME; the workspace router must not see its keys.
            return;
        }
        let isearch = m.control && matches!(key, "s" | "r");
        let find = m.platform && key == "f";
        let replace = m.alt && (key == "%" || (m.shift && key == "5"));
        if isearch || find || replace {
            self.open_search(isearch, key == "r", replace, cx);
            cx.stop_propagation();
        }
    }

    pub(crate) fn search_owns_input(&self, id: gpui::EntityId) -> bool {
        self.search.session.as_ref().is_some_and(|s| {
            s.input.entity_id() == id
                || (s.mode.replacing() && s.replacement_input.entity_id() == id)
        })
    }

    pub(crate) fn search_input_key(
        &mut self,
        id: gpui::EntityId,
        key: &str,
        modifiers: Modifiers,
        cx: &mut Context<Self>,
    ) {
        if !self.search_owns_input(id) {
            return;
        }
        if let Some(s) = self.search.session.as_mut()
            && s.mode.is_query_replace()
            && key == "enter"
            && s.input.entity_id() == id
        {
            s.replacement_focus_pending = true;
            cx.notify();
            return;
        }
        self.search_key(key, modifiers, cx);
    }

    pub(crate) fn search_input_changed(
        &mut self,
        input_id: gpui::EntityId,
        value: String,
        cx: &mut Context<Self>,
    ) {
        if !self.search_owns_input(input_id) {
            return;
        }
        let Some(s) = self.search.session.as_mut() else {
            return;
        };
        if input_id == s.replacement_input.entity_id() {
            s.replacement = value;
            if s.planning {
                s.generation += 1;
                s.planning = false;
            }
            cx.notify();
            return;
        }
        if s.query.pattern == value {
            return;
        }
        if s.mode.is_incremental() {
            s.steps.push(Step {
                successful: s.query.pattern.is_empty() || s.current_index().is_some(),
                query: s.query.pattern.clone(),
                current: s.current,
                backwards: s.backwards,
                boundary: s.boundary,
            });
        }
        s.pending_navigation.clear();
        s.query.pattern = value;
        if s.mode.is_query_replace() {
            s.progress = s.query.scope.start;
            s.current = None;
        }
        s.after_replace = false;
        s.boundary = false;
        self.start_search(cx);
    }

    pub(crate) fn search_key(&mut self, key: &str, m: Modifiers, cx: &mut Context<Self>) {
        let Some(s) = self.search.session.as_mut() else {
            return;
        };
        if m.platform && key == "f" {
            s.mode = if s.mode.replacing() {
                super::session::SearchMode::Replace
            } else {
                super::session::SearchMode::Find
            };
            s.steps.clear();
            s.input.update(cx, |i, _| i.append_only = false);
            s.focus_pending = true;
            s.replacement_focus_pending = false;
            cx.notify();
            return;
        }
        if key == "escape" {
            if s.more_open {
                s.more_open = false;
                cx.notify();
                return;
            }
            let cancel = s.mode.is_incremental();
            self.close_search(cancel, cx);
            return;
        }
        if m.control && key == "g" {
            if s.mode.is_incremental()
                && s.results
                    .as_ref()
                    .is_some_and(|r| r.matches.is_empty() && r.completion == Completion::Complete)
                && let Some(index) = s.steps.iter().rposition(|v| v.successful)
            {
                let step = s.steps[index].clone();
                s.steps.truncate(index);
                s.query.pattern = step.query;
                s.current = step.current;
                s.backwards = step.backwards;
                s.boundary = step.boundary;
                s.input.update(cx, |i, cx| i.sync(&s.query.pattern, cx));
                self.start_search(cx);
                return;
            }
            self.close_search(true, cx);
            return;
        }
        if key == "backspace" && s.mode.is_incremental() {
            if let Some(step) = s.steps.pop() {
                s.query.pattern = step.query;
                s.current = step.current;
                s.backwards = step.backwards;
                s.boundary = step.boundary;
                s.input.update(cx, |i, cx| i.sync(&s.query.pattern, cx));
                self.start_search(cx);
            }
            return;
        }
        if key == "enter" {
            if s.mode.is_query_replace() && !s.mode.confirming() {
                s.mode = super::session::SearchMode::QueryReplaceConfirm;
                s.notice = Notice::None;
                cx.notify();
                return;
            }
            if s.mode.is_incremental() {
                self.close_search(false, cx)
            } else {
                self.search_navigate(m.shift, cx)
            };
            return;
        }
        if m.control && matches!(key, "s" | "r") {
            self.search_navigate(key == "r", cx)
        } else if s.mode.is_incremental() && matches!(key, "up" | "down") {
            let pane = s.pane;
            self.close_search(false, cx);
            if let Some(editor) = self.editor(pane) {
                editor.update(cx, |e, cx| {
                    e.search_move_vertical(if key == "up" { -1 } else { 1 }, cx)
                });
            }
        }
    }
}
