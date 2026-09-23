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
}

enum RequestedResult {
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
}

impl BabelExecutor {
    fn language_name(self) -> &'static str {
        match self {
            Self::Diagram { language, .. } => language.display_name(),
            Self::Typst(_) => "Typst",
            Self::Python => "Python",
            Self::Shell => "Shell",
            Self::Bash => "Bash",
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
            RequestedResult::Text { .. } => None,
        }
    }

    #[cfg(test)]
    fn file_edit(&self) -> Option<&TextEdit> {
        match &self.result {
            RequestedResult::File { edit, .. } => edit.as_ref(),
            RequestedResult::Text { .. } => None,
        }
    }
}

impl PreparedBabelOutput {
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
    })
}

pub(crate) fn execute_source_block(
    request: BabelExecutionRequest,
) -> Result<PreparedBabelOutput, String> {
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
        BabelExecutor::Python | BabelExecutor::Shell | BabelExecutor::Bash => {
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
        kind: PreparedResult::File {
            artifact,
            link,
            warnings,
        },
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
