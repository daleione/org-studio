use super::*;
use std::{fs, sync::atomic::Ordering};

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
    let source =
        "#+begin_src mermaid :file images/flow.svg\nflowchart TB; A[Start] --> B[End]\n#+end_src\n";
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
    assert_eq!(
        request.file_target(),
        Some(Path::new("images/hello world.svg"))
    );
    let edit = request.file_edit().unwrap();
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
    let edit = request.file_edit().unwrap();
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
    let edit = request.file_edit().unwrap();
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

    assert!(request.file_edit().is_none());
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
    assert!(request.file_edit().is_none());
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

#[test]
fn shell_stdout_replaces_previous_fixed_width_result() {
    let source = "#+begin_src sh :results output\nprintf 'new\\n'\n#+end_src\n\n#+RESULTS:\n: old\n\nafter\n";
    let document = snapshot(source);
    let request = prepare_source_block_execution(
        &document,
        Path::new("notes.org"),
        Selection::caret(ByteOffset(source.find("printf").unwrap() as u64)),
    )
    .unwrap();
    assert_eq!(request.external_program(), Some("/bin/sh"));
    let output = execute_source_block(request).unwrap().publish().unwrap();
    let edit = output.result_edit.as_ref().unwrap();
    assert_eq!(&source[edit.range.as_usize()], "\n#+RESULTS:\n: old\n");
    assert_eq!(edit.replacement, "\n#+RESULTS:\n: new\n");
    assert!(output.file().is_none());
}

#[test]
fn repeated_shell_runs_replace_every_result_line() {
    use crate::document::{DocumentBuffer, EditTransaction};

    let source = "#+begin_src sh :results output\nprintf 'One\\nTwo\\n'\n#+end_src\n\n#+RESULTS:\n: One\n: Two\n: Two\n\n* Next\n";
    let expected = source.replacen(": Two\n: Two\n", ": Two\n", 1);
    let mut buffer = DocumentBuffer::from_utf8(source.as_bytes().to_vec()).unwrap();

    for _ in 0..3 {
        let before = buffer.snapshot();
        let request = prepare_source_block_execution(
            &before,
            Path::new("notes.org"),
            Selection::caret(ByteOffset(source.find("printf").unwrap() as u64)),
        )
        .unwrap();
        let output = execute_source_block(request).unwrap();
        if let Some(edit) = output.result_edit {
            buffer
                .commit(EditTransaction::new(before.revision(), vec![edit]))
                .unwrap();
        }
        let current = buffer.snapshot();
        assert_eq!(
            current.copy_range(ByteRange::new(0, current.len_bytes())),
            expected
        );
    }
}

#[test]
fn shell_silent_does_not_edit_and_eval_never_does_not_run() {
    let source = "#+begin_src sh :results silent\nprintf 'ok\\n'\n#+end_src\n";
    let document = snapshot(source);
    let request = prepare_source_block_execution(
        &document,
        Path::new("notes.org"),
        Selection::caret(ByteOffset(source.find("printf").unwrap() as u64)),
    )
    .unwrap();
    assert!(execute_source_block(request).unwrap().result_edit.is_none());

    let source = source.replace(":results silent", ":eval never");
    let document = snapshot(&source);
    let error = prepare_source_block_execution(
        &document,
        Path::new("notes.org"),
        Selection::caret(ByteOffset(source.find("printf").unwrap() as u64)),
    )
    .err()
    .unwrap();
    assert!(error.contains("disables execution"));
}

#[test]
fn python_stdout_is_written_as_org_fixed_width_text() {
    let source = "#+begin_src python :results output\nprint(1 + 2)\n#+end_src\n";
    let document = snapshot(source);
    let request = prepare_source_block_execution(
        &document,
        Path::new("notes.org"),
        Selection::caret(ByteOffset(source.find("print").unwrap() as u64)),
    )
    .unwrap();
    let output = execute_source_block(request).unwrap();
    assert_eq!(
        output.result_edit.unwrap().replacement,
        "\n#+RESULTS:\n: 3\n"
    );
}
