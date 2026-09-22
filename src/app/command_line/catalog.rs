use crate::{
    command::{CommandKey, CommandRegistry, TableScope},
    i18n::Language,
    preview::*,
};

#[derive(Clone, Debug)]
pub(super) struct Entry {
    pub key: CommandKey,
    pub input: String,
    pub description: String,
    pub scope: Option<TableScope>,
    words: String,
}

pub(super) fn entries(
    commands: &CommandRegistry,
    language: Language,
    read_only: bool,
    editing: bool,
    selection: bool,
) -> Vec<Entry> {
    let chinese = language == Language::Chinese;
    let specs = [
        (
            "org-studio.table.align",
            "table-align --all",
            "对齐所有表格",
            "Align all tables",
            Some(TableScope::Document),
        ),
        (
            "org-studio.table.align",
            "table-align",
            "对齐当前表格",
            "Align current table",
            Some(TableScope::Current),
        ),
        (
            "org-studio.table.align",
            "table-align --selection",
            "对齐选区中的表格",
            "Align tables in selection",
            Some(TableScope::Selection),
        ),
        (
            SAVE_DOCUMENT_COMMAND,
            "save",
            "保存文档",
            "Save document",
            None,
        ),
        (
            RELOAD_DOCUMENT_COMMAND,
            "reload",
            "重新加载文档",
            "Reload document",
            None,
        ),
        (
            SHOW_READING_COMMAND,
            "preview",
            "切换预览",
            "Show preview",
            None,
        ),
        (SHOW_EDITOR_COMMAND, "edit", "切换编辑", "Show editor", None),
        (
            SHOW_SPLIT_COMMAND,
            "split",
            "切换分屏",
            "Show split view",
            None,
        ),
        (
            EXPORT_DOCUMENT_COMMAND,
            "export",
            "导出文档",
            "Export document",
            None,
        ),
        (UNDO_DOCUMENT_COMMAND, "undo", "撤销", "Undo", None),
        (REDO_DOCUMENT_COMMAND, "redo", "重做", "Redo", None),
    ];
    specs
        .into_iter()
        .filter_map(|(id, input, zh, en, scope)| {
            let descriptor = commands.descriptor(commands.key(id)?)?;
            if (read_only
                && (scope.is_some() || matches!(input, "save" | "reload" | "undo" | "redo")))
                || (scope == Some(TableScope::Selection) && (!editing || !selection))
                || (scope == Some(TableScope::Current) && !editing)
            {
                return None;
            }
            let description = match scope {
                Some(TableScope::Document) => {
                    if chinese {
                        "当前文档 · 包含折叠表格 · 一次撤销"
                    } else {
                        "Current document · includes folded tables · one undo"
                    }
                }
                Some(TableScope::Selection) => {
                    if chinese {
                        "对齐选区涉及的完整表格"
                    } else {
                        "Align whole tables touched by the selection"
                    }
                }
                Some(TableScope::Current) => {
                    if chinese {
                        "对齐光标所在表格"
                    } else {
                        "Align the table at the caret"
                    }
                }
                None => descriptor.description.as_ref(),
            }
            .to_owned();
            let aliases = descriptor
                .aliases
                .iter()
                .map(|a| a.as_str())
                .collect::<Vec<_>>()
                .join(" ");
            Some(Entry {
                key: descriptor.key,
                input: input.into(),
                description,
                scope,
                words: format!("{input} {zh} {en} {id} {aliases}").to_lowercase(),
            })
        })
        .collect()
}

pub(super) fn normalize(query: &str) -> &str {
    query.trim().trim_start_matches(':').trim_start()
}

pub(super) fn candidates(all: &[Entry], query: &str, history: &[String]) -> Vec<Entry> {
    let query = normalize(query).to_lowercase();
    let query = if let Some(arguments) = query.strip_prefix("org-studio.table.align") {
        format!("table-align{arguments}")
    } else if query == "w" {
        "save".to_owned()
    } else {
        query
    };
    // An unavailable current-table command must not fall back to formatting the whole document.
    if query == "table-align" && !all.iter().any(|entry| entry.input == query) {
        return Vec::new();
    }
    let mut found = all
        .iter()
        .filter(|entry| {
            query
                .split_whitespace()
                .all(|word| entry.words.contains(word))
        })
        .cloned()
        .collect::<Vec<_>>();
    found.sort_by_key(|entry| {
        (
            entry.input != query,
            if query.is_empty() {
                history
                    .iter()
                    .position(|h| h == &entry.input)
                    .unwrap_or(usize::MAX)
            } else {
                0
            },
        )
    });
    found
}

/// Flags are syntax once a command is named. Never execute a fuzzy fallback for an invalid flag.
pub(super) fn validate(query: &str) -> Result<(), &'static str> {
    let mut words = normalize(query).split_whitespace();
    let Some(command) = words.next() else {
        return Ok(());
    };
    if matches!(command, "table-align" | "org-studio.table.align") {
        let args = words.collect::<Vec<_>>();
        if !matches!(args.as_slice(), [] | ["--all"] | ["--selection"]) {
            return Err("table-align [--all | --selection]");
        }
    } else if matches!(
        command,
        "save" | "w" | "reload" | "preview" | "edit" | "split" | "export" | "undo" | "redo"
    ) && words.next().is_some()
    {
        return Err("此命令不接受参数 / This command takes no arguments");
    }
    Ok(())
}
