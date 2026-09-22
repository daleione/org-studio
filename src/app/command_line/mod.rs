mod catalog;
mod run;
#[cfg(test)]
mod tests;
mod view;

use crate::{
    app::{
        ContentRoute, PaneSide, PaneSurface, WorkspaceWindow,
        native_input::{InputConfig, InputEvent, NativeInput},
    },
    document::{DocumentId, Revision, Selection},
};
use catalog::Entry;
use gpui::{AppContext, Context, Entity, KeyDownEvent, Modifiers, Subscription, Task, Window};
use std::time::{Duration, Instant};

const RUNNING_DELAY: Duration = Duration::from_millis(150);
const CANDIDATE_ROW_HEIGHT: f32 = 32.;

#[derive(Default)]
pub(crate) struct CommandLineHost {
    session: Option<Session>,
    history: Vec<String>,
    next_id: u64,
}

enum Phase {
    Input,
    Running(Instant),
    Result {
        message: String,
        undo: Option<Revision>,
    },
}

struct Session {
    id: u64,
    document: DocumentId,
    pane: PaneSide,
    surface: PaneSurface,
    input: Entity<NativeInput>,
    _subscription: Subscription,
    query: String,
    all: Vec<Entry>,
    candidates: Vec<Entry>,
    selected: usize,
    scroll_remainder: f32,
    visible_limit: usize,
    error: Option<String>,
    phase: Phase,
    // Keep the displayed phase while the shared shell animates back to the status line.
    returning: bool,
    focus_pending: bool,
    execute_pending: bool,
    task: Option<Task<()>>,
    timer: Option<Task<()>>,
}

impl Session {
    fn visible_start(&self) -> usize {
        self.selected.saturating_sub(self.visible_limit - 1)
    }

    fn accepts_input(&self, cx: &gpui::App) -> bool {
        !self.returning && matches!(self.phase, Phase::Input) && !self.input.read(cx).is_composing()
    }

    fn set_query(&mut self, query: String, history: &[String]) {
        self.query = query;
        self.candidates = catalog::candidates(&self.all, &self.query, history);
        self.selected = 0;
        self.scroll_remainder = 0.;
        self.error = None;
        self.execute_pending = false;
    }

    fn complete(&mut self, index: usize, history: &[String], cx: &mut gpui::App) {
        if let Some(entry) = self.candidates.get(index) {
            let value = if entry.input == "goto-line" {
                "goto-line ".to_owned()
            } else {
                entry.input.clone()
            };
            self.input.update(cx, |input, cx| input.sync(&value, cx));
            self.set_query(value, history);
        }
    }

    /// Trackpads emit many sub-row deltas. Selection follows distance, not event count.
    fn scroll_candidates(&mut self, pixels: f32, phase: gpui::TouchPhase) -> bool {
        if phase == gpui::TouchPhase::Started || self.scroll_remainder * pixels < 0. {
            self.scroll_remainder = 0.;
        }
        self.scroll_remainder += pixels;
        let steps = (self.scroll_remainder / CANDIDATE_ROW_HEIGHT).trunc() as isize;
        self.scroll_remainder -= steps as f32 * CANDIDATE_ROW_HEIGHT;
        let previous = self.selected;
        let last = self.candidates.len().saturating_sub(1);
        self.selected = self.selected.saturating_add_signed(steps).min(last);
        if phase == gpui::TouchPhase::Ended
            || (self.selected == 0 && pixels < 0.)
            || (self.selected == last && pixels > 0.)
        {
            self.scroll_remainder = 0.;
        }
        self.selected != previous
    }
}

impl WorkspaceWindow {
    pub(crate) fn prompt_goto_line(&mut self, cx: &mut Context<Self>) {
        if !self.command_line_is_open() {
            self.open_command_line(cx);
        }
        if let Some(s) = self.command_line.session.as_mut() {
            let query = "goto-line ";
            s.input.update(cx, |input, cx| input.sync(query, cx));
            s.set_query(query.into(), &self.command_line.history);
            s.focus_pending = true;
            cx.notify();
        }
    }

    pub(crate) fn command_line_is_open(&self) -> bool {
        self.command_line
            .session
            .as_ref()
            .is_some_and(|s| !s.returning)
    }

    pub(crate) fn open_command_line(&mut self, cx: &mut Context<Self>) {
        if self.content_route != ContentRoute::Document || self.buffer_busy() {
            return;
        }
        let Some(doc) = self.document_session().cloned() else {
            return;
        };
        self.end_prefix(cx);
        self.close_search(false, cx);
        self.dismiss_buffer_panel(cx);
        self.status.dismiss_popover();
        let pane = self.document_workspace.active_pane;
        let surface = self.document_workspace.active_surface();
        if let Some(editor) = self.editor(pane) {
            editor.update(cx, |e, cx| e.finish_composition(cx));
        }
        let editing = surface == PaneSurface::Editor;
        let selected = self
            .editor(pane)
            .is_some_and(|e| !e.read(cx).selection().is_empty());
        let all = catalog::entries(
            &self.commands,
            self.language,
            doc.read(cx).is_read_only(),
            editing,
            selected,
        );
        let candidates = catalog::candidates(&all, "", &self.command_line.history);
        let input = cx.new(|cx| {
            NativeInput::new(
                InputConfig {
                    id: "command-line-input",
                    placeholder: self
                        .command_text("输入命令或搜索操作…", "Type a command or find an action…")
                        .into(),
                    font_size: 13.,
                    command: |key, m, _| {
                        matches!(key, "escape" | "enter" | "tab" | "up" | "down")
                            || (m.control && key == "g")
                    },
                    ..Default::default()
                },
                cx,
            )
        });
        let subscription = cx.subscribe(&input, |this, input, event, cx| {
            if !this
                .command_line
                .session
                .as_ref()
                .is_some_and(|s| s.input == input)
            {
                return;
            }
            match event {
                InputEvent::Changed(query) => this.command_query_changed(query.clone(), cx),
                InputEvent::Command { key, modifiers } => {
                    this.command_input_key(key, *modifiers, cx)
                }
                InputEvent::MetricsChanged => cx.notify(),
            }
        });
        self.command_line.next_id = self.command_line.next_id.wrapping_add(1);
        self.command_line.session = Some(Session {
            id: self.command_line.next_id,
            document: doc.read(cx).id(),
            pane,
            surface,
            input,
            _subscription: subscription,
            query: String::new(),
            all,
            candidates,
            selected: 0,
            scroll_remainder: 0.,
            visible_limit: 6,
            error: None,
            phase: Phase::Input,
            returning: false,
            focus_pending: true,
            execute_pending: false,
            task: None,
            timer: None,
        });
        self.cancel_pending_document_focus(cx);
        cx.notify();
    }

    pub(crate) fn close_command_line(&mut self, cx: &mut Context<Self>) {
        let Some(s) = self.command_line.session.as_mut() else {
            return;
        };
        if s.returning {
            return;
        }
        s.returning = true;
        s.execute_pending = false;
        s.task = None;
        s.timer = None;
        s.focus_pending = false;
        s.input.update(cx, |input, cx| {
            input.enabled = false;
            cx.notify();
        });
        self.focus_active_surface(cx);
        cx.notify();
    }

    fn command_text<'a>(&self, zh: &'a str, en: &'a str) -> &'a str {
        if self.language == crate::i18n::Language::Chinese {
            zh
        } else {
            en
        }
    }

    fn command_query_changed(&mut self, query: String, cx: &mut Context<Self>) {
        let Some(s) = self.command_line.session.as_mut() else {
            return;
        };
        if s.returning || !matches!(s.phase, Phase::Input) {
            return;
        }
        s.set_query(query, &self.command_line.history);
        cx.notify();
    }

    fn activate_command_candidate(&mut self, index: usize, cx: &mut Context<Self>) {
        let Some(s) = self.command_line.session.as_mut() else {
            return;
        };
        if !s.accepts_input(cx) || index >= s.candidates.len() {
            return;
        }
        s.complete(index, &self.command_line.history, cx);
        s.execute_pending = true;
        cx.notify();
    }

    fn command_input_key(&mut self, key: &str, m: Modifiers, cx: &mut Context<Self>) {
        let Some(s) = self.command_line.session.as_mut() else {
            return;
        };
        if s.input.read(cx).is_composing() {
            return;
        }
        if key == "escape" || (m.control && key == "g") {
            self.close_command_line(cx);
            return;
        }
        if s.returning || !matches!(s.phase, Phase::Input) {
            return;
        }
        match key {
            "up" if !s.candidates.is_empty() => {
                s.scroll_remainder = 0.;
                s.selected = (s.selected + s.candidates.len() - 1) % s.candidates.len()
            }
            "down" if !s.candidates.is_empty() => {
                s.scroll_remainder = 0.;
                s.selected = (s.selected + 1) % s.candidates.len()
            }
            "tab" => {
                s.complete(s.selected, &self.command_line.history, cx);
            }
            "enter" => s.execute_pending = true,
            _ => {}
        }
        cx.notify();
    }

    /// The native input receives its text and IME events. Only entry/exit shortcuts are captured.
    pub(crate) fn command_line_capture(
        &mut self,
        event: &KeyDownEvent,
        cx: &mut Context<Self>,
    ) -> bool {
        let m = event.keystroke.modifiers;
        let key = event.keystroke.key.as_str();
        let composing = self
            .command_line
            .session
            .as_ref()
            .is_some_and(|s| s.input.read(cx).is_composing())
            || self.search_input_composing(cx)
            || self.buffers.input_composing(cx);
        if composing {
            return self.command_line_is_open();
        }
        let entry = (m.alt && !m.control && !m.platform && key == "x")
            || (m.platform && m.shift && key == "p")
            || (!m.control
                && !m.alt
                && !m.platform
                && self.document_workspace.active_surface() == PaneSurface::Reading
                && !self.search_is_open()
                && self.buffers.panel.is_none()
                && !self.command_line_is_open()
                && (key == ":" || (m.shift && key == ";")));
        if entry {
            self.open_command_line(cx);
            cx.stop_propagation();
            return true;
        }
        // Result feedback does not own the keyboard: resume document shortcuts immediately.
        if self
            .command_line
            .session
            .as_ref()
            .is_some_and(|s| !s.returning && matches!(s.phase, Phase::Result { .. }))
        {
            self.close_command_line(cx);
            return false;
        }
        if self.command_line_is_open() {
            if m.control
                && !m.alt
                && !m.platform
                && !m.shift
                && let Ok(number @ 1..=9) = key.parse::<usize>()
            {
                if let Some(s) = self.command_line.session.as_ref()
                    && number <= s.visible_limit
                {
                    self.activate_command_candidate(s.visible_start() + number - 1, cx);
                }
                cx.stop_propagation();
            } else if (m.platform && key == "f") || (m.control && matches!(key, "s" | "r")) {
                self.open_search(m.control, key == "r", false, cx);
                cx.stop_propagation();
            } else if key == "escape" || (m.control && key == "g") {
                self.close_command_line(cx);
                cx.stop_propagation();
            }
            return true;
        }
        false
    }

    pub(crate) fn command_line_tick(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let valid = self
            .command_line
            .session
            .as_ref()
            .is_none_or(|s| self.command_context_is_current(s, cx));
        if !valid {
            self.command_line.session = None;
            return;
        }
        let Some(s) = self.command_line.session.as_mut() else {
            return;
        };
        if std::mem::take(&mut s.focus_pending) {
            let focus = s.input.read(cx).focus.clone();
            window.focus(&focus, cx);
        }
        if std::mem::take(&mut s.execute_pending) {
            self.execute_command_line(window, cx);
        }
    }

    fn command_context_is_current(&self, s: &Session, cx: &gpui::App) -> bool {
        self.content_route == ContentRoute::Document
            && self.document_workspace.active_pane == s.pane
            && self.document_workspace.active_surface() == s.surface
            && matches!(self.state, crate::app::WorkspaceLoadState::Ready { .. })
            && self
                .document_session()
                .is_some_and(|doc| doc.read(cx).id() == s.document)
    }
}
