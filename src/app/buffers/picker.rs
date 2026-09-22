use super::*;
use crate::app::native_input::{InputConfig, InputEvent, NativeInput};
use gpui::{AppContext, Window};

#[derive(Clone)]
pub(crate) struct Candidate {
    pub heading: Option<crate::document::OutlineEntry>,
    pub id: Option<DocumentId>,
    pub name: String,
    pub path: String,
    pub dirty: bool,
    pub current: bool,
}

pub(crate) fn match_score(name: &str, path: &str, query: &str) -> Option<u16> {
    let q = query.trim().to_lowercase();
    let name = name.to_lowercase();
    if q.is_empty() {
        return Some(0);
    }
    if name == q {
        return Some(1000);
    }
    if name.starts_with(&q) {
        return Some(800);
    }
    if name.contains(&q) {
        return Some(600);
    }
    if path.to_lowercase().contains(&q) {
        return Some(300);
    }
    let mut chars = q.chars();
    let mut next = chars.next();
    for c in name.chars() {
        if next == Some(c) {
            next = chars.next();
        }
    }
    next.is_none().then_some(100)
}

fn shortcut_number(key: &str) -> Option<usize> {
    match key.as_bytes() {
        [digit @ b'0'..=b'9'] => Some((digit - b'0') as usize),
        _ => None,
    }
}

impl WorkspaceWindow {
    /// Consume picker shortcuts before global editor actions such as Cmd-1/2/3.
    pub fn intercept_quick_open_shortcuts(
        event: &gpui::KeystrokeEvent,
        window: &mut Window,
        cx: &mut App,
    ) {
        let modifiers = event.keystroke.modifiers;
        let key = event.keystroke.key.as_str();
        if !modifiers.platform || modifiers.alt || modifiers.shift {
            return;
        }
        let Some(number) = shortcut_number(key) else {
            return;
        };
        let Some(workspace) = window.root::<Self>().flatten() else {
            return;
        };
        if !matches!(&workspace.read(cx).buffers.panel, Some(Panel::Picker(p)) if p.intent == PickerIntent::Switch)
        {
            return;
        }
        workspace.update(cx, |this, cx| {
            if this.buffers.input_composing(cx) {
                return;
            }
            this.pick_buffer_number(number, cx);
        });
        // Even a missing candidate must not fall through to a display-mode shortcut.
        cx.stop_propagation();
    }

    fn pick_buffer_number(&mut self, number: usize, cx: &mut Context<Self>) {
        let index = if number == 0 { 9 } else { number - 1 };
        if index < self.buffer_candidates(cx).len() {
            if let Some(Panel::Picker(p)) = &mut self.buffers.panel {
                p.selected = index;
            }
            self.accept_buffer_picker(cx);
        }
    }

    pub(crate) fn open_buffer_picker(&mut self, intent: PickerIntent, cx: &mut Context<Self>) {
        self.close_command_line(cx);
        if self.buffer_busy() {
            return;
        }
        self.end_prefix(cx);
        self.dismiss_buffer_panel(cx);
        let return_search = self.search_is_open();
        self.status.dismiss_popover();
        self.buffers.cycle.clear();
        self.buffers.pane = self.document_workspace.active_pane;
        self.buffers.returning = false;
        let placeholder = self.buffer_text("筛选文件名…", "Filter documents…");
        let input = cx.new(|cx| {
            NativeInput::new(
                InputConfig {
                    id: "buffer-query",
                    placeholder: placeholder.into(),
                    outlined: intent != PickerIntent::Switch,
                    font_size: 13.,
                    command: |key, m, _| {
                        matches!(key, "escape" | "enter" | "up" | "down")
                            || m.control
                                && (matches!(key, "g" | "n" | "p")
                                    || shortcut_number(key).is_some())
                    },
                    ..InputConfig::default()
                },
                cx,
            )
        });
        let subscription = cx.subscribe(&input, |this, _, event, cx| {
            match event {
                InputEvent::Changed(_) => {
                    if let Some(Panel::Picker(p)) = &mut this.buffers.panel {
                        p.selected = 0;
                        p.pending_confirm = false;
                        p.message = None;
                    }
                    this.refresh_file_candidates(cx);
                    this.buffers.scroll.scroll_to_item(0);
                }
                InputEvent::Command { key, modifiers } => match key.as_str() {
                    "escape" | "g" if key == "escape" || modifiers.control => {
                        this.cancel_buffer_panel(cx)
                    }
                    "enter" => this.accept_buffer_picker(cx),
                    "up" | "p" => this.step_buffer_picker(-1, cx),
                    "down" | "n" => this.step_buffer_picker(1, cx),
                    digit if modifiers.control && matches!(&this.buffers.panel, Some(Panel::Picker(p)) if p.intent == PickerIntent::Switch) => {
                        if let Some(number) = shortcut_number(digit) {
                            this.pick_buffer_number(number, cx);
                        }
                    }
                    _ => {}
                },
                InputEvent::MetricsChanged => {}
            }
            cx.notify();
        });
        self.buffers.panel = Some(Panel::Picker(Picker {
            intent,
            input,
            _subscription: subscription,
            selected: 0,
            pending_confirm: false,
            message: None,
            return_search,
            markdown: false,
        }));
        self.buffers.focus_pending = true;
        self.cancel_pending_document_focus(cx);
        self.buffers.scroll = gpui::ScrollHandle::default();
        self.buffers.scroll.scroll_to_item(0);
        if intent == PickerIntent::File {
            let directory = self
                .document_session()
                .and_then(|s| s.read(cx).file_path().and_then(|p| p.parent()))
                .map(PathBuf::from)
                .or_else(|| std::env::var_os("HOME").map(PathBuf::from))
                .unwrap_or_default();
            if let Some(Panel::Picker(p)) = &self.buffers.panel {
                p.input
                    .update(cx, |i, cx| i.sync(&format!("{}/", directory.display()), cx));
            }
            self.refresh_file_candidates(cx);
        }
        cx.notify();
    }

    pub(crate) fn buffer_candidates(&self, cx: &App) -> Vec<Candidate> {
        let Some(Panel::Picker(p)) = &self.buffers.panel else {
            return Vec::new();
        };
        let query = &p.input.read(cx).text;
        if p.intent == PickerIntent::File {
            return self.buffers.file_candidates.clone();
        }
        if p.intent == PickerIntent::Switch
            && let Some(query) = query.trim_start().strip_prefix('@')
        {
            return self.heading_candidates(query, cx);
        }
        let active = self.document_session().map(|s| s.read(cx).id());
        let mut items = self
            .buffer_sessions()
            .map(|s| {
                let s = s.read(cx);
                Candidate {
                    heading: None,
                    id: Some(s.id()),
                    name: s.display_name(),
                    path: s
                        .file_path()
                        .map_or_else(String::new, |p| p.to_string_lossy().into_owned()),
                    dirty: s.is_dirty(),
                    current: active == Some(s.id()),
                }
            })
            .collect::<Vec<_>>();
        if p.intent == PickerIntent::Switch {
            items.extend(
                self.recent_documents
                    .iter()
                    .filter(|r| self.buffer_for_path(&r.path, cx).is_none())
                    .map(|r| Candidate {
                        heading: None,
                        id: None,
                        name: r
                            .path
                            .file_name()
                            .unwrap_or_default()
                            .to_string_lossy()
                            .into_owned(),
                        path: r.path.to_string_lossy().into_owned(),
                        dirty: false,
                        current: false,
                    }),
            );
        }
        let mut scored = items
            .into_iter()
            .filter_map(|c| match_score(&c.name, &c.path, query).map(|score| (score, c)))
            .collect::<Vec<_>>();
        scored.sort_by_key(|(score, _)| std::cmp::Reverse(*score));
        scored.into_iter().map(|(_, c)| c).collect()
    }

    fn heading_candidates(&self, query: &str, cx: &App) -> Vec<Candidate> {
        let Some(session) = self.document_session() else {
            return Vec::new();
        };
        let id = session.read(cx).id();
        let pane = self.document_workspace.active_pane;
        let entries = match self.document_workspace.active_surface() {
            crate::app::PaneSurface::Editor => self
                .editor(pane)
                .map(|editor| editor.read(cx).outline_entries(cx))
                .unwrap_or_default(),
            // Source offsets require the preview of the same revision.
            crate::app::PaneSurface::Reading if self.latest_preview_is_current(cx) => self
                .reading_panel_for(pane)
                .map(|panel| panel.read(cx).document().outline_entries())
                .unwrap_or_default(),
            crate::app::PaneSurface::Reading => Vec::new(),
        };
        let mut parents: Vec<(u16, String)> = Vec::new();
        let mut matches = Vec::new();
        for heading in entries {
            while parents
                .last()
                .is_some_and(|(level, _)| *level >= heading.level)
            {
                parents.pop();
            }
            let path = parents
                .iter()
                .map(|(_, title)| title.as_str())
                .collect::<Vec<_>>()
                .join(" › ");
            let name = heading.title.to_string();
            parents.push((heading.level, name.clone()));
            if let Some(score) = match_score(&name, &path, query) {
                matches.push((
                    score,
                    Candidate {
                        heading: Some(heading),
                        id: Some(id),
                        name,
                        path,
                        dirty: false,
                        current: false,
                    },
                ));
            }
        }
        matches.sort_by_key(|(score, _)| std::cmp::Reverse(*score));
        matches.into_iter().map(|(_, item)| item).collect()
    }

    pub(crate) fn step_buffer_picker(&mut self, step: i32, cx: &mut Context<Self>) {
        let len = self.buffer_candidates(cx).len();
        if len == 0 {
            return;
        }
        if let Some(Panel::Picker(p)) = &mut self.buffers.panel {
            p.selected = (p.selected as i32 + step).rem_euclid(len as i32) as usize;
            self.buffers.scroll.scroll_to_item(p.selected);
        }
        cx.notify();
    }

    pub(crate) fn accept_buffer_picker(&mut self, cx: &mut Context<Self>) {
        let candidates = self.buffer_candidates(cx);
        let Some(Panel::Picker(p)) = &self.buffers.panel else {
            return;
        };
        let intent = p.intent;
        if intent == PickerIntent::File && self.buffers.file_task.is_some() {
            return;
        }
        let mut value = p.input.read(cx).text.trim().to_owned();
        if value.is_empty() && intent == PickerIntent::New {
            self.buffers.draft_serial += 1;
            value = format!(
                "{} {}",
                self.buffer_text("未命名", "Untitled"),
                self.buffers.draft_serial
            );
        }
        if intent != PickerIntent::New
            && let Some(c) = candidates.get(p.selected)
        {
            if let Some(heading) = &c.heading {
                let target = self
                    .document_session()
                    .and_then(|session| session.read(cx).map_range_to_current(heading.source).ok());
                if let Some(target) = target {
                    self.dismiss_buffer_panel(cx);
                    self.navigate_to_source(target.range.start, cx);
                }
                return;
            }
            if let Some(id) = c.id {
                if intent == PickerIntent::Close {
                    self.request_close_buffer(id, cx);
                } else {
                    self.activate_buffer(id, cx);
                }
            } else {
                let path = PathBuf::from(&c.path);
                if path.is_dir() {
                    p.input
                        .update(cx, |i, cx| i.sync(&format!("{}/", path.display()), cx));
                    self.refresh_file_candidates(cx);
                } else {
                    self.dismiss_buffer_panel(cx);
                    self.open(path, cx);
                }
            }
            return;
        }
        if value.is_empty() || matches!(intent, PickerIntent::Close | PickerIntent::Switch) {
            return;
        }
        if intent == PickerIntent::File {
            let path = expand_path(&value);
            if path.is_file() {
                self.dismiss_buffer_panel(cx);
                self.open(path, cx);
                return;
            }
            if path.exists() {
                let message = self
                    .buffer_text("请输入文件路径", "Enter a file path")
                    .to_owned();
                if let Some(Panel::Picker(p)) = &mut self.buffers.panel {
                    p.message = Some(message);
                }
                return;
            }
        }
        if !p.pending_confirm && intent != PickerIntent::New {
            if let Some(Panel::Picker(p)) = &mut self.buffers.panel {
                p.pending_confirm = true;
            }
            cx.notify();
            return;
        }
        if intent == PickerIntent::New && p.markdown && Path::new(&value).extension().is_none() {
            value.push_str(".md");
        }
        self.create_buffer(
            value.clone(),
            (intent == PickerIntent::File).then(|| expand_path(&value)),
            cx,
        );
    }

    pub(crate) fn dismiss_buffer_panel(&mut self, cx: &mut Context<Self>) {
        self.buffers.file_task = None;
        self.buffers.file_request = self.buffers.file_request.wrapping_add(1);
        self.buffers.file_candidates.clear();
        if self.buffers.panel.take().is_some() {
            self.buffers.returning = true;
            self.buffers.focus_pending = false;
            cx.notify();
        }
    }

    pub(crate) fn cancel_buffer_panel(&mut self, cx: &mut Context<Self>) {
        let search = matches!(&self.buffers.panel, Some(Panel::Picker(p)) if p.return_search);
        self.dismiss_buffer_panel(cx);
        if search {
            self.focus_search_input(cx);
        } else {
            self.request_document_focus(cx);
        }
    }

    pub(crate) fn buffer_tick(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.restore_navigation_location(cx);
        self.ensure_buffer_watch(cx);
        let placeholder = match &self.buffers.panel {
            Some(Panel::Picker(p)) if p.intent == PickerIntent::File => {
                self.buffer_text("文件路径…", "File path…")
            }
            Some(Panel::Picker(p)) if p.intent == PickerIntent::New => {
                self.buffer_text("文档名称…", "Document name…")
            }
            Some(Panel::Picker(p)) if p.intent == PickerIntent::Switch => self.buffer_text(
                "搜索文件名或路径 · @ 搜索标题…",
                "Find file or path · @ headings…",
            ),
            _ => self.buffer_text("筛选文件名…", "Filter documents…"),
        };
        if let Some(Panel::Picker(p)) = &self.buffers.panel {
            p.input.update(cx, |i, cx| {
                if i.config.placeholder.as_ref() != placeholder {
                    i.config.placeholder = placeholder.into();
                    cx.notify();
                }
            });
        }
        if self.buffers.focus_pending {
            self.buffers.focus_pending = false;
            if let Some(Panel::Picker(p)) = &self.buffers.panel {
                p.input.read(cx).focus.clone().focus(window, cx);
            } else if let Some(focus) = &self.focus_handle {
                focus.focus(window, cx);
            }
        }
    }

    pub(crate) fn buffer_shell_request(
        &mut self,
        window: &Window,
    ) -> Option<crate::app::status_line::shell::ShellRequest> {
        if self.buffers.panel.is_none() && !self.buffers.returning {
            return None;
        }
        let viewport = f32::from(window.viewport_size().width);
        let has_document = self.content_route == crate::app::ContentRoute::Document
            && self.state.ready().is_some();
        let pane_width = if !has_document {
            viewport
        } else {
            self.document_pane_width(viewport, self.buffers.pane)
        };
        let available = (pane_width - 2. * crate::app::status_line::FLOATING_STATUS_INSET).max(1.);
        self.buffers.available = available;
        self.buffers.height = (f32::from(window.viewport_size().height) - 100.).clamp(150., 410.);
        let target = if self.buffers.panel.is_some() {
            let (width, height) = match &self.buffers.panel {
                Some(Panel::Picker(p)) if p.intent == PickerIntent::New => {
                    (560., if available < 420. { 92. } else { 60. })
                }
                Some(Panel::Picker(p)) if p.intent == PickerIntent::Switch => {
                    (760., self.buffers.height.min(310.))
                }
                _ => (650., self.buffers.height),
            };
            crate::app::status_line::shell::ShellShape {
                width: available.min(width),
                height,
                expansion_height: 0.,
                content_opacity: 1.,
            }
        } else {
            crate::app::status_line::shell::ShellShape::status(available)
        };
        Some(crate::app::status_line::shell::ShellRequest {
            kind: crate::app::status_line::shell::ShellKind::Buffers,
            pane: self.buffers.pane,
            target,
            available,
            returning: self.buffers.returning,
        })
    }

    fn refresh_file_candidates(&mut self, cx: &mut Context<Self>) {
        let Some(Panel::Picker(p)) = &self.buffers.panel else {
            return;
        };
        if p.intent != PickerIntent::File {
            return;
        }
        let value = p.input.read(cx).text.clone();
        let path = expand_path(&value);
        let directory = if value.ends_with('/') {
            path.clone()
        } else {
            path.parent().map(PathBuf::from).unwrap_or_default()
        };
        let query = if value.ends_with('/') {
            String::new()
        } else {
            path.file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .into_owned()
        };
        self.buffers.file_request = self.buffers.file_request.wrapping_add(1);
        let request = self.buffers.file_request;
        self.buffers.file_candidates.clear();
        let load = cx.background_executor().spawn(async move {
            let mut entries = std::fs::read_dir(directory)?
                .filter_map(Result::ok)
                .filter_map(|entry| {
                    let name = entry.file_name().to_string_lossy().into_owned();
                    if query.is_empty() && name.starts_with('.') {
                        return None;
                    }
                    let path = entry.path();
                    let directory = path.is_dir();
                    if !directory && !crate::preview::is_supported_document(&path) {
                        return None;
                    }
                    let score = match_score(&name, "", &query)?;
                    Some((
                        !directory,
                        std::cmp::Reverse(score),
                        name.to_lowercase(),
                        Candidate {
                            heading: None,
                            id: None,
                            name: if directory { format!("{name}/") } else { name },
                            path: path.to_string_lossy().into_owned(),
                            dirty: false,
                            current: false,
                        },
                    ))
                })
                .collect::<Vec<_>>();
            entries.sort_by(|a, b| (&a.0, &a.1, &a.2).cmp(&(&b.0, &b.1, &b.2)));
            Ok::<_, std::io::Error>(
                entries
                    .into_iter()
                    .map(|(_, _, _, c)| c)
                    .collect::<Vec<_>>(),
            )
        });
        self.buffers.file_task = Some(cx.spawn(async move |this, cx| {
            let result = load.await;
            let _ = this.update(cx, |this, cx| {
                if request != this.buffers.file_request {
                    return;
                }
                match result {
                    Ok(items) => this.buffers.file_candidates = items,
                    Err(error) => {
                        if let Some(Panel::Picker(p)) = &mut this.buffers.panel {
                            p.message = Some(error.to_string());
                        }
                    }
                }
                if let Some(Panel::Picker(p)) = &mut this.buffers.panel {
                    p.selected = 0;
                }
                this.buffers.file_task = None;
                cx.notify();
            });
        }));
    }
}

fn expand_path(value: &str) -> PathBuf {
    if let Some(tail) = value.strip_prefix("~/") {
        return std::env::var_os("HOME")
            .map(PathBuf::from)
            .unwrap_or_default()
            .join(tail);
    }
    let path = PathBuf::from(value);
    if path.is_absolute() {
        path
    } else {
        std::env::current_dir().unwrap_or_default().join(path)
    }
}
