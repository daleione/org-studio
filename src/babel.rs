use std::{
    fs::{self, OpenOptions},
    io::Write,
    path::{Component, Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
};

use crate::{
    document::{
        ByteOffset, ByteRange, DocumentId, DocumentSnapshot, Revision, Selection, TextEdit,
        TextSnapshot,
    },
    org_syntax::{BlockArena, BlockKind, parse},
};

static NEXT_TEMP_OUTPUT: AtomicU64 = AtomicU64::new(1);

pub(crate) struct BabelExecutionRequest {
    pub(crate) document_id: DocumentId,
    pub(crate) revision: Revision,
    pub(crate) selection: Selection,
    pub(crate) source_block_start: ByteOffset,
    source: String,
    executor: BabelExecutor,
    document_root: PathBuf,
    target: PathBuf,
    result_edit: Option<TextEdit>,
    result_link: String,
}

pub(crate) struct PreparedBabelOutput {
    pub(crate) document_id: DocumentId,
    pub(crate) revision: Revision,
    pub(crate) selection: Selection,
    pub(crate) source_block_start: ByteOffset,
    pub(crate) language_name: &'static str,
    pub(crate) target: PathBuf,
    pub(crate) result_edit: Option<TextEdit>,
    pub(crate) result_link: String,
    pub(crate) warnings: Vec<String>,
    temporary: PathBuf,
    allowed_root: PathBuf,
}

#[derive(Clone, Copy)]
enum BabelExecutor {
    Diagram {
        language: crate::preview::DiagramLanguage,
        format: typstuml::render::Format,
    },
    Typst(crate::typst_runtime::OutputFormat),
}

impl BabelExecutor {
    fn language_name(self) -> &'static str {
        match self {
            Self::Diagram { language, .. } => language.display_name(),
            Self::Typst(_) => "Typst",
        }
    }
}

impl BabelExecutionRequest {
    pub(crate) fn language_name(&self) -> &'static str {
        self.executor.language_name()
    }
}

impl PreparedBabelOutput {
    pub(crate) fn publish(mut self) -> Result<Self, String> {
        validate_output_parent(&self.target, &self.allowed_root)?;
        fs::rename(&self.temporary, &self.target).map_err(|error| {
            format!(
                "Could not publish {}: {error}",
                self.target.to_string_lossy()
            )
        })?;
        self.temporary = PathBuf::new();
        Ok(self)
    }
}

impl Drop for PreparedBabelOutput {
    fn drop(&mut self) {
        if !self.temporary.as_os_str().is_empty() {
            let _ = fs::remove_file(&self.temporary);
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
    let executor_name = match (diagram_language, typst_source) {
        (Some(diagram), _) => diagram.display_name(),
        (None, true) => "Typst",
        (None, false) => {
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
    let file = header_argument(&tokens, ":file")
        .filter(|file| !file.is_empty())
        .ok_or_else(|| format!("{executor_name} execution requires a :file header argument"))?;
    if file.contains(['\n', '\r', ']']) {
        return Err("The :file value contains characters that cannot form an Org link".to_owned());
    }
    let document_root = document_path
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .to_path_buf();
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
    let silent = header_values(&tokens, ":results")
        .iter()
        .any(|value| value.eq_ignore_ascii_case("silent"));
    if header_values(&tokens, ":results")
        .iter()
        .any(|value| value.eq_ignore_ascii_case("append") || value.eq_ignore_ascii_case("prepend"))
    {
        return Err(format!(
            "{} currently supports :results replace or silent, not append/prepend",
            executor.language_name()
        ));
    }
    let newline = if opening.contains("\r\n") {
        "\r\n"
    } else {
        "\n"
    };
    let result_link = format!("[[file:{file}]]");
    let result_edit = (!silent)
        .then(|| {
            let range = existing_result_range(snapshot, &blocks, block_index);
            let existing = snapshot.copy_range(range);
            // Re-running a block must not drop attributes the user attached to
            // the results image, such as the width set by the resize grip.
            let replacement = match preserved_result_attributes(&existing) {
                Some(attributes) => {
                    format!(
                        "{newline}#+RESULTS:{newline}{attributes}{newline}{result_link}{newline}"
                    )
                }
                None => format!("{newline}#+RESULTS:{newline}{result_link}{newline}"),
            };
            (existing != replacement).then(|| TextEdit::new(range, replacement))
        })
        .flatten();

    Ok(BabelExecutionRequest {
        document_id: snapshot.document_id(),
        revision: snapshot.revision(),
        selection,
        source_block_start: block.source.start,
        source: snapshot.copy_range(block.content),
        executor,
        document_root,
        target,
        result_edit,
        result_link,
    })
}

pub(crate) fn execute_source_block(
    request: BabelExecutionRequest,
) -> Result<PreparedBabelOutput, String> {
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
    };
    let (temporary, allowed_root) =
        write_temporary_output(&request.target, &request.document_root, &bytes)?;
    Ok(PreparedBabelOutput {
        document_id: request.document_id,
        revision: request.revision,
        selection: request.selection,
        source_block_start: request.source_block_start,
        language_name: request.executor.language_name(),
        target: request.target,
        result_edit: request.result_edit,
        result_link: request.result_link,
        warnings,
        temporary,
        allowed_root,
    })
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

fn resolve_output_target(document_root: &Path, file: &str) -> Result<PathBuf, String> {
    let file = Path::new(file);
    if file.is_absolute() {
        return Err("Babel :file must be relative to the Org document directory".to_owned());
    }
    let mut relative = PathBuf::new();
    for component in file.components() {
        match component {
            Component::Normal(part) => relative.push(part),
            Component::CurDir => {}
            Component::ParentDir => {
                return Err("Babel :file must not leave the Org document directory".to_owned());
            }
            Component::RootDir | Component::Prefix(_) => {
                return Err("Babel :file must be relative to the Org document directory".to_owned());
            }
        }
    }
    if relative.file_name().is_none() {
        return Err("The :file target has no valid filename".to_owned());
    }
    Ok(document_root.join(relative))
}

fn validate_output_parent(target: &Path, allowed_root: &Path) -> Result<PathBuf, String> {
    let parent = target.parent().unwrap_or_else(|| Path::new("."));
    let parent = fs::canonicalize(parent)
        .map_err(|error| format!("Could not resolve {}: {error}", parent.display()))?;
    if !parent.starts_with(allowed_root) {
        return Err(format!(
            "Babel :file resolves outside the Org document directory: {}",
            target.display()
        ));
    }
    Ok(parent)
}

fn write_temporary_output(
    target: &Path,
    document_root: &Path,
    bytes: &[u8],
) -> Result<(PathBuf, PathBuf), String> {
    let parent = target.parent().unwrap_or_else(|| Path::new("."));
    let allowed_root = fs::canonicalize(document_root).map_err(|error| {
        format!(
            "Could not resolve document directory {}: {error}",
            document_root.display()
        )
    })?;
    fs::create_dir_all(parent)
        .map_err(|error| format!("Could not create {}: {error}", parent.display()))?;
    let parent = validate_output_parent(target, &allowed_root)?;
    let name = target
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| "The :file target has no valid filename".to_owned())?;
    for _ in 0..16 {
        let sequence = NEXT_TEMP_OUTPUT.fetch_add(1, Ordering::Relaxed);
        let temporary = parent.join(format!(
            ".{name}.org-studio-{}-{sequence}.tmp",
            std::process::id()
        ));
        match OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&temporary)
        {
            Ok(mut file) => {
                if let Err(error) = file.write_all(bytes).and_then(|_| file.sync_all()) {
                    let _ = fs::remove_file(&temporary);
                    return Err(format!("Could not write {}: {error}", target.display()));
                }
                return Ok((temporary, allowed_root));
            }
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(error) => {
                return Err(format!("Could not create {}: {error}", target.display()));
            }
        }
    }
    Err(format!(
        "Could not reserve a temporary file beside {}",
        target.display()
    ))
}

fn existing_result_range(
    snapshot: &DocumentSnapshot,
    blocks: &BlockArena,
    source_index: usize,
) -> ByteRange {
    let source = &blocks.nodes()[source_index];
    let mut index = source_index + 1;
    while blocks.nodes().get(index).is_some_and(|candidate| {
        candidate.parent == source.parent && matches!(candidate.kind, BlockKind::BlankLine)
    }) {
        index += 1;
    }
    let Some(marker) = blocks.nodes().get(index).filter(|candidate| {
        candidate.parent == source.parent
            && matches!(candidate.kind, BlockKind::Keyword)
            && is_results_marker(&snapshot.copy_range(candidate.content))
    }) else {
        return ByteRange::new(source.source.end.0, source.source.end.0);
    };
    let mut end = marker.source.end;
    let mut payload_index = index + 1;
    // Affiliated keywords such as `#+ATTR_ORG:` sit between the marker and the
    // image; they belong to the results block so a re-run may rewrite them.
    while let Some(keyword) = blocks.nodes().get(payload_index).filter(|candidate| {
        candidate.parent == source.parent
            && matches!(candidate.kind, BlockKind::Keyword)
            && candidate.source.start >= end
            && crate::org_syntax::attributes::is_affiliated_keyword(
                &snapshot.copy_range(candidate.content),
            )
    }) {
        end = keyword.source.end;
        payload_index += 1;
    }
    if let Some(payload) = blocks
        .nodes()
        .get(payload_index)
        .filter(|candidate| candidate.parent == source.parent && candidate.source.start >= end)
    {
        let payload_source = snapshot.copy_range(payload.source);
        let line = payload_source
            .split_inclusive('\n')
            .next()
            .unwrap_or(&payload_source);
        let trimmed = line.trim();
        if trimmed.starts_with("[[file:") && trimmed.ends_with("]]") {
            end = ByteOffset(payload.source.start.0 + line.len() as u64);
        }
    }
    ByteRange {
        start: source.source.end,
        end,
    }
}

/// Keeps an `#+ATTR_ORG:` line attached to the results image.
fn preserved_result_attributes(existing: &str) -> Option<String> {
    existing
        .lines()
        .map(str::trim_end)
        .find(|line| crate::org_syntax::attributes::is_attr_org_line(line))
        .map(str::to_owned)
}

fn is_results_marker(source: &str) -> bool {
    let lower = source.trim().to_ascii_lowercase();
    lower.starts_with("#+results:")
        || lower
            .strip_prefix("#+results[")
            .is_some_and(|rest| rest.contains("]:"))
}

fn header_argument<'a>(tokens: &'a [String], name: &str) -> Option<&'a str> {
    tokens.windows(2).find_map(|pair| {
        pair[0]
            .eq_ignore_ascii_case(name)
            .then_some(pair[1].as_str())
    })
}

fn header_values<'a>(tokens: &'a [String], name: &str) -> Vec<&'a str> {
    let Some(start) = tokens
        .iter()
        .position(|token| token.eq_ignore_ascii_case(name))
    else {
        return Vec::new();
    };
    tokens[start + 1..]
        .iter()
        .take_while(|token| !token.starts_with(':'))
        .map(String::as_str)
        .collect()
}

fn tokenize_header(source: &str) -> Result<Vec<String>, String> {
    let mut tokens = Vec::new();
    let mut token = String::new();
    let mut quote = None;
    let mut escaped = false;
    for character in source.trim().chars() {
        if escaped {
            token.push(character);
            escaped = false;
            continue;
        }
        if character == '\\' {
            escaped = true;
            continue;
        }
        if let Some(delimiter) = quote {
            if character == delimiter {
                quote = None;
            } else {
                token.push(character);
            }
            continue;
        }
        if matches!(character, '\'' | '"') {
            quote = Some(character);
        } else if character.is_whitespace() {
            if !token.is_empty() {
                tokens.push(std::mem::take(&mut token));
            }
        } else {
            token.push(character);
        }
    }
    if escaped {
        token.push('\\');
    }
    if quote.is_some() {
        return Err("Unclosed quote in source block header".to_owned());
    }
    if !token.is_empty() {
        tokens.push(token);
    }
    Ok(tokens)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn snapshot(source: &str) -> DocumentSnapshot {
        DocumentSnapshot::from_utf8(source.as_bytes().to_vec()).unwrap()
    }

    #[test]
    fn plantuml_requires_an_explicit_output_file() {
        let text = snapshot("#+begin_src plantuml\nA -> B\n#+end_src\n");
        let error = prepare_source_block_execution(
            &text,
            Path::new("notes.org"),
            Selection::caret(ByteOffset(24)),
        )
        .err()
        .unwrap();
        assert!(error.contains(":file"));
    }

    #[test]
    fn mermaid_requires_an_explicit_output_file() {
        let source = "#+begin_src mermaid\nflowchart TB; A --> B\n#+end_src\n";
        let text = snapshot(source);
        let error = prepare_source_block_execution(
            &text,
            Path::new("notes.org"),
            Selection::caret(ByteOffset(source.find("flowchart").unwrap() as u64)),
        )
        .err()
        .unwrap();
        assert!(error.contains(":file"), "{error}");
        assert!(error.contains("Mermaid"), "{error}");
    }

    #[test]
    fn mermaid_execution_publishes_svg_and_reports_the_language() {
        use crate::document::{DocumentBuffer, EditTransaction};

        let unique = NEXT_TEMP_OUTPUT.fetch_add(1, Ordering::Relaxed);
        let root = std::env::temp_dir().join(format!(
            "org-studio-mermaid-babel-test-{}-{unique}",
            std::process::id()
        ));
        fs::create_dir(&root).unwrap();
        let document_path = root.join("notes.org");
        let source = "#+begin_src mermaid :file images/flow.svg\nflowchart TB; A[Start] --> B[End]\n#+end_src\n";
        let mut buffer = DocumentBuffer::from_utf8(source.as_bytes().to_vec()).unwrap();
        let before = buffer.snapshot();
        let request = prepare_source_block_execution(
            &before,
            &document_path,
            Selection::caret(ByteOffset(source.find("flowchart").unwrap() as u64)),
        )
        .unwrap();
        assert_eq!(request.language_name(), "Mermaid");

        let mut output = execute_source_block(request).unwrap().publish().unwrap();
        assert_eq!(output.language_name, "Mermaid");
        let edit = output.result_edit.take().unwrap();
        buffer
            .commit(EditTransaction::new(before.revision(), vec![edit]))
            .unwrap();

        let svg = fs::read(root.join("images/flow.svg")).unwrap();
        assert!(String::from_utf8_lossy(&svg).contains("<svg"));
        assert!(
            buffer
                .snapshot()
                .copy_range(ByteRange::new(0, buffer.snapshot().len_bytes()))
                .contains("#+RESULTS:\n[[file:images/flow.svg]]")
        );

        fs::remove_file(root.join("images/flow.svg")).unwrap();
        fs::remove_dir(root.join("images")).unwrap();
        fs::remove_dir(root).unwrap();
    }

    #[test]
    fn typst_requires_an_explicit_supported_output_file() {
        let source = "#+begin_src typst\nHello\n#+end_src\n";
        let text = snapshot(source);
        let error = prepare_source_block_execution(
            &text,
            Path::new("notes.org"),
            Selection::caret(ByteOffset(source.find("Hello").unwrap() as u64)),
        )
        .err()
        .unwrap();
        assert!(error.contains("Typst"));
        assert!(error.contains(":file"));

        let source = "#+begin_src typst :file result.txt\nHello\n#+end_src\n";
        let text = snapshot(source);
        let error = prepare_source_block_execution(
            &text,
            Path::new("notes.org"),
            Selection::caret(ByteOffset(source.find("Hello").unwrap() as u64)),
        )
        .err()
        .unwrap();
        assert!(error.contains(".svg, .png, or .pdf"));
    }

    #[test]
    fn quoted_file_header_builds_a_real_results_transaction() {
        let source = "#+begin_src plantuml :file \"images/hello world.svg\"\nA -> B\n#+end_src\n";
        let text = snapshot(source);
        let request = prepare_source_block_execution(
            &text,
            Path::new("notes.org"),
            Selection::caret(ByteOffset(source.find("A -> B").unwrap() as u64)),
        )
        .unwrap();
        assert_eq!(request.target, PathBuf::from("images/hello world.svg"));
        let edit = request.result_edit.unwrap();
        assert!(edit.range.is_empty());
        assert_eq!(
            edit.replacement,
            "\n#+RESULTS:\n[[file:images/hello world.svg]]\n"
        );
    }

    #[test]
    fn output_file_cannot_escape_the_document_directory() {
        for file in ["../outside.svg", "/tmp/outside.svg"] {
            let source = format!("#+begin_src plantuml :file {file}\nA -> B\n#+end_src\n");
            let text = snapshot(&source);
            let error = prepare_source_block_execution(
                &text,
                Path::new("notes/notes.org"),
                Selection::caret(ByteOffset(source.find("A -> B").unwrap() as u64)),
            )
            .err()
            .unwrap();
            assert!(error.contains("document directory"), "{error}");
        }
    }

    #[cfg(unix)]
    #[test]
    fn output_file_cannot_follow_a_parent_symlink_outside_the_document_directory() {
        use std::os::unix::fs::symlink;

        let unique = NEXT_TEMP_OUTPUT.fetch_add(1, Ordering::Relaxed);
        let base = std::env::temp_dir().join(format!(
            "org-studio-babel-sandbox-test-{}-{unique}",
            std::process::id()
        ));
        let root = base.join("document");
        let outside = base.join("outside");
        fs::create_dir_all(&root).unwrap();
        fs::create_dir_all(&outside).unwrap();
        symlink(&outside, root.join("images")).unwrap();

        let source = "#+begin_src plantuml :file images/diagram.svg\nA -> B\n#+end_src\n";
        let document = snapshot(source);
        let request = prepare_source_block_execution(
            &document,
            &root.join("notes.org"),
            Selection::caret(ByteOffset(source.find("A -> B").unwrap() as u64)),
        )
        .unwrap();
        let error = execute_source_block(request).err().unwrap();
        assert!(error.contains("outside"), "{error}");

        fs::remove_file(root.join("images")).unwrap();
        fs::remove_dir(root).unwrap();
        fs::remove_dir(outside).unwrap();
        fs::remove_dir(base).unwrap();
    }

    #[test]
    fn repeated_execution_replaces_the_existing_file_result() {
        let source = "#+begin_src plantuml :file diagram.svg\nA -> B\n#+end_src\n\n#+RESULTS:\n[[file:old.svg]]\n\nafter\n";
        let text = snapshot(source);
        let request = prepare_source_block_execution(
            &text,
            Path::new("notes.org"),
            Selection::caret(ByteOffset(source.find("A -> B").unwrap() as u64)),
        )
        .unwrap();
        let edit = request.result_edit.unwrap();
        assert_eq!(
            &source[edit.range.as_usize()],
            "\n#+RESULTS:\n[[file:old.svg]]\n"
        );
        assert_eq!(edit.replacement, "\n#+RESULTS:\n[[file:diagram.svg]]\n");
    }

    #[test]
    fn repeated_execution_preserves_the_results_image_attributes() {
        let source = "#+begin_src plantuml :file diagram.svg\nA -> B\n#+end_src\n\n#+RESULTS:\n#+ATTR_ORG: :width 320\n[[file:old.svg]]\n\nafter\n";
        let text = snapshot(source);
        let request = prepare_source_block_execution(
            &text,
            Path::new("notes.org"),
            Selection::caret(ByteOffset(source.find("A -> B").unwrap() as u64)),
        )
        .unwrap();
        let edit = request.result_edit.unwrap();
        assert_eq!(
            &source[edit.range.as_usize()],
            "\n#+RESULTS:\n#+ATTR_ORG: :width 320\n[[file:old.svg]]\n"
        );
        assert_eq!(
            edit.replacement,
            "\n#+RESULTS:\n#+ATTR_ORG: :width 320\n[[file:diagram.svg]]\n"
        );
    }

    #[test]
    fn repeated_execution_with_the_same_result_link_does_not_edit_the_document() {
        let source = "#+begin_src plantuml :file diagram.svg\nA -> B\n#+end_src\n\n#+RESULTS:\n[[file:diagram.svg]]\n";
        let text = snapshot(source);
        let request = prepare_source_block_execution(
            &text,
            Path::new("notes.org"),
            Selection::caret(ByteOffset(source.find("A -> B").unwrap() as u64)),
        )
        .unwrap();

        assert!(request.result_edit.is_none());
    }

    #[test]
    fn silent_results_write_the_file_without_editing_the_document() {
        let source = "#+begin_src plantuml :file diagram.svg :results silent\nA -> B\n#+end_src\n";
        let text = snapshot(source);
        let request = prepare_source_block_execution(
            &text,
            Path::new("notes.org"),
            Selection::caret(ByteOffset(source.find("A -> B").unwrap() as u64)),
        )
        .unwrap();
        assert!(request.result_edit.is_none());
    }

    #[test]
    fn execution_publishes_svg_and_a_committable_results_edit() {
        use crate::document::{DocumentBuffer, EditTransaction};

        let unique = NEXT_TEMP_OUTPUT.fetch_add(1, Ordering::Relaxed);
        let root = std::env::temp_dir().join(format!(
            "org-studio-babel-test-{}-{unique}",
            std::process::id()
        ));
        fs::create_dir(&root).unwrap();
        let document_path = root.join("notes.org");
        let source = "#+begin_src plantuml :file images/diagram.svg\n@startuml\nAlice -> Bob: hello\n@enduml\n#+end_src\n";
        let mut buffer = DocumentBuffer::from_utf8(source.as_bytes().to_vec()).unwrap();
        let before = buffer.snapshot();
        let request = prepare_source_block_execution(
            &before,
            &document_path,
            Selection::caret(ByteOffset(source.find("Alice").unwrap() as u64)),
        )
        .unwrap();

        let mut output = execute_source_block(request).unwrap().publish().unwrap();
        let edit = output.result_edit.take().unwrap();
        buffer
            .commit(EditTransaction::new(before.revision(), vec![edit]))
            .unwrap();

        let svg = fs::read(root.join("images/diagram.svg")).unwrap();
        assert!(String::from_utf8_lossy(&svg).contains("<svg"));
        assert!(
            buffer
                .snapshot()
                .copy_range(ByteRange::new(0, buffer.snapshot().len_bytes()))
                .contains("#+RESULTS:\n[[file:images/diagram.svg]]")
        );

        fs::remove_file(root.join("images/diagram.svg")).unwrap();
        fs::remove_dir(root.join("images")).unwrap();
        fs::remove_dir(root).unwrap();
    }

    #[test]
    fn typst_execution_publishes_svg_and_reports_source_lines() {
        let unique = NEXT_TEMP_OUTPUT.fetch_add(1, Ordering::Relaxed);
        let root = std::env::temp_dir().join(format!(
            "org-studio-typst-babel-test-{}-{unique}",
            std::process::id()
        ));
        fs::create_dir(&root).unwrap();
        let document_path = root.join("notes.org");
        let source = "#+begin_src typst :file images/card.svg\n#set page(width: 120pt, height: 80pt, margin: 8pt)\n#rect(fill: red)[Hello]\n#+end_src\n";
        let document = snapshot(source);
        let request = prepare_source_block_execution(
            &document,
            &document_path,
            Selection::caret(ByteOffset(source.find("#rect").unwrap() as u64)),
        )
        .unwrap();
        assert_eq!(request.language_name(), "Typst");

        let output = execute_source_block(request).unwrap().publish().unwrap();
        assert_eq!(output.language_name, "Typst");
        let svg = fs::read(root.join("images/card.svg")).unwrap();
        assert!(String::from_utf8_lossy(&svg).contains("<svg"));

        let updated = "#+begin_src typst :file images/card.svg\n#set page(width: 120pt, height: 80pt, margin: 8pt)\n#rect(fill: blue)[Updated]\n#+end_src\n";
        let document = snapshot(updated);
        let request = prepare_source_block_execution(
            &document,
            &document_path,
            Selection::caret(ByteOffset(updated.find("#rect").unwrap() as u64)),
        )
        .unwrap();
        execute_source_block(request).unwrap().publish().unwrap();
        let updated_svg = fs::read(root.join("images/card.svg")).unwrap();
        assert_ne!(svg, updated_svg, "the second run must replace the output");

        let invalid = "#+begin_src typst :file invalid.svg\n#set page(width:)\n#+end_src\n";
        let document = snapshot(invalid);
        let request = prepare_source_block_execution(
            &document,
            &document_path,
            Selection::caret(ByteOffset(invalid.find("#set").unwrap() as u64)),
        )
        .unwrap();
        let error = execute_source_block(request).err().unwrap();
        assert!(error.contains("line 1"), "{error}");

        fs::remove_file(root.join("images/card.svg")).unwrap();
        fs::remove_dir(root.join("images")).unwrap();
        fs::remove_dir(root).unwrap();
    }

    #[test]
    fn typst_font_size_change_replaces_the_auto_sized_svg() {
        let unique = NEXT_TEMP_OUTPUT.fetch_add(1, Ordering::Relaxed);
        let root = std::env::temp_dir().join(format!(
            "org-studio-typst-font-babel-test-{}-{unique}",
            std::process::id()
        ));
        fs::create_dir(&root).unwrap();
        let document_path = root.join("notes.org");
        let render = |font_size: u8| {
            let source = format!(
                "#+begin_src typst :file result.svg\n#set page(width: auto, height: auto, margin: 18pt)\n#set text(size: {font_size}pt)\nA deliberately long line whose auto-sized page follows the text width.\n#+end_src\n"
            );
            let document = snapshot(&source);
            let request = prepare_source_block_execution(
                &document,
                &document_path,
                Selection::caret(ByteOffset(source.find("deliberately").unwrap() as u64)),
            )
            .unwrap();
            execute_source_block(request).unwrap().publish().unwrap();
            fs::read(root.join("result.svg")).unwrap()
        };

        let small = render(12);
        let large = render(30);

        assert_ne!(
            small, large,
            "the second run must publish the new font size"
        );
        fs::remove_file(root.join("result.svg")).unwrap();
        fs::remove_dir(root).unwrap();
    }
}
