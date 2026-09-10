use std::time::Instant;

use gpui::{
    Context, CursorStyle, ExternalPaths, IntoElement, MouseButton, Render, Window, div, prelude::*,
    px, rgb, svg,
};

use crate::{
    app::{WorkspaceLoadState, WorkspaceWindow},
    editor::Copy,
    preview::{
        DOCUMENT_WORKSPACE_KEY_CONTEXT, DecreaseContentFontSize, EXPORT_DOCUMENT_COMMAND,
        ExportDocument, IncreaseContentFontSize, OPEN_DOCUMENT_COMMAND, OpenDocument,
        OpenFileManager, QUIT_APPLICATION_COMMAND, QuitApplication, RELOAD_DOCUMENT_COMMAND,
        ReloadDocument, ResetContentFontSize, ReturnToDocument, SAVE_DOCUMENT_AS_COMMAND,
        SAVE_DOCUMENT_COMMAND, SHOW_HOME_COMMAND, SaveDocument, SaveDocumentAs, ShowEditor,
        ShowHome, ShowReading, ShowSplit, ToggleMinimap, ToggleSidebar, ToggleSoftWrap, UseChinese,
        UseEnglish,
    },
    theme::current_theme,
};

use super::export_ui::render_export_panel;
use super::overlays::{dired_help_window, which_key_window};

fn sidebar_icon(color: u32) -> gpui::Svg {
    svg()
        .debug_selector(|| "document-titlebar-sidebar-icon".to_owned())
        .data(include_bytes!("assets/sidebar.svg"))
        .w(px(19.0))
        .h(px(16.0))
        .text_color(rgb(color))
}

fn minimap_icon(color: u32) -> gpui::Svg {
    svg()
        .debug_selector(|| "document-titlebar-minimap-icon".to_owned())
        .data(include_bytes!("assets/minimap.svg"))
        .w(px(19.0))
        .h(px(16.0))
        .text_color(rgb(color))
}

fn export_icon(color: u32) -> gpui::Svg {
    svg()
        .debug_selector(|| "document-titlebar-export-icon".to_owned())
        .data(include_bytes!("assets/export.svg"))
        .w(px(19.0))
        .h(px(16.0))
        .text_color(rgb(color))
}

fn agenda_icon(color: u32) -> gpui::Svg {
    svg()
        .debug_selector(|| "document-titlebar-agenda-icon".to_owned())
        .data(include_bytes!("assets/agenda.svg"))
        .w(px(19.0))
        .h(px(16.0))
        .text_color(rgb(color))
}

fn document_titlebar(
    workspace: gpui::Entity<WorkspaceWindow>,
    sidebar_visible: bool,
    minimap_visible: bool,
    export_open: bool,
    titlebar_inset: f32,
) -> gpui::Div {
    let theme = current_theme();
    let sidebar_workspace = workspace.clone();
    let sidebar_icon_color = if sidebar_visible {
        theme.foreground
    } else {
        theme.quote
    };
    let minimap_icon_color = if minimap_visible {
        theme.foreground
    } else {
        theme.quote
    };
    let export_workspace = workspace.clone();
    let export_icon_color = if export_open {
        theme.foreground
    } else {
        theme.quote
    };
    let agenda_workspace = workspace.clone();
    let agenda_icon_color = theme.foreground;
    div()
        .debug_selector(|| "document-titlebar".to_owned())
        .absolute()
        .top_0()
        .left_0()
        .right_0()
        .h(px(crate::app::TITLEBAR_HEIGHT))
        .pl(px(titlebar_inset))
        .pr(px(crate::app::TITLEBAR_TRAILING_INSET))
        .flex()
        .items_center()
        .bg(rgb(theme.background))
        .border_b_1()
        .border_color(rgb(theme.border))
        .child(
            div()
                .id("document-titlebar-sidebar-toggle")
                .debug_selector(|| "document-titlebar-sidebar-toggle".to_owned())
                .size(px(32.0))
                .flex()
                .items_center()
                .justify_center()
                .rounded_full()
                .bg(rgb(theme.background_alt))
                .cursor_pointer()
                .when(sidebar_visible, |button| {
                    button.bg(rgb(theme.code_active_background))
                })
                .hover(move |style| style.bg(rgb(theme.code_active_background)))
                .child(sidebar_icon(sidebar_icon_color))
                .on_mouse_down(MouseButton::Left, move |_, _, cx| {
                    cx.stop_propagation();
                    sidebar_workspace.update(cx, |this, cx| this.toggle_sidebar(cx));
                }),
        )
        .child(
            div()
                .id("document-titlebar-agenda-toggle")
                .debug_selector(|| "document-titlebar-agenda-toggle".to_owned())
                .size(px(32.0))
                .flex()
                .items_center()
                .justify_center()
                .rounded_full()
                .bg(rgb(theme.background_alt))
                .cursor_pointer()
                .hover(move |style| style.bg(rgb(theme.code_active_background)))
                .child(agenda_icon(agenda_icon_color))
                .on_mouse_down(MouseButton::Left, move |_, _, cx| {
                    cx.stop_propagation();
                    agenda_workspace.update(cx, |this, cx| this.open_agenda(cx));
                }),
        )
        .child(div().flex_1())
        .child(
            div()
                .id("document-titlebar-export-toggle")
                .debug_selector(|| "document-titlebar-export-toggle".to_owned())
                .size(px(32.0))
                .flex()
                .items_center()
                .justify_center()
                .rounded_full()
                .bg(rgb(theme.background_alt))
                .cursor_pointer()
                .when(export_open, |button| {
                    button.bg(rgb(theme.code_active_background))
                })
                .hover(move |style| style.bg(rgb(theme.code_active_background)))
                .child(export_icon(export_icon_color))
                .on_mouse_down(MouseButton::Left, move |_, _, cx| {
                    cx.stop_propagation();
                    export_workspace.update(cx, |this, cx| this.show_export_panel(cx));
                }),
        )
        .child(
            div()
                .id("document-titlebar-minimap-toggle")
                .debug_selector(|| "document-titlebar-minimap-toggle".to_owned())
                .size(px(32.0))
                .flex()
                .items_center()
                .justify_center()
                .rounded_full()
                .bg(rgb(theme.background_alt))
                .cursor_pointer()
                .when(minimap_visible, |button| {
                    button.bg(rgb(theme.code_active_background))
                })
                .hover(move |style| style.bg(rgb(theme.code_active_background)))
                .child(minimap_icon(minimap_icon_color))
                .on_mouse_down(MouseButton::Left, move |_, _, cx| {
                    cx.stop_propagation();
                    workspace.update(cx, |this, cx| this.toggle_minimap(cx));
                }),
        )
}

impl Render for WorkspaceWindow {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        profiling::scope!("WorkspaceWindow::render");
        self.install_close_guard(window, cx);
        self.ensure_document_subscription(cx);
        self.ensure_agenda_runtime(cx);
        // Fullscreen hides the traffic lights, so the titlebar buttons slide
        // to the left edge; windowed mode reserves their slot again.
        let titlebar_inset = if window.is_fullscreen() {
            0.0
        } else {
            crate::app::TITLEBAR_LEADING_INSET
        };
        if std::mem::take(&mut self.agenda.state.search_focus_pending)
            && let Some(input) = &self.agenda.search_input
        {
            let focus = input.read(cx).focus.clone();
            window.focus(&focus, cx);
        }
        let viewport = window.viewport_size();
        let viewport_key = (
            f32::from(viewport.width).to_bits(),
            f32::from(viewport.height).to_bits(),
        );
        let mut viewport_changed = false;
        for panel in self
            .visible_panes()
            .filter_map(|pane| self.reading_panel_for(pane))
        {
            viewport_changed |= panel.update(cx, |panel, _| panel.note_viewport(viewport_key));
        }
        if viewport_changed {
            self.cancel_sidebar_resize();
            self.cancel_split_resize();
        }
        let focus_handle = self
            .focus_handle
            .get_or_insert_with(|| {
                let handle = cx.focus_handle();
                window.focus(&handle, cx);
                handle
            })
            .clone();
        if self.focus_workspace_on_render {
            self.focus_workspace_on_render = false;
            window.focus(&focus_handle, cx);
        }
        if self.focus_lost_subscription.is_none() {
            self.focus_lost_subscription = Some(cx.on_focus_lost(window, |this, _, cx| {
                if this.cancel_minimap_interaction(cx)
                    || this.cancel_sidebar_resize()
                    || this.cancel_split_resize()
                {
                    cx.notify();
                }
            }));
        }
        window.set_window_title(&self.window_title(cx));
        self.schedule_file_manager_presentation(window, cx);
        if self.scroll_benchmark.is_some() && !self.minimap_visible {
            window.request_animation_frame();
        }
        if matches!(self.state, WorkspaceLoadState::Ready { .. })
            && self.first_frame_scheduled != Some(self.generation)
        {
            let generation = self.generation;
            let opened_at = self.opened_at.unwrap_or_else(Instant::now);
            self.first_frame_scheduled = Some(generation);
            cx.on_next_frame(window, move |this, window, cx| {
                let elapsed = opened_at.elapsed();
                eprintln!(
                    "org_preview_first_readable_frame generation={} elapsed_ms={:.3} display_id={:?} display_bounds={:?}",
                    generation,
                    elapsed.as_secs_f64() * 1000.0,
                    window.display(cx).map(|display| display.id()),
                    window.display(cx).map(|display| display.bounds()),
                );
                // The list viewport is only known after the first layout pass. Render once
                // more so pane-local status (notably bottom-edge progress) uses real bounds.
                cx.notify();
                if generation == 1
                    && std::env::var_os("ORG_STUDIO_RELOAD_BENCH").is_some()
                    && matches!(this.state, WorkspaceLoadState::Ready { .. })
                {
                    this.reload_current(cx);
                    return;
                }
                if this.scroll_benchmark.is_some() {
                    window.activate_window();
                    if let Some(benchmark) = this.scroll_benchmark.as_mut() {
                        benchmark.last_frame = Instant::now();
                    }
                    this.schedule_scroll_sample(window, cx);
                } else if std::env::var_os("ORG_STUDIO_EXIT_AFTER_FIRST_FRAME").is_some() {
                    cx.quit();
                }
            });
        }
        let entity = cx.entity();
        let echo_message = self.displayed_echo_message();
        let which_key_items = self.which_key_items.clone();
        let dired_help_visible = self.file_manager.help_visible();
        let command_window_width = f32::from(window.viewport_size().width);
        let resizing_sidebar = self.file_manager.is_resizing_sidebar();
        let resizing_split = self.split_resize.is_some();
        let export_panel = self.export.panel().cloned();
        let export_status = self.export.status().cloned();
        let show_echo_area = !matches!(self.content_route, crate::app::ContentRoute::Agenda)
            && !(self.content_route == crate::app::ContentRoute::Document
                && self.state.ready().is_some());
        // The agenda route draws its own toolbar into the titlebar row, so it
        // does not reserve space for a separate titlebar.
        let show_agenda = matches!(self.content_route, crate::app::ContentRoute::Agenda);
        let show_document_titlebar =
            matches!(self.content_route, crate::app::ContentRoute::Document)
                && self.state.ready().is_some();
        let sidebar_visible = self.file_manager.sidebar_visible();
        let minimap_visible = self.minimap_visible();
        let minimap_transitioning = self.minimap_visibility_animation.is_some();
        let (minimap_reveal, minimap_animating) = self.minimap_reveal_at(Instant::now());
        if minimap_animating {
            window.request_animation_frame();
        } else {
            self.minimap_visibility_animation = None;
        }
        if minimap_transitioning {
            let editors = self
                .state
                .ready()
                .into_iter()
                .flat_map(|document| [&document.editors.left, &document.editors.right])
                .flatten()
                .cloned()
                .collect::<Vec<_>>();
            let minimap_width = self.minimap_width;
            for editor in editors {
                editor.update(cx, |editor, cx| {
                    editor.set_minimap_presentation(
                        minimap_visible,
                        minimap_width,
                        minimap_reveal,
                        cx,
                    );
                });
            }
        }
        let content_font_size_actions_enabled = self.content_font_size_command_available();
        let resize_entity = entity.clone();
        let finish_resize_entity = entity.clone();
        let split_resize_entity = entity.clone();
        let finish_split_resize_entity = entity.clone();
        div()
            .relative()
            .track_focus(&focus_handle)
            .size_full()
            .bg(rgb(current_theme().background))
            .text_color(rgb(current_theme().foreground))
            .font_family("Menlo")
            .text_size(px(14.0))
            .when(content_font_size_actions_enabled, |view| {
                view.key_context(DOCUMENT_WORKSPACE_KEY_CONTEXT)
            })
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, _, _, cx| {
                    if this.status.dismiss_popover() {
                        cx.notify();
                    }
                }),
            )
            .on_key_down(cx.listener(|this, event, window, cx| this.key_down(event, window, cx)))
            .on_action(cx.listener(
                |this, action: &crate::editor::ActivateReadOnlyLine, window, cx| {
                    this.activate_generated_line(action.line, window, cx);
                },
            ))
            .on_action(cx.listener(|this, _: &OpenDocument, window, cx| {
                this.dispatch_command(OPEN_DOCUMENT_COMMAND, window, cx)
            }))
            .on_action(cx.listener(|this, _: &ShowHome, window, cx| {
                this.dispatch_command(SHOW_HOME_COMMAND, window, cx)
            }))
            .on_action(cx.listener(|this, _: &ReloadDocument, window, cx| {
                this.dispatch_command(RELOAD_DOCUMENT_COMMAND, window, cx)
            }))
            .on_action(cx.listener(|this, _: &SaveDocument, window, cx| {
                this.dispatch_command(SAVE_DOCUMENT_COMMAND, window, cx)
            }))
            .on_action(cx.listener(|this, _: &SaveDocumentAs, window, cx| {
                this.dispatch_command(SAVE_DOCUMENT_AS_COMMAND, window, cx)
            }))
            .on_action(cx.listener(|this, _: &ExportDocument, window, cx| {
                this.dispatch_command(EXPORT_DOCUMENT_COMMAND, window, cx)
            }))
            .on_action(cx.listener(|this, _: &QuitApplication, window, cx| {
                this.dispatch_command(QUIT_APPLICATION_COMMAND, window, cx)
            }))
            .on_action(cx.listener(|this, _: &OpenFileManager, _, cx| this.choose_directory(cx)))
            .on_action(cx.listener(|this, _: &ReturnToDocument, _, cx| this.return_to_document(cx)))
            .on_action(cx.listener(|this, _: &ToggleSidebar, _, cx| this.toggle_sidebar(cx)))
            .on_action(cx.listener(|this, _: &ToggleMinimap, _, cx| this.toggle_minimap(cx)))
            .on_action(cx.listener(|this, _: &ShowEditor, _, cx| this.show_editor(cx)))
            .on_action(cx.listener(|this, _: &ShowReading, _, cx| this.show_reading(cx)))
            .on_action(cx.listener(|this, _: &ShowSplit, _, cx| this.show_split(cx)))
            .on_action(cx.listener(|this, _: &ToggleSoftWrap, _, cx| this.toggle_soft_wrap(cx)))
            .on_action(
                cx.listener(|this, _: &crate::editor::RunSourceBlock, window, cx| {
                    this.execute_source_block(window, cx)
                }),
            )
            .on_action(cx.listener(
                |this, action: &crate::editor::RunSourceBlockAt, window, cx| {
                    this.execute_source_block_at(action.source_offset, window, cx)
                },
            ))
            .on_action(cx.listener(|this, _: &IncreaseContentFontSize, _, cx| {
                this.increase_content_font_size(cx)
            }))
            .on_action(cx.listener(|this, _: &DecreaseContentFontSize, _, cx| {
                this.decrease_content_font_size(cx)
            }))
            .on_action(
                cx.listener(|this, _: &ResetContentFontSize, _, cx| {
                    this.reset_content_font_size(cx)
                }),
            )
            .on_action(cx.listener(|this, _: &UseEnglish, _, cx| {
                this.set_language(crate::i18n::Language::English, cx)
            }))
            .on_action(cx.listener(|this, _: &UseChinese, _, cx| {
                this.set_language(crate::i18n::Language::Chinese, cx)
            }))
            .on_action(cx.listener(|this, _: &Copy, _, cx| {
                this.copy_reading_selection(cx);
            }))
            .on_drop(cx.listener(|this, paths: &ExternalPaths, window, cx| {
                this.open_dropped_paths(paths, window, cx)
            }))
            .when(show_document_titlebar, |view| {
                view.child(document_titlebar(
                    entity.clone(),
                    sidebar_visible,
                    minimap_visible,
                    self.export.is_open(),
                    titlebar_inset,
                ))
            })
            .child(
                div()
                    .size_full()
                    .flex()
                    .flex_col()
                    .child(
                        div()
                            .flex_1()
                            .min_h_0()
                            .when(!show_agenda, |body| {
                                body.pt(px(crate::app::TITLEBAR_HEIGHT))
                            })
                            .child(self.workspace_body(
                                entity.clone(),
                                command_window_width,
                                minimap_reveal,
                                window,
                                cx,
                                titlebar_inset,
                            )),
                    )
                    .when(show_echo_area, |workspace| {
                        workspace.child(crate::app::echo_area::render_echo_area(
                            echo_message,
                            entity.clone(),
                            self.keyboard.pending_keys(),
                            self.language,
                        ))
                    }),
            )
            .when(resizing_sidebar, |view| {
                view.child(
                    div()
                        .id("file-sidebar-resize-overlay")
                        .absolute()
                        .top_0()
                        .right_0()
                        .bottom_0()
                        .left_0()
                        .cursor(CursorStyle::ResizeLeftRight)
                        .on_mouse_move(move |event, _, cx| {
                            if event.dragging() {
                                resize_entity.update(cx, |this, cx| {
                                    this.update_sidebar_resize(
                                        f32::from(event.position.x),
                                        command_window_width,
                                        cx,
                                    );
                                });
                            }
                        })
                        .on_mouse_up(MouseButton::Left, move |_, _, cx| {
                            finish_resize_entity
                                .update(cx, |this, cx| this.finish_sidebar_resize(cx));
                        }),
                )
            })
            .when(resizing_split, |view| {
                view.child(
                    div()
                        .id("split-resize-overlay")
                        .absolute()
                        .top_0()
                        .right_0()
                        .bottom_0()
                        .left_0()
                        .cursor(CursorStyle::ResizeLeftRight)
                        .on_mouse_move(move |event, _, cx| {
                            if event.dragging() {
                                split_resize_entity.update(cx, |this, cx| {
                                    this.update_split_resize(f32::from(event.position.x), cx);
                                });
                            }
                        })
                        .on_mouse_up(MouseButton::Left, move |_, _, cx| {
                            finish_split_resize_entity
                                .update(cx, |this, cx| this.finish_split_resize(cx));
                        }),
                )
            })
            .when(!which_key_items.is_empty(), |view| {
                view.child(if dired_help_visible {
                    dired_help_window(which_key_items.clone(), command_window_width)
                } else {
                    which_key_window(which_key_items.clone(), command_window_width)
                })
            })
            .when_some(export_panel, |view, panel| {
                view.child(render_export_panel(
                    entity.clone(),
                    panel,
                    export_status,
                    self.language,
                ))
            })
    }
}

impl WorkspaceWindow {
    fn ensure_document_subscription(&mut self, cx: &mut Context<Self>) {
        let Some(document) = self.state.ready() else {
            self.document_subscription = None;
            self.editor_minimap_width_subscriptions.clear();
            self.subscribed_document = None;
            return;
        };
        let session = document.session.clone();
        let editors = document.editors.clone();
        let document_id = session.read(cx).id();
        if self.subscribed_document != Some(document_id) {
            self.subscribed_document = Some(document_id);
            self.document_subscription = Some(cx.subscribe(
                &session,
                |this, _, event: &crate::document::DocumentEvent, cx| {
                    match event {
                        crate::document::DocumentEvent::Edited { delta, .. }
                        | crate::document::DocumentEvent::Reloaded { delta, .. } => {
                            this.schedule_derived_update_with_delta(Some(delta.clone()), cx);
                        }
                        crate::document::DocumentEvent::PathChanged { .. } => {
                            this.schedule_derived_update(cx);
                        }
                        crate::document::DocumentEvent::ResourceChanged { path, .. } => {
                            gpui::ImageSource::from(path.clone()).remove_asset(cx);
                            this.schedule_derived_resource_update(cx);
                        }
                        crate::document::DocumentEvent::Saved { .. } => {
                            this.flush_agenda_clock(cx);
                        }
                        crate::document::DocumentEvent::DiskChanged { .. } => {}
                    }
                    this.sync_agenda_document(cx);
                    cx.notify();
                },
            ));
        }
        let editors = [&editors.left, &editors.right]
            .into_iter()
            .flatten()
            .cloned()
            .collect::<Vec<_>>();
        if self.editor_minimap_width_subscriptions.len() != editors.len() {
            self.editor_minimap_width_subscriptions.clear();
            for editor in editors {
                self.editor_minimap_width_subscriptions.push(cx.subscribe(
                    &editor,
                    |this, _, event: &crate::editor::EditorMinimapWidthEvent, cx| {
                        this.change_minimap_width(
                            crate::preview::minimap::MinimapWidthChange::Commit(event.0),
                            cx,
                        );
                    },
                ));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::Modifiers;

    struct TitlebarHarness(gpui::Entity<WorkspaceWindow>);

    impl Render for TitlebarHarness {
        fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
            let sidebar_visible = self.0.read(cx).sidebar_visible();
            let minimap_visible = self.0.read(cx).minimap_visible();
            let export_open = self.0.read(cx).export.is_open();
            document_titlebar(
                self.0.clone(),
                sidebar_visible,
                minimap_visible,
                export_open,
                crate::app::TITLEBAR_LEADING_INSET,
            )
        }
    }

    #[gpui::test]
    fn document_titlebar_places_working_sidebar_and_minimap_controls(
        cx: &mut gpui::TestAppContext,
    ) {
        let workspace = cx.new(|_| WorkspaceWindow::with_split_layout(false));
        let workspace_for_view = workspace.clone();
        let (_, cx) = cx.add_window_view(move |_, _| TitlebarHarness(workspace_for_view));

        let bar = cx
            .debug_bounds("document-titlebar")
            .expect("document titlebar should be rendered");
        let button = cx
            .debug_bounds("document-titlebar-sidebar-toggle")
            .expect("sidebar toggle should be rendered");
        assert_eq!(bar.size.height, px(crate::app::TITLEBAR_HEIGHT));
        let icon = cx
            .debug_bounds("document-titlebar-sidebar-icon")
            .expect("sidebar icon should be rendered");
        let minimap_button = cx
            .debug_bounds("document-titlebar-minimap-toggle")
            .expect("minimap toggle should be rendered");
        let minimap_icon = cx
            .debug_bounds("document-titlebar-minimap-icon")
            .expect("minimap icon should be rendered");
        let export_button = cx
            .debug_bounds("document-titlebar-export-toggle")
            .expect("export toggle should be rendered");
        let export_icon = cx
            .debug_bounds("document-titlebar-export-icon")
            .expect("export icon should be rendered");
        let agenda_button = cx
            .debug_bounds("document-titlebar-agenda-toggle")
            .expect("agenda toggle should be rendered");
        let agenda_icon = cx
            .debug_bounds("document-titlebar-agenda-icon")
            .expect("agenda icon should be rendered");
        assert_eq!(button.size, gpui::size(px(32.0), px(32.0)));
        assert_eq!(icon.size, gpui::size(px(19.0), px(16.0)));
        assert_eq!(minimap_button.size, gpui::size(px(32.0), px(32.0)));
        assert_eq!(minimap_icon.size, gpui::size(px(19.0), px(16.0)));
        assert_eq!(export_button.size, gpui::size(px(32.0), px(32.0)));
        assert_eq!(export_icon.size, gpui::size(px(19.0), px(16.0)));
        assert_eq!(agenda_button.size, gpui::size(px(32.0), px(32.0)));
        assert_eq!(agenda_icon.size, gpui::size(px(19.0), px(16.0)));
        assert_eq!(button.left(), px(crate::app::TITLEBAR_LEADING_INSET));
        assert!(
            button.right() <= agenda_button.left(),
            "agenda toggle must sit to the right of the sidebar toggle"
        );
        assert!(
            export_button.right() <= minimap_button.left(),
            "export toggle must sit to the left of the minimap toggle"
        );
        assert!(minimap_button.right() <= bar.right());

        cx.simulate_mouse_move(button.center(), None, Modifiers::default());
        cx.simulate_click(button.center(), Modifiers::default());
        assert!(workspace.read_with(cx, |workspace, _| workspace.sidebar_visible()));

        let minimap_was_visible =
            workspace.read_with(cx, |workspace, _| workspace.minimap_visible());
        cx.simulate_mouse_move(minimap_button.center(), None, Modifiers::default());
        cx.simulate_click(minimap_button.center(), Modifiers::default());
        assert_ne!(
            workspace.read_with(cx, |workspace, _| workspace.minimap_visible()),
            minimap_was_visible
        );
    }

    #[gpui::test]
    fn titlebar_export_toggle_opens_the_export_panel(cx: &mut gpui::TestAppContext) {
        let workspace = cx.new(|_| WorkspaceWindow::with_split_layout(false));
        let path =
            std::env::temp_dir().join(format!("org-studio-titlebar-{}.md", std::process::id()));
        std::fs::write(&path, "# Title\n").unwrap();
        let loaded = crate::preview::load_document(path.clone()).unwrap();
        let _ = std::fs::remove_file(&path);
        workspace.update(cx, |workspace, cx| {
            assert!(workspace.apply_load_result(0, Ok(loaded), cx));
        });

        let workspace_for_view = workspace.clone();
        let (_, cx) = cx.add_window_view(move |_, _| TitlebarHarness(workspace_for_view));

        let export_button = cx
            .debug_bounds("document-titlebar-export-toggle")
            .expect("export toggle should be rendered");
        assert!(
            !workspace.read_with(cx, |workspace, _| workspace.export.is_open()),
            "export panel starts closed"
        );
        cx.simulate_mouse_move(export_button.center(), None, Modifiers::default());
        cx.simulate_click(export_button.center(), Modifiers::default());
        assert!(
            workspace.read_with(cx, |workspace, _| workspace.export.is_open()),
            "clicking the titlebar export toggle must open the export panel"
        );
    }

    #[gpui::test]
    fn titlebar_agenda_toggle_opens_the_agenda_route(cx: &mut gpui::TestAppContext) {
        let workspace = cx.new(|_| WorkspaceWindow::with_split_layout(false));
        let workspace_for_view = workspace.clone();
        let (_, cx) = cx.add_window_view(move |_, _| TitlebarHarness(workspace_for_view));

        let agenda_button = cx
            .debug_bounds("document-titlebar-agenda-toggle")
            .expect("agenda toggle should be rendered");
        cx.simulate_mouse_move(agenda_button.center(), None, Modifiers::default());
        cx.simulate_click(agenda_button.center(), Modifiers::default());
        assert_eq!(
            workspace.read_with(cx, |workspace, _| workspace.content_route),
            crate::app::ContentRoute::Agenda,
            "clicking the titlebar agenda toggle must enter the agenda route"
        );
    }
}
