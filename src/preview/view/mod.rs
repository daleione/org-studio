use super::{
    Arc, BlockKind, BlockNode, CodeHighlightKind, CodeHighlightSpan, Context, DocumentFormat,
    FoldDirection, FoldSegment, FontStyle, FontWeight, HighlightStyle, InlineKind, InlineSpan,
    InlineText, Instant, IntoElement, OPEN_DOCUMENT_COMMAND, OpenDocument, OpenFileManager,
    PreviewLoadState, PreviewRow, PreviewSnapshot, QUIT_APPLICATION_COMMAND, QuitApplication,
    RELOAD_DOCUMENT_COMMAND, ReloadDocument, Render, ReturnToDocument, ReturnToEditor,
    SAVE_DOCUMENT_AS_COMMAND, SAVE_DOCUMENT_COMMAND, SHOW_HOME_COMMAND, SaveDocument,
    SaveDocumentAs, ShowHome, StyledText, ToggleMinimap, ToggleRightPreview, ToggleSidebar,
    ToggleSoftWrap, UseChinese, UseEnglish, Window, WorkspaceWindow, current_theme, div, img,
    markdown, minimap, parse_inline, px, render_table_row, resolve_image_path, rgb,
};
use super::{EXPORT_DOCUMENT_COMMAND, ExportDocument, export_ui::render_export_panel};
use gpui::{CursorStyle, ExternalPaths, MouseButton, prelude::*};

mod code_block;
mod document;
mod home;
mod markdown_block;
mod overlays;
mod styled_text;
use code_block::{org_code_row_role, render_code_row};
pub(super) use document::{PreviewRenderOptions, render_document};
pub(super) use home::{render_home, render_loading};
pub(super) use markdown_block::parse_document_inline;
use markdown_block::render_markdown_block;
use overlays::{dired_help_window, which_key_window};
pub(super) use styled_text::code_highlight_style;
use styled_text::styled_inline_runs;

impl Render for WorkspaceWindow {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        profiling::scope!("WorkspaceWindow::render");
        self.install_close_guard(window, cx);
        self.ensure_document_subscription(cx);
        self.ensure_right_preview_scroll_sync(cx);
        let viewport = window.viewport_size();
        let viewport_key = (
            f32::from(viewport.width).to_bits(),
            f32::from(viewport.height).to_bits(),
        );
        if let Some(panel) = self.preview_panel() {
            let viewport_changed = panel.update(cx, |panel, _| panel.note_viewport(viewport_key));
            if viewport_changed {
                self.cancel_sidebar_resize();
                self.cancel_right_preview_resize();
            }
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
                    || this.cancel_right_preview_resize()
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
        if matches!(self.state, PreviewLoadState::Ready { .. })
            && self.first_frame_scheduled != Some(self.generation)
        {
            let generation = self.generation;
            let opened_at = self.opened_at.unwrap_or_else(Instant::now);
            self.first_frame_scheduled = Some(generation);
            cx.on_next_frame(window, move |this, window, cx| {
                let elapsed = opened_at.elapsed();
                eprintln!(
                    "org_preview_first_readable_frame generation={} elapsed_ms={:.3}",
                    generation,
                    elapsed.as_secs_f64() * 1000.0
                );
                // The list viewport is only known after the first layout pass. Render once
                // more so pane-local status (notably bottom-edge progress) uses real bounds.
                cx.notify();
                if generation == 1
                    && std::env::var_os("ORG_STUDIO_RELOAD_BENCH").is_some()
                    && matches!(this.state, PreviewLoadState::Ready { .. })
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
        let which_key_items = self.which_key_items.clone();
        let dired_help_visible = self.file_manager.help_visible();
        let command_window_width = f32::from(window.viewport_size().width);
        let resizing_sidebar = self.file_manager.is_resizing_sidebar();
        let resizing_right_preview = self.right_preview_resize.is_some();
        let export_panel = self.export.panel().cloned();
        let export_status = self.export.status().cloned();
        let resize_entity = entity.clone();
        let finish_resize_entity = entity.clone();
        let right_resize_entity = entity.clone();
        let finish_right_resize_entity = entity.clone();
        div()
            .relative()
            .track_focus(&focus_handle)
            .size_full()
            .bg(rgb(current_theme().background))
            .text_color(rgb(current_theme().foreground))
            .font_family("Menlo")
            .text_size(px(14.0))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, _, _, cx| {
                    if this.status.dismiss_popover() {
                        cx.notify();
                    }
                }),
            )
            .on_key_down(cx.listener(|this, event, window, cx| this.key_down(event, window, cx)))
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
            .on_action(cx.listener(|this, _: &ReturnToEditor, _, cx| this.return_to_editor(cx)))
            .on_action(
                cx.listener(|this, _: &ToggleRightPreview, _, cx| this.toggle_right_preview(cx)),
            )
            .on_action(cx.listener(|this, _: &ToggleSoftWrap, _, cx| this.toggle_soft_wrap(cx)))
            .on_action(cx.listener(|this, _: &UseEnglish, _, cx| {
                this.set_language(crate::i18n::Language::English, cx)
            }))
            .on_action(cx.listener(|this, _: &UseChinese, _, cx| {
                this.set_language(crate::i18n::Language::Chinese, cx)
            }))
            .on_drop(cx.listener(|this, paths: &ExternalPaths, window, cx| {
                this.open_dropped_paths(paths, window, cx)
            }))
            .child(self.workspace_body(entity.clone(), command_window_width, window, cx))
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
            .when(resizing_right_preview, |view| {
                view.child(
                    div()
                        .id("right-preview-resize-overlay")
                        .absolute()
                        .top_0()
                        .right_0()
                        .bottom_0()
                        .left_0()
                        .cursor(CursorStyle::ResizeLeftRight)
                        .on_mouse_move(move |event, _, cx| {
                            if event.dragging() {
                                right_resize_entity.update(cx, |this, cx| {
                                    this.update_right_preview_resize(
                                        f32::from(event.position.x),
                                        cx,
                                    );
                                });
                            }
                        })
                        .on_mouse_up(MouseButton::Left, move |_, _, cx| {
                            finish_right_resize_entity
                                .update(cx, |this, cx| this.finish_right_preview_resize(cx));
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
        let Some(session) = self.document_session().cloned() else {
            self.document_subscription = None;
            self.subscribed_document = None;
            return;
        };
        let document_id = session.read(cx).id();
        if self.subscribed_document == Some(document_id) {
            return;
        }
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
                    crate::document::DocumentEvent::Saved { .. }
                    | crate::document::DocumentEvent::DiskChanged { .. } => {}
                }
                cx.notify();
            },
        ));
    }
}
