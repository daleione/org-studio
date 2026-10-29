use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::Arc,
};

use gpui::{
    App, Context, Entity, FontWeight, MouseButton, Window, canvas, div, list, prelude::*, px, rgb,
};

use super::super::DiredDrag;
use super::{MenuKind, TOOLBAR_COMMANDS, WorkspaceWindow};
use crate::{
    command::{FileTreeCommand, file_tree_command},
    components::{
        UiTextExt,
        icon::{Icon, file_icon, icon},
        native_input::NativeInput,
    },
    file_manager::{ConflictPolicy, EntryId, OperationPlan, TreeRow, TreeRowKind, TreeSelection},
    theme::{NavigationColors, current_theme, navigation_colors},
    typography::ui_px,
};

struct RowContext {
    active: Option<PathBuf>,
    dirty: HashMap<PathBuf, bool>,
    drag_sources: Arc<[Arc<Path>]>,
}

impl WorkspaceWindow {
    pub(in crate::app::file_manager) fn file_sidebar(
        &self,
        entity: Entity<Self>,
        width: f32,
        window: &Window,
        cx: &App,
    ) -> gpui::Stateful<gpui::Div> {
        let theme = current_theme();
        let colors = navigation_colors();
        let tree = &self.file_manager.tree;
        tree.row_bounds.borrow_mut().clear();
        let selected_paths = tree.model.operation_targets();
        let row_context = Arc::new(RowContext {
            active: self
                .document_session()
                .and_then(|session| session.read(cx).file_path())
                .map(Path::to_path_buf),
            dirty: self
                .buffer_sessions()
                .filter_map(|session| {
                    let session = session.read(cx);
                    session
                        .file_path()
                        .map(|path| (path.to_path_buf(), session.is_dirty()))
                })
                .collect(),
            drag_sources: if selected_paths
                .iter()
                .any(|path| Some(path.as_ref()) == tree.model.root())
            {
                Arc::from([])
            } else {
                selected_paths
            },
        });
        let rows = tree.model.rows().clone();
        let input = tree.input.clone();
        let list_entity = entity.clone();
        let list_context = row_context.clone();
        let list_rows = rows.clone();
        let background_entity = entity.clone();
        let drag_entity = entity.clone();
        let bounds = tree.bounds.clone();
        let input_bounds = tree.row_bounds.clone();
        let input_cursor = tree.model.cursor();
        let navigation_input = input.clone();
        let horizontal_scroll = tree.horizontal_scroll.clone();
        let mut sidebar = div()
            .id("file-tree-sidebar")
            .w(px(width))
            .h_full()
            .flex()
            .flex_col()
            .font_family(crate::typography::ui_family("system-ui").to_owned())
            .text_color(rgb(theme.foreground))
            .bg(rgb(theme.surface))
            .on_drag_move::<DiredDrag>(move |event, _, cx| {
                let sources = event.drag(cx).sources.clone();
                drag_entity.update(cx, |this, cx| {
                    this.update_file_tree_drag(event.event.position, &sources, cx)
                });
            })
            .on_mouse_down(MouseButton::Left, move |_, _, cx| {
                background_entity.update(cx, |this, cx| {
                    this.file_manager.tree.menu = None;
                    this.file_manager.tree.model.clear_selection();
                    this.focus_sidebar(cx);
                });
            })
            .child(self.file_tree_toolbar(entity.clone(), colors, window, cx))
            .children(self.file_manager.watch_error.as_ref().map(|_| {
                let entity = entity.clone();
                div()
                    .id("file-tree-watch-error")
                    .px(ui_px(12.))
                    .py(ui_px(6.))
                    .flex()
                    .items_center()
                    .gap(ui_px(6.))
                    .text_size(ui_px(11.))
                    .text_color(rgb(colors.secondary))
                    .child(icon(Icon::Warning, ui_px(13.), theme.warning, window))
                    .child(
                        div()
                            .flex_1()
                            .child_text(self.language.text("files.watch_failed")),
                    )
                    .child(
                        div()
                            .id("file-tree-retry-watcher")
                            .cursor_pointer()
                            .text_color(rgb(colors.accent))
                            .child_text(self.language.text("files.refresh"))
                            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                            .on_click(move |_, window, cx| {
                                cx.stop_propagation();
                                entity.update(cx, |this, cx| {
                                    this.dispatch_file_tree_command(
                                        FileTreeCommand::Refresh,
                                        window,
                                        cx,
                                    )
                                });
                            }),
                    )
            }))
            .children(
                row_context
                    .active
                    .as_deref()
                    .filter(|path| {
                        tree.model
                            .root()
                            .is_some_and(|root| !path.starts_with(root))
                    })
                    .map(|_| {
                        div()
                            .px(ui_px(12.))
                            .py(ui_px(6.))
                            .text_size(ui_px(11.))
                            .text_color(rgb(colors.secondary))
                            .child_text(self.language.text("files.outside_root"))
                    }),
            )
            .child(
                canvas(
                    move |bounds_value, _, _| bounds.set(bounds_value),
                    move |bounds, _, window, cx| {
                        if let Some(input) = navigation_input
                            .as_ref()
                            .filter(|input| input.read(cx).focus.is_focused(window))
                        {
                            let row = input_cursor
                                .and_then(|id| input_bounds.borrow().get(&id).copied())
                                .unwrap_or(bounds);
                            NativeInput::register_composite_input(input, row, window, cx);
                        }
                    },
                )
                .absolute()
                .size_full(),
            );
        if rows.is_empty() {
            let button_entity = entity.clone();
            return sidebar.child(
                div()
                    .flex_1()
                    .p(ui_px(16.))
                    .flex()
                    .flex_col()
                    .justify_center()
                    .gap(ui_px(12.))
                    .text_size(ui_px(12.5))
                    .text_color(rgb(colors.secondary))
                    .child_text(self.language.text("files.no_folder"))
                    .child(
                        div()
                            .id("file-tree-open-folder")
                            .when_some(input.as_ref(), |button, input| {
                                button.track_focus(&input.read(cx).focus)
                            })
                            .px(ui_px(10.))
                            .py(ui_px(6.))
                            .rounded_md()
                            .border_1()
                            .border_color(rgb(theme.border))
                            .cursor_pointer()
                            .child_text(self.language.text("files.open_folder"))
                            .on_click(move |_, window, cx| {
                                cx.stop_propagation();
                                button_entity.update(cx, |this, cx| {
                                    this.dispatch_file_tree_command(
                                        FileTreeCommand::ChangeRoot,
                                        window,
                                        cx,
                                    )
                                });
                            }),
                    ),
            );
        }
        let focus = input.as_ref().map(|input| input.read(cx).focus.clone());
        let outline = div()
            .id("file-tree-outline")
            .flex_1()
            .min_h(px(0.))
            .flex()
            .flex_col()
            .on_scroll_wheel(move |event, _, cx| {
                let delta = event.delta.pixel_delta(px(16.));
                if horizontal_scroll.max_offset().x > px(0.) && delta.x.abs() > delta.y.abs() {
                    cx.stop_propagation();
                }
            })
            .when_some(focus.as_ref(), |outline, focus| outline.track_focus(focus))
            .child(self.file_tree_row(entity.clone(), &rows[0], &row_context, colors, window, cx))
            .child(
                div()
                    .id("file-tree-horizontal-scroll")
                    .flex_1()
                    .min_h(px(0.))
                    .overflow_x_scroll()
                    .track_scroll(&tree.horizontal_scroll)
                    .child(
                        list(tree.list.clone(), move |index, window, cx| {
                            let row = &list_rows[index + 1];
                            list_entity.read(cx).file_tree_row(
                                list_entity.clone(),
                                row,
                                &list_context,
                                colors,
                                window,
                                cx,
                            )
                        })
                        .h_full()
                        .w(px(width.max(tree.model.maximum_depth() as f32 * 16. + 180.))),
                    ),
            );
        sidebar = sidebar.child(outline);
        if let Some(input) = input
            && !input.read(cx).text.is_empty()
        {
            let query = &input.read(cx).text;
            sidebar = sidebar.child(
                div()
                    .debug_selector(|| "file-tree-name-query".into())
                    .h(ui_px(26.))
                    .px(ui_px(12.))
                    .flex()
                    .items_center()
                    .text_size(ui_px(10.5))
                    .text_color(rgb(colors.secondary))
                    .child_text(format!(
                        "{} {query}",
                        self.language.text("files.type_ahead")
                    )),
            );
        }
        if let Some(status) = self.file_manager.status.as_ref() {
            sidebar = sidebar.child(
                div()
                    .px(ui_px(12.))
                    .py(ui_px(6.))
                    .text_size(ui_px(11.))
                    .text_color(rgb(colors.secondary))
                    .child_text(status.message().to_string()),
            );
        }
        sidebar
    }

    fn file_tree_toolbar(
        &self,
        entity: Entity<Self>,
        colors: NavigationColors,
        window: &Window,
        _cx: &App,
    ) -> gpui::Div {
        let tree = &self.file_manager.tree;
        let first = usize::from(tree.model.parent_root().is_none());
        let enabled = TOOLBAR_COMMANDS.map(|command| {
            command != FileTreeCommand::EnterRoot || tree.directory_root_target(None).is_some()
        });
        if first > 0 {
            let mut bounds = tree.toolbar_bounds.get();
            bounds[0] = Default::default();
            tree.toolbar_bounds.set(bounds);
        }
        div()
            .h(ui_px(44.))
            .px(ui_px(12.))
            .flex_none()
            .flex()
            .items_center()
            .gap(ui_px(4.))
            .child(
                div()
                    .flex_1()
                    .text_size(ui_px(13.))
                    .font_weight(FontWeight::SEMIBOLD)
                    .child_text(self.language.text("files.title")),
            )
            .children(TOOLBAR_COMMANDS.into_iter().enumerate().skip(first).map(
                |(index, command)| {
                    let click_entity = entity.clone();
                    let key_entity = entity.clone();
                    let toolbar_bounds = tree.toolbar_bounds.clone();
                    let geometry_entity = entity.clone();
                    let (symbol, selector, kind) = match command {
                        FileTreeCommand::ParentRoot => (Icon::ArrowUp, "file-tree-parent", None),
                        FileTreeCommand::EnterRoot => (Icon::ArrowDown, "file-tree-enter", None),
                        FileTreeCommand::NewMenu => {
                            (Icon::Plus, "file-tree-new", Some(MenuKind::New))
                        }
                        FileTreeCommand::MoreMenu => {
                            (Icon::More, "file-tree-more", Some(MenuKind::More))
                        }
                        _ => unreachable!("file tree toolbar command"),
                    };
                    let open = kind.is_some_and(|kind| {
                        tree.menu.as_ref().is_some_and(|menu| menu.kind == kind)
                    });
                    let focus = tree.toolbar.as_ref().map(|toolbar| toolbar[index].clone());
                    let spec = file_tree_command(command);
                    let label = self.language.text(spec.label);
                    let hint = if spec.keys.is_empty() {
                        label.to_owned()
                    } else {
                        format!("{label} · {}", spec.shortcut_label())
                    };
                    div()
                        .id(("file-tree-toolbar", index))
                        .debug_selector(move || selector.into())
                        .when(!open, |button| {
                            button.tooltip(move |_, cx| cx.new(|_| TreeHint(hint.clone())).into())
                        })
                        .when_some(focus.as_ref(), |button, focus| button.track_focus(focus))
                        .size(ui_px(28.))
                        .relative()
                        .rounded_md()
                        .flex()
                        .items_center()
                        .justify_center()
                        .when(enabled[index], |button| {
                            button
                                .cursor_pointer()
                                .hover(|style| style.bg(rgb(current_theme().hover)))
                        })
                        .when(open, |button| button.bg(rgb(current_theme().hover)))
                        .focus(|style| style.border_2().border_color(rgb(colors.focus)))
                        .child(
                            div()
                                .when(!enabled[index], |icon| icon.opacity(0.4))
                                .child(icon(symbol, ui_px(16.), colors.secondary, window)),
                        )
                        .child(
                            canvas(
                                move |bounds, _, cx| {
                                    let mut buttons = toolbar_bounds.get();
                                    let changed = buttons[index] != bounds;
                                    buttons[index] = bounds;
                                    toolbar_bounds.set(buttons);
                                    if changed && open {
                                        cx.notify(geometry_entity.entity_id());
                                    }
                                },
                                |_, _, _, _| {},
                            )
                            .absolute()
                            .top_0()
                            .left_0()
                            .size_full(),
                        )
                        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                        .on_click(move |event, window, cx| {
                            cx.stop_propagation();
                            click_entity.update(cx, |this, cx| {
                                if !enabled[index] {
                                    return;
                                }
                                if let Some(kind) = kind {
                                    this.open_file_tree_menu(
                                        kind,
                                        Some(event.position()),
                                        window,
                                        cx,
                                    );
                                } else {
                                    this.file_manager.tree.menu = None;
                                    this.dispatch_file_tree_command(command, window, cx);
                                }
                            });
                        })
                        .on_key_down(move |event, window, cx| {
                            let key = event.keystroke.key.as_str();
                            key_entity.update(cx, |this, cx| {
                                match key {
                                    "enter" | "space" => {
                                        if enabled[index] {
                                            this.dispatch_file_tree_command(command, window, cx);
                                        }
                                    }
                                    "left" | "right" | "tab" => {
                                        if let Some(toolbar) = &this.file_manager.tree.toolbar {
                                            let previous =
                                                (first..index).rev().find(|next| enabled[*next]);
                                            let next = (index + 1..toolbar.len())
                                                .find(|next| enabled[*next]);
                                            let target = match key {
                                                "left" => previous.or_else(|| {
                                                    (first..toolbar.len())
                                                        .rev()
                                                        .find(|next| enabled[*next])
                                                }),
                                                "right" => next.or_else(|| {
                                                    (first..toolbar.len())
                                                        .find(|next| enabled[*next])
                                                }),
                                                _ if event.keystroke.modifiers.shift => previous,
                                                _ => next,
                                            };
                                            if let Some(target) = target {
                                                window.focus(&toolbar[target], cx);
                                            } else {
                                                this.file_manager.tree.focus_requested = true;
                                                cx.notify();
                                            }
                                        }
                                    }
                                    "escape" => {
                                        this.file_manager.tree.focus_requested = true;
                                        cx.notify();
                                    }
                                    _ => return,
                                }
                                cx.stop_propagation();
                            });
                        })
                },
            ))
    }

    fn file_tree_row(
        &self,
        entity: Entity<Self>,
        row: &TreeRow,
        row_context: &RowContext,
        colors: NavigationColors,
        window: &Window,
        cx: &App,
    ) -> gpui::AnyElement {
        let Some(entry) = row.entry() else {
            return self.file_tree_status(entity, row, colors, window);
        };
        let theme = current_theme();
        let tree = &self.file_manager.tree;
        let selected = tree.model.selected(entry.id);
        let input_focused = tree
            .input
            .as_ref()
            .is_some_and(|input| input.read(cx).focus.is_focused(window));
        let focused = window.is_window_active() && input_focused;
        let primary = tree.model.cursor() == Some(entry.id);
        let active = row_context.active.as_deref() == Some(entry.path.as_ref());
        let dirty = row_context
            .dirty
            .get(entry.path.as_ref())
            .copied()
            .unwrap_or(false);
        let foreground = if selected && focused {
            colors.selected_foreground
        } else {
            theme.foreground
        };
        let icon_color = if selected && focused {
            colors.selected_foreground
        } else if entry.is_directory() || active {
            colors.accent
        } else {
            colors.secondary
        };
        let h = ui_px(if row.depth == 0 { 32. } else { 28. });
        let id = entry.id;
        let is_directory = entry.is_directory();
        let click_entity = entity.clone();
        let context_entity = entity.clone();
        let drop_entity = entity.clone();
        let path = entry.path.clone();
        let root = tree.model.root() == Some(path.as_ref());
        let drag_sources = if root {
            Arc::from([])
        } else if selected {
            row_context.drag_sources.clone()
        } else {
            vec![path.clone()].into()
        };
        let can_drag = !drag_sources.is_empty();
        let row_bounds = tree.row_bounds.clone();
        let mut contents = div()
            .id(("file-tree-row", id.0 as usize))
            .debug_selector(move || {
                if primary {
                    "file-tree-current-row".into()
                } else {
                    format!("file-tree-row-{}", id.0)
                }
            })
            .relative()
            .w_full()
            .h(h)
            .rounded_md()
            .flex()
            .items_center()
            .pl(ui_px(row.depth as f32 * 16.))
            .pr(ui_px(8.))
            .text_size(ui_px(13.))
            .text_color(rgb(foreground))
            .cursor_pointer()
            .when(active, |row| row.bg(rgb(colors.accent).opacity(0.1)))
            .when(selected && (focused || !active), |row| {
                row.bg(rgb(if focused {
                    colors.selected
                } else {
                    colors.inactive
                }))
            })
            .when(primary && focused, |row| {
                row.border_1().border_color(rgb(if selected {
                    colors.selected_foreground
                } else {
                    colors.focus
                }))
            })
            .hover(move |style| {
                if selected || active {
                    style
                } else {
                    style.bg(rgb(theme.hover))
                }
            })
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .on_click(move |event, _, cx| {
                cx.stop_propagation();
                let modifiers = event.modifiers();
                click_entity.update(cx, |this, cx| {
                    this.focus_sidebar(cx);
                    this.file_manager.tree.menu = None;
                    let mode = if modifiers.platform {
                        TreeSelection::Toggle
                    } else if modifiers.shift {
                        TreeSelection::Extend
                    } else {
                        TreeSelection::Single
                    };
                    this.file_manager.tree.model.select(id, mode);
                    if mode == TreeSelection::Single {
                        this.open_file_tree_selection(event.click_count() >= 2, cx);
                    }
                    this.sync_file_directory_watches(cx);
                    this.reveal_file_tree_cursor(cx);
                });
            })
            .on_mouse_down(MouseButton::Right, move |event, window, cx| {
                cx.stop_propagation();
                context_entity.update(cx, |this, cx| {
                    if !this.file_manager.tree.model.selected(id) {
                        this.file_manager
                            .tree
                            .model
                            .select(id, TreeSelection::Single);
                    }
                    this.open_file_tree_menu(MenuKind::Context, Some(event.position), window, cx);
                });
            })
            .when(can_drag, |row| {
                row.on_drag(
                    DiredDrag {
                        sources: drag_sources,
                    },
                    |drag, _, _, cx| {
                        cx.new(|_| {
                            DragLabel(
                                drag.sources
                                    .first()
                                    .and_then(|path| path.file_name())
                                    .unwrap_or_default()
                                    .to_string_lossy()
                                    .into_owned(),
                            )
                        })
                    },
                )
            })
            .when(is_directory, |row| {
                row.drag_over::<DiredDrag>(|style, _, _, _| style.bg(rgb(current_theme().selected)))
                    .on_drop::<DiredDrag>(move |drag, _, cx| {
                        cx.stop_propagation();
                        let plan = OperationPlan::move_to(
                            drag.sources.clone(),
                            path.to_path_buf(),
                            ConflictPolicy::Error,
                        );
                        drop_entity.update(cx, |this, cx| {
                            this.finish_file_tree_drag(true, cx);
                            this.execute_dired_operation(plan, cx);
                        });
                    })
            })
            .child(
                div()
                    .id(("file-tree-disclosure", id.0 as usize))
                    .when(primary, |caret| {
                        caret.debug_selector(|| "file-tree-current-disclosure".into())
                    })
                    .w(ui_px(20.))
                    .h_full()
                    .flex_none()
                    .flex()
                    .items_center()
                    .justify_center()
                    .when(is_directory, |caret| {
                        caret.child(icon(
                            if tree.model.expanded(&entry.path) {
                                Icon::ChevronDown
                            } else {
                                Icon::ChevronRight
                            },
                            ui_px(10.),
                            icon_color,
                            window,
                        ))
                    }),
            )
            .child(
                file_icon(
                    entry,
                    ui_px(16.),
                    icon_color,
                    selected && focused,
                    window,
                    cx,
                )
                .when(primary, |icon| {
                    icon.debug_selector(|| "file-tree-current-icon".into())
                }),
            );
        let editing = tree
            .edit
            .as_ref()
            .filter(|edit| edit.source.as_deref() == Some(entry.path.as_ref()));
        contents = if let Some(edit) = editing {
            contents.child(
                div()
                    .ml(ui_px(6.))
                    .flex_1()
                    .min_w(px(0.))
                    .child(edit.input.clone()),
            )
        } else {
            contents.child(
                div()
                    .when(primary, |name| {
                        name.debug_selector(|| "file-tree-current-name".into())
                    })
                    .ml(ui_px(6.))
                    .flex_1()
                    .min_w(px(0.))
                    .overflow_hidden()
                    .whitespace_nowrap()
                    .text_ellipsis_middle()
                    .when(active || root, |name| {
                        name.font_weight(FontWeight::SEMIBOLD)
                    })
                    .child(file_name(id, entry.display_name.clone().into())),
            )
        };
        if dirty {
            contents = contents.child(
                div()
                    .ml(ui_px(6.))
                    .size(ui_px(6.))
                    .rounded_full()
                    .bg(rgb(foreground)),
            );
        }
        contents = contents.child(
            canvas(
                move |bounds, _, _| {
                    row_bounds.borrow_mut().insert(id, bounds);
                },
                |_, _, _, _| {},
            )
            .absolute()
            .size_full(),
        );
        let mut outer = div().w_full().px(ui_px(8.)).child(contents);
        if let Some(error) = editing.and_then(|edit| edit.error.as_ref()) {
            outer = outer.child(
                div()
                    .id("file-tree-name-error")
                    .pl(ui_px(row.depth as f32 * 16. + 42.))
                    .py(ui_px(4.))
                    .text_size(ui_px(11.))
                    .text_color(rgb(theme.error))
                    .child_text(error.clone()),
            );
        }
        if let Some(edit) = tree
            .edit
            .as_ref()
            .filter(|edit| edit.source.is_none() && edit.directory == entry.path.as_ref())
        {
            outer = outer.child(
                div()
                    .pl(ui_px((row.depth + 1) as f32 * 16. + 20.))
                    .flex()
                    .items_center()
                    .gap(ui_px(6.))
                    .min_h(ui_px(28.))
                    .child(icon(
                        if edit.folder {
                            Icon::Folder
                        } else {
                            Icon::Text
                        },
                        ui_px(16.),
                        colors.secondary,
                        window,
                    ))
                    .child(div().flex_1().min_w(px(0.)).child(edit.input.clone())),
            );
            if let Some(error) = &edit.error {
                outer = outer.child(
                    div()
                        .id("file-tree-name-error")
                        .py(ui_px(4.))
                        .text_size(ui_px(11.))
                        .text_color(rgb(theme.error))
                        .child_text(error.clone()),
                );
            }
        }
        outer.into_any_element()
    }

    fn file_tree_status(
        &self,
        entity: Entity<Self>,
        row: &TreeRow,
        colors: NavigationColors,
        window: &Window,
    ) -> gpui::AnyElement {
        let label = match &row.kind {
            TreeRowKind::Loading => self.language.text("files.loading"),
            TreeRowKind::Empty => self.language.text("files.empty"),
            TreeRowKind::Error(_) => self.language.text("files.read_failed"),
            TreeRowKind::Entry(_) => unreachable!(),
        };
        let mut status = div()
            .w_full()
            .pl(ui_px(8. + row.depth as f32 * 16. + 20.))
            .pr(ui_px(8.))
            .py(ui_px(5.))
            .flex()
            .flex_col()
            .gap(ui_px(4.))
            .text_size(ui_px(12.))
            .text_color(rgb(colors.secondary));
        if let TreeRowKind::Error(error) = &row.kind {
            let directory = row.parent.clone();
            status = status
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(ui_px(6.))
                        .child(icon(
                            Icon::Warning,
                            ui_px(13.),
                            current_theme().warning,
                            window,
                        ))
                        .child_text(label),
                )
                .child_text(error.to_string())
                .child(
                    div()
                        .id(("file-tree-retry", EntryId::for_path(&directory).0 as usize))
                        .when_some(
                            self.file_manager.tree.retry_focus.get(&row.parent),
                            |button, focus| button.track_focus(focus),
                        )
                        .py(ui_px(3.))
                        .text_color(rgb(colors.accent))
                        .cursor_pointer()
                        .child_text(self.language.text("files.retry"))
                        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                        .on_key_down({
                            let entity = entity.clone();
                            let directory = directory.clone();
                            move |event, _, cx| {
                                let key = event.keystroke.key.as_str();
                                entity.update(cx, |this, cx| match key {
                                    "enter" | "space" => {
                                        this.request_file_directory(directory.clone(), true, cx);
                                        this.rebuild_file_tree(cx);
                                    }
                                    "tab" if event.keystroke.modifiers.shift => {
                                        this.file_manager.tree.focus_requested = true;
                                        cx.notify();
                                    }
                                    "tab" | "escape" => this.focus_document(cx),
                                    _ => (),
                                });
                                if matches!(key, "enter" | "space" | "tab" | "escape") {
                                    cx.stop_propagation();
                                }
                            }
                        })
                        .on_click(move |_, _, cx| {
                            cx.stop_propagation();
                            entity.update(cx, |this, cx| {
                                this.request_file_directory(directory.clone(), true, cx);
                                this.rebuild_file_tree(cx);
                            });
                        }),
                );
        } else {
            status = status.child_text(label);
        }
        status.into_any_element()
    }
}

struct DragLabel(String);
impl gpui::Render for DragLabel {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div()
            .px(ui_px(10.))
            .py(ui_px(6.))
            .rounded_md()
            .shadow_md()
            .bg(rgb(current_theme().surface))
            .text_color(rgb(current_theme().foreground))
            .child_text(self.0.clone())
    }
}

struct TreeHint(String);
impl gpui::Render for TreeHint {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div()
            .debug_selector(|| "file-tree-hint".into())
            .max_w(ui_px(360.))
            .px(ui_px(10.))
            .py(ui_px(6.))
            .rounded_md()
            .shadow_md()
            .bg(rgb(current_theme().elevated))
            .text_color(rgb(current_theme().foreground))
            .font_family(crate::typography::ui_family("system-ui").to_owned())
            .text_size(ui_px(12.))
            .child_text(self.0.clone())
    }
}

/// The tooltip callback runs after layout, so only ellipsized names need a hint.
fn file_name(id: EntryId, name: gpui::SharedString) -> gpui::InteractiveText {
    let text = gpui::StyledText::new(name.clone());
    let layout = text.layout().clone();
    gpui::InteractiveText::new(("file-tree-name", id.0 as usize), text).tooltip(move |_, _, cx| {
        (layout.text() != name.as_ref()).then(|| cx.new(|_| TreeHint(name.to_string())).into())
    })
}
