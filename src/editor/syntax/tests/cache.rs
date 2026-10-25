use super::*;

#[test]
fn cold_random_queries_synchronously_cover_their_requested_range() {
    let mut source = String::from("#+begin_quote\n");
    source.push_str(&"body\n".repeat(1_100));
    source.push_str("#+end_quote\n");
    let snapshot = DocumentSnapshot::from_utf8(source.into_bytes()).unwrap();
    let service = EditorSyntaxService::default();

    // A cold query must resolve synchronously: the visible range never falls
    // back to Plain chrome while checkpoints are missing.
    let first =
        SparseEditorStyleSnapshot::query_lines(Path::new("cold.org"), &snapshot, &[800], &service);
    assert!(!first.pending);
    assert!(!first.start_builder);
    let style = first.snapshot.line(800).unwrap();
    assert_eq!(style.id, EditorStyleId::Code);
    assert_eq!(style.block.as_ref().unwrap().kind, EditorBlockKind::Quote);
    // Only the checkpoints needed to cover line 800 were built — the shared
    // focus line is never the synchronous build target.
    let contexts = service
        .inner
        .lock()
        .unwrap()
        .contexts
        .keys()
        .copied()
        .collect::<Vec<_>>();
    assert!(contexts.iter().all(|line| *line <= 768));

    // A second range is covered on demand; the prefetch build is a no-op.
    let second =
        SparseEditorStyleSnapshot::query_lines(Path::new("cold.org"), &snapshot, &[900], &service);
    assert!(!second.pending);
    assert!(!second.start_builder);
    service.build_focused(Path::new("cold.org"), &snapshot);
    let ready = SparseEditorStyleSnapshot::query_lines(
        Path::new("cold.org"),
        &snapshot,
        &[800, 900],
        &service,
    );
    assert!(!ready.pending);
    assert!(!ready.start_builder);
    for line in [800, 900] {
        let style = ready.snapshot.line(line).unwrap();
        assert_eq!(style.id, EditorStyleId::Code);
        assert_eq!(style.block.as_ref().unwrap().kind, EditorBlockKind::Quote);
    }
}

#[test]
fn stale_background_queries_preserve_current_revision_styles() {
    use crate::document::{DocumentBuffer, EditTransaction, TextEdit};

    let source = format!(
        "#+begin_src rust\n{}#+end_src\n",
        "let value = 1;\n".repeat(1_100)
    );
    let mut buffer = DocumentBuffer::from_utf8(source.into_bytes()).unwrap();
    let old = buffer.snapshot();
    let path = Path::new("editing.org");
    let service = EditorSyntaxService::default();
    SparseEditorStyleSnapshot::query_lines(path, &old, &[900], &service);
    service.build_focused(path, &old);

    let offset = old.line_content_range(LineIndex(301)).unwrap().start;
    buffer
        .commit(EditTransaction::new(
            old.revision(),
            vec![TextEdit::new(ByteRange::new(offset.0, offset.0), " ")],
        ))
        .unwrap();
    let current = buffer.snapshot();
    service.invalidate_from(current.document_id(), current.revision(), 301);
    let visible = SparseEditorStyleSnapshot::query_lines(path, &current, &[290], &service);
    assert!(!visible.pending);

    // A cancelled layout/raster job can still reach its next syntax query.
    SparseEditorStyleSnapshot::query_lines(path, &old, &[900], &service);
    service.build_focused(path, &old);
    let visible = SparseEditorStyleSnapshot::query_lines(path, &current, &[290], &service);
    assert!(
        !visible.pending,
        "stale background work discarded current checkpoints"
    );
    assert_eq!(visible.snapshot.line(290).unwrap().id, EditorStyleId::Code);

    // Also cover invalidation between ready_context_at and context_at.
    assert!(service.context_at(&old, Language::Org, 900).in_block);
    let visible = SparseEditorStyleSnapshot::query_lines(path, &current, &[290], &service);
    assert!(!visible.pending);
    assert_eq!(service.inner.lock().unwrap().revision, current.revision());
}

#[test]
fn non_org_display_settings_preserve_markdown_syntax_checkpoints() {
    let source = format!(
        "#+STARTUP: literallinks\n```text\n{}\n```\n",
        "body\n".repeat(700)
    );
    let snapshot = DocumentSnapshot::from_utf8(source.into_bytes()).unwrap();
    let service = EditorSyntaxService::default();
    let path = Path::new("notes.md");
    let query = SparseEditorStyleSnapshot::query_lines(path, &snapshot, &[650], &service);
    assert_eq!(query.snapshot.line(650).unwrap().id, EditorStyleId::Code);
    let checkpoints = service
        .inner
        .lock()
        .unwrap()
        .contexts
        .keys()
        .copied()
        .collect::<Vec<_>>();
    for path in [path, Path::new("program.rs")] {
        assert!(service.display_options(path, &snapshot).descriptive_links);
        let cache = service.inner.lock().unwrap();
        assert_eq!(cache.language, Language::Markdown);
        assert_eq!(
            cache.contexts.keys().copied().collect::<Vec<_>>(),
            checkpoints
        );
    }
}
