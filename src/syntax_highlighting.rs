//! Surface-neutral source-code highlighting shared by Editor and Reading.

use std::{
    collections::HashMap,
    sync::{Arc, Mutex, OnceLock},
};
use tree_sitter_highlight::{HighlightConfiguration, HighlightEvent, Highlighter};

const HIGHLIGHT_NAMES: &[&str] = &[
    "attribute",
    "boolean",
    "comment",
    "conditional",
    "constant",
    "constructor",
    "delimiter",
    "embedded",
    "escape",
    "field",
    "function",
    "function.call",
    "keyword",
    "keyword.operator",
    "label",
    "number",
    "operator",
    "parameter",
    "property",
    "punctuation.bracket",
    "punctuation.delimiter",
    "string",
    "storageclass",
    "type",
    "type.builtin",
    "type.qualifier",
    "variable",
];

#[derive(Clone, Copy, Debug)]
pub(crate) struct CodeHighlightSpan {
    pub(crate) start: usize,
    pub(crate) end: usize,
    pub(crate) kind: CodeHighlightKind,
}

#[derive(Clone, Copy, Debug)]
pub(crate) enum CodeHighlightKind {
    Attribute,
    Boolean,
    Comment,
    Constant,
    Function,
    Keyword,
    Number,
    Operator,
    Property,
    Punctuation,
    String,
    Type,
    Variable,
}

pub(crate) fn highlight_code(
    language: &str,
    source: &str,
) -> Result<Vec<CodeHighlightSpan>, String> {
    let normalized = language.trim().to_ascii_lowercase();
    static CONFIGURATIONS: OnceLock<Mutex<HashMap<String, Arc<HighlightConfiguration>>>> =
        OnceLock::new();
    let lock_wait = tracing::info_span!("highlighter_configuration_lock_wait").entered();
    let configurations = CONFIGURATIONS
        .get_or_init(|| Mutex::new(HashMap::new()))
        .lock()
        .expect("highlight configuration cache poisoned");
    drop(lock_wait);
    if let Some(configuration) = configurations.get(&normalized).cloned() {
        drop(configurations);
        let _highlight = tracing::info_span!("syntax_highlight").entered();
        return highlight_with_configuration(&configuration, source);
    }
    drop(configurations);
    let combined_query: String;
    let (language, name, highlights, injections, locals) = match normalized.as_str() {
        "sql" | "postgres" | "postgresql" => (
            tree_sitter_sequel::LANGUAGE.into(),
            "sql",
            tree_sitter_sequel::HIGHLIGHTS_QUERY,
            "",
            "",
        ),
        "rust" | "rs" => (
            tree_sitter_rust::LANGUAGE.into(),
            "rust",
            tree_sitter_rust::HIGHLIGHTS_QUERY,
            tree_sitter_rust::INJECTIONS_QUERY,
            "",
        ),
        "python" | "py" => (
            tree_sitter_python::LANGUAGE.into(),
            "python",
            tree_sitter_python::HIGHLIGHTS_QUERY,
            "",
            "",
        ),
        "sh" | "shell" | "bash" | "zsh" => (
            tree_sitter_bash::LANGUAGE.into(),
            "bash",
            tree_sitter_bash::HIGHLIGHT_QUERY,
            "",
            "",
        ),
        "javascript" | "js" => (
            tree_sitter_javascript::LANGUAGE.into(),
            "javascript",
            tree_sitter_javascript::HIGHLIGHT_QUERY,
            tree_sitter_javascript::INJECTIONS_QUERY,
            tree_sitter_javascript::LOCALS_QUERY,
        ),
        "jsx" => {
            combined_query = format!(
                "{}\n{}",
                tree_sitter_javascript::HIGHLIGHT_QUERY,
                tree_sitter_javascript::JSX_HIGHLIGHT_QUERY
            );
            (
                tree_sitter_javascript::LANGUAGE.into(),
                "jsx",
                combined_query.as_str(),
                tree_sitter_javascript::INJECTIONS_QUERY,
                tree_sitter_javascript::LOCALS_QUERY,
            )
        }
        "typescript" | "ts" => {
            combined_query = format!(
                "{}\n{}",
                tree_sitter_javascript::HIGHLIGHT_QUERY,
                tree_sitter_typescript::HIGHLIGHTS_QUERY
            );
            (
                tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into(),
                "typescript",
                combined_query.as_str(),
                "",
                tree_sitter_typescript::LOCALS_QUERY,
            )
        }
        "tsx" => {
            combined_query = format!(
                "{}\n{}\n{}",
                tree_sitter_javascript::HIGHLIGHT_QUERY,
                tree_sitter_javascript::JSX_HIGHLIGHT_QUERY,
                tree_sitter_typescript::HIGHLIGHTS_QUERY
            );
            (
                tree_sitter_typescript::LANGUAGE_TSX.into(),
                "tsx",
                combined_query.as_str(),
                "",
                tree_sitter_typescript::LOCALS_QUERY,
            )
        }
        "json" => (
            tree_sitter_json::LANGUAGE.into(),
            "json",
            tree_sitter_json::HIGHLIGHTS_QUERY,
            "",
            "",
        ),
        "go" | "golang" => (
            tree_sitter_go::LANGUAGE.into(),
            "go",
            tree_sitter_go::HIGHLIGHTS_QUERY,
            "",
            "",
        ),
        "c" | "h" => (
            tree_sitter_c::LANGUAGE.into(),
            "c",
            tree_sitter_c::HIGHLIGHT_QUERY,
            "",
            "",
        ),
        "cpp" | "c++" | "cc" | "cxx" | "hpp" => {
            combined_query = format!(
                "{}\n{}",
                tree_sitter_c::HIGHLIGHT_QUERY,
                tree_sitter_cpp::HIGHLIGHT_QUERY
            );
            (
                tree_sitter_cpp::LANGUAGE.into(),
                "cpp",
                combined_query.as_str(),
                "",
                "",
            )
        }
        _ => return Ok(Vec::new()),
    };
    let mut configuration =
        HighlightConfiguration::new(language, name, highlights, injections, locals)
            .map_err(|error| error.to_string())?;
    configuration.configure(HIGHLIGHT_NAMES);

    let configuration = Arc::new(configuration);
    let lock_wait = tracing::info_span!("highlighter_configuration_lock_wait").entered();
    let mut configurations = CONFIGURATIONS
        .get_or_init(|| Mutex::new(HashMap::new()))
        .lock()
        .expect("highlight configuration cache poisoned");
    drop(lock_wait);
    let configuration = configurations
        .entry(normalized)
        .or_insert_with(|| configuration.clone())
        .clone();
    drop(configurations);
    let _highlight = tracing::info_span!("syntax_highlight").entered();
    highlight_with_configuration(&configuration, source)
}

fn highlight_with_configuration(
    configuration: &HighlightConfiguration,
    source: &str,
) -> Result<Vec<CodeHighlightSpan>, String> {
    let mut highlighter = Highlighter::new();
    let events = highlighter
        .highlight(configuration, source.as_bytes(), None, |_| None)
        .map_err(|error| error.to_string())?;
    let mut active = Vec::new();
    let mut spans = Vec::new();
    for event in events {
        match event.map_err(|error| error.to_string())? {
            HighlightEvent::HighlightStart(highlight) => active.push(highlight.0),
            HighlightEvent::HighlightEnd => {
                active.pop();
            }
            HighlightEvent::Source { start, end } => {
                if start < end
                    && let Some(index) = active.last()
                    && let Some(kind) = code_highlight_kind(HIGHLIGHT_NAMES[*index])
                {
                    spans.push(CodeHighlightSpan { start, end, kind });
                }
            }
        }
    }
    Ok(spans)
}

fn code_highlight_kind(name: &str) -> Option<CodeHighlightKind> {
    let root = name.split('.').next().unwrap_or(name);
    Some(match root {
        "attribute" => CodeHighlightKind::Attribute,
        "boolean" => CodeHighlightKind::Boolean,
        "comment" => CodeHighlightKind::Comment,
        "constant" => CodeHighlightKind::Constant,
        "constructor" => CodeHighlightKind::Type,
        "embedded" | "escape" => CodeHighlightKind::String,
        "delimiter" | "punctuation" => CodeHighlightKind::Punctuation,
        "field" | "property" => CodeHighlightKind::Property,
        "function" => CodeHighlightKind::Function,
        "keyword" | "conditional" | "storageclass" => CodeHighlightKind::Keyword,
        "number" => CodeHighlightKind::Number,
        "operator" => CodeHighlightKind::Operator,
        "label" => CodeHighlightKind::Attribute,
        "string" => CodeHighlightKind::String,
        "type" => CodeHighlightKind::Type,
        "parameter" | "variable" => CodeHighlightKind::Variable,
        _ => return None,
    })
}
