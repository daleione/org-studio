//! Mouse table actions use source cell coordinates; popovers never enter text layout.
use super::*;
use crate::{
    command::TableEdit,
    document::{DocumentFormat, table as source_table},
};
use gpui::{AnyElement, KeyDownEvent, ScrollHandle, anchored, deferred, point, rgba, size};

#[cfg(test)]
mod tests;
mod view;

const BUTTON_SIZE: f32 = 20.;

#[derive(Clone, Debug, PartialEq)]
struct CellHit {
    revision: Revision,
    offset: ByteOffset,
    line: LineIndex,
    column: usize,
    header: bool,
    bounds: Bounds<Pixels>,
    button: Bounds<Pixels>,
}

#[derive(Default)]
pub(super) struct TableActions {
    hover: Option<CellHit>,
    popup: Option<TableMenu>,
}

struct TableMenu {
    target: CellHit,
    viewport: Bounds<Pixels>,
    items: Vec<MenuItem>,
    selected: usize,
    sorting: bool,
    focus: FocusHandle,
    focus_pending: bool,
    scroll: ScrollHandle,
}

#[derive(Clone, Copy, PartialEq)]
enum MenuAction {
    Edit(TableEdit),
    Align,
    Sort,
}

struct MenuItem {
    action: MenuAction,
    zh: &'static str,
    en: &'static str,
    shortcut: &'static str,
    group: u8,
}

impl SemanticEditor {
    pub(super) fn hovered_table_cell(&self) -> Option<(LineIndex, usize, bool)> {
        self.table_actions
            .hover
            .as_ref()
            .filter(|_| self.table_actions.popup.is_none())
            .map(|hit| (hit.line, hit.column, hit.header))
    }

    pub(crate) fn table_menu_is_open(&self) -> bool {
        self.table_actions.popup.is_some()
    }

    fn table_cell_hit(&self, position: Point<Pixels>, cx: &App) -> Option<CellHit> {
        let viewport = self.viewport?;
        if self.is_read_only(cx) || !viewport.contains(&position) {
            return None;
        }
        let row = self
            .hit_rows
            .iter()
            .find(|row| position.y >= row.visible_top && position.y < row.visible_bottom)?;
        if position.x < row.text_origin_x || row.inline_image_preview {
            return None;
        }
        let snapshot = self.snapshot(cx);
        let range = snapshot.line_content_range(row.line).ok()?;
        let text = snapshot.copy_range(range);
        let format = DocumentFormat::detect(self.session.read(cx).syntax_path())?;
        if !source_table::is_table_row(&text, format) {
            return None;
        }
        let query = syntax::SparseEditorStyleSnapshot::query_lines(
            self.session.read(cx).syntax_path(),
            &snapshot,
            &[row.line.0],
            &self.syntax_service,
        );
        if query.snapshot.line(row.line.0)?.id != syntax::EditorStyleId::Table {
            return None;
        }
        let parsed = source_table::parse_line(&text, format);
        let header = org_commands::table_start(&snapshot, row.line.0, format) == Some(row.line.0)
            && snapshot
                .line_content_range(LineIndex(row.line.0 + 1))
                .ok()
                .is_some_and(|range| {
                    let next = snapshot.copy_range(range);
                    source_table::is_table_row(&next, format) && source_table::is_separator(&next)
                });
        let delimiters = row.table_layout.as_ref().map(|table| {
            table
                .fragments
                .iter()
                .filter(|fragment| fragment.delimiter)
                .collect::<Vec<_>>()
        });
        for (column, cell) in parsed.cells.iter().enumerate() {
            let start = row.display.source_to_display(cell.raw_range.start);
            let end = row.display.source_to_display(cell.raw_range.end);
            let (left, right, top, height) = if let Some(table) = &row.table_layout {
                let delimiters = delimiters.as_ref()?;
                let left = delimiters.get(column)?;
                let right = delimiters.get(column + 1)?;
                (
                    left.x + left.width,
                    right.x,
                    row.origin_y,
                    row.line_height * table.visual_rows,
                )
            } else {
                let left = row.position_for_display_index(start)?;
                let right = row.position_for_display_index(end)?;
                if left.y != right.y {
                    continue;
                }
                (left.x, right.x, row.origin_y + left.y, row.line_height)
            };
            let bounds = Bounds::new(
                point(row.text_origin_x + left, top),
                size(right - left, height),
            )
            .intersect(&viewport);
            if !bounds.contains(&position) {
                continue;
            }
            let offset = ByteOffset(range.start.0 + cell.text_range.start as u64);
            if format == DocumentFormat::Markdown
                && !org_commands::EditorCommandContext::at(
                    self.session.read(cx).syntax_path(),
                    &snapshot,
                    offset,
                )
                .is_some_and(|context| {
                    matches!(
                        context.kind,
                        org_commands::EditorCommandKind::TableCell { .. }
                    )
                })
            {
                return None;
            }
            // Use the final source space, or the visual right padding of a wrapped cell.
            // A fixed-width button would overlap the last letters of a full cell.
            let padding_start = row
                .table_layout
                .as_ref()
                .and_then(|table| {
                    table
                        .fragments
                        .iter()
                        .find(|fragment| {
                            !fragment.delimiter
                                && fragment.display_range == (start..end)
                                && fragment.content_range != fragment.display_range
                        })
                        .map(|fragment| fragment.x + fragment.width)
                })
                .or_else(|| {
                    let last = text[cell.raw_range.clone()].chars().next_back()?;
                    if !last.is_whitespace() {
                        return None;
                    }
                    let index = row
                        .display
                        .source_to_display(cell.raw_range.end - last.len_utf8());
                    row.position_for_display_index(index)
                        .map(|position| position.x)
                })
                .unwrap_or(right);
            let button_width = (right - padding_start - px(2.)).clamp(px(0.), px(BUTTON_SIZE));
            let button = Bounds::new(
                point(
                    row.text_origin_x + (padding_start + right - button_width) / 2.,
                    top.max(row.visible_top),
                ),
                size(button_width, row.line_height),
            );
            return Some(CellHit {
                revision: snapshot.revision(),
                offset,
                line: row.line,
                column,
                header,
                bounds,
                button,
            });
        }
        None
    }

    pub(super) fn table_hover(&mut self, position: Point<Pixels>, cx: &mut Context<Self>) {
        if self.table_actions.popup.is_some() {
            return;
        }
        if self
            .table_actions
            .hover
            .as_ref()
            .is_some_and(|hit| hit.button.contains(&position) || hit.bounds.contains(&position))
            && !self.is_selecting
        {
            return;
        }
        let hit = if self.is_selecting {
            None
        } else {
            self.table_cell_hit(position, cx)
        };
        if self.table_actions.hover != hit {
            self.table_actions.hover = hit;
            cx.notify();
        }
    }

    pub(super) fn dismiss_table_actions(&mut self, cx: &mut Context<Self>) {
        if self.table_actions.popup.take().is_some() {
            self.autofocus = true;
            cx.notify();
        }
        if self.table_actions.hover.take().is_some() {
            cx.notify();
        }
    }

    fn open_table_menu(&mut self, target: CellHit, cx: &mut Context<Self>) {
        self.finish_composition(cx);
        let snapshot = self.snapshot(cx);
        if snapshot.revision() != target.revision {
            return;
        }
        let Some(context) = org_commands::EditorCommandContext::at(
            self.session.read(cx).syntax_path(),
            &snapshot,
            target.offset,
        ) else {
            return;
        };
        let Some(table) = org_commands::EditableTable::at(&snapshot, &context) else {
            return;
        };
        let Some(viewport) = self.viewport else {
            return;
        };
        let items = menu_items()
            .into_iter()
            .filter(|item| match item.action {
                MenuAction::Edit(edit) => table.available(edit),
                MenuAction::Sort => table.available(TableEdit::Sort {
                    numeric: false,
                    reverse: false,
                }),
                MenuAction::Align => true,
            })
            .collect();
        self.dismiss_inline(cx);
        self.dismiss_todo(cx);
        self.dismiss_timestamp(cx);
        self.is_selecting = false;
        self.table_actions.popup = Some(TableMenu {
            target,
            viewport,
            items,
            selected: 0,
            sorting: false,
            focus: cx.focus_handle(),
            focus_pending: true,
            scroll: ScrollHandle::new(),
        });
        cx.notify();
    }

    pub(super) fn table_right_click(
        &mut self,
        event: &MouseDownEvent,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(hit) = self.table_cell_hit(event.position, cx) {
            self.open_table_menu(hit, cx);
            cx.stop_propagation();
        }
    }

    fn table_menu_choose(&mut self, action: MenuAction, cx: &mut Context<Self>) {
        let Some(popup) = &mut self.table_actions.popup else {
            return;
        };
        if action == MenuAction::Sort {
            popup.sorting = true;
            popup.selected = 0;
            popup.scroll.set_offset(Point::default());
            cx.notify();
            return;
        }
        let target = popup.target.clone();
        self.dismiss_table_actions(cx);
        if self.snapshot(cx).revision() != target.revision || self.is_read_only(cx) {
            return;
        }
        let result = match action {
            MenuAction::Edit(edit) => self.edit_table_at(edit, Selection::caret(target.offset), cx),
            MenuAction::Align => {
                if self.align_table_at(target.offset, cx) {
                    Ok(())
                } else {
                    Err("当前光标不在表格中 / No table at point")
                }
            }
            MenuAction::Sort => unreachable!(),
        };
        if let Err(message) = result {
            self.show_command_feedback(message, cx);
        }
    }

    fn table_menu_key(&mut self, event: &KeyDownEvent, _: &mut Window, cx: &mut Context<Self>) {
        cx.stop_propagation();
        let Some(popup) = self.table_actions.popup.as_mut() else {
            return;
        };
        let key = event.keystroke.key.as_str();
        let control = event.keystroke.modifiers.control;
        match key {
            "escape" | "g" if key == "escape" || control => self.dismiss_table_actions(cx),
            "left" if popup.sorting => {
                popup.sorting = false;
                popup.selected = popup
                    .items
                    .iter()
                    .position(|item| item.action == MenuAction::Sort)
                    .unwrap_or(0);
                popup.scroll.scroll_to_item(popup.selected);
                cx.notify();
            }
            "up" | "down" | "n" | "p" if matches!(key, "up" | "down") || control => {
                let count = if popup.sorting {
                    sort_items().len()
                } else {
                    popup.items.len()
                };
                let down = matches!(key, "down" | "n");
                popup.selected = if down {
                    (popup.selected + 1) % count
                } else {
                    (popup.selected + count - 1) % count
                };
                popup.scroll.scroll_to_item(popup.selected);
                cx.notify();
            }
            "enter" | "space" | "right" => {
                let action = if popup.sorting {
                    sort_items()[popup.selected].action
                } else {
                    popup.items[popup.selected].action
                };
                if key != "right" || action == MenuAction::Sort {
                    self.table_menu_choose(action, cx);
                }
            }
            _ => {}
        }
    }
}

fn menu_items() -> Vec<MenuItem> {
    use TableEdit::*;
    [
        (
            MenuAction::Edit(InsertRow),
            "上方插入行",
            "Insert row above",
            "⌥⇧↓",
            0,
        ),
        (
            MenuAction::Edit(InsertRowBelow),
            "下方插入行",
            "Insert row below",
            "",
            0,
        ),
        (
            MenuAction::Edit(InsertColumn),
            "左侧插入列",
            "Insert column before",
            "⌥⇧→",
            0,
        ),
        (
            MenuAction::Edit(MoveRowUp),
            "上移行",
            "Move row up",
            "⌥↑",
            1,
        ),
        (
            MenuAction::Edit(MoveRowDown),
            "下移行",
            "Move row down",
            "⌥↓",
            1,
        ),
        (
            MenuAction::Edit(MoveColumnLeft),
            "左移列",
            "Move column left",
            "⌥←",
            1,
        ),
        (
            MenuAction::Edit(MoveColumnRight),
            "右移列",
            "Move column right",
            "⌥→",
            1,
        ),
        (MenuAction::Align, "对齐表格", "Align table", "⌃C ⌃C", 2),
        (
            MenuAction::Sort,
            "按当前列排序",
            "Sort by this column",
            "›",
            2,
        ),
        (
            MenuAction::Edit(InsertHline),
            "插入分隔线",
            "Insert separator",
            "⌃C −",
            2,
        ),
        (MenuAction::Edit(KillRow), "剪切当前行", "Cut row", "⌥⇧↑", 3),
        (
            MenuAction::Edit(DeleteColumn),
            "删除当前列",
            "Delete column",
            "⌥⇧←",
            3,
        ),
    ]
    .into_iter()
    .map(|(action, zh, en, shortcut, group)| MenuItem {
        action,
        zh,
        en,
        shortcut,
        group,
    })
    .collect()
}

fn sort_items() -> [MenuItem; 4] {
    [
        (false, false, "文本升序", "Text ascending"),
        (false, true, "文本降序", "Text descending"),
        (true, false, "数值升序", "Numbers ascending"),
        (true, true, "数值降序", "Numbers descending"),
    ]
    .map(|(numeric, reverse, zh, en)| MenuItem {
        action: MenuAction::Edit(TableEdit::Sort { numeric, reverse }),
        zh,
        en,
        shortcut: "",
        group: 0,
    })
}
