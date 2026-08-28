use super::{
    Arc, BlockId, BlockKind, BlockNode, CodeHighlightKind, CodeHighlightSpan, Context,
    DocumentFormat, FoldDirection, FoldSegment, FoldTransition, FontStyle, FontWeight, HashSet,
    HighlightStyle, InlineKind, InlineSpan, InlineText, Instant, IntoElement, ListOffset,
    ListState, OPEN_DOCUMENT_COMMAND, OpenDocument, OpenFileManager, PreviewApp, PreviewDocument,
    PreviewLoadState, PreviewRow, RELOAD_DOCUMENT_COMMAND, ReloadDocument, Render,
    ReturnToDocument, SHOW_HOME_COMMAND, ShowHome, StyledText, ToggleMinimap, ToggleSidebar,
    Window, accept_generation, current_theme, div, img, markdown, minimap, parse_inline, px,
    render_table_row, resolve_image_path, rgb,
};
use gpui::{CursorStyle, ExternalPaths, MouseButton, prelude::*};

mod code_block;
mod document;
mod home;
mod markdown_block;
mod overlays;
mod styled_text;
use code_block::{org_code_row_role, render_code_row};
pub(super) use document::render_document;
pub(super) use home::{render_home, render_loading};
pub(super) use markdown_block::parse_document_inline;
use markdown_block::render_markdown_block;
use overlays::{dired_help_window, which_key_window};
pub(super) use styled_text::code_highlight_style;
use styled_text::styled_inline_runs;

impl Render for PreviewApp {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        profiling::scope!("PreviewApp::render");
        let viewport = window.viewport_size();
        let viewport_key = (
            f32::from(viewport.width).to_bits(),
            f32::from(viewport.height).to_bits(),
        );
        if self.viewport_revision_key != Some(viewport_key) {
            self.viewport_revision_key = Some(viewport_key);
            self.presentation_revision = self.presentation_revision.wrapping_add(1);
            self.cancel_minimap_interaction();
            self.cancel_sidebar_resize();
        }
        let focus_handle = self
            .focus_handle
            .get_or_insert_with(|| {
                let handle = cx.focus_handle();
                window.focus(&handle, cx);
                handle
            })
            .clone();
        if self.focus_lost_subscription.is_none() {
            self.focus_lost_subscription = Some(cx.on_focus_lost(window, |this, _, cx| {
                if this.cancel_minimap_interaction() || this.cancel_sidebar_resize() {
                    cx.notify();
                }
            }));
        }
        window.set_window_title(&self.window_title());
        if (self.dired_pending_presentation.is_some()
            || self.sidebar_pending_presentation.is_some())
            && !self.dired_presentation_scheduled
        {
            self.dired_presentation_scheduled = true;
            cx.on_next_frame(window, |this, _, cx| {
                this.dired_presentation_scheduled = false;
                if let Some((transaction, view_revision, rank, offset)) =
                    this.dired_pending_presentation.take()
                    && this.dired.as_ref().is_some_and(|session| {
                        session.presentation_is_current(transaction, view_revision)
                    })
                {
                    this.dired_list_state.scroll_to(ListOffset {
                        item_ix: rank,
                        offset_in_item: px(offset),
                    });
                }
                if let Some((transaction, view_revision, rank, offset)) =
                    this.sidebar_pending_presentation.take()
                    && this.dired.as_ref().is_some_and(|session| {
                        session.presentation_is_current(transaction, view_revision)
                    })
                {
                    this.sidebar_list_state.scroll_to(ListOffset {
                        item_ix: rank,
                        offset_in_item: px(offset),
                    });
                }
                cx.notify();
            });
        }
        if self.scroll_benchmark.is_some() && !self.minimap_visible {
            window.request_animation_frame();
        }
        if let PreviewLoadState::Ready { generation, .. } = &self.state
            && self.first_frame_scheduled != Some(*generation)
        {
            let generation = *generation;
            let opened_at = self.opened_at.unwrap_or_else(Instant::now);
            self.first_frame_scheduled = Some(generation);
            cx.on_next_frame(window, move |this, window, cx| {
                let elapsed = opened_at.elapsed();
                eprintln!(
                    "org_preview_first_readable_frame generation={} elapsed_ms={:.3}",
                    generation,
                    elapsed.as_secs_f64() * 1000.0
                );
                if generation == 1
                    && std::env::var_os("ORG_STUDIO_RELOAD_BENCH").is_some()
                    && let PreviewLoadState::Ready { document, .. } = &this.state
                {
                    this.open(document.path.clone(), cx);
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
        let key_status = self.keyboard.status().map(Arc::<str>::from);
        let which_key_items = self.which_key_items.clone();
        let dired_help_visible = self.dired_help_visible;
        let command_window_width = f32::from(window.viewport_size().width);
        let resizing_sidebar = self.sidebar_resize.is_some();
        let resize_entity = entity.clone();
        let finish_resize_entity = entity.clone();
        div()
            .relative()
            .track_focus(&focus_handle)
            .size_full()
            .bg(rgb(current_theme().background))
            .text_color(rgb(current_theme().foreground))
            .font_family("Menlo")
            .text_size(px(14.0))
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
            .on_action(cx.listener(|this, _: &OpenFileManager, _, cx| this.choose_directory(cx)))
            .on_action(cx.listener(|this, _: &ReturnToDocument, _, cx| this.return_to_document(cx)))
            .on_action(cx.listener(|this, _: &ToggleSidebar, _, cx| this.toggle_sidebar(cx)))
            .on_action(cx.listener(|this, _: &ToggleMinimap, _, cx| this.toggle_minimap(cx)))
            .on_drop(
                cx.listener(|this, paths: &ExternalPaths, _, cx| {
                    this.open_dropped_paths(paths, cx)
                }),
            )
            .child(self.workspace_body(entity, command_window_width))
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
            .when(!which_key_items.is_empty(), |view| {
                view.child(if dired_help_visible {
                    dired_help_window(which_key_items.clone(), command_window_width)
                } else {
                    which_key_window(which_key_items.clone(), command_window_width)
                })
            })
            .when_some(
                if which_key_items.is_empty() {
                    key_status
                } else {
                    None
                },
                |view, status| {
                    view.child(
                        div()
                            .absolute()
                            .left(px(108.0))
                            .bottom(px(10.0))
                            .px_2()
                            .py_1()
                            .rounded_sm()
                            .bg(rgb(current_theme().background))
                            .text_color(rgb(current_theme().foreground))
                            .text_size(px(12.0))
                            .child(status.to_string()),
                    )
                },
            )
    }
}
