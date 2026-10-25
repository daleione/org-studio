use crate::{
    command::{
        CommandDescriptor, CommandKey, CommandRegistry, TABLE_COMMANDS, TableEdit, TableScope,
    },
    document::DocumentFormat,
    i18n::Language,
    preview::*,
};
use std::ops::Range;

#[cfg(test)]
mod tests;

#[derive(Clone, Debug)]
pub(super) struct Entry {
    pub key: CommandKey,
    pub input: String,
    pub description: String,
    pub scope: Option<TableScope>,
    names: Vec<String>,
    qualified_input: String,
}

impl Entry {
    fn new<'a>(
        command: &'a CommandDescriptor,
        input: &str,
        description: impl Into<String>,
        scope: Option<TableScope>,
        labels: [&str; 2],
        extra_aliases: impl IntoIterator<Item = &'a str>,
    ) -> Self {
        // An alias keeps this entry's arguments so scoped invocations remain distinct.
        let arguments = input.find(char::is_whitespace).map_or("", |i| &input[i..]);
        let names = labels
            .into_iter()
            .map(str::to_owned)
            .chain(
                command
                    .aliases
                    .iter()
                    .map(|alias| alias.as_str())
                    .chain(extra_aliases)
                    .map(|alias| format!("{alias}{arguments}")),
            )
            .filter(|name| name != input)
            .collect();
        Self {
            key: command.key,
            input: input.into(),
            description: description.into(),
            scope,
            names,
            qualified_input: format!("{}{arguments}", command.name.as_str()),
        }
    }
}

#[derive(Clone, Debug)]
pub(super) struct Candidate {
    pub entry: Entry,
    pub input_match: Option<Range<usize>>,
    pub alternative_match: Option<TextMatch>,
}

#[derive(Clone, Debug)]
pub(super) struct TextMatch {
    pub text: String,
    pub range: Range<usize>,
}

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum MatchKind {
    Exact,
    Prefix,
    Substring,
}

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum MatchSource {
    Input,
    Name,
    Identifier,
}

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
struct MatchRank {
    kind: MatchKind,
    source: MatchSource,
    start: usize,
}

fn match_text(text: &str, query: &str, source: MatchSource) -> Option<(MatchRank, Range<usize>)> {
    // ASCII folding preserves UTF-8 byte offsets for the displayed text, including Chinese.
    let start = text.to_ascii_lowercase().find(query)?;
    let kind = if text.len() == query.len() {
        MatchKind::Exact
    } else if start == 0 {
        MatchKind::Prefix
    } else {
        MatchKind::Substring
    };
    Some((
        MatchRank {
            kind,
            source,
            start,
        },
        start..start + query.len(),
    ))
}

fn match_entry(entry: &Entry, query: &str) -> Option<(MatchRank, Candidate)> {
    let input_match = match_text(&entry.input, query, MatchSource::Input);
    let alternatives = entry.names.iter().filter_map(|text| {
        match_text(text, query, MatchSource::Name).map(|(rank, range)| (rank, text, range))
    });
    // Internal identifiers remain executable, but namespace fragments are not search names.
    let identifier = (query.split_whitespace().next()
        == entry.qualified_input.split_whitespace().next())
    .then(|| match_text(&entry.qualified_input, query, MatchSource::Identifier))
    .flatten()
    .map(|(rank, range)| (rank, &entry.qualified_input, range));
    let (rank, text, range) = input_match
        .iter()
        .map(|(rank, range)| (*rank, &entry.input, range.clone()))
        .chain(alternatives)
        .chain(identifier)
        .min_by_key(|(rank, _, _)| *rank)?;
    Some((
        rank,
        Candidate {
            entry: entry.clone(),
            input_match: input_match.map(|(_, range)| range),
            alternative_match: (rank.source != MatchSource::Input).then(|| TextMatch {
                text: text.clone(),
                range,
            }),
        },
    ))
}

pub(super) struct CatalogContext {
    pub language: Language,
    pub format: Option<DocumentFormat>,
    pub read_only: bool,
    pub editing: bool,
    pub selection: bool,
}

pub(super) fn entries(commands: &CommandRegistry, context: CatalogContext) -> Vec<Entry> {
    let CatalogContext {
        language,
        format,
        read_only,
        editing,
        selection,
    } = context;
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
            "org-studio.workspace.file-overview",
            "file-overview",
            "文件总览",
            "Open Files overview",
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
    let standard = specs.into_iter().filter_map(|(id, input, zh, en, scope)| {
        let descriptor = commands.descriptor(commands.key(id)?)?;
        if (read_only && (scope.is_some() || matches!(input, "save" | "reload" | "undo" | "redo")))
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
        Some(Entry::new(
            descriptor,
            input,
            description,
            scope,
            if chinese { [zh, en] } else { [en, zh] },
            (input == "save").then_some("w"),
        ))
    });
    let org = crate::command::ORG_EDITOR_COMMANDS
        .iter()
        .filter(|spec| {
            format == Some(DocumentFormat::Org) && editing && (!read_only || !spec.edits_source())
        })
        .filter_map(|spec| {
            let descriptor = commands.descriptor(commands.key(spec.name)?)?;
            let input = spec.aliases[0];
            let description = language.text(spec.label).to_owned();
            Some(Entry::new(
                descriptor,
                input,
                description,
                None,
                [
                    language.text(spec.label),
                    if chinese {
                        Language::English
                    } else {
                        Language::Chinese
                    }
                    .text(spec.label),
                ],
                [],
            ))
        });
    standard.chain(org).collect()
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
            let descriptor = commands.descriptor(commands.key(spec.name)?)?;
            Some(Entry::new(
                descriptor,
                spec.alias,
                if language == Language::Chinese {
                    spec.zh
                } else {
                    spec.en
                },
                None,
                if language == Language::Chinese {
                    [spec.zh, spec.en]
                } else {
                    [spec.en, spec.zh]
                },
                [],
            ))
        })
        .collect()
}

pub(super) fn candidates(all: &[Entry], query: &str, history: &[String]) -> Vec<Candidate> {
    profiling::scope!("CommandPalette::filter");
    let query = normalize(query).to_ascii_lowercase();
    let history_position = |entry: &Entry| {
        history
            .iter()
            .position(|h| {
                h == &entry.input || (entry.input == "goto-line" && h.starts_with("goto-line "))
            })
            .unwrap_or(usize::MAX)
    };
    if query.is_empty() {
        let mut entries = all.iter().collect::<Vec<_>>();
        entries.sort_by_key(|entry| history_position(entry));
        return entries
            .into_iter()
            .map(|entry| Candidate {
                entry: entry.clone(),
                input_match: None,
                alternative_match: None,
            })
            .collect();
    }
    let command = query.split_whitespace().next().unwrap_or("");
    let canonical = all
        .iter()
        .find(|entry| entry.qualified_input.split_whitespace().next() == Some(command))
        .and_then(|entry| entry.input.split_whitespace().next())
        .or_else(|| {
            TABLE_COMMANDS
                .iter()
                .find(|spec| command == spec.name)
                .map(|spec| spec.alias)
        })
        .unwrap_or(command);
    // Preserve arguments when completing or selecting a parameterized command.
    if matches!(canonical, "goto-line" | "table-sort") {
        return all
            .iter()
            .filter(|entry| entry.input == canonical)
            .cloned()
            .map(|mut entry| {
                entry.input = format!("{canonical}{}", &query[command.len()..]);
                Candidate {
                    input_match: (command == canonical).then_some(0..entry.input.len()),
                    alternative_match: (command != canonical).then(|| TextMatch {
                        text: query.clone(),
                        range: 0..query.len(),
                    }),
                    entry,
                }
            })
            .collect();
    }
    // An unavailable current-table command must not fall back to formatting the whole document.
    if query == command
        && (canonical == "table-align" || TABLE_COMMANDS.iter().any(|spec| spec.alias == canonical))
        && !all.iter().any(|entry| entry.input == canonical)
    {
        return Vec::new();
    }
    let mut found = all
        .iter()
        .filter_map(|entry| match_entry(entry, &query))
        .collect::<Vec<_>>();
    found.sort_by_key(|(rank, candidate)| {
        (
            *rank,
            candidate.entry.input.len(),
            history_position(&candidate.entry),
        )
    });
    found.into_iter().map(|(_, candidate)| candidate).collect()
}

/// Flags are syntax once a command is named. Invalid flags cannot fall back to another command.
pub(super) fn validate(query: &str) -> Result<(), &'static str> {
    let query = normalize(query).to_ascii_lowercase();
    let mut words = query.split_whitespace();
    let Some(command) = words.next() else {
        return Ok(());
    };
    if matches!(command, "table-align" | "org-studio.table.align") {
        let args = words.collect::<Vec<_>>();
        if !matches!(args.as_slice(), [] | ["--all"] | ["--selection"]) {
            return Err("table-align [--all | --selection]");
        }
    } else if matches!(command, "table-sort" | "org-table-sort-lines") {
        sort_edit(&query)?;
    } else if (TABLE_COMMANDS
        .iter()
        .any(|spec| command == spec.name || command == spec.alias)
        || crate::command::ORG_EDITOR_COMMANDS
            .iter()
            .any(|spec| command == spec.name || spec.aliases.contains(&command))
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
    for flag in normalize(input)
        .to_ascii_lowercase()
        .split_whitespace()
        .skip(1)
    {
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
