//! Heading-only hover host for the reusable TODO menu.
mod edits;
mod geometry;

use super::*;
use crate::{
    components::{
        selection_style::QUICK_BAR_HEIGHT,
        todo_picker::{MORE_HEIGHT, TodoPicker, TodoPickerEvent},
    },
    document::DocumentFormat,
    org_semantic::{OrgFileConfig, extract_file_config},
};

/// All state tied to a TODO hover session lives with its host.
#[derive(Default)]
pub(super) struct TodoInteraction {
    pub(super) popup: Option<TodoPopup>,
    pub(super) hover_range: Option<ByteRange>,
    pub(super) dismissed: Option<ByteRange>,
    pub(super) hover_task: Option<Task<()>>,
    pub(super) dismiss_task: Option<Task<()>>,
    pub(super) config: Option<(Revision, Arc<OrgFileConfig>)>,
}

struct TodoHit {
    range: ByteRange,
    remove_end: ByteOffset,
    keyword: Arc<str>,
    bounds: Bounds<Pixels>,
}
pub(super) struct TodoPopup {
    picker: Entity<TodoPicker>,
    _subscription: Subscription,
    _observation: Subscription,
    interaction_bounds: Option<Bounds<Pixels>>,
    hit: TodoHit,
    revision: Revision,
}

fn heading_keyword(text: &str) -> Option<Range<usize>> {
    let trimmed = text.trim_start();
    let stars = trimmed.bytes().take_while(|b| *b == b'*').count();
    if stars == 0 || trimmed.as_bytes().get(stars) != Some(&b' ') {
        return None;
    }
    let after_stars = &trimmed[stars..];
    let start =
        text.len() - trimmed.len() + stars + after_stars.len() - after_stars.trim_start().len();
    let end = start
        + text[start..]
            .find(char::is_whitespace)
            .unwrap_or(text.len() - start);
    (start < end).then_some(start..end)
}

impl SemanticEditor {
    pub(super) fn todo_highlight(&self) -> Option<ByteRange> {
        self.todo.popup.as_ref().map(|popup| popup.hit.range)
    }
    pub(super) fn todo_hover(&mut self, position: Point<Pixels>, cx: &mut Context<Self>) {
        if self.inline_actions.popup.is_some()
            || self.timestamp.popup.is_some()
            || self.is_selecting
            || self.minimap.drag.is_some()
            || self.minimap.resizing.is_some()
        {
            return;
        }
        if self
            .todo
            .popup
            .as_ref()
            .is_some_and(|p| p.interaction_bounds.is_some_and(|b| b.contains(&position)))
        {
            self.todo.dismiss_task = None;
            return;
        }
        let hit = self.todo_hit(position, cx);
        if hit.is_some() {
            self.todo.dismiss_task = None;
        } else {
            self.todo_pointer_left(cx);
        }
        let range = hit.as_ref().map(|h| h.range);
        if range != self.todo.dismissed {
            self.todo.dismissed = None;
        }
        if range == self.todo.hover_range {
            return;
        }
        self.todo.hover_task = None;
        self.todo.hover_range = range;
        let Some(hit) = hit else {
            return;
        };
        if self.todo.dismissed == Some(hit.range) {
            return;
        }
        let snapshot = self.snapshot(cx);
        let revision = snapshot.revision();
        let cached = self
            .todo
            .config
            .as_ref()
            .filter(|(r, _)| *r == revision)
            .map(|(_, c)| c.clone());
        if let Some(popup) = &self.todo.popup {
            if popup.hit.range == hit.range {
                return;
            }
            // An already-open menu follows another visible status immediately.
            // Moving through ordinary text or into the menu keeps it usable.
            if let Some(config) = cached {
                self.open_todo_picker(hit, config, cx);
                return;
            }
        }
        let executor = cx.background_executor().clone();
        self.todo.hover_task = Some(cx.spawn(async move |this, cx| {
            executor.timer(Duration::from_millis(350)).await;
            // File-wide configuration is parsed off the UI thread, once per revision.
            let config = if let Some(config) = cached {
                config
            } else {
                executor
                    .spawn(async move { Arc::new(extract_file_config(&snapshot)) })
                    .await
            };
            let _ = this.update(cx, |this, cx| {
                if this.snapshot(cx).revision() != revision {
                    return;
                }
                this.todo.config = Some((revision, config.clone()));
                if this.todo.hover_range == Some(hit.range)
                    && !this.is_selecting
                    && this.inline_actions.popup.is_none()
                    && this.timestamp.popup.is_none()
                    && this.todo.popup.is_none()
                    && !this.is_read_only(cx)
                    && config.todo_state(&hit.keyword).is_some()
                {
                    this.open_todo_picker(hit, config, cx);
                }
            });
        }));
    }
    pub(super) fn todo_pointer_left(&mut self, cx: &mut Context<Self>) {
        if self.todo.popup.is_none() || self.todo.dismiss_task.is_some() {
            return;
        }
        let executor = cx.background_executor().clone();
        self.todo.dismiss_task = Some(cx.spawn(async move |this, cx| {
            executor.timer(Duration::from_millis(300)).await;
            let _ = this.update(cx, |this, cx| {
                this.dismiss_todo(cx);
                this.todo.hover_range = None;
                this.autofocus = true;
            });
        }));
    }

    fn open_todo_picker(
        &mut self,
        hit: TodoHit,
        config: Arc<OrgFileConfig>,
        cx: &mut Context<Self>,
    ) {
        self.todo.dismiss_task = None;
        self.todo.config = Some((self.snapshot(cx).revision(), config.clone()));
        let states = config
            .todo_keywords()
            .iter()
            .filter_map(|k| config.todo_state(k).cloned())
            .collect();
        let picker =
            cx.new(|cx| TodoPicker::new(states, hit.keyword.clone(), self.ui_language, cx));
        let subscription = cx.subscribe(&picker, |this, _, event: &TodoPickerEvent, cx| {
            match event {
                TodoPickerEvent::Selected(state) => this.apply_todo(state.as_deref(), cx),
                TodoPickerEvent::Cancelled => this.dismiss_todo(cx),
                TodoPickerEvent::Customize => this.customize_todo(cx),
            }
            this.autofocus = true;
            cx.notify();
        });
        self.timestamp.hover_task = None;
        self.timestamp.hover_range = None;
        self.hovered_link = None;
        self.hover_position = None;
        let observation = cx.observe(&picker, |_, _, cx| cx.notify());
        self.todo.popup = Some(TodoPopup {
            picker,
            _subscription: subscription,
            _observation: observation,
            interaction_bounds: None,
            hit,
            revision: self.snapshot(cx).revision(),
        });
        cx.notify();
    }
    pub(super) fn dismiss_todo(&mut self, cx: &mut Context<Self>) {
        self.todo.dismiss_task = None;
        if let Some(popup) = self.todo.popup.take() {
            self.todo.dismissed = Some(popup.hit.range);
            cx.notify();
        }
        self.todo.hover_task = None;
    }
    pub(super) fn todo_key(&mut self, event: &gpui::KeyDownEvent, cx: &mut Context<Self>) -> bool {
        self.todo.popup.as_ref().is_some_and(|popup| {
            popup
                .picker
                .update(cx, |picker, cx| picker.handle_key(event, cx))
        })
    }
    pub(super) fn set_todo_language(
        &mut self,
        language: crate::i18n::Language,
        cx: &mut Context<Self>,
    ) {
        if let Some(popup) = &self.todo.popup {
            popup
                .picker
                .update(cx, |picker, cx| picker.set_language(language, cx));
        }
    }
}

#[cfg(test)]
mod tests;
