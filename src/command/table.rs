/// Structural edits share one transaction and keep the caret in its logical cell.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TableEdit {
    InsertRow,
    InsertRowBelow,
    KillRow,
    MoveRowUp,
    MoveRowDown,
    InsertColumn,
    DeleteColumn,
    MoveColumnLeft,
    MoveColumnRight,
    InsertHline,
    InsertHlineAbove,
    HlineAndMove,
    Sort { numeric: bool, reverse: bool },
}

pub struct TableCommand {
    pub name: &'static str,
    pub alias: &'static str,
    pub zh: &'static str,
    pub en: &'static str,
    pub edit: TableEdit,
}

pub const TABLE_COMMANDS: &[TableCommand] = &[
    TableCommand {
        name: "org-table-insert-row",
        alias: "table-insert-row",
        zh: "在当前行上方插入行",
        en: "Insert row above",
        edit: TableEdit::InsertRow,
    },
    TableCommand {
        name: "org-table-insert-row-below",
        alias: "table-insert-row-below",
        zh: "在当前行下方插入行",
        en: "Insert row below",
        edit: TableEdit::InsertRowBelow,
    },
    TableCommand {
        name: "org-table-kill-row",
        alias: "table-kill-row",
        zh: "剪切当前行",
        en: "Kill current row",
        edit: TableEdit::KillRow,
    },
    TableCommand {
        name: "org-table-move-row-up",
        alias: "table-move-row-up",
        zh: "上移当前行",
        en: "Move row up",
        edit: TableEdit::MoveRowUp,
    },
    TableCommand {
        name: "org-table-move-row-down",
        alias: "table-move-row-down",
        zh: "下移当前行",
        en: "Move row down",
        edit: TableEdit::MoveRowDown,
    },
    TableCommand {
        name: "org-table-insert-column",
        alias: "table-insert-column",
        zh: "在当前列左侧插入列",
        en: "Insert column before",
        edit: TableEdit::InsertColumn,
    },
    TableCommand {
        name: "org-table-delete-column",
        alias: "table-delete-column",
        zh: "删除当前列",
        en: "Delete current column",
        edit: TableEdit::DeleteColumn,
    },
    TableCommand {
        name: "org-table-move-column-left",
        alias: "table-move-column-left",
        zh: "左移当前列",
        en: "Move column left",
        edit: TableEdit::MoveColumnLeft,
    },
    TableCommand {
        name: "org-table-move-column-right",
        alias: "table-move-column-right",
        zh: "右移当前列",
        en: "Move column right",
        edit: TableEdit::MoveColumnRight,
    },
    TableCommand {
        name: "org-table-insert-hline",
        alias: "table-insert-hline",
        zh: "在下方插入水平分隔线",
        en: "Insert horizontal separator below",
        edit: TableEdit::InsertHline,
    },
    TableCommand {
        name: "org-table-hline-and-move",
        alias: "table-hline-and-move",
        zh: "插入分隔线并进入下一行",
        en: "Insert separator and move to next row",
        edit: TableEdit::HlineAndMove,
    },
    TableCommand {
        name: "org-table-insert-hline-above",
        alias: "table-insert-hline-above",
        zh: "在上方插入水平分隔线",
        en: "Insert horizontal separator above",
        edit: TableEdit::InsertHlineAbove,
    },
    TableCommand {
        name: "org-table-sort-lines",
        alias: "table-sort",
        zh: "按当前列排序 · --numeric 数值 · --reverse 降序",
        en: "Sort by current column · --numeric · --reverse",
        edit: TableEdit::Sort {
            numeric: false,
            reverse: false,
        },
    },
];
