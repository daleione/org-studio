//! Key discovery backed by the active keymap, inside the statusline shell.
use std::time::Duration;

use gpui::{Context, Task, Window};

use super::{
    ContentRoute, PaneSide, WorkspaceWindow,
    status_line::shell::{ShellKind, ShellRequest, ShellShape},
};
use crate::{document::DocumentId, input::WhichKeyCandidate};

mod labels;
mod layout;
#[cfg(test)]
mod tests;
mod view;

#[derive(Default)]
pub(crate) struct PrefixHint {
    task: Option<Task<()>>,
    generation: u64,
    presentation: Option<Presentation>,
}

struct Presentation {
    prefix: String,
    candidates: Vec<WhichKeyCandidate>,
    route: ContentRoute,
    pane: PaneSide,
    document: Option<DocumentId>,
    returning: bool,
    layout: layout::Layout,
    scroll: gpui::ScrollHandle,
}

impl WorkspaceWindow {
    pub(crate) fn prefix_hint_visible(&self) -> bool {
        self.prefix_hint.presentation.is_some()
    }

    pub(crate) fn prefix_hint_on(&self, pane: PaneSide) -> bool {
        self.status.shell.owns(ShellKind::Prefix, pane)
    }

    pub(crate) fn finish_prefix_return(&mut self, retained: bool) {
        if !retained
            && self
                .prefix_hint
                .presentation
                .as_ref()
                .is_some_and(|p| p.returning)
        {
            self.prefix_hint.presentation = None;
        }
    }

    /// End the complete interaction, including a hint that has not appeared yet.
    /// Focus is restored before the next render gives a new panel its own focus.
    pub(crate) fn end_prefix(&mut self, cx: &mut Context<Self>) {
        if self.keyboard.pending_keys().is_some() {
            self.keyboard.cancel();
            self.keyboard.dismiss_status();
            self.cancel_key_feedback();
            cx.notify();
        }
        self.cancel_which_key(cx);
    }

    pub(crate) fn schedule_which_key(&mut self, cx: &mut Context<Self>) {
        self.prefix_hint.generation = self.prefix_hint.generation.wrapping_add(1);
        self.prefix_hint.task = None;
        self.file_manager.set_help_visible(false);
        // Once discovery is visible, descend immediately without a second delay or blank frame.
        if self.prefix_hint.presentation.is_some() {
            self.show_prefix_hint(cx);
            return;
        }
        let generation = self.prefix_hint.generation;
        let route = self.content_route;
        let pane = self.document_workspace.active_pane;
        let document = self.document_session().map(|s| s.read(cx).id());
        let delay = cx.background_executor().timer(Duration::from_millis(400));
        self.prefix_hint.task = Some(cx.spawn(async move |this, cx| {
            delay.await;
            let _ = this.update(cx, |this, cx| {
                if this.prefix_hint.generation == generation {
                    this.prefix_hint.task = None;
                    if route == this.content_route
                        && pane == this.document_workspace.active_pane
                        && document == this.document_session().map(|s| s.read(cx).id())
                    {
                        this.show_prefix_hint(cx);
                    } else {
                        this.end_prefix(cx);
                    }
                }
            });
        }));
    }

    fn show_prefix_hint(&mut self, cx: &mut Context<Self>) {
        let Some(prefix) = self.keyboard.pending_keys().map(str::to_owned) else {
            return;
        };
        let candidates = self.keyboard.which_key_candidates();
        if candidates.is_empty() {
            self.cancel_which_key(cx);
            return;
        }
        self.status.dismiss_popover();
        self.prefix_hint.presentation = Some(Presentation {
            prefix,
            candidates,
            route: self.content_route,
            pane: self.document_workspace.active_pane,
            document: self.document_session().map(|s| s.read(cx).id()),
            returning: false,
            layout: layout::Layout::default(),
            scroll: gpui::ScrollHandle::new(),
        });
        cx.notify();
    }

    pub(crate) fn cancel_which_key(&mut self, cx: &mut Context<Self>) {
        let visible = self.prefix_hint_visible() || self.file_manager.help_visible();
        self.prefix_hint.generation = self.prefix_hint.generation.wrapping_add(1);
        self.prefix_hint.task = None;
        self.file_manager.set_help_visible(false);
        if let Some(p) = &mut self.prefix_hint.presentation {
            p.returning = true;
        }
        if visible {
            cx.notify();
        }
    }

    pub(crate) fn prefix_shell_request(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<ShellRequest> {
        let p = self.prefix_hint.presentation.as_ref()?;
        if p.route != self.content_route
            || p.pane != self.document_workspace.active_pane
            || p.document != self.document_session().map(|s| s.read(cx).id())
        {
            self.prefix_hint.presentation = None;
            self.cancel_prefix_input(window, cx);
            return None;
        }
        let viewport = f32::from(window.viewport_size().width);
        let width = if p.route == ContentRoute::Document && p.document.is_some() {
            self.document_pane_width(viewport, p.pane)
        } else {
            viewport
        };
        let available = (width - 2. * super::status_line::FLOATING_STATUS_INSET).max(1.);
        let items = self.prefix_items(p);
        let layout = layout::Layout::new(
            items,
            available,
            f32::from(window.viewport_size().height),
            window,
        );
        let p = self.prefix_hint.presentation.as_mut().unwrap();
        let target = if p.returning {
            ShellShape::status(available)
        } else {
            layout.shape()
        };
        p.layout = layout;
        Some(ShellRequest {
            kind: ShellKind::Prefix,
            pane: p.pane,
            target,
            available,
            returning: p.returning,
        })
    }

    fn prefix_items(&self, p: &Presentation) -> Vec<labels::Item> {
        p.candidates
            .iter()
            .map(|candidate| labels::item(&p.prefix, candidate, &self.commands, self.language))
            .collect()
    }
}
