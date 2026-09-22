use crate::{
    command::{CommandKey, CommandRegistry, TABLE_COMMANDS, TableEdit, TableScope},
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
        (
            "org-studio.document.goto-line",
            "goto-line",
            "跳到指定行号",
            "Go to source line",
            None,
        ),
        (
            "org-studio.workspace.switch-buffer",
            "quick-open",
            "快速打开与跳转",
            "Quick open and navigate",
            None,
        ),
        (
            "org-studio.workspace.navigate-back",
            "navigate-back",
            "返回上个位置",
            "Go back",
            None,
        ),
        (
            "org-studio.workspace.navigate-forward",
            "navigate-forward",
            "前往下个位置",
            "Go forward",
            None,
        ),
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
                None if input == "goto-line" => {
                    if chinese {
                        "goto-line <行号> · 源文件行号从 1 开始"
                    } else {
                        "goto-line <line> · source line numbers start at 1"
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

pub(super) fn table_entries(
    commands: &CommandRegistry,
    language: Language,
    available: &[TableEdit],
) -> Vec<Entry> {
    TABLE_COMMANDS
        .iter()
        .filter(|spec| available.contains(&spec.edit))
        .filter_map(|spec| {
            Some(Entry {
                key: commands.key(spec.name)?,
                input: spec.alias.into(),
                description: if language == Language::Chinese {
                    spec.zh
                } else {
                    spec.en
                }
                .into(),
                scope: None,
                words: format!("{} {} {} {}", spec.name, spec.alias, spec.zh, spec.en)
                    .to_lowercase(),
            })
        })
        .collect()
}

pub(super) fn candidates(all: &[Entry], query: &str, history: &[String]) -> Vec<Entry> {
    let query = normalize(query).to_lowercase();
    let query = if let Some(spec) = TABLE_COMMANDS
        .iter()
        .find(|spec| query.split_whitespace().next() == Some(spec.name))
    {
        format!("{}{}", spec.alias, &query[spec.name.len()..])
    } else {
        query
    };
    let query = if let Some(arguments) = query.strip_prefix("org-studio.table.align") {
        format!("table-align{arguments}")
    } else if let Some(arguments) = query.strip_prefix("org-studio.document.goto-line") {
        format!("goto-line{arguments}")
    } else if query == "w" {
        "save".to_owned()
    } else {
        query
    };
    // Preserve arguments when completing or selecting a parameterized command.
    let command = query.split_whitespace().next().unwrap_or("");
    if matches!(command, "goto-line" | "table-sort") {
        return all
            .iter()
            .filter(|entry| entry.input == command)
            .cloned()
            .map(|mut entry| {
                entry.input = query.clone();
                entry
            })
            .collect();
    }
    // An unavailable current-table command must not fall back to formatting the whole document.
    if (query == "table-align" || TABLE_COMMANDS.iter().any(|spec| spec.alias == query))
        && !all.iter().any(|entry| entry.input == query)
    {
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
                    .position(|h| {
                        h == &entry.input
                            || (entry.input == "goto-line" && h.starts_with("goto-line "))
                    })
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
    } else if matches!(command, "table-sort" | "org-table-sort-lines") {
        sort_edit(query)?;
    } else if (TABLE_COMMANDS
        .iter()
        .any(|spec| command == spec.name || command == spec.alias)
        || matches!(
            command,
            "save" | "w" | "reload" | "preview" | "edit" | "split" | "export" | "undo" | "redo"
        ))
        && words.next().is_some()
    {
        return Err("此命令不接受参数 / This command takes no arguments");
    }
    Ok(())
}

pub(super) fn sort_edit(input: &str) -> Result<TableEdit, &'static str> {
    let mut numeric = false;
    let mut reverse = false;
    for flag in normalize(input).split_whitespace().skip(1) {
        match flag {
            "--numeric" if !numeric => numeric = true,
            "--reverse" if !reverse => reverse = true,
            _ => return Err("table-sort [--numeric] [--reverse]"),
        }
    }
    Ok(TableEdit::Sort { numeric, reverse })
}

pub(super) fn line_number(input: &str) -> Result<u64, &'static str> {
    let mut words = normalize(input).split_whitespace().skip(1);
    let line = words
        .next()
        .filter(|value| value.bytes().all(|b| b.is_ascii_digit()));
    match line.and_then(|value| value.parse::<u64>().ok()) {
        Some(line) if line > 0 && words.next().is_none() => Ok(line),
        _ => Err("goto-line <行号 / line> · 请输入正整数 / Enter a positive integer"),
    }
}
