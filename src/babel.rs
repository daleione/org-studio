use std::{
    fs::{self, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
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
    source: String,
    format: typstuml::render::Format,
    target: PathBuf,
    result_edit: Option<TextEdit>,
    result_link: String,
}

pub(crate) struct PreparedBabelOutput {
    pub(crate) document_id: DocumentId,
    pub(crate) revision: Revision,
    pub(crate) selection: Selection,
    pub(crate) target: PathBuf,
    pub(crate) result_edit: Option<TextEdit>,
    pub(crate) result_link: String,
    pub(crate) warnings: Vec<String>,
    temporary: PathBuf,
}

impl PreparedBabelOutput {
    pub(crate) fn publish(mut self) -> Result<Self, String> {
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
    if !crate::preview::is_plantuml_language(Some(language)) {
        return Err(format!(
            "No Babel executor is registered for {}",
            if language.is_empty() {
                "this block"
            } else {
                language
            }
        ));
    }

    let opening = snapshot.copy_range(ByteRange {
        start: block.source.start,
        end: block.content.start,
    });
    let tokens = tokenize_header(&opening)?;
    let file = header_argument(&tokens, ":file")
        .filter(|file| !file.is_empty())
        .ok_or_else(|| "PlantUML execution requires a :file header argument".to_owned())?;
    if file.contains(['\n', '\r', ']']) {
        return Err("The :file value contains characters that cannot form an Org link".to_owned());
    }
    let target = if Path::new(file).is_absolute() {
        PathBuf::from(file)
    } else {
        document_path
            .parent()
            .unwrap_or_else(|| Path::new("."))
            .join(file)
    };
    if target == document_path {
        return Err("The Babel result must not overwrite the Org document".to_owned());
    }
    let format = typstuml::render::Format::infer_from_path(&target)
        .ok_or_else(|| "PlantUML :file must end in .svg, .png, or .pdf".to_owned())?;
    let silent = header_values(&tokens, ":results")
        .iter()
        .any(|value| value.eq_ignore_ascii_case("silent"));
    if header_values(&tokens, ":results")
        .iter()
        .any(|value| value.eq_ignore_ascii_case("append") || value.eq_ignore_ascii_case("prepend"))
    {
        return Err(
            "PlantUML currently supports :results replace or silent, not append/prepend".to_owned(),
        );
    }
    let newline = if opening.contains("\r\n") {
        "\r\n"
    } else {
        "\n"
    };
    let result_link = format!("[[file:{file}]]");
    let result_edit = (!silent).then(|| {
        let range = existing_result_range(snapshot, &blocks, block_index);
        TextEdit::new(
            range,
            format!("{newline}#+RESULTS:{newline}{result_link}{newline}"),
        )
    });

    Ok(BabelExecutionRequest {
        document_id: snapshot.document_id(),
        revision: snapshot.revision(),
        selection,
        source: snapshot.copy_range(block.content),
        format,
        target,
        result_edit,
        result_link,
    })
}

pub(crate) fn execute_source_block(
    request: BabelExecutionRequest,
) -> Result<PreparedBabelOutput, String> {
    let rendered =
        typstuml::render::render_source(&request.source, request.format).map_err(|error| {
            error
                .to_diagnostics()
                .into_iter()
                .map(|diagnostic| diagnostic.to_string())
                .collect::<Vec<_>>()
                .join("\n")
        })?;
    let temporary = write_temporary_output(&request.target, &rendered.bytes)?;
    Ok(PreparedBabelOutput {
        document_id: request.document_id,
        revision: request.revision,
        selection: request.selection,
        target: request.target,
        result_edit: request.result_edit,
        result_link: request.result_link,
        warnings: rendered
            .warnings
            .into_iter()
            .map(|warning| warning.to_string())
            .collect(),
        temporary,
    })
}

fn write_temporary_output(target: &Path, bytes: &[u8]) -> Result<PathBuf, String> {
    let parent = target.parent().unwrap_or_else(|| Path::new("."));
    fs::create_dir_all(parent)
        .map_err(|error| format!("Could not create {}: {error}", parent.display()))?;
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
                return Ok(temporary);
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
    if let Some(payload) = blocks.nodes().get(index + 1).filter(|candidate| {
        candidate.parent == source.parent && candidate.source.start >= marker.source.end
    }) {
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
}
