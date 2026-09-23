use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
};

use crate::{
    document::{DocumentId, DocumentSnapshot, Revision, TextSnapshot},
    org_syntax::parse,
};

use super::{
    artifact::{
        PendingArtifact, resolve_output_target, validate_output_parent, validate_publish_target,
        write_temporary_output,
    },
    source::{expand, noweb_enabled, source_blocks},
    syntax::header_argument,
};

pub(crate) struct TangleRequest {
    pub(crate) document_id: DocumentId,
    pub(crate) revision: Revision,
    root: PathBuf,
    files: BTreeMap<PathBuf, String>,
}

pub(crate) struct PreparedTangleOutput {
    pub(crate) document_id: DocumentId,
    pub(crate) revision: Revision,
    files: Vec<PendingArtifact>,
}

impl PreparedTangleOutput {
    pub(crate) fn publish(mut self) -> Result<Vec<PathBuf>, String> {
        for file in &self.files {
            validate_output_parent(&file.target, &file.allowed_root)?;
            validate_publish_target(&file.target)?;
        }
        let mut paths = Vec::with_capacity(self.files.len());
        for file in &mut self.files {
            file.publish()?;
            paths.push(file.target.clone());
        }
        Ok(paths)
    }
}

pub(crate) fn prepare_tangle(
    snapshot: &DocumentSnapshot,
    document_path: &Path,
) -> Result<TangleRequest, String> {
    let arena = parse(snapshot);
    let blocks = source_blocks(snapshot, &arena);
    let root = document_path
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .to_path_buf();
    let mut files: BTreeMap<PathBuf, String> = BTreeMap::new();
    for (index, block) in blocks.iter().enumerate() {
        let tokens = match &block.tokens {
            Ok(tokens) => tokens,
            Err(error) if block.has_tangle_argument => return Err(error.clone()),
            Err(_) => continue,
        };
        let Some(destination) = header_argument(tokens, ":tangle") else {
            continue;
        };
        if destination.eq_ignore_ascii_case("no") {
            continue;
        }
        let destination = if destination.eq_ignore_ascii_case("yes") {
            let stem = document_path
                .file_stem()
                .and_then(|stem| stem.to_str())
                .ok_or_else(|| "The Org document needs a filename for :tangle yes".to_owned())?;
            let extension = language_extension(&block.language)?;
            format!("{stem}.{extension}")
        } else {
            destination.to_owned()
        };
        let target = resolve_output_target(&root, &destination)?;
        if target == document_path {
            return Err("Tangling must not overwrite the Org document".to_owned());
        }
        let body = if noweb_enabled(tokens, "tangle") {
            expand(&blocks, index, &mut Vec::new())?
        } else {
            block.body.clone()
        };
        let output = files.entry(target).or_default();
        if !output.is_empty() && !output.ends_with('\n') {
            output.push('\n');
        }
        output.push_str(&body);
        if !output.ends_with('\n') {
            output.push('\n');
        }
    }
    if files.is_empty() {
        return Err("No source blocks have a :tangle target".to_owned());
    }
    Ok(TangleRequest {
        document_id: snapshot.document_id(),
        revision: snapshot.revision(),
        root,
        files,
    })
}

pub(crate) fn tangle_document(request: TangleRequest) -> Result<PreparedTangleOutput, String> {
    let mut files = Vec::with_capacity(request.files.len());
    for (target, body) in request.files {
        files.push(write_temporary_output(
            &target,
            &request.root,
            body.as_bytes(),
        )?);
    }
    Ok(PreparedTangleOutput {
        document_id: request.document_id,
        revision: request.revision,
        files,
    })
}

fn language_extension(language: &str) -> Result<&str, String> {
    match language.to_ascii_lowercase().as_str() {
        "python" | "python3" => Ok("py"),
        "sh" | "shell" | "bash" => Ok("sh"),
        "rust" => Ok("rs"),
        "go" => Ok("go"),
        "javascript" | "js" => Ok("js"),
        "typescript" | "ts" => Ok("ts"),
        "typst" | "typ" => Ok("typ"),
        _ => Err(format!(
            "No filename extension is known for {language}; use an explicit :tangle path"
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::{ByteOffset, Selection};
    use std::fs;

    fn snapshot(source: &str) -> DocumentSnapshot {
        DocumentSnapshot::from_utf8(source.as_bytes().to_vec()).unwrap()
    }

    #[test]
    fn tangle_expands_named_blocks_and_merges_targets_in_document_order() {
        let source = "#+NAME: greeting\n#+begin_src python\nprint('hello')\n#+end_src\n\n#+begin_src python :tangle output.py :noweb yes\n<<greeting>>\n#+end_src\n\n#+begin_src python :tangle output.py\nprint('world')\n#+end_src\n";
        let document = snapshot(source);
        let root = std::env::temp_dir().join(format!(
            "org-studio-tangle-test-{}-{}",
            std::process::id(),
            super::super::artifact::NEXT_TEMP_OUTPUT
                .fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        ));
        fs::create_dir(&root).unwrap();
        let request = prepare_tangle(&document, &root.join("notes.org")).unwrap();
        let prepared = tangle_document(request).unwrap();
        let paths = prepared.publish().unwrap();
        assert_eq!(paths, vec![root.join("output.py")]);
        let output = fs::read_to_string(&paths[0]).unwrap();
        assert_eq!(output, "print('hello')\nprint('world')\n");
        fs::remove_file(&paths[0]).unwrap();
        fs::remove_dir(&root).unwrap();
    }

    #[test]
    fn noweb_expands_for_execution_and_rejects_cycles() {
        let source = "#+NAME: shared\n#+begin_src sh\nprintf 'hello'\n#+end_src\n#+begin_src sh :noweb yes :results output\n<<shared>>\n#+end_src\n";
        let document = snapshot(source);
        let request = super::super::prepare_source_block_execution(
            &document,
            Path::new("notes.org"),
            Selection::caret(ByteOffset(source.find("<<shared>>").unwrap() as u64)),
        )
        .unwrap();
        assert_eq!(request.source.trim(), "printf 'hello'");

        let source =
            "#+NAME: loop\n#+begin_src sh :tangle loop.sh :noweb yes\n<<loop>>\n#+end_src\n";
        let error = prepare_tangle(&snapshot(source), Path::new("notes.org"))
            .err()
            .unwrap();
        assert!(error.contains("cycle"), "{error}");
    }

    #[test]
    fn noweb_ref_groups_preserve_indentation() {
        let source = "#+begin_src python :noweb-ref shared\nprint('a')\n#+end_src\n#+begin_src python :noweb-ref shared\nprint('b')\n#+end_src\n#+begin_src python :tangle result.py :noweb yes\nif True:\n    <<shared>>\n#+end_src\n";
        let request = prepare_tangle(&snapshot(source), Path::new("notes.org")).unwrap();
        assert_eq!(
            request.files[&PathBuf::from("result.py")],
            "if True:\n    print('a')\n    print('b')\n"
        );
    }

    #[test]
    fn tangle_rejects_paths_outside_document_directory() {
        let source = "#+begin_src rust :tangle ../escape.rs\nfn main() {}\n#+end_src\n";
        let error = prepare_tangle(&snapshot(source), Path::new("notes/notes.org"))
            .err()
            .unwrap();
        assert!(error.contains("document directory"), "{error}");
    }

    #[test]
    fn tangle_yes_uses_language_file_extensions_without_running_code() {
        let source = "#+begin_src rust :tangle yes\nfn main() {}\n#+end_src\n#+begin_src go :tangle yes\npackage main\n#+end_src\n";
        let request = prepare_tangle(&snapshot(source), Path::new("notes.org")).unwrap();
        assert_eq!(request.files[&PathBuf::from("notes.rs")], "fn main() {}\n");
        assert_eq!(request.files[&PathBuf::from("notes.go")], "package main\n");
    }

    #[test]
    fn invalid_second_target_does_not_publish_first_target() {
        let source = "#+begin_src sh :tangle a.sh\necho new\n#+end_src\n#+begin_src sh :tangle z.sh\necho other\n#+end_src\n";
        let root = std::env::temp_dir().join(format!(
            "org-studio-tangle-publish-test-{}-{}",
            std::process::id(),
            super::super::artifact::NEXT_TEMP_OUTPUT
                .fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        ));
        fs::create_dir(&root).unwrap();
        fs::write(root.join("a.sh"), "original\n").unwrap();
        fs::create_dir(root.join("z.sh")).unwrap();

        let request = prepare_tangle(&snapshot(source), &root.join("notes.org")).unwrap();
        let error = tangle_document(request).unwrap().publish().unwrap_err();
        assert!(error.contains("not a file"), "{error}");
        assert_eq!(fs::read_to_string(root.join("a.sh")).unwrap(), "original\n");

        fs::remove_file(root.join("a.sh")).unwrap();
        fs::remove_dir(root.join("z.sh")).unwrap();
        fs::remove_dir(root).unwrap();
    }

    #[test]
    fn unrelated_invalid_header_does_not_block_tangling() {
        let source = "#+begin_src sh :var broken=\"quote\necho ignored\n#+end_src\n#+begin_src sh :tangle output.sh\necho kept\n#+end_src\n";
        let request = prepare_tangle(&snapshot(source), Path::new("notes.org")).unwrap();
        assert_eq!(request.files[&PathBuf::from("output.sh")], "echo kept\n");

        let invalid_target = source.replace(":var broken=\"quote", ":tangle \"broken");
        let error = prepare_tangle(&snapshot(&invalid_target), Path::new("notes.org"))
            .err()
            .unwrap();
        assert!(error.contains("Unclosed quote"), "{error}");
    }

    #[test]
    fn unrelated_invalid_header_does_not_block_noweb_execution() {
        let source = "#+begin_src sh :var broken=\"quote\necho ignored\n#+end_src\n#+NAME: shared\n#+begin_src sh\nprintf 'kept'\n#+end_src\n#+begin_src sh :results output :noweb yes\n<<shared>>\n#+end_src\n";
        let request = super::super::prepare_source_block_execution(
            &snapshot(source),
            Path::new("notes.org"),
            Selection::caret(ByteOffset(source.find("<<shared>>").unwrap() as u64)),
        )
        .unwrap();
        assert_eq!(request.source.trim(), "printf 'kept'");
    }
}
