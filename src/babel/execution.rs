use std::path::{Path, PathBuf};

use crate::{
    document::{
        ByteOffset, ByteRange, DocumentId, DocumentSnapshot, Revision, Selection, TextEdit,
        TextSnapshot,
    },
    org_syntax::{BlockKind, parse},
};

#[cfg(test)]
use super::artifact::NEXT_TEMP_OUTPUT;
use super::artifact::{PendingArtifact, resolve_output_target, write_temporary_output};
use super::process::execute_external;
use super::syntax::{
    existing_result_range, header_argument, header_values, preserved_result_attributes,
    tokenize_header,
};

pub(crate) struct BabelExecutionRequest {
    pub(crate) document_id: DocumentId,
    pub(crate) revision: Revision,
    pub(crate) selection: Selection,
    pub(crate) source_block_start: ByteOffset,
    pub(super) source: String,
    executor: BabelExecutor,
    document_root: PathBuf,
    result: RequestedResult,
    pub(crate) mysql: Option<super::mysql::MySqlRequest>,
}

enum RequestedResult {
    Table {
        range: ByteRange,
        existing: String,
        newline: &'static str,
        silent: bool,
        attributes: String,
    },
    File {
        target: PathBuf,
        link: String,
        edit: Option<TextEdit>,
    },
    Text {
        range: ByteRange,
        existing: String,
        newline: &'static str,
        silent: bool,
    },
}

pub(crate) struct PreparedBabelOutput {
    pub(crate) document_id: DocumentId,
    pub(crate) revision: Revision,
    pub(crate) selection: Selection,
    pub(crate) source_block_start: ByteOffset,
    pub(crate) language_name: &'static str,
    pub(crate) result_edit: Option<TextEdit>,
    pub(crate) table: Option<super::mysql::QueryTable>,
    kind: PreparedResult,
}

enum PreparedResult {
    File {
        artifact: PendingArtifact,
        link: String,
        warnings: Vec<String>,
    },
    Text,
}

#[derive(Clone, Copy)]
pub(super) enum BabelExecutor {
    Diagram {
        language: crate::preview::DiagramLanguage,
        format: typstuml::render::Format,
    },
    Typst(crate::typst_runtime::OutputFormat),
    Python,
    Shell,
    Bash,
    MySql,
}

impl BabelExecutor {
    fn language_name(self) -> &'static str {
        match self {
            Self::Diagram { language, .. } => language.display_name(),
            Self::Typst(_) => "Typst",
            Self::Python => "Python",
            Self::Shell => "Shell",
            Self::Bash => "Bash",
            Self::MySql => "MySQL",
        }
    }
}

impl BabelExecutionRequest {
    pub(crate) fn language_name(&self) -> &'static str {
        self.executor.language_name()
    }

    pub(crate) fn external_program(&self) -> Option<&'static str> {
        match self.executor {
            BabelExecutor::Python => Some("python3"),
            BabelExecutor::Shell => Some("/bin/sh"),
            BabelExecutor::Bash => Some("/bin/bash"),
            _ => None,
        }
    }

    #[cfg(test)]
    fn file_target(&self) -> Option<&Path> {
        match &self.result {
            RequestedResult::File { target, .. } => Some(target),
            RequestedResult::Text { .. } | RequestedResult::Table { .. } => None,
        }
    }

    #[cfg(test)]
    fn file_edit(&self) -> Option<&TextEdit> {
        match &self.result {
            RequestedResult::File { edit, .. } => edit.as_ref(),
            RequestedResult::Text { .. } | RequestedResult::Table { .. } => None,
        }
    }
}

impl PreparedBabelOutput {
    pub(crate) fn offset_after_result_edit(&self, offset: ByteOffset) -> ByteOffset {
        let Some(edit) = &self.result_edit else {
            return offset;
        };
        if offset < edit.range.start {
            offset
        } else if offset >= edit.range.end {
            ByteOffset(
                offset.0 - (edit.range.end.0 - edit.range.start.0) + edit.replacement.len() as u64,
            )
        } else {
            ByteOffset(edit.range.start.0 + edit.replacement.len() as u64)
        }
    }

    pub(crate) fn publish(mut self) -> Result<Self, String> {
        if let PreparedResult::File { artifact, .. } = &mut self.kind {
            artifact.publish()?;
        }
        Ok(self)
    }

    pub(crate) fn file(&self) -> Option<(&Path, &str, Option<&str>)> {
        match &self.kind {
            PreparedResult::File {
                artifact,
                link,
                warnings,
            } => Some((&artifact.target, link, warnings.first().map(String::as_str))),
            PreparedResult::Text => None,
        }
    }
}

pub(crate) fn prepare_source_block_execution(
    snapshot: &DocumentSnapshot,
    document_path: &Path,
    selection: Selection,
) -> Result<BabelExecutionRequest, String> {
    let blocks = parse(snapshot);
    let caret = selection.head();
    let (block_index, block) = blocks
        .nodes()
        .iter()
        .enumerate()
        .find(|(_, block)| {
            matches!(block.kind, BlockKind::SourceBlock { .. })
                && caret >= block.source.start
                && (caret < block.source.end
                    || caret == block.source.end && block.source.end.0 == snapshot.len_bytes())
        })
        .ok_or_else(|| "Point is not inside an Org source block".to_owned())?;
    let BlockKind::SourceBlock { language } = &block.kind else {
        unreachable!("source block was selected above")
    };
    let language = language.as_deref().unwrap_or("");
    if language.eq_ignore_ascii_case("sql") {
        return prepare_mysql(snapshot, document_path, selection, &blocks, block_index);
    }
    let diagram_language = crate::preview::DiagramLanguage::from_source_language(Some(language));
    let typst_source =
        language.eq_ignore_ascii_case("typst") || language.eq_ignore_ascii_case("typ");
    let external =
        if language.eq_ignore_ascii_case("python") || language.eq_ignore_ascii_case("python3") {
            Some(BabelExecutor::Python)
        } else if ["sh", "shell"]
            .iter()
            .any(|name| language.eq_ignore_ascii_case(name))
        {
            Some(BabelExecutor::Shell)
        } else if language.eq_ignore_ascii_case("bash") {
            Some(BabelExecutor::Bash)
        } else {
            None
        };
    let executor_name = match (diagram_language, typst_source, external) {
        (Some(diagram), _, _) => diagram.display_name(),
        (None, true, _) => "Typst",
        (None, false, Some(executor)) => executor.language_name(),
        (None, false, None) => {
            return Err(format!(
                "No Babel executor is registered for {}",
                if language.is_empty() {
                    "this block"
                } else {
                    language
                }
            ));
        }
    };

    let opening = snapshot.copy_range(ByteRange {
        start: block.source.start,
        end: block.content.start,
    });
    let tokens = tokenize_header(&opening)?;
    if header_values(&tokens, ":eval").iter().any(|value| {
        ["no", "never"]
            .iter()
            .any(|prohibited| value.eq_ignore_ascii_case(prohibited))
    }) {
        return Err("This source block disables execution with :eval no/never".to_owned());
    }
    let document_root = document_path
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .to_path_buf();
    let results = header_values(&tokens, ":results");
    let silent = results
        .iter()
        .any(|value| value.eq_ignore_ascii_case("silent"));
    if results
        .iter()
        .any(|value| value.eq_ignore_ascii_case("append") || value.eq_ignore_ascii_case("prepend"))
    {
        return Err(format!(
            "{executor_name} currently supports :results replace or silent, not append/prepend"
        ));
    }
    let newline = if opening.contains("\r\n") {
        "\r\n"
    } else {
        "\n"
    };
    let result_range = existing_result_range(snapshot, &blocks, block_index);
    let existing_result = snapshot.copy_range(result_range);
    let file = header_argument(&tokens, ":file").filter(|file| !file.is_empty());
    let (executor, result) = if let Some(executor) = external {
        if file.is_some() {
            return Err(format!(
                "{executor_name} does not support :file results yet"
            ));
        }
        if !results.iter().any(|value| {
            value.eq_ignore_ascii_case("output") || value.eq_ignore_ascii_case("silent")
        }) {
            return Err(format!(
                "{executor_name} requires :results output (or :results silent)"
            ));
        }
        if let Some(unsupported) = results.iter().find(|value| {
            !["output", "replace", "silent"]
                .iter()
                .any(|supported| value.eq_ignore_ascii_case(supported))
        }) {
            return Err(format!(
                "{executor_name} does not support :results {unsupported}"
            ));
        }
        (
            executor,
            RequestedResult::Text {
                range: result_range,
                existing: existing_result,
                newline,
                silent,
            },
        )
    } else {
        let file = file
            .ok_or_else(|| format!("{executor_name} execution requires a :file header argument"))?;
        if file.contains(['\n', '\r', ']']) {
            return Err(
                "The :file value contains characters that cannot form an Org link".to_owned(),
            );
        }
        let target = resolve_output_target(&document_root, file)?;
        if target == document_path {
            return Err("The Babel result must not overwrite the Org document".to_owned());
        }
        let executor = if typst_source {
            BabelExecutor::Typst(
                crate::typst_runtime::OutputFormat::infer_from_path(&target)
                    .ok_or_else(|| "Typst :file must end in .svg, .png, or .pdf".to_owned())?,
            )
        } else {
            let language = diagram_language.expect("diagram language checked above");
            BabelExecutor::Diagram {
                language,
                format: typstuml::render::Format::infer_from_path(&target).ok_or_else(|| {
                    format!(
                        "{} :file must end in .svg, .png, or .pdf",
                        language.display_name()
                    )
                })?,
            }
        };
        let link = format!("[[file:{file}]]");
        let edit = if silent {
            None
        } else {
            // Keep user-supplied image attributes when replacing a file result.
            let replacement = match preserved_result_attributes(&existing_result) {
                Some(attributes) => {
                    format!("{newline}#+RESULTS:{newline}{attributes}{newline}{link}{newline}")
                }
                None => format!("{newline}#+RESULTS:{newline}{link}{newline}"),
            };
            (existing_result != replacement).then(|| TextEdit::new(result_range, replacement))
        };
        (executor, RequestedResult::File { target, link, edit })
    };

    Ok(BabelExecutionRequest {
        document_id: snapshot.document_id(),
        revision: snapshot.revision(),
        selection,
        source_block_start: block.source.start,
        source: if header_values(&tokens, ":noweb").iter().any(|value| {
            ["yes", "eval"]
                .iter()
                .any(|mode| value.eq_ignore_ascii_case(mode))
        }) {
            super::source::expanded_execution_source(snapshot, &blocks, block_index)?
        } else {
            snapshot.copy_range(block.content)
        },
        executor,
        document_root,
        result,
        mysql: None,
    })
}

pub(crate) fn execute_source_block(
    request: BabelExecutionRequest,
) -> Result<PreparedBabelOutput, String> {
    if let Some(mysql) = &request.mysql {
        let table = super::mysql::execute(mysql, &request.source)?;
        let RequestedResult::Table {
            range,
            existing,
            newline,
            silent,
            attributes,
        } = &request.result
        else {
            unreachable!("MySQL produces a table");
        };
        let payload = table.serialize(
            !mysql
                .headers
                .get(":colnames")
                .is_some_and(|v| v.eq_ignore_ascii_case("no")),
            newline,
            mysql.byte_limit(),
        )?;
        let marker = mysql
            .source_name
            .as_ref()
            .map(|name| format!(" {name}"))
            .unwrap_or_default();
        let replacement = format!("{newline}#+RESULTS:{marker}{newline}{attributes}{payload}");
        let result_edit =
            (!silent && *existing != replacement).then(|| TextEdit::new(*range, replacement));
        return Ok(PreparedBabelOutput {
            document_id: request.document_id,
            revision: request.revision,
            selection: request.selection,
            source_block_start: request.source_block_start,
            language_name: "MySQL",
            result_edit,
            table: Some(table),
            kind: PreparedResult::Text,
        });
    }
    if matches!(
        request.executor,
        BabelExecutor::Python | BabelExecutor::Shell | BabelExecutor::Bash
    ) {
        let stdout = execute_external(request.executor, &request.source, &request.document_root)?;
        let RequestedResult::Text {
            range,
            existing,
            newline,
            silent,
        } = request.result
        else {
            unreachable!("external executors produce text")
        };
        let result_edit = (!silent)
            .then(|| {
                let replacement = format_text_result(&stdout, newline);
                (existing != replacement).then(|| TextEdit::new(range, replacement))
            })
            .flatten();
        return Ok(PreparedBabelOutput {
            document_id: request.document_id,
            revision: request.revision,
            selection: request.selection,
            source_block_start: request.source_block_start,
            language_name: request.executor.language_name(),
            result_edit,
            table: None,
            kind: PreparedResult::Text,
        });
    }
    let (bytes, warnings) = match request.executor {
        BabelExecutor::Diagram { language, format } => {
            let rendered = crate::typst_runtime::run_with_cache_cleanup(|| {
                typstuml::render::render_source_with_language(
                    &request.source,
                    language.input_language(),
                    format,
                )
            })
            .map_err(|error| {
                error
                    .to_diagnostics()
                    .into_iter()
                    .map(|diagnostic| diagnostic.to_string())
                    .collect::<Vec<_>>()
                    .join("\n")
            })?;
            (
                rendered.bytes,
                rendered
                    .warnings
                    .into_iter()
                    .map(|warning| warning.to_string())
                    .collect(),
            )
        }
        BabelExecutor::Typst(format) => {
            let output = crate::typst_runtime::shared_engine()
                .compile(
                    request.source.clone(),
                    format,
                    false,
                    144.0,
                    Some(&request.document_root),
                )
                .map_err(|error| format_typst_error(error, &request.source))?;
            let mut pages = output.pages;
            if pages.len() != 1 {
                return Err(format!(
                    "Typst produced {} output artifacts; Babel requires exactly one",
                    pages.len()
                ));
            }
            let warnings = output
                .diagnostics
                .iter()
                .map(|diagnostic| format_typst_diagnostic(diagnostic, &request.source))
                .collect();
            (
                pages.pop().expect("one Typst artifact checked above"),
                warnings,
            )
        }
        BabelExecutor::Python
        | BabelExecutor::Shell
        | BabelExecutor::Bash
        | BabelExecutor::MySql => {
            unreachable!("external languages handled above")
        }
    };
    let RequestedResult::File { target, link, edit } = request.result else {
        unreachable!("renderers produce files")
    };
    let artifact = write_temporary_output(&target, &request.document_root, &bytes)?;
    Ok(PreparedBabelOutput {
        document_id: request.document_id,
        revision: request.revision,
        selection: request.selection,
        source_block_start: request.source_block_start,
        language_name: request.executor.language_name(),
        result_edit: edit,
        table: None,
        kind: PreparedResult::File {
            artifact,
            link,
            warnings,
        },
    })
}

fn prepare_mysql(
    snapshot: &DocumentSnapshot,
    document_path: &Path,
    selection: Selection,
    blocks: &crate::org_syntax::BlockArena,
    index: usize,
) -> Result<BabelExecutionRequest, String> {
    let headers = super::headers::effective_headers(snapshot, blocks, index)?;
    let engine = headers.get(":engine").unwrap_or("mysql");
    if !engine.eq_ignore_ascii_case("mysql") {
        return Err(format!(
            "SQL engine {engine} is not supported yet; choose :engine mysql"
        ));
    }
    if headers
        .get(":eval")
        .is_some_and(|v| ["no", "never"].iter().any(|p| v.eq_ignore_ascii_case(p)))
    {
        return Err("This source block disables execution with :eval no/never".into());
    }
    for key in headers.keys() {
        if ![
            ":engine",
            ":dbhost",
            ":dbport",
            ":database",
            ":dbuser",
            ":dbpassword",
            ":dbconnection",
            ":results",
            ":colnames",
            ":eval",
            ":exports",
            ":dir",
            ":noweb",
            ":tangle",
            ":comments",
            ":noweb-ref",
            ":cache",
            ":session",
        ]
        .contains(&key)
        {
            return Err(format!(
                "MySQL does not support the {key} header argument yet"
            ));
        }
    }
    if headers
        .get(":cache")
        .is_some_and(|v| !v.eq_ignore_ascii_case("no"))
        || headers
            .get(":session")
            .is_some_and(|v| !v.eq_ignore_ascii_case("none"))
    {
        return Err("MySQL :cache and persistent :session are not supported yet".into());
    }
    for value in headers.values(":results") {
        if !["table", "vector", "value", "output", "replace", "silent"]
            .contains(&value.to_ascii_lowercase().as_str())
        {
            return Err(format!("MySQL does not support :results {value}"));
        }
    }
    if headers
        .values(":results")
        .iter()
        .any(|v| v.eq_ignore_ascii_case("value"))
        && headers
            .values(":results")
            .iter()
            .any(|v| v.eq_ignore_ascii_case("output"))
    {
        return Err("Choose one result collection mode: value or output".into());
    }
    let block = &blocks.nodes()[index];
    let source = if headers
        .get(":noweb")
        .is_some_and(|v| v.eq_ignore_ascii_case("yes") || v.eq_ignore_ascii_case("eval"))
    {
        super::source::expanded_execution_source(snapshot, blocks, index)?
    } else {
        snapshot.copy_range(block.content)
    };
    super::mysql::validate_query(&source)?;
    let opening = snapshot.copy_range(ByteRange::new(block.source.start.0, block.content.start.0));
    let newline = if opening.contains("\r\n") {
        "\r\n"
    } else {
        "\n"
    };
    let (range, attributes, name) = super::syntax::mysql_result_range(snapshot, blocks, index)?;
    let silent = headers
        .values(":results")
        .iter()
        .any(|v| v.eq_ignore_ascii_case("silent"));
    let mut mysql = super::mysql::MySqlRequest::from_headers(headers)?;
    mysql.source_name = name;
    Ok(BabelExecutionRequest {
        document_id: snapshot.document_id(),
        revision: snapshot.revision(),
        selection,
        source_block_start: block.source.start,
        source,
        executor: BabelExecutor::MySql,
        document_root: document_path.parent().unwrap_or(Path::new(".")).to_owned(),
        result: RequestedResult::Table {
            range,
            existing: snapshot.copy_range(range),
            newline,
            silent,
            attributes,
        },
        mysql: Some(mysql),
    })
}

fn format_text_result(stdout: &str, newline: &str) -> String {
    let mut result = format!("{newline}#+RESULTS:{newline}");
    for line in stdout.lines() {
        if line.is_empty() {
            result.push(':');
        } else {
            result.push_str(": ");
            result.push_str(line);
        }
        result.push_str(newline);
    }
    result
}

fn format_typst_error(error: crate::typst_runtime::CompileError, source: &str) -> String {
    match error {
        crate::typst_runtime::CompileError::Compile(diagnostics) => diagnostics
            .iter()
            .map(|diagnostic| format_typst_diagnostic(diagnostic, source))
            .collect::<Vec<_>>()
            .join("\n"),
        crate::typst_runtime::CompileError::Render(message) => message,
    }
}

fn format_typst_diagnostic(
    diagnostic: &crate::typst_runtime::CompileDiagnostic,
    source: &str,
) -> String {
    let Some(range) = diagnostic.source.as_ref() else {
        return diagnostic.message.clone();
    };
    let offset = range.start.min(source.len());
    let prefix = source.get(..offset).unwrap_or(source);
    let line = prefix.bytes().filter(|byte| *byte == b'\n').count() + 1;
    let line_start = prefix.rfind('\n').map_or(0, |position| position + 1);
    let column = prefix[line_start..].chars().count() + 1;
    format!("line {line}, column {column}: {}", diagnostic.message)
}

#[cfg(test)]
#[path = "tests.rs"]
mod tests;
