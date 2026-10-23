use std::path::{Path, PathBuf};

use crate::{
    document::{
        ByteOffset, ByteRange, DocumentId, DocumentSnapshot, Revision, Selection, TextEdit,
        TextSnapshot,
    },
    org_syntax::BlockKind,
};

#[cfg(test)]
use super::artifact::NEXT_TEMP_OUTPUT;
use super::artifact::{PendingArtifact, resolve_output_target, write_temporary_output};
use super::process::execute_external;
use super::syntax::preserved_result_attributes;

pub(crate) struct BabelExecutionRequest {
    pub(crate) document_id: DocumentId,
    pub(crate) revision: Revision,
    pub(crate) selection: Selection,
    pub(crate) source_block_start: ByteOffset,
    pub(super) source: String,
    pub(super) headers: super::headers::Headers,
    pub(super) variables: Vec<(String, super::value::Value)>,
    pub(crate) cancel: super::CancelToken,
    name: Option<String>,
    pub(super) result_names: Option<super::value::Data>,
    pub(super) executor: BabelExecutor,
    pub(super) document_root: PathBuf,
    result: RequestedResult,
    pub(crate) mysql: Option<super::mysql::MySqlRequest>,
}

enum RequestedResult {
    Data {
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
}

pub(crate) struct PreparedBabelOutput {
    pub(crate) document_id: DocumentId,
    pub(crate) revision: Revision,
    pub(crate) selection: Selection,
    pub(crate) source_block_start: ByteOffset,
    pub(crate) language_name: &'static str,
    pub(crate) result_edit: Option<TextEdit>,
    pub(crate) table: Option<super::mysql::QueryTable>,
    pub(super) data: super::value::Data,
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
    pub(super) fn language_name(self) -> &'static str {
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
    pub(crate) fn block_name(&self) -> Option<&str> {
        self.name.as_deref()
    }

    pub(crate) fn source_preview(&self) -> &str {
        &self.source
    }

    pub(crate) fn writes_result(&self) -> bool {
        self.headers.writes_result()
    }

    pub(crate) fn headers_eval_query(&self) -> bool {
        self.headers
            .get(":eval")
            .is_some_and(|mode| mode.eq_ignore_ascii_case("query"))
    }
    pub(crate) fn execution_directory(&self) -> &Path {
        &self.document_root
    }

    pub(super) fn file_result(&self) -> bool {
        matches!(self.result, RequestedResult::File { .. })
    }

    pub(super) fn set_result_site(
        &mut self,
        snapshot: &DocumentSnapshot,
        blocks: &crate::org_syntax::BlockArena,
        index: usize,
        selection: Selection,
    ) -> Result<(), String> {
        let (range, attributes, name) =
            super::syntax::result_range(snapshot, blocks, index, self.file_result())?;
        let existing = snapshot.copy_range(range);
        let newline = if snapshot
            .copy_range(blocks.nodes()[index].source)
            .contains("\r\n")
        {
            "\r\n"
        } else {
            "\n"
        };
        let silent = !self.writes_result();
        let marker = name.as_ref().map(|n| format!(" {n}")).unwrap_or_default();
        self.result = match &self.result {
            RequestedResult::File { target, link, .. } => RequestedResult::File {
                target: target.clone(),
                link: link.clone(),
                edit: (!silent)
                    .then(|| {
                        let replacement = format!(
                            "{newline}#+RESULTS:{marker}{newline}{attributes}{link}{newline}"
                        );
                        (existing != replacement).then(|| TextEdit::new(range, replacement))
                    })
                    .flatten(),
            },
            RequestedResult::Data { .. } => RequestedResult::Data {
                range,
                existing,
                newline,
                silent,
                attributes,
            },
        };
        self.name = name.clone();
        if let Some(mysql) = &mut self.mysql {
            mysql.source_name = name;
        }
        self.document_id = snapshot.document_id();
        self.revision = snapshot.revision();
        self.selection = selection;
        self.source_block_start = blocks.nodes()[index].source.start;
        Ok(())
    }
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
            RequestedResult::Data { .. } => None,
        }
    }

    #[cfg(test)]
    fn file_edit(&self) -> Option<&TextEdit> {
        match &self.result {
            RequestedResult::File { edit, .. } => edit.as_ref(),
            RequestedResult::Data { .. } => None,
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

#[cfg(test)]
pub(crate) fn prepare_source_block_execution(
    snapshot: &DocumentSnapshot,
    document_path: &Path,
    selection: Selection,
) -> Result<BabelExecutionRequest, String> {
    let blocks = crate::org_syntax::parse(snapshot);
    let caret = selection.head();
    let (block_index, _) = blocks
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
    prepare_at(
        snapshot,
        document_path,
        selection,
        &blocks,
        block_index,
        None,
    )
}

pub(super) fn prepare_at(
    snapshot: &DocumentSnapshot,
    document_path: &Path,
    selection: Selection,
    blocks: &crate::org_syntax::BlockArena,
    block_index: usize,
    override_headers: Option<super::headers::Headers>,
) -> Result<BabelExecutionRequest, String> {
    let block = &blocks.nodes()[block_index];
    let headers = override_headers
        .map(Ok)
        .unwrap_or_else(|| super::headers::effective_headers(snapshot, blocks, block_index))?;
    validate_headers(&headers)?;
    let BlockKind::SourceBlock { language } = &block.kind else {
        unreachable!("source block was selected above")
    };
    let language = language.as_deref().unwrap_or("");
    if language.eq_ignore_ascii_case("sql") {
        return prepare_mysql(
            snapshot,
            document_path,
            selection,
            blocks,
            block_index,
            headers,
        );
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
    let document_root = execution_directory(&headers, document_path)?;
    let results = headers.values(":results");
    let silent = !headers.writes_result();
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
    let (result_range, attributes, name) =
        super::syntax::result_range(snapshot, blocks, block_index, true)?;
    let existing_result = snapshot.copy_range(result_range);
    let file = headers.get(":file").filter(|file| !file.is_empty());
    let (executor, result) = if let Some(executor) = external {
        if file.is_some() {
            return Err(format!(
                "{executor_name} does not support :file results yet"
            ));
        }
        if !matches!(executor, BabelExecutor::Python) && results.iter().any(|v| v == "value") {
            return Err(format!("{executor_name} supports :results output only"));
        }
        if results.iter().any(|v| v == "output")
            && results
                .iter()
                .any(|v| matches!(v.as_str(), "table" | "vector"))
        {
            return Err(
                "Structured table results require :results value; output is fixed-width text"
                    .into(),
            );
        }
        for value in results {
            if ![
                "value", "output", "table", "vector", "scalar", "replace", "silent", "none",
                "verbatim",
            ]
            .contains(&value.as_str())
            {
                return Err(format!("{executor_name} does not support :results {value}"));
            }
        }
        (
            executor,
            RequestedResult::Data {
                range: result_range,
                existing: existing_result,
                newline,
                silent,
                attributes,
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
        headers: headers.clone(),
        variables: literal_variables(&headers),
        result_names: None,
        cancel: super::CancelToken::default(),
        name,
        document_id: snapshot.document_id(),
        revision: snapshot.revision(),
        selection,
        source_block_start: block.source.start,
        source: if headers.values(":noweb").iter().any(|value| {
            ["yes", "eval"]
                .iter()
                .any(|mode| value.eq_ignore_ascii_case(mode))
        }) {
            super::source::expanded_execution_source(snapshot, blocks, block_index)?
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
    if request.cancel.cancelled() {
        return Err("Babel execution cancelled".into());
    }
    if let Some(mysql) = &request.mysql {
        let table =
            super::mysql::execute_with_variables(mysql, &request.source, &request.variables)?;
        let RequestedResult::Data {
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
            data: {
                let mut data = table.data();
                if request.headers.get(":colnames") == Some("no") {
                    data.columns = None;
                }
                data
            },
            table: Some(table),
            kind: PreparedResult::Text,
        });
    }
    if matches!(
        request.executor,
        BabelExecutor::Python | BabelExecutor::Shell | BabelExecutor::Bash
    ) {
        let mut data = execute_external(
            request.executor,
            &request.source,
            &request.document_root,
            &request.variables,
            matches!(request.executor, BabelExecutor::Python)
                && !request
                    .headers
                    .values(":results")
                    .iter()
                    .any(|v| v == "output"),
            &request.cancel,
        )?;
        if request.headers.get(":hlines") == Some("yes")
            && let super::value::Value::List(rows) = &mut data.value
        {
            for row in rows {
                if *row == super::value::Value::Text("hline".into()) {
                    *row = super::value::Value::Hline;
                }
            }
        }
        if let Some(names) = &request.result_names {
            if let super::value::Value::List(rows) = &data.value {
                let width = rows.first().and_then(|r| {
                    if let super::value::Value::List(cells) = r {
                        Some(cells.len())
                    } else {
                        None
                    }
                });
                if names.columns.as_ref().is_some_and(|c| {
                    rows.is_empty()
                        || Some(c.len())
                            == width.map(|w| w + usize::from(!names.row_names.is_empty()))
                }) {
                    data.columns = names.columns.clone();
                }
                if names.row_names.len() == rows.len() {
                    data.row_names = names.row_names.clone();
                }
            }
        } else if matches!(request.executor, BabelExecutor::Python) {
            data = data.preprocess(&request.headers)?;
        }
        let RequestedResult::Data {
            range,
            existing,
            newline,
            silent,
            attributes,
        } = request.result
        else {
            unreachable!("external executors produce text")
        };
        let result_edit = (!silent)
            .then(|| {
                let marker = request
                    .name
                    .as_ref()
                    .map(|n| format!(" {n}"))
                    .unwrap_or_default();
                let payload = super::result::payload(&data, &request.headers, newline)?;
                let replacement =
                    format!("{newline}#+RESULTS:{marker}{newline}{attributes}{payload}");
                Ok::<_, String>(
                    (existing != replacement).then(|| TextEdit::new(range, replacement)),
                )
            })
            .transpose()?
            .flatten();
        return Ok(PreparedBabelOutput {
            document_id: request.document_id,
            revision: request.revision,
            selection: request.selection,
            source_block_start: request.source_block_start,
            language_name: request.executor.language_name(),
            result_edit,
            table: None,
            data,
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
        data: super::value::Data::default(),
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
    headers: super::headers::Headers,
) -> Result<BabelExecutionRequest, String> {
    let engine = headers.get(":engine").unwrap_or("mysql");
    if !engine.eq_ignore_ascii_case("mysql") {
        return Err(format!(
            "SQL engine {engine} is not supported yet; choose :engine mysql"
        ));
    }
    if headers.get(":file").is_some() {
        return Err("MySQL does not support :file results yet".into());
    }
    for value in headers.values(":results") {
        if ![
            "table", "vector", "value", "output", "replace", "silent", "none",
        ]
        .contains(&value.to_ascii_lowercase().as_str())
        {
            return Err(format!("MySQL does not support :results {value}"));
        }
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
    let parameters = headers
        .variables
        .iter()
        .map(|(n, e)| {
            (
                n.clone(),
                if let super::expression::Expression::Literal(v) = e {
                    v.clone()
                } else {
                    super::value::Value::Nil
                },
            )
        })
        .collect::<Vec<_>>();
    let (checked, _) = super::mysql::parameters::preflight(&source, &parameters)?;
    super::mysql::validate_query(&checked)?;
    let opening = snapshot.copy_range(ByteRange::new(block.source.start.0, block.content.start.0));
    let newline = if opening.contains("\r\n") {
        "\r\n"
    } else {
        "\n"
    };
    let (range, attributes, name) = super::syntax::result_range(snapshot, blocks, index, false)?;
    let silent = !headers.writes_result();
    let mut mysql = super::mysql::MySqlRequest::from_headers(headers.clone())?;
    mysql.source_name = name.clone();
    Ok(BabelExecutionRequest {
        headers: headers.clone(),
        variables: literal_variables(&headers),
        result_names: None,
        cancel: mysql.cancel.clone(),
        name,
        document_id: snapshot.document_id(),
        revision: snapshot.revision(),
        selection,
        source_block_start: block.source.start,
        source,
        executor: BabelExecutor::MySql,
        document_root: execution_directory(&headers, document_path)?,
        result: RequestedResult::Data {
            range,
            existing: snapshot.copy_range(range),
            newline,
            silent,
            attributes,
        },
        mysql: Some(mysql),
    })
}

pub(super) fn validate_headers(headers: &super::headers::Headers) -> Result<(), String> {
    if headers
        .get(":eval")
        .is_some_and(|v| v.eq_ignore_ascii_case("no") || v.eq_ignore_ascii_case("never"))
    {
        return Err("This source block disables execution with :eval no/never".into());
    }
    if headers.get(":cache").is_some_and(|v| v != "no")
        || headers.get(":session").is_some_and(|v| v != "none")
    {
        return Err("Babel :cache and persistent :session are not supported yet".into());
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
            ":file",
            ":results",
            ":colnames",
            ":rownames",
            ":hlines",
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
            return Err(format!("Babel does not support {key} yet"));
        }
    }
    for (key, allowed) in [
        (":colnames", &["nil", "yes", "no"][..]),
        (":rownames", &["yes", "no"]),
        (":hlines", &["yes", "no"]),
    ] {
        if headers.get(key).is_some_and(|v| !allowed.contains(&v)) {
            return Err(format!("Invalid {key}; choose {}", allowed.join("/")));
        }
    }
    Ok(())
}

fn execution_directory(headers: &super::headers::Headers, path: &Path) -> Result<PathBuf, String> {
    let base = path.parent().unwrap_or(Path::new("."));
    if let Some(dir) = headers.get(":dir") {
        let directory = base.join(dir);
        let directory = std::fs::canonicalize(directory)
            .map_err(|_| "Babel :dir must name an existing local directory")?;
        if !directory.is_dir() {
            return Err("Babel :dir is not a directory".into());
        }
        Ok(directory)
    } else {
        Ok(base.to_owned())
    }
}

fn literal_variables(headers: &super::headers::Headers) -> Vec<(String, super::value::Value)> {
    headers
        .variables
        .iter()
        .filter_map(|(name, expression)| {
            if let super::expression::Expression::Literal(value) = expression {
                Some((name.clone(), value.clone()))
            } else {
                None
            }
        })
        .collect()
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
