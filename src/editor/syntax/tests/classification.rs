use super::*;

#[test]
fn markdown_pipe_rows_have_table_source_style_outside_fences() {
    let mut context = CodeContext::default();
    assert_eq!(
        classify_line(Language::Markdown, "| Name | Value |", &mut context),
        EditorStyleId::Table
    );
    assert_eq!(
        classify_line(Language::Markdown, "```md", &mut context),
        EditorStyleId::CodeBoundary
    );
    assert_eq!(
        classify_line(Language::Markdown, "| literal | row |", &mut context),
        EditorStyleId::Code
    );
}

#[test]
fn markdown_fences_follow_commonmark_closing_and_opening_rules() {
    fn styles_for(source: &str) -> Vec<(EditorStyleId, Option<EditorBlockKind>, u64)> {
        let snapshot = DocumentSnapshot::from_utf8(source.as_bytes().to_vec()).unwrap();
        let lines = (0..snapshot.len_lines()).collect::<Vec<_>>();
        let styles = SparseEditorStyleSnapshot::for_lines(
            Path::new("probe.md"),
            &snapshot,
            &lines,
            &EditorSyntaxService::default(),
        );
        lines
            .iter()
            .map(|line| {
                let style = styles.line(*line).unwrap();
                (
                    style.id,
                    style.block.as_ref().map(|block| block.kind.clone()),
                    style
                        .block
                        .as_ref()
                        .and_then(|block| block.body_line)
                        .unwrap_or(0),
                )
            })
            .collect()
    }

    // A 4-backtick fence containing a ```json fence: the inner fence must not
    // close the outer one, so the json body keeps its code styling.
    let nested = styles_for("````markdown\n```json\n{ \"a\": 1 }\n```\n````\n");
    let expected = [
        (
            EditorStyleId::CodeBoundary,
            Some(EditorBlockKind::MarkdownFence),
            0,
        ), // ````markdown open
        (EditorStyleId::Code, Some(EditorBlockKind::MarkdownFence), 1), // ```json body
        (EditorStyleId::Code, Some(EditorBlockKind::MarkdownFence), 2), // { "a": 1 } body
        (EditorStyleId::Code, Some(EditorBlockKind::MarkdownFence), 3), // ``` body
        (
            EditorStyleId::CodeBoundary,
            Some(EditorBlockKind::MarkdownFence),
            4,
        ), // ```` close
    ];
    assert_eq!(&nested[..5], &expected[..]);

    // A closing run shorter than the opening run does not close the fence.
    let short = styles_for("````json\n{ \"a\": 1 }\n```\n```\n````\n");
    assert_eq!(
        short[0],
        (
            EditorStyleId::CodeBoundary,
            Some(EditorBlockKind::MarkdownFence),
            0
        )
    );
    assert_eq!(
        short[1],
        (EditorStyleId::Code, Some(EditorBlockKind::MarkdownFence), 1)
    );
    assert_eq!(
        short[2],
        (EditorStyleId::Code, Some(EditorBlockKind::MarkdownFence), 2)
    );
    assert_eq!(
        short[3],
        (EditorStyleId::Code, Some(EditorBlockKind::MarkdownFence), 3)
    );
    assert_eq!(
        short[4],
        (
            EditorStyleId::CodeBoundary,
            Some(EditorBlockKind::MarkdownFence),
            4
        )
    );

    // A fence indented by more than three spaces is an indented code block,
    // not a fence, so it stays plain text.
    let indented = styles_for("    ```json\n    { \"a\": 1 }\n    ```\n");
    for (id, block, _) in indented {
        assert_eq!(id, EditorStyleId::Plain);
        assert_eq!(block, None);
    }
}

#[test]
fn long_markdown_table_rows_keep_their_source_style() {
    let source = format!("| {} | value |\n", "a".repeat(5_000));
    let snapshot = DocumentSnapshot::from_utf8(source.into_bytes()).unwrap();
    let styles = EditorStyleSnapshot::for_lines(
        Path::new("a.md"),
        &snapshot,
        0..1,
        &EditorSyntaxService::default(),
    );

    assert_eq!(styles.lines[0].id, EditorStyleId::Table);
}

#[test]
fn style_snapshot_is_viewport_bounded_and_revision_coherent() {
    let snapshot = DocumentSnapshot::from_utf8(b"* One\nbody\n** Two\n".to_vec()).unwrap();
    let styles = EditorStyleSnapshot::for_lines(
        Path::new("a.org"),
        &snapshot,
        1..3,
        &EditorSyntaxService::default(),
    );
    assert_eq!(styles.revision, snapshot.revision());
    assert_eq!(styles.lines.len(), 2);
    assert_eq!(styles.lines[0].id, EditorStyleId::Plain);
    assert_eq!(styles.lines[1].id, EditorStyleId::Heading(2));
}

#[test]
fn style_snapshot_marks_code_body_without_hiding_boundaries() {
    let snapshot = DocumentSnapshot::from_utf8(
        b"#+begin_src rust\nlet value = 1;\n#+end_src\nafter\n".to_vec(),
    )
    .unwrap();
    let cache = EditorSyntaxService::default();
    let styles = EditorStyleSnapshot::for_lines(Path::new("a.org"), &snapshot, 0..4, &cache);
    assert_eq!(styles.lines[0].id, EditorStyleId::CodeBoundary);
    assert_eq!(
        styles.lines[0].block,
        Some(EditorBlockDecoration {
            kind: EditorBlockKind::Source,
            edge: EditorBlockEdge::Open,
            body_line: None,
        })
    );
    assert_eq!(styles.lines[1].id, EditorStyleId::Code);
    assert_eq!(styles.lines[1].code_language.as_deref(), Some("rust"));
    assert_eq!(styles.lines[1].block.as_ref().unwrap().body_line, Some(1));
    assert_eq!(styles.lines[2].id, EditorStyleId::CodeBoundary);
    assert_eq!(
        styles.lines[2].block,
        Some(EditorBlockDecoration {
            kind: EditorBlockKind::Source,
            edge: EditorBlockEdge::Close,
            body_line: Some(2),
        })
    );
    assert_eq!(styles.lines[3].id, EditorStyleId::Plain);
    let body_only = EditorStyleSnapshot::for_lines(Path::new("a.org"), &snapshot, 1..2, &cache);
    assert_eq!(body_only.lines[0].id, EditorStyleId::Code);
    assert_eq!(
        body_only.lines[0].block.as_ref().unwrap().body_line,
        Some(1)
    );
}

#[test]
fn org_end_marker_closes_blocks_when_building_checkpoint_context() {
    // Checkpoint scans consume complete lines (including their newline),
    // unlike viewport classification.  The end marker must still close the
    // block or every later scrolled-to range is misclassified as source.
    let mut source = String::from("#+begin_src rust\nlet value = 1;\n#+end_src\n");
    source.push_str(&"after\n".repeat(300));
    let snapshot = DocumentSnapshot::from_utf8(source.into_bytes()).unwrap();
    let cache = EditorSyntaxService::default();

    let context = cache.context_at(&snapshot, Language::Org, 256);
    assert!(!context.in_block);

    let styles = SparseEditorStyleSnapshot::query_lines(
        Path::new("checkpoint.org"),
        &snapshot,
        &[256],
        &cache,
    );
    assert_eq!(styles.snapshot.line(256).unwrap().id, EditorStyleId::Plain);
}

#[test]
fn source_body_line_numbers_are_stable_when_the_viewport_starts_inside_the_block() {
    let snapshot = DocumentSnapshot::from_utf8(
        b"#+begin_src plantuml\n@startuml\nAlice -> Bob\n@enduml\n#+end_src\n".to_vec(),
    )
    .unwrap();
    let cache = EditorSyntaxService::default();
    let styles = EditorStyleSnapshot::for_lines(Path::new("a.org"), &snapshot, 0..5, &cache);
    assert_eq!(styles.lines[1].block.as_ref().unwrap().body_line, Some(1));
    assert_eq!(styles.lines[2].block.as_ref().unwrap().body_line, Some(2));
    assert_eq!(styles.lines[3].block.as_ref().unwrap().body_line, Some(3));
    assert_eq!(styles.lines[4].block.as_ref().unwrap().body_line, Some(4));

    let middle = EditorStyleSnapshot::for_lines(Path::new("a.org"), &snapshot, 2..3, &cache);
    assert_eq!(middle.lines[0].block.as_ref().unwrap().body_line, Some(2));
}

#[test]
fn every_standard_and_custom_org_block_keeps_its_decoration_kind() {
    let source = concat!(
        "#+begin_src rust\n#+end_src\n",
        "#+begin_example\n#+end_example\n",
        "#+begin_quote\n#+end_quote\n",
        "#+begin_verse\n#+end_verse\n",
        "#+begin_center\n#+end_center\n",
        "#+begin_comment\n#+end_comment\n",
        "#+begin_export html\n#+end_export\n",
        "#+begin_details\n#+end_details\n",
    );
    let snapshot = DocumentSnapshot::from_utf8(source.as_bytes().to_vec()).unwrap();
    let styles = EditorStyleSnapshot::for_lines(
        Path::new("a.org"),
        &snapshot,
        0..16,
        &EditorSyntaxService::default(),
    );
    let expected = [
        EditorBlockKind::Source,
        EditorBlockKind::Example,
        EditorBlockKind::Quote,
        EditorBlockKind::Verse,
        EditorBlockKind::Center,
        EditorBlockKind::Comment,
        EditorBlockKind::Export,
        EditorBlockKind::Special(Arc::from("details")),
    ];

    for (index, kind) in expected.into_iter().enumerate() {
        let open = styles.lines[index * 2].block.as_ref().unwrap();
        let close = styles.lines[index * 2 + 1].block.as_ref().unwrap();
        assert_eq!(open.kind, kind);
        assert_eq!(open.edge, EditorBlockEdge::Open);
        assert_eq!(close.kind, open.kind);
        assert_eq!(close.edge, EditorBlockEdge::Close);
    }
}

#[test]
fn example_block_keeps_nested_org_blocks_verbatim() {
    let snapshot = DocumentSnapshot::from_utf8(
        b"#+begin_example\n#+name: hello-rust\n#+begin_src rust :results output\nfn main() {\n    println!(\"hello\");\n}\n#+end_src\n#+end_example\nafter\n".to_vec(),
    )
    .unwrap();
    let cache = EditorSyntaxService::default();
    let styles = EditorStyleSnapshot::for_lines(Path::new("a.org"), &snapshot, 0..9, &cache);

    assert_eq!(styles.lines[0].id, EditorStyleId::CodeBoundary);
    assert!(
        styles.lines[1..7]
            .iter()
            .all(|line| line.id == EditorStyleId::Code)
    );
    assert_eq!(styles.lines[7].id, EditorStyleId::CodeBoundary);
    assert_eq!(styles.lines[8].id, EditorStyleId::Plain);

    let tail = EditorStyleSnapshot::for_lines(Path::new("a.org"), &snapshot, 6..9, &cache);
    assert_eq!(tail.lines[0].id, EditorStyleId::Code);
    assert_eq!(tail.lines[1].id, EditorStyleId::CodeBoundary);
    assert_eq!(tail.lines[2].id, EditorStyleId::Plain);
}

#[test]
fn markdown_links_classify_dest_urls() {
    let text = "[web](https://x.org) [file](a.md) [sec](#top) [mail](mailto:a@b.c)";
    let style = line_style(text, &mut CodeContext::default());
    let spans = semantic_spans(Path::new("a.md"), text, &style);
    let colors: Vec<EditorColorToken> = spans
        .iter()
        .filter_map(|span| span.link.as_ref().map(|_| span.color.expect("colored")))
        .collect();
    assert_eq!(
        colors,
        vec![
            EditorColorToken::LinkExternal,
            EditorColorToken::LinkFile,
            EditorColorToken::LinkInternal,
            EditorColorToken::LinkMail,
        ]
    );
}

#[test]
fn long_code_block_style_is_independent_of_viewport_start() {
    let mut source = String::from("#+begin_src rust\n");
    source.push_str(&"let value = 1;\n".repeat(700));
    source.push_str("#+end_src\nafter\n");
    let snapshot = DocumentSnapshot::from_utf8(source.into_bytes()).unwrap();
    let cache = EditorSyntaxService::default();

    let middle = EditorStyleSnapshot::for_lines(Path::new("a.org"), &snapshot, 400..402, &cache);
    let later = EditorStyleSnapshot::for_lines(Path::new("a.org"), &snapshot, 650..652, &cache);
    let middle_again =
        EditorStyleSnapshot::for_lines(Path::new("a.org"), &snapshot, 400..402, &cache);

    assert!(
        middle
            .lines
            .iter()
            .all(|line| line.id == EditorStyleId::Code)
    );
    assert!(
        later
            .lines
            .iter()
            .all(|line| line.id == EditorStyleId::Code)
    );
    assert_eq!(middle.lines[0].id, middle_again.lines[0].id);
    assert!(
        cache
            .inner
            .lock()
            .expect("editor syntax cache poisoned")
            .contexts
            .len()
            >= 3
    );
}
