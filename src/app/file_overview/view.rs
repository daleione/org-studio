use super::*;
use crate::{
    components::UiTextExt,
    document::{ByteOffset, TextSnapshot},
    theme::current_theme,
    typography::{ui_font, ui_px},
};
use gpui::{
    AnyElement, App, ElementInputHandler, IntoElement, MouseButton, Role, SharedString, Window,
    canvas, div, prelude::*, px, rgb,
};

pub(crate) fn toolbar_button(entity: Entity<WorkspaceWindow>) -> gpui::Stateful<gpui::Div> {
    let theme = current_theme();
    let accessible_entity = entity.clone();
    div()
        .id("document-titlebar-file-overview")
        .debug_selector(|| "document-titlebar-file-overview".into())
        .role(Role::Button)
        .aria_label("文件总览 / Open Files")
        .size(px(32.))
        .flex()
        .items_center()
        .justify_center()
        .rounded_full()
        .bg(rgb(theme.background_alt))
        .cursor_pointer()
        .hover(|style| style.bg(rgb(theme.code_active_background)))
        .tooltip(|_, cx| cx.new(|_| Hint("文件总览 / Open Files".into())).into())
        .on_a11y_action(gpui::AccessibleAction::Click, move |_, window, cx| {
            accessible_entity.update(cx, |this, cx| this.open_file_overview(window, cx));
        })
        .child(
            gpui::svg()
                .data(include_bytes!("../assets/overview.svg"))
                .size(px(17.))
                .text_color(rgb(theme.foreground)),
        )
        .on_click(move |_, window, cx| {
            entity.update(cx, |this, cx| this.open_file_overview(window, cx));
        })
}

struct Hint(SharedString);
impl gpui::Render for Hint {
    fn render(&mut self, _: &mut Window, _: &mut gpui::Context<Self>) -> impl IntoElement {
        let theme = current_theme();
        div()
            .max_w(px(520.))
            .px(px(10.))
            .py(px(7.))
            .rounded(px(6.))
            .bg(rgb(theme.elevated))
            .text_color(rgb(theme.foreground))
            .text_size(ui_px(12.))
            .font(ui_font(".SystemUIFont"))
            .child_text(self.0.clone())
    }
}

impl WorkspaceWindow {
    /// Runs after painting the real body, so the next render can retain the
    /// correct previous document even when the active session has changed.
    pub(crate) fn overview_presented_marker(&self, cx: &gpui::Context<Self>) -> AnyElement {
        let entity = cx.entity();
        canvas(
            |_, _, _| (),
            move |_, _, _, cx| {
                entity.update(cx, |this, _| {
                    if this.file_overview_is_open()
                        || this.content_route != ContentRoute::Document
                        || !matches!(this.state, super::super::WorkspaceLoadState::Ready { .. })
                    {
                        this.file_overview.presented = None;
                    }
                    #[cfg(feature = "visual-check")]
                    if !this.file_overview_is_open() {
                        if let Some(presented) = this.file_overview.presented {
                            this.file_overview.trace.paint(
                                "Document",
                                0.,
                                0.,
                                0.,
                                presented.bounds,
                            );
                        }
                    }
                });
            },
        )
        .absolute()
        .top_0()
        .left_0()
        .size(px(1.))
        .into_any_element()
    }

    pub(crate) fn overview_stage_marker(&self, entity: Entity<Self>, ready: bool) -> AnyElement {
        canvas(
            |_, _, _| (),
            move |bounds, _, window, cx| {
                entity.update(cx, |this, cx| {
                    if !matches!(this.state, super::super::WorkspaceLoadState::Ready { .. }) {
                        return;
                    }
                    let document = this.document_session().map(|session| {
                        let doc = session.read(cx);
                        (doc.id(), doc.revision())
                    });
                    if let Some(state) = this.file_overview.session.as_mut() {
                        if state.phase == Phase::Handoff
                            && state.target == document.map(|doc| doc.0)
                        {
                            state.handoff_frames = if ready {
                                state.handoff_frames.saturating_add(1)
                            } else {
                                0
                            };
                        }
                        return;
                    }
                    this.file_overview.presented = document.map(|(id, revision)| Presented {
                        id,
                        revision,
                        bounds,
                        scale: window.scale_factor(),
                        palette: current_theme().background,
                    });
                    if let Some(presented) = this.file_overview.presented {
                        this.file_overview
                            .frame_sizes
                            .insert(presented.id, bounds.size);
                    }
                });
            },
        )
        .absolute()
        .inset_0()
        .into_any_element()
    }

    pub(crate) fn render_file_overview(
        &self,
        window: &Window,
        cx: &gpui::Context<Self>,
    ) -> Option<AnyElement> {
        let state = self.file_overview.session.as_ref()?;
        let items = self.overview_items(cx);
        let grid = self.overview_grid(window, &items);
        let theme = current_theme();
        let entity = cx.entity();
        let entry = matches!(state.phase, Phase::Entering | Phase::Returning);
        let opening = matches!(
            state.phase,
            Phase::Opening | Phase::Rebounding | Phase::Handoff
        );
        let handoff = state.phase == Phase::Handoff;
        let p = state.progress;
        let appearance = if entry {
            p
        } else if opening {
            1. - p
        } else {
            1.
        };
        let chrome_opacity = smoothstep(0.62, 0.94, appearance);
        let origin_card = items
            .iter()
            .position(|item| Some(item.id) == state.origin)
            .map(|index| {
                grid.document_preview(index, state.scroll.offset().y, state.origin_bounds.size)
            })
            .unwrap_or(state.origin_card);
        let active_frame = if entry {
            state.origin_frame.as_ref().map(|frame| {
                let target = layout::contain(frame.source.bounds.size, origin_card);
                (
                    frame,
                    layout::document_transition(state.origin_bounds, target, p),
                )
            })
        } else if opening {
            state.target_frame.as_ref().map(|frame| {
                let progress = if handoff { 1. } else { p };
                (
                    frame,
                    layout::document_transition(state.target_from, state.target_to, progress),
                )
            })
        } else {
            None
        };
        let transition = if !state.reduced_motion && !handoff {
            active_frame.as_ref().map(|(_, bounds)| {
                (
                    if entry {
                        origin_card
                    } else {
                        state.target_from
                    },
                    *bounds,
                )
            })
        } else {
            None
        };
        let grid_top = grid.header * if transition.is_some() { appearance } else { 1. };
        let grid_bottom = layout::FOOTER * if transition.is_some() { appearance } else { 1. };
        let root_opacity = if state.reduced_motion {
            if entry {
                p
            } else if opening {
                1. - p
            } else {
                1.
            }
        } else if handoff {
            0.0
        } else {
            1.0
        };
        let dirty = self
            .buffer_sessions()
            .filter(|s| s.read(cx).is_dirty())
            .count();
        let total = self
            .buffer_sessions()
            .filter(|s| !s.read(cx).is_generated())
            .count();
        let stats = if self.language == crate::i18n::Language::Chinese {
            format!("{total} 个已打开 · {dirty} 个未保存")
        } else {
            format!("{total} open · {dirty} unsaved")
        };
        let done_entity = entity.clone();
        let accessible_done_entity = entity.clone();
        let input = state.query.clone();
        let grid_focus = state.grid_focus.clone();
        let input_bounds_entity = entity.clone();
        let viewport = window.viewport_size();
        let narrow = f32::from(viewport.width) < 680.;
        let header = div()
            .absolute()
            .top_0()
            .left_0()
            .right_0()
            .h(px(grid.header))
            .px(px(grid.margin))
            .child(
                div()
                    .absolute()
                    .top(px(if narrow { 16. } else { 20. }))
                    .left(px(grid.margin))
                    .child(
                        div()
                            .font(ui_font(".SystemUIFont"))
                            .text_size(ui_px(22.))
                            .font_weight(gpui::FontWeight::SEMIBOLD)
                            .child_text(self.buffer_text("文件总览", "Open Files")),
                    )
                    .child(
                        div()
                            .mt(px(4.))
                            .text_size(ui_px(12.))
                            .text_color(rgb(theme.foreground).opacity(0.75))
                            .child_text(stats),
                    ),
            )
            .child(
                div()
                    .absolute()
                    .top(px(if narrow { 16. } else { 26. }))
                    .right(px(grid.margin))
                    .id("file-overview-done")
                    .debug_selector(|| "file-overview-done".into())
                    .role(Role::Button)
                    .aria_label(
                        self.buffer_text("完成，返回原视图", "Done, return to previous view"),
                    )
                    .track_focus(&state.done_focus)
                    .tab_index(0)
                    .px(px(14.))
                    .h(px(32.))
                    .flex()
                    .items_center()
                    .justify_center()
                    .rounded(px(7.))
                    .border_1()
                    .border_color(rgb(theme.border))
                    .bg(rgb(theme.surface))
                    .text_size(ui_px(13.))
                    .cursor_pointer()
                    .hover(|style| style.bg(rgb(theme.hover)))
                    .child_text(self.buffer_text("完成", "Done"))
                    .on_a11y_action(gpui::AccessibleAction::Click, move |_, window, cx| {
                        accessible_done_entity.update(cx, |this, cx| {
                            this.cancel_file_overview(window, cx);
                        });
                    })
                    .on_click(move |_, window, cx| {
                        done_entity.update(cx, |this, cx| this.cancel_file_overview(window, cx));
                    }),
            )
            .child(
                div()
                    .absolute()
                    .top(px(if narrow { 77. } else { 26. }))
                    .left(px(if narrow {
                        grid.margin
                    } else {
                        (f32::from(viewport.width) - 398.).max(260.)
                    }))
                    .right(px(if narrow {
                        grid.margin
                    } else {
                        grid.margin + 78.
                    }))
                    .h(px(34.))
                    .child(input.clone())
                    .child(
                        canvas(
                            move |bounds, _, cx| {
                                input_bounds_entity.update(cx, |this, _| {
                                    if let Some(state) = this.file_overview.session.as_mut() {
                                        state.query_bounds = bounds;
                                    }
                                });
                            },
                            |_, _, _, _| (),
                        )
                        .absolute()
                        .size_full(),
                    ),
            );
        let range = if let Some((anchor, document)) = transition {
            let viewport = Bounds::new(
                point(px(0.), px(super::super::TITLEBAR_HEIGHT)),
                size(
                    viewport.width,
                    viewport.height - px(super::super::TITLEBAR_HEIGHT),
                ),
            );
            let mut visible = layout::place_in_transition(viewport, document, anchor);
            visible.origin.y -= state.scroll.offset().y;
            grid.visible_range_in(items.len(), visible)
        } else {
            grid.visible_range(items.len(), state.scroll.offset().y)
        };
        let cards = range
            .map(|index| {
                let item = &items[index];
                let mut bounds = grid.document_preview(
                    index,
                    if transition.is_some() {
                        state.scroll.offset().y
                    } else {
                        px(0.)
                    },
                    self.overview_item_size(item, window),
                );
                if let Some((anchor, document)) = transition {
                    bounds = layout::place_in_transition(bounds, anchor, document);
                }
                bounds.origin.y -= px(super::super::TITLEBAR_HEIGHT + grid_top);
                self.overview_card(item, index, &items, bounds, appearance, entity.clone(), cx)
            })
            .collect::<Vec<_>>();
        let empty = items.is_empty();
        let open_entity = entity.clone();
        let grid_view = div()
            .id("file-overview-grid")
            .debug_selector(|| "file-overview-grid".into())
            .role(Role::ListBox)
            .aria_label(self.buffer_text("已打开的文件", "Open files"))
            .track_focus(&grid_focus)
            .tab_index(0)
            .absolute()
            .top(px(grid_top))
            .bottom(px(grid_bottom))
            .left_0()
            .right_0()
            .opacity(
                root_opacity
                    * if transition.is_some() {
                        1.
                    } else {
                        chrome_opacity
                    },
            )
            .when(transition.is_some(), |view| view.overflow_hidden())
            .when(transition.is_none(), |view| {
                view.overflow_y_scroll().track_scroll(&state.scroll)
            })
            .child(
                div()
                    .relative()
                    .w_full()
                    .h(if transition.is_some() {
                        viewport.height - px(super::super::TITLEBAR_HEIGHT + grid_top + grid_bottom)
                    } else {
                        px(grid.total_height)
                    })
                    .children(cards),
            )
            .when(empty, |view| {
                view.child(
                    div()
                        .absolute()
                        .inset_0()
                        .flex()
                        .flex_col()
                        .items_center()
                        .justify_center()
                        .gap(px(14.))
                        .child(
                            div()
                                .text_size(ui_px(16.))
                                .text_color(rgb(theme.foreground).opacity(0.75))
                                .child_text(if total == 0 {
                                    self.buffer_text("还没有打开的文件", "No open files")
                                } else {
                                    self.buffer_text(
                                        "没有匹配的已打开文件",
                                        "No matching open files",
                                    )
                                }),
                        )
                        .when(total == 0, |view| {
                            view.child(
                                div()
                                    .id("file-overview-open-file")
                                    .role(Role::Button)
                                    .px(px(16.))
                                    .py(px(8.))
                                    .rounded(px(7.))
                                    .bg(rgb(theme.accent_bg))
                                    .cursor_pointer()
                                    .child_text(self.buffer_text("打开文件…", "Open file…"))
                                    .on_click(move |_, window, cx| {
                                        open_entity.update(cx, |this, cx| {
                                            this.close_file_overview_now(window, cx);
                                            this.choose_file(window, cx);
                                        });
                                    }),
                            )
                        }),
                )
            });
        let proxy_input = state.query.clone();
        let proxy_focus = state.grid_focus.clone();
        let proxy_bounds = state.query_bounds;
        let backdrop_bounds = Bounds::new(
            point(px(0.), px(0.)),
            size(
                viewport.width,
                (viewport.height - px(super::super::TITLEBAR_HEIGHT)).max(px(0.)),
            ),
        );
        let backdrops = if handoff {
            let mut target = state.target_to;
            target.origin.y -= px(super::super::TITLEBAR_HEIGHT);
            layout::surrounding_regions(backdrop_bounds, target)
                .into_iter()
                .map(|bounds| (bounds, state.handoff_backdrop * (1. - p)))
                .collect::<Vec<_>>()
        } else if entry && state.origin_frame.is_some() && !state.reduced_motion {
            let mut origin = state.origin_bounds;
            origin.origin.y -= px(super::super::TITLEBAR_HEIGHT);
            let mut regions = layout::surrounding_regions(backdrop_bounds, origin)
                .into_iter()
                .map(|bounds| (bounds, smoothstep(0., 0.18, p)))
                .collect::<Vec<_>>();
            // Cover the real document immediately, so the moving frame never
            // leaves a second full-size copy behind. Only the sidebar fades.
            regions.push((origin.intersect(&backdrop_bounds), 1.));
            regions
        } else {
            vec![(
                backdrop_bounds,
                if state.reduced_motion {
                    root_opacity
                } else if entry && state.origin_frame.is_none() {
                    p
                } else {
                    1.
                },
            )]
        };
        let mut overlay = div()
            .id("file-overview")
            .debug_selector(|| "file-overview".into())
            .absolute()
            .top(px(super::super::TITLEBAR_HEIGHT))
            .left_0()
            .right_0()
            .bottom_0()
            .overflow_hidden()
            .font(ui_font(".SystemUIFont"))
            .text_color(rgb(theme.foreground))
            .on_mouse_down(MouseButton::Left, move |_, _, cx| {
                cx.stop_propagation();
            })
            .on_scroll_wheel(|_, _, cx| cx.stop_propagation())
            .children(backdrops.into_iter().map(|(bounds, opacity)| {
                div()
                    .absolute()
                    .left(bounds.left())
                    .top(bounds.top())
                    .w(bounds.size.width)
                    .h(bounds.size.height)
                    .opacity(opacity)
                    .child(
                        canvas(
                            |_, _, _| (),
                            |bounds, _, window, _| background::paint(bounds, window),
                        )
                        .size_full(),
                    )
            }))
            .child(grid_view)
            .child(
                div()
                    .absolute()
                    .inset_0()
                    .opacity(chrome_opacity * root_opacity)
                    .child(header)
                    .child(
                        div()
                            .absolute()
                            .bottom_0()
                            .left_0()
                            .right_0()
                            .h(px(layout::FOOTER))
                            .flex()
                            .items_center()
                            .justify_center()
                            .text_size(ui_px(11.))
                            .text_color(rgb(theme.foreground).opacity(0.75))
                            .child_text(if let Some(notice) = state.notice.as_ref() {
                                notice.clone()
                            } else if self.buffers.loads.is_empty() {
                                self.buffer_text(
                                    "Enter 打开 · 双指展开返回 · Esc / C-g 取消",
                                    "Enter to open · Pinch out to return · Esc / C-g to cancel",
                                )
                                .to_owned()
                            } else {
                                format!(
                                    "{} · {}",
                                    self.buffer_text("正在打开文件", "Opening files"),
                                    self.buffers.loads.len()
                                )
                            }),
                    ),
            )
            .child(
                canvas(
                    |_, _, _| (),
                    move |_, _, window, cx| {
                        if proxy_focus.is_focused(window) {
                            // Let the grid receive IME/text insertion through the same
                            // native input, without routing a character into the editor.
                            window.handle_input(
                                &proxy_focus,
                                ElementInputHandler::new(proxy_bounds, proxy_input.clone()),
                                cx,
                            );
                        }
                    },
                )
                .absolute()
                .top_0()
                .left_0()
                .size(px(1.)),
            );
        if handoff
            || (entry
                && !state.reduced_motion
                && !items.iter().any(|item| Some(item.id) == state.origin))
        {
            if let Some((frame, bounds)) = active_frame {
                // Keep the frame opaque until the ready live view takes over.
                // Per-primitive fading would make overlapping text lighter.
                overlay = overlay.child(
                    render_frame(frame)
                        .absolute()
                        .left(bounds.left())
                        .top(bounds.top() - px(super::super::TITLEBAR_HEIGHT))
                        .w(bounds.size.width)
                        .h(bounds.size.height),
                );
            }
        }
        if state.phase == Phase::Opening
            && state
                .preparation
                .as_ref()
                .is_some_and(|p| p.started.elapsed() > Duration::from_millis(120))
        {
            overlay = overlay.child(
                div()
                    .absolute()
                    .bottom(px(40.))
                    .left_0()
                    .right_0()
                    .flex()
                    .justify_center()
                    .text_size(ui_px(13.))
                    .text_color(rgb(theme.foreground).opacity(0.75))
                    .child_text(self.buffer_text("正在准备文档…", "Preparing document…")),
            );
        }
        #[cfg(feature = "visual-check")]
        if self.file_overview.trace.enabled() {
            let bounds = active_frame.map_or(origin_card, |(_, bounds)| bounds);
            let phase = format!("{:?}", state.phase);
            let target = state.gesture.as_ref().map_or(f32::NAN, Gesture::progress);
            let velocity = state
                .follow
                .map(|f| f.sample(Instant::now()))
                .or_else(|| state.motion.map(|m| m.sample(Instant::now())))
                .map_or(0., |s| s.velocity);
            overlay = overlay.child(
                canvas(
                    |_, _, _| (),
                    move |_, _, _, cx| {
                        entity.update(cx, |this, _| {
                            this.file_overview
                                .trace
                                .paint(&phase, target, p, velocity, bounds)
                        });
                    },
                )
                .absolute()
                .size(px(1.)),
            );
        }
        Some(overlay.into_any_element())
    }

    fn overview_card(
        &self,
        item: &Item,
        index: usize,
        items: &[Item],
        bounds: Bounds<Pixels>,
        appearance: f32,
        entity: Entity<Self>,
        cx: &App,
    ) -> AnyElement {
        let state = self.file_overview.session.as_ref().unwrap();
        let theme = current_theme();
        let selected = state.selected == Some(item.id);
        let chrome_opacity = smoothstep(0.62, 0.94, appearance);
        let id = item.id;
        let selector = format!("file-overview-card-{index}");
        let name = item.name.clone();
        let tooltip: SharedString = format!(
            "{}\n{}",
            name,
            if item.path.is_empty() {
                self.buffer_text("草稿", "Draft")
            } else {
                &item.path
            }
        )
        .into();
        let path = distinguishing_path(item, items, self.buffer_text("草稿", "Draft"));
        let status = if item.conflict {
            self.buffer_text("磁盘冲突", "Disk conflict")
        } else if item.missing {
            self.buffer_text("文件已移除", "File missing")
        } else if item.saving {
            self.buffer_text("正在保存", "Saving")
        } else if item.dirty {
            self.buffer_text("未保存", "Unsaved")
        } else if item.read_only {
            self.buffer_text("只读", "Read only")
        } else {
            ""
        };
        let frame = if matches!(state.phase, Phase::Opening | Phase::Rebounding)
            && state.target == Some(id)
        {
            state.target_frame.as_deref()
        } else {
            None
        }
        .or_else(|| {
            if state.origin == Some(id) {
                state
                    .origin_frame
                    .as_deref()
                    .filter(|frame| frame.revision == item.revision)
            } else {
                None
            }
        })
        .or_else(|| self.file_overview.cache.get(id, item.revision));
        let preview = match frame {
            Some(frame) => render_frame(frame).size_full().into_any_element(),
            None => summary_preview(&item.session.read(cx).snapshot())
                .size_full()
                .into_any_element(),
        };
        let a11y = format!(
            "{} · {}{}{}",
            name,
            path,
            if item.current {
                self.buffer_text(" · 当前", " · Current")
            } else {
                ""
            },
            if status.is_empty() {
                String::new()
            } else {
                format!(" · {status}")
            }
        );
        let accessible_entity = entity.clone();
        div()
            .id(SharedString::from(format!("overview-{id:?}")))
            .debug_selector(move || selector.clone())
            .role(Role::ListBoxOption)
            .aria_label(a11y)
            .aria_selected(selected)
            .when(selected, |view| view.aria_active_descendant())
            .tooltip(move |_, cx| cx.new(|_| Hint(tooltip.clone())).into())
            .on_a11y_action(gpui::AccessibleAction::Click, move |_, window, cx| {
                accessible_entity.update(cx, |this, cx| this.select_overview_item(id, window, cx));
            })
            .absolute()
            .left(bounds.left())
            .top(bounds.top())
            .w(bounds.size.width)
            .h(bounds.size.height + px(layout::CAPTION))
            .cursor_pointer()
            .child(
                div()
                    .id(("file-overview-preview", index))
                    .debug_selector(move || format!("file-overview-preview-{index}"))
                    .relative()
                    .w_full()
                    .h(bounds.size.height)
                    .bg(rgb(theme.editor_background))
                    .shadow(vec![
                        gpui::BoxShadow::new(
                            px(0.),
                            px(3.),
                            gpui::hsla(0., 0., 0., 0.12 * appearance),
                        )
                        .blur_radius(px(12.)),
                    ])
                    .overflow_hidden()
                    .child(preview)
                    .child(
                        div()
                            .absolute()
                            .inset_0()
                            .opacity(chrome_opacity)
                            .border_1()
                            .border_color(rgb(theme.border))
                            .hover(|style| style.border_color(rgb(theme.border_hover))),
                    ),
            )
            .child(
                div()
                    .h(px(layout::CAPTION))
                    .opacity(chrome_opacity)
                    .pt(px(8.))
                    .px(px(3.))
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(6.))
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .truncate()
                                    .text_size(ui_px(14.))
                                    .font_weight(gpui::FontWeight::MEDIUM)
                                    .child_text(name),
                            )
                            .when(item.current, |view| {
                                view.child(
                                    div()
                                        .px(px(6.))
                                        .py(px(1.))
                                        .rounded(px(4.))
                                        .text_size(ui_px(10.))
                                        .text_color(rgb(theme.accent))
                                        .bg(rgb(theme.accent_bg))
                                        .child_text(self.buffer_text("当前", "Current")),
                                )
                            }),
                    )
                    .child(
                        div()
                            .mt(px(3.))
                            .flex()
                            .items_center()
                            .gap(px(6.))
                            .text_size(ui_px(11.))
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .truncate()
                                    .text_color(rgb(theme.foreground).opacity(0.75))
                                    .child_text(path),
                            )
                            .when(!status.is_empty(), |view| {
                                view.child(
                                    div()
                                        .flex_none()
                                        .text_color(rgb(if item.conflict || item.missing {
                                            theme.error
                                        } else {
                                            theme.warning
                                        }))
                                        .child_text(status),
                                )
                            }),
                    ),
            )
            .on_click(move |_, window, cx| {
                entity.update(cx, |this, cx| this.select_overview_item(id, window, cx));
            })
            .into_any_element()
    }
}

fn render_frame(frame: &DocumentFrame) -> gpui::Div {
    let content = frame.source.clone();
    div().child(
        canvas(
            |_, _, _| (),
            move |bounds, _, window, cx| {
                content.paint(bounds, window, cx);
            },
        )
        .size_full(),
    )
}

fn summary_preview(snapshot: &dyn TextSnapshot) -> gpui::Div {
    let theme = current_theme();
    let mut text = String::new();
    let mut offset = 0u64;
    while text.len() < 4096 && offset < snapshot.len_bytes() {
        let Some(chunk) = snapshot.chunk_at(ByteOffset(offset)) else {
            break;
        };
        let local = (offset - chunk.start.0) as usize;
        for ch in chunk.text[local..].chars() {
            if text.len() + ch.len_utf8() > 4096 {
                break;
            }
            text.push(ch);
        }
        let next = chunk.start.0 + chunk.text.len() as u64;
        if next <= offset {
            break;
        }
        offset = next;
    }
    let empty = text.is_empty();
    div()
        .p(px(14.))
        .overflow_hidden()
        .font(crate::typography::editor_font())
        .text_size(px(10.))
        .line_height(px(16.))
        .when(empty, |view| {
            view.child(
                div()
                    .text_color(rgb(theme.foreground_muted))
                    .child_text("空白文档 / Empty document"),
            )
        })
        .children(text.lines().take(24).map(|line| {
            let heading = line.starts_with('*') || line.starts_with("# ");
            let color = if heading {
                theme.heading[0]
            } else if line.starts_with("#+") {
                theme.meta
            } else {
                theme.foreground_dim
            };
            div()
                .h(px(16.))
                .truncate()
                .text_color(rgb(color))
                .child_text(line.chars().take(100).collect::<String>())
        }))
}

fn distinguishing_path(item: &Item, items: &[Item], draft: &str) -> String {
    if item.path.is_empty() {
        return draft.to_owned();
    }
    let parent = std::path::Path::new(&item.path)
        .parent()
        .unwrap_or(std::path::Path::new(""));
    let parts = parent
        .components()
        .map(|p| p.as_os_str().to_string_lossy().into_owned())
        .collect::<Vec<_>>();
    for depth in 1..=parts.len() {
        let suffix = parts[parts.len() - depth..].join("/");
        if !items.iter().any(|other| {
            other.id != item.id
                && other.name == item.name
                && std::path::Path::new(&other.path)
                    .parent()
                    .is_some_and(|p| p.to_string_lossy().ends_with(&suffix))
        }) {
            return suffix;
        }
    }
    parent.to_string_lossy().into_owned()
}

fn smoothstep(start: f32, end: f32, progress: f32) -> f32 {
    let p = ((progress - start) / (end - start)).clamp(0., 1.);
    p * p * (3. - 2. * p)
}
