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
    if matches!(normalized.as_str(), "typst" | "typ") {
        let _highlight = tracing::info_span!("syntax_highlight").entered();
        return Ok(highlight_typst(source));
    }
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

/// Typst's native syntax tags avoid compiling its Tree-sitter query on the
/// rendering thread (~170 ms for the upstream query). The parser is already a
/// dependency of our Typst renderer and requires no startup prewarming.
fn highlight_typst(source: &str) -> Vec<CodeHighlightSpan> {
    use typst::syntax::{LinkedNode, SyntaxKind, Tag, highlight, parse};

    fn visit(
        node: &LinkedNode<'_>,
        inherited: Option<CodeHighlightKind>,
        spans: &mut Vec<CodeHighlightSpan>,
    ) {
        let kind = match node.kind() {
            SyntaxKind::Hash => Some(CodeHighlightKind::Punctuation),
            SyntaxKind::Bool => Some(CodeHighlightKind::Boolean),
            _ => highlight(node).and_then(|tag| {
                Some(match tag {
                    Tag::Comment => CodeHighlightKind::Comment,
                    Tag::Punctuation
                    | Tag::MathDelimiter
                    | Tag::MathGroupingParens
                    | Tag::ListMarker => CodeHighlightKind::Punctuation,
                    Tag::Escape | Tag::String | Tag::Raw | Tag::Link => CodeHighlightKind::String,
                    Tag::Strong
                    | Tag::Emph
                    | Tag::Heading
                    | Tag::ListTerm
                    | Tag::Label
                    | Tag::Ref => CodeHighlightKind::Attribute,
                    Tag::MathOperator | Tag::Operator => CodeHighlightKind::Operator,
                    Tag::Keyword => CodeHighlightKind::Keyword,
                    Tag::Number => CodeHighlightKind::Number,
                    Tag::Function => CodeHighlightKind::Function,
                    Tag::Interpolated => CodeHighlightKind::Variable,
                    Tag::Error => return None,
                })
            }),
        }
        .or(inherited);
        // Inherit markup colors, but emit only leaves: consumers require sorted,
        // non-overlapping UTF-8 byte ranges, even for nested markup and code.
        if !node.leaf_text().is_empty() {
            if let Some(kind) = kind {
                let range = node.range();
                spans.push(CodeHighlightSpan {
                    start: range.start,
                    end: range.end,
                    kind,
                });
            }
        } else {
            for child in node.children() {
                visit(&child, kind, spans);
            }
        }
    }

    let root = parse(source);
    let mut spans = Vec::new();
    visit(&LinkedNode::new(&root), None, &mut spans);
    spans
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn typst_highlights_code_math_and_nested_unicode_markup() {
        let source = "= 标题 *粗体😀*\n#let size = 12pt\n#text(\"你好😀\") // 注释\n$ x^2 $";
        for language in ["typst", "typ", " Typst "] {
            let spans = highlight_code(language, source).unwrap();
            for (text, expected) in [
                ("标题", CodeHighlightKind::Attribute),
                ("粗体😀", CodeHighlightKind::Attribute),
                ("let", CodeHighlightKind::Keyword),
                ("12pt", CodeHighlightKind::Number),
                ("text", CodeHighlightKind::Function),
                ("\"你好😀\"", CodeHighlightKind::String),
                ("// 注释", CodeHighlightKind::Comment),
                ("^", CodeHighlightKind::Operator),
            ] {
                let start = source.find(text).unwrap();
                assert!(
                    spans.iter().any(|span| {
                        span.start <= start
                            && span.end >= start + text.len()
                            && std::mem::discriminant(&span.kind)
                                == std::mem::discriminant(&expected)
                    }),
                    "missing {expected:?} for {text} in {language}"
                );
            }
            assert!(spans.iter().all(|span| span.start < span.end
                && source.is_char_boundary(span.start)
                && source.is_char_boundary(span.end)));
            assert!(spans.windows(2).all(|pair| pair[0].end <= pair[1].start));
        }
    }

    #[test]
    fn typst_accepts_empty_and_unfinished_source() {
        assert!(highlight_code("typst", "").unwrap().is_empty());
        let source = "#text(\"未完成😀";
        let spans = highlight_code("typst", source).unwrap();
        assert!(!spans.is_empty());
        assert!(spans.iter().all(|span| span.end <= source.len()
            && source.is_char_boundary(span.start)
            && source.is_char_boundary(span.end)));
    }

    #[test]
    fn typst_nested_code_overrides_markup_without_comment_colored_hashes() {
        let source = "= 标题 #text(\"中文😀\")\n#let enabled = true";
        let spans = highlight_code("typst", source).unwrap();
        for (token, kind) in [
            ("标题", CodeHighlightKind::Attribute),
            ("#", CodeHighlightKind::Punctuation),
            ("text", CodeHighlightKind::Function),
            ("\"中文😀\"", CodeHighlightKind::String),
            ("true", CodeHighlightKind::Boolean),
        ] {
            let offset = source.find(token).unwrap();
            let span = spans
                .iter()
                .find(|span| span.start <= offset && offset < span.end)
                .unwrap();
            assert_eq!(
                std::mem::discriminant(&span.kind),
                std::mem::discriminant(&kind),
                "{token}"
            );
        }
        assert!(spans.windows(2).all(|pair| pair[0].end <= pair[1].start));
    }

    /// Run alone with --release --ignored --nocapture to measure a cold call.
    #[test]
    #[ignore = "manual cold-start timing; no machine-dependent pass threshold"]
    fn typst_fixture_highlight_timing() {
        let fixture = include_str!("../tests/fixtures/preview-basics.org");
        let source = fixture
            .split("#+begin_src typst")
            .nth(1)
            .unwrap()
            .split_once('\n')
            .unwrap()
            .1
            .split("#+end_src")
            .next()
            .unwrap();
        for round in 0..3 {
            let start = std::time::Instant::now();
            let spans = highlight_code("typst", source).unwrap();
            let elapsed = start.elapsed();
            assert!(!spans.is_empty());
            eprintln!("typst fixture round={round} elapsed={elapsed:?}");
        }
    }
}
