use super::{geometry::BarLayout, presentation::PresentationPhase};
use crate::app::status_line::{
    FLOATING_STATUS_HEIGHT, FLOATING_STATUS_INSET, floating_status_container,
};
use crate::{
    app::{PaneSide, PaneSurface, WorkspaceWindow, component::ModeSwitch},
    document::{ByteOffset, ByteRange, TextSnapshot},
    search::CaseMode,
};
use gpui::{Context, Entity, MouseButton, Window, div, prelude::*, px, rgb};
use std::time::Instant;

#[derive(Clone, Copy)]
enum Action {
    Previous,
    Next,
    Close,
    Case(CaseMode),
    Find,
    Replace,
    ReplaceOne,
    ReplaceAll,
    Source,
    Selection,
    Full,
    More,
    Clear,
}

#[derive(Clone, Copy)]
enum ToolbarButton {
    Previous,
    Next,
    More,
    Close,
    ReplaceOne,
    ReplaceAll,
}

#[derive(Clone, Copy)]
enum SearchIcon {
    Up,
    Down,
    More,
    Close,
    Check,
}

fn search_icon(icon: SearchIcon) -> gpui::Svg {
    let data: &'static [u8] = match icon {
        SearchIcon::Up => br#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 16 16"><path d="m4 10 4-4 4 4" fill="none" stroke="black" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round"/></svg>"#,
        SearchIcon::Down => br#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 16 16"><path d="m4 6 4 4 4-4" fill="none" stroke="black" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round"/></svg>"#,
        SearchIcon::More => br#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 16 16"><g fill="black"><circle cx="3" cy="8" r="1"/><circle cx="8" cy="8" r="1"/><circle cx="13" cy="8" r="1"/></g></svg>"#,
        SearchIcon::Close => br#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 16 16"><path d="m4.5 4.5 7 7m-7 0 7-7" fill="none" stroke="black" stroke-width="1.5" stroke-linecap="round"/></svg>"#,
        SearchIcon::Check => br#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 16 16"><path d="m3.5 8 3 3 6-6" fill="none" stroke="black" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round"/></svg>"#,
    };
    // GPUI's SVG painter requires a color on the SVG itself; parent text color is insufficient.
    gpui::svg()
        .data(data)
        .size(px(16.))
        .flex_none()
        .text_color(rgb(0x626e80))
}

#[cfg(test)]
#[test]
fn search_icons_supply_the_color_required_by_gpui_svg_painting() {
    for kind in [
        SearchIcon::Up,
        SearchIcon::Down,
        SearchIcon::More,
        SearchIcon::Close,
        SearchIcon::Check,
    ] {
        let mut icon = search_icon(kind);
        assert!(
            icon.style().text.color.is_some(),
            "SVG painting skips icons without their own color"
        );
    }
}

struct SearchTooltip(&'static str);
impl gpui::Render for SearchTooltip {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div()
            .px(px(9.))
            .py(px(6.))
            .rounded(px(6.))
            .border_1()
            .border_color(rgb(0xdce4ee))
            .bg(rgb(0xffffff))
            .shadow_md()
            .text_size(px(12.))
            .text_color(rgb(0x626e80))
            .child(self.0)
    }
}

impl WorkspaceWindow {
    pub(crate) fn search_bar(
        &self,
        pane: PaneSide,
        entity: Entity<Self>,
        width: f32,
        cx: &gpui::App,
        status_content: Option<gpui::AnyElement>,
    ) -> Option<gpui::Div> {
        let presentation = self
            .search
            .presentation
            .as_ref()
            .filter(|p| p.pane == pane)?;
        let session = self.search.session.as_ref().filter(|s| s.pane == pane);
        let snapshot = match &presentation.phase {
            PresentationPhase::Search => session?.bar_snapshot(self.language, cx),
            PresentationPhase::Returning(snapshot) => snapshot.clone(),
        };
        let s = &snapshot;
        let interactive = session.is_some();
        let pane_width = width;
        let width = (width - 2. * FLOATING_STATUS_INSET).max(1.);
        let shape = presentation.motion.sample(width, Instant::now()).0;
        let rendered_width = shape.width;
        let height = shape.height;
        let inset = (pane_width - rendered_width) / 2.;
        let layout = BarLayout::new(width, self.language);
        let narrow = layout.narrow;
        let controls_width = layout.controls_width;
        let mode_item_width = layout.mode_item_width;
        let mode_width = layout.mode_width();
        let reveal = (shape.replacement / layout.replacement_height).clamp(0., 1.);
        let animating = (shape.replacement
            - if s.replacement_expanded {
                layout.replacement_height
            } else {
                0.
            })
        .abs()
            > 0.5;
        let can_navigate = s.can_navigate;
        let can_replace = s.can_replace;
        let can_replace_all = s.can_replace_all;
        let field = |replacement: bool| -> gpui::AnyElement {
            if let Some(session) = session {
                if replacement {
                    session.replacement_input.clone().into_any_element()
                } else {
                    session.input.clone().into_any_element()
                }
            } else {
                let text = if replacement {
                    s.replacement.clone()
                } else {
                    s.query.clone()
                };
                let empty = if replacement {
                    s.replacement_empty
                } else {
                    s.query_empty
                };
                crate::app::native_input::input_frame(true, !replacement && s.failed)
                    .text_size(px(13.))
                    .text_color(rgb(if empty { 0x858990 } else { 0x34373d }))
                    .child(
                        div()
                            .min_w_0()
                            .overflow_hidden()
                            .text_ellipsis()
                            .child(text),
                    )
                    .into_any_element()
            }
        };
        let button = |kind: ToolbarButton, enabled: bool| {
            let entity = entity.clone();
            let (name, label, hint, icon, action) = match kind {
                ToolbarButton::Previous => (
                    "previous",
                    "",
                    self.language.text("search.previous_hint"),
                    Some(SearchIcon::Up),
                    Action::Previous,
                ),
                ToolbarButton::Next => (
                    "next",
                    "",
                    self.language.text("search.next_hint"),
                    Some(SearchIcon::Down),
                    Action::Next,
                ),
                ToolbarButton::More => (
                    "more",
                    "",
                    self.language.text("search.options"),
                    Some(SearchIcon::More),
                    Action::More,
                ),
                ToolbarButton::Close => (
                    "close",
                    "",
                    self.language.text("search.close_hint"),
                    Some(SearchIcon::Close),
                    Action::Close,
                ),
                ToolbarButton::ReplaceOne => (
                    "replace-one",
                    self.language.text("search.replace_one"),
                    self.language.text("search.replace_one_hint"),
                    None,
                    Action::ReplaceOne,
                ),
                ToolbarButton::ReplaceAll => (
                    "replace-all",
                    if s.query_replace {
                        self.language.text("search.replace_remaining")
                    } else {
                        self.language.text("search.replace_all")
                    },
                    self.language.text("search.replace_all_hint"),
                    None,
                    Action::ReplaceAll,
                ),
            };
            div()
                .id(name)
                .debug_selector(move || format!("search-{name}"))
                .flex_none()
                .h(px(28.))
                .px(px(if narrow { 5. } else { 7. }))
                .when(icon.is_some(), |button| {
                    button.w(px(if narrow { 24. } else { 28. })).px_0()
                })
                .flex()
                .items_center()
                .justify_center()
                .rounded(px(5.))
                .text_size(px(12.))
                .text_color(rgb(0x626e80))
                .when(matches!(action, Action::ReplaceAll), |b| {
                    b.bg(rgb(0x3266d5)).text_color(rgb(0xffffff))
                })
                .when(!enabled, |b| b.opacity(0.35))
                .when(matches!(action, Action::More) && s.more_open, |b| {
                    b.bg(rgb(0xedf2ff)).text_color(rgb(0x3266d5))
                })
                .when(enabled && interactive, |b| {
                    b.cursor_pointer()
                        .hover(|style| style.bg(rgb(0xe4ebf8)).text_color(rgb(0x3266d5)))
                })
                .when_some(icon, |b, icon| b.child(search_icon(icon)))
                .when(icon.is_none(), |b| b.child(label))
                .when(interactive, |b| {
                    b.tooltip(move |_, cx| cx.new(|_| SearchTooltip(hint)).into())
                        .on_mouse_down(MouseButton::Left, move |_, _, cx| {
                            cx.stop_propagation();
                            if enabled {
                                entity.update(cx, |w, cx| w.search_action(action, pane, cx));
                            }
                        })
                })
        };
        let mode_switch = ModeSwitch::new(
            "document-search-mode",
            usize::from(s.replacement_expanded),
            px(mode_item_width),
        )
        .motion_enabled(interactive && !cx.reduce_motion());
        let modes = [
            (
                self.language.text("search.find"),
                Action::Find,
                "search-mode-find",
            ),
            (
                self.language.text("search.replace"),
                Action::Replace,
                "search-mode-replace",
            ),
        ]
        .into_iter()
        .enumerate()
        .map(|(index, (label, action, name))| {
            let entity = entity.clone();
            let enabled = !s.planning;
            mode_switch
                .item(index, name, |_| label)
                .debug_selector(move || name.to_owned())
                .text_size(px(12.))
                .when(!enabled, |item| item.opacity(0.35))
                .when(interactive, |item| {
                    item.on_mouse_down(MouseButton::Left, move |_, _, cx| {
                        cx.stop_propagation();
                        if enabled {
                            entity.update(cx, |w, cx| w.search_action(action, pane, cx));
                        }
                    })
                })
        });
        let row = div()
            .debug_selector(|| "search-query-row".to_owned())
            .flex_none()
            .w_full()
            .h(px(40.))
            .px(px(7.))
            .flex()
            .gap(px(layout.column_gap))
            .items_center()
            .child(mode_switch.render(modes))
            .child(div().flex_1().min_w_0().h(px(30.)).child(field(false)))
            .child(
                div()
                    .flex_none()
                    .w(px(controls_width))
                    .flex()
                    .gap(px(4.))
                    .items_center()
                    .justify_end()
                    .child(
                        div()
                            .debug_selector(|| "search-result-count".to_owned())
                            .flex_none()
                            .flex_1()
                            .min_w_0()
                            .text_center()
                            .whitespace_nowrap()
                            .overflow_hidden()
                            .text_ellipsis()
                            .text_size(px(12.))
                            .text_color(rgb(if s.failed { 0xb43b42 } else { 0x60718c }))
                            .child(s.count.clone()),
                    )
                    .child(button(ToolbarButton::Previous, can_navigate))
                    .child(button(ToolbarButton::Next, can_navigate))
                    .child(button(ToolbarButton::More, true))
                    .child(button(ToolbarButton::Close, true)),
            );
        let mut content = div().w_full().flex().flex_col().child(row);
        if reveal > 0. || s.replacement_expanded {
            let actions = || {
                div()
                    .flex_none()
                    .when(!narrow, |actions| actions.w(px(controls_width)))
                    .flex()
                    .justify_end()
                    .gap(px(5.))
                    .child(button(ToolbarButton::ReplaceOne, can_replace && !animating))
                    .child(button(
                        ToolbarButton::ReplaceAll,
                        can_replace_all && !animating,
                    ))
            };
            let replacement = div()
                .w_full()
                .h(px(41.))
                .px(px(7.))
                .pb(px(7.))
                .flex()
                .gap(px(layout.column_gap))
                .items_center()
                .when(!narrow, |row| {
                    row.child(
                        div()
                            .flex_none()
                            .w(px(mode_width))
                            .text_size(px(12.))
                            .text_color(rgb(0x737986))
                            .text_center()
                            .child(self.language.text("search.replace_with")),
                    )
                })
                .child(div().flex_1().min_w_0().h(px(30.)).child(field(true)))
                .when(!narrow, |row| row.child(actions()));
            let height = layout.replacement_height;
            content = content.child(
                div()
                    .debug_selector(|| "search-replacement-reveal".to_owned())
                    .flex_none()
                    .w_full()
                    .h(px(height * reveal))
                    .overflow_hidden()
                    .opacity(reveal)
                    .child(
                        div()
                            .w_full()
                            .h(px(height))
                            .flex()
                            .flex_col()
                            .child(replacement)
                            .when(narrow, |body| {
                                body.child(
                                    div()
                                        .h(px(35.))
                                        .px(px(7.))
                                        .flex()
                                        .justify_end()
                                        .child(actions()),
                                )
                            }),
                    ),
            );
        }
        if let Some(detail) = &s.detail {
            content = content.child(
                div()
                    .flex_none()
                    .h(px(26.))
                    .px(px(12.))
                    .overflow_hidden()
                    .text_size(px(11.))
                    .text_color(rgb(if s.range_blocked { 0xb43b42 } else { 0x737986 }))
                    .child(detail.to_owned()),
            );
        }
        Some(
            floating_status_container(height)
                .left(px(inset))
                .right(px(inset))
                .rounded(px(10. + 2. * reveal))
                .text_size(px(12.))
                .font_family(".SystemUIFont")
                .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                .child(content.opacity(shape.search_opacity))
                .when_some(status_content, |shell, status| {
                    shell.child(
                        div()
                            .debug_selector(|| "search-returning-status-content".to_owned())
                            .absolute()
                            .bottom_0()
                            .left(px((rendered_width - width) / 2.))
                            .w(px((width - 2.).max(0.)))
                            .h(px(FLOATING_STATUS_HEIGHT - 2.))
                            .opacity(1. - shape.search_opacity)
                            .child(status),
                    )
                }),
        )
    }

    pub(crate) fn search_is_closing(&self, pane: PaneSide) -> bool {
        self.search
            .presentation
            .as_ref()
            .is_some_and(|p| p.pane == pane && p.returning())
    }

    pub(crate) fn search_options_menu(
        &self,
        pane: PaneSide,
        entity: Entity<Self>,
        width: f32,
        window: &Window,
        cx: &gpui::App,
    ) -> Option<gpui::Div> {
        let s = self
            .search
            .session
            .as_ref()
            .filter(|s| s.pane == pane && s.more_open)?;
        let bar_width = (width - 2. * FLOATING_STATUS_INSET).max(1.);
        let p = self.search.presentation.as_ref()?;
        let shape = p.motion.sample(bar_width, Instant::now()).0;
        let inset = (width - shape.width) / 2.;
        let height = shape.height;
        let has_selection = s.surface == PaneSurface::Editor
            && self
                .editor(pane)
                .is_some_and(|e| !e.read(cx).selection().is_empty());
        let row = |name: &'static str,
                   label: &'static str,
                   action: Action,
                   selected: bool,
                   enabled: bool| {
            let entity = entity.clone();
            div()
                .id(name)
                .debug_selector(move || format!("search-option-{name}"))
                .h(px(30.))
                .px(px(8.))
                .flex()
                .items_center()
                .gap(px(8.))
                .rounded(px(5.))
                .text_size(px(12.))
                .text_color(rgb(0x3f4958))
                .when(!enabled, |row| row.opacity(0.35))
                .when(enabled, |row| {
                    row.cursor_pointer().hover(|style| style.bg(rgb(0xedf2ff)))
                })
                .child(div().w(px(16.)).h(px(16.)).when(selected, |slot| {
                    slot.child(search_icon(SearchIcon::Check).text_color(rgb(0x3266d5)))
                }))
                .child(label)
                .on_mouse_down(MouseButton::Left, move |_, _, cx| {
                    cx.stop_propagation();
                    if enabled {
                        entity.update(cx, |w, cx| w.search_action(action, pane, cx));
                    }
                })
        };
        let heading = |label: &'static str| {
            div()
                .h(px(24.))
                .px(px(8.))
                .flex()
                .items_center()
                .text_size(px(11.))
                .text_color(rgb(0x7e8795))
                .child(label)
        };
        let separator = || div().h(px(1.)).mx(px(8.)).my(px(5.)).bg(rgb(0xe9edf2));
        let mut menu = div()
            .id("search-options")
            .debug_selector(|| "search-options-menu".to_owned())
            .absolute()
            .right(px(inset))
            .bottom(px(crate::app::status_line::FLOATING_STATUS_BOTTOM
                + height
                + 8.))
            .w(px(236_f32.min(bar_width)))
            .max_h(px((f32::from(window.viewport_size().height)
                - height
                - 110.)
                .max(120.)))
            .overflow_y_scroll()
            .p(px(6.))
            .rounded(px(10.))
            .border_1()
            .border_color(rgb(0xdce4ee))
            .bg(rgb(0xffffff))
            .shadow_lg()
            .font_family(".SystemUIFont")
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .child(heading(self.language.text("search.case")))
            .child(row(
                "case-auto",
                self.language.text("search.case_auto"),
                Action::Case(CaseMode::Auto),
                s.query.case == CaseMode::Auto,
                !s.planning,
            ))
            .child(row(
                "case-sensitive",
                self.language.text("search.case_sensitive"),
                Action::Case(CaseMode::Sensitive),
                s.query.case == CaseMode::Sensitive,
                !s.planning,
            ))
            .child(row(
                "case-insensitive",
                self.language.text("search.case_insensitive"),
                Action::Case(CaseMode::Insensitive),
                s.query.case == CaseMode::Insensitive,
                !s.planning,
            ))
            .child(separator())
            .child(heading(self.language.text("search.scope")))
            .child(row(
                "full",
                self.language.text("search.scope_full"),
                Action::Full,
                s.scope == super::session::SearchScope::WholeDocument,
                !s.planning,
            ))
            .child(row(
                "selection",
                self.language.text("search.scope_selection"),
                Action::Selection,
                s.scope == super::session::SearchScope::Selection,
                has_selection && !s.planning,
            ))
            .child(separator())
            .child(row(
                "clear",
                self.language.text("search.clear"),
                Action::Clear,
                false,
                !s.query.pattern.is_empty() && !s.planning,
            ));
        if s.surface == PaneSurface::Reading {
            menu = menu.child(row(
                "source",
                self.language.text("search.source"),
                Action::Source,
                false,
                true,
            ));
            if let Some(range) = s.current
                && let Some(doc) = self.document_session()
            {
                let snapshot = doc.read(cx).snapshot();
                let mut end = ByteOffset((range.start.0 + 160).min(range.end.0));
                while end > range.start && snapshot.byte_to_utf16(end).is_err() {
                    end.0 -= 1;
                }
                let snippet = snapshot
                    .copy_range(ByteRange::new(range.start.0, end.0))
                    .replace(['\n', '\r', '\t'], " ");
                menu = menu.child(
                    div()
                        .px(px(8.))
                        .py(px(5.))
                        .text_size(px(11.))
                        .text_color(rgb(0x7e8795))
                        .overflow_hidden()
                        .child(
                            self.language
                                .text("search.source_snippet")
                                .replace(
                                    "{line}",
                                    &(snapshot.line_of_byte(range.start) + 1).to_string(),
                                )
                                .replace("{snippet}", &snippet),
                        ),
                );
            }
        }
        Some(
            div()
                .block_mouse_except_scroll()
                .absolute()
                .top_0()
                .left_0()
                .size_full()
                .on_mouse_down(MouseButton::Left, move |_, _, cx| {
                    cx.stop_propagation();
                    entity.update(cx, |w, cx| {
                        if let Some(s) = w.search.session.as_mut() {
                            s.more_open = false;
                        }
                        cx.notify();
                    });
                })
                .child(menu),
        )
    }

    fn search_action(&mut self, action: Action, pane: PaneSide, cx: &mut Context<Self>) {
        if !matches!(action, Action::More)
            && let Some(s) = self.search.session.as_mut()
        {
            s.more_open = false;
        }
        match action {
            Action::Previous => self.search_navigate(true, cx),
            Action::Next => self.search_navigate(false, cx),
            Action::Close => self.close_search(
                self.search
                    .session
                    .as_ref()
                    .is_some_and(|s| s.mode.is_incremental()),
                cx,
            ),
            Action::Find | Action::Replace => {
                let expand = matches!(action, Action::Replace);
                if self
                    .search
                    .session
                    .as_ref()
                    .is_some_and(|s| s.mode.replacing() != expand)
                {
                    self.search_toggle_replacement(cx);
                }
            }
            Action::ReplaceOne => self.search_replace(false, cx),
            Action::ReplaceAll => self.search_replace(true, cx),
            Action::Case(case) => {
                if let Some(s) = self.search.session.as_mut() {
                    s.query.case = case;
                }
                self.start_search(cx);
            }
            Action::More => {
                if let Some(s) = self.search.session.as_mut() {
                    s.more_open = !s.more_open;
                }
                cx.notify();
            }
            Action::Clear => {
                if let Some(s) = self.search.session.as_mut() {
                    s.input.update(cx, |i, cx| i.sync("", cx));
                    s.query.pattern.clear();
                    s.current = None;
                    s.steps.clear();
                    s.pending_navigation.clear();
                    s.boundary = false;
                    s.after_replace = false;
                    s.progress = s.query.scope.start;
                    s.mode.stop_confirming();
                    s.focus_pending = true;
                }
                self.start_search(cx);
            }
            Action::Selection | Action::Full => {
                let selection = self.editor(pane).map(|e| e.read(cx).selection());
                let snapshot = self.document_session().map(|d| d.read(cx).snapshot());
                if let (Some(s), Some(snapshot)) = (self.search.session.as_mut(), snapshot) {
                    if matches!(action, Action::Selection) {
                        let Some(selection) = selection.filter(|s| !s.is_empty()) else {
                            return;
                        };
                        s.query.scope = selection.range();
                        s.scope = super::session::SearchScope::Selection;
                    } else {
                        s.query.scope = ByteRange::new(0, snapshot.len_bytes());
                        s.scope = super::session::SearchScope::WholeDocument;
                    }
                    s.range_blocked = false;
                    s.progress = s.query.scope.start;
                    s.current = None;
                    s.mode.stop_confirming();
                    self.start_search(cx);
                }
            }
            Action::Source => self.search_show_source(cx),
        }
    }
}
