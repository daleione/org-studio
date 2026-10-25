use super::*;

#[test]
fn source_files_use_code_highlighting_without_org_markup() {
    for (path, source, language) in [
        ("main.rs", "fn main() { let value = 42; }\n", "rust"),
        ("main.go", "package main\nfunc main() {}\n", "go"),
        ("main.py", "def main():\n    return 42\n", "python"),
    ] {
        let snapshot = DocumentSnapshot::from_utf8(source.as_bytes().to_vec()).unwrap();
        let query = SparseEditorStyleSnapshot::query_lines(
            Path::new(path),
            &snapshot,
            &[0],
            &EditorSyntaxService::default(),
        );
        let style = query.snapshot.line(0).unwrap();
        assert!(!query.pending);
        assert_eq!(style.id, EditorStyleId::Plain);
        assert_eq!(style.code_language.as_deref(), Some(language));
        assert!(style.block.is_none());
        let text = snapshot.copy_range(style.source_range);
        let spans = semantic_spans(Path::new(path), &text, style);
        assert!(spans.iter().any(|span| span.color.is_some()), "{path}");
        assert!(spans.iter().all(|span| !span.pill), "{path}");
    }
}

#[test]
fn org_markup_uses_semantic_theme_colors() {
    let theme = &crate::theme::ORG_STUDIO_LIGHT;
    let text = "~code~ =verbatim= [[https://orgmode.org][Org]] <2026-09-02 Wed> :ui:mac:";
    let style = line_style(text, &mut CodeContext::default());
    let markup_runs = runs(
        Path::new("a.org"),
        text,
        base_run(text.len()),
        &style,
        None,
        theme,
    );

    let color = |offset| run_at(&markup_runs, offset).color;
    assert_eq!(
        color(text.find("code").unwrap()),
        rgb(theme.inline_code).into()
    );
    assert_eq!(
        color(text.find("verbatim").unwrap()),
        rgb(theme.verbatim).into()
    );
    let link = text.find("[[").unwrap();
    assert_eq!(color(link), rgb(theme.link).into());
    assert_eq!(run_at(&markup_runs, link).font.weight, FontWeight::NORMAL);
    assert_eq!(
        run_at(&markup_runs, text.find("https").unwrap())
            .font
            .weight,
        FontWeight::BOLD
    );
    assert_eq!(color(text.find("<2026").unwrap()), rgb(theme.date).into());
    assert_eq!(
        color(text.find(":ui:mac:").unwrap()),
        rgb(theme.attribute).into()
    );
    assert!(markup_runs.iter().all(|run| run.background_color.is_none()));

    for (keyword, expected) in [
        ("TODO", theme.todo),
        ("PROJ", theme.todo_project),
        ("STRT", theme.todo_active),
        ("WAIT", theme.waiting),
        ("HOLD", theme.waiting),
        ("DONE", theme.done),
        ("KILL", theme.done),
    ] {
        let heading = format!("** {keyword} Task");
        let style = line_style(&heading, &mut CodeContext::default());
        let runs = runs(
            Path::new("a.org"),
            &heading,
            base_run(heading.len()),
            &style,
            None,
            theme,
        );
        assert_eq!(
            run_at(&runs, heading.find(keyword).unwrap()).color,
            rgb(expected).into()
        );
    }
}

#[test]
fn heading_runs_preserve_semantic_colors_metrics_and_source_bytes() {
    let theme = current_theme();
    for (text, title, level, scale) in [
        ("* Headline", "Headline", 1, 1.50),
        ("** Sub-headline :tag:", "Sub", 2, 1.34),
        ("*** Third", "Third", 3, 1.20),
        ("**** Fourth", "Fourth", 4, 1.08),
        ("***** Fifth", "Fifth", 4, 1.08),
        ("** TODO 标题 *bold* [[target]] :tag:", "标题", 2, 1.34),
    ] {
        let style = line_style(text, &mut CodeContext::default());
        let spans = semantic_spans(Path::new("a.org"), text, &style);
        let runs = runs_from_spans(&spans, base_run(text.len()), &style, None, theme);
        let expected: gpui::Hsla = rgb(theme.heading[level - 1]).into();
        assert_eq!(runs.iter().map(|run| run.len).sum::<usize>(), text.len());
        assert_eq!(run_at(&runs, 0).color, expected);
        assert_eq!(run_at(&runs, text.find(title).unwrap()).color, expected);
        assert_eq!(run_at(&runs, 0).font.weight, FontWeight::BOLD);
        assert_eq!(style.metrics.font_scale, scale);
        assert!(style.metrics.line_height >= 15.0 * scale);
        if let Some(todo) = text.find("TODO") {
            assert_eq!(run_at(&runs, todo).color, rgb(theme.todo).into());
        }
        if let Some(tag) = text.find(":tag:") {
            assert_eq!(run_at(&runs, tag).color, rgb(theme.link).into());
        }
    }
}

#[test]
fn heading_statistics_cookie_takes_the_meta_face_its_progress_bar_uses() {
    let text = "** DONE Phase 3：颜色收敛与深色验收 [4/7]";
    let theme = current_theme();
    let style = line_style(text, &mut CodeContext::default());
    let spans = semantic_spans(Path::new("a.org"), text, &style);
    let direct = runs_from_spans(&spans, base_run(text.len()), &style, None, theme);
    // The cookie is meta-coloured so it matches the bar the editor paints under
    // it; the heading hue and meta only agree in the light palette.
    assert_eq!(
        run_at(&direct, text.find("[4/7]").expect("cookie")).color,
        rgb(theme.meta).into()
    );
    assert_eq!(
        run_at(&direct, text.find("颜色").expect("title")).color,
        rgb(theme.heading[1]).into()
    );

    // A cookie in running text is not a heading progress token.
    let plain = "see [4/7] for details";
    let plain_style = line_style(plain, &mut CodeContext::default());
    let plain_spans = semantic_spans(Path::new("a.org"), plain, &plain_style);
    let plain_runs = runs_from_spans(
        &plain_spans,
        base_run(plain.len()),
        &plain_style,
        None,
        theme,
    );
    assert_eq!(
        run_at(&plain_runs, plain.find("[4/7]").expect("cookie")).color,
        run_at(&plain_runs, 0).color
    );
}

#[test]
fn custom_todo_sequences_follow_the_org_separator_and_official_overrides() {
    let source = "#+TODO: PLAN(p) BUILD(b) WAIT(w@/!) | SHIPPED(s!)\n\
                  #+SEQ_TODO: OPEN(o) CLOSED(c)\n\
                  * PLAN Plan\n\
                  * BUILD Build\n\
                  * WAIT Wait\n\
                  * SHIPPED Ship\n\
                  * OPEN Open\n\
                  * CLOSED Close\n\
                  * TODO Plain title\n";
    let snapshot = DocumentSnapshot::from_utf8(source.as_bytes().to_vec()).unwrap();
    let styles = EditorStyleSnapshot::for_lines(
        Path::new("a.org"),
        &snapshot,
        2..9,
        &EditorSyntaxService::default(),
    );
    let theme = current_theme();
    for (index, keyword, expected) in [
        (0, "PLAN", theme.todo),
        (1, "BUILD", theme.todo),
        (2, "WAIT", theme.waiting),
        (3, "SHIPPED", theme.done),
        (4, "OPEN", theme.todo),
        (5, "CLOSED", theme.done),
    ] {
        let text = snapshot.copy_range(styles.lines[index].source_range);
        let runs = runs(
            Path::new("a.org"),
            &text,
            base_run(text.len()),
            &styles.lines[index],
            None,
            theme,
        );
        assert_eq!(
            run_at(&runs, text.find(keyword).unwrap()).color,
            rgb(expected).into(),
            "wrong face for {keyword}"
        );
    }

    let plain = snapshot.copy_range(styles.lines[6].source_range);
    let runs = runs(
        Path::new("a.org"),
        &plain,
        base_run(plain.len()),
        &styles.lines[6],
        None,
        theme,
    );
    assert_eq!(
        run_at(&runs, plain.find("TODO").unwrap()).color,
        rgb(theme.heading[0]).into()
    );
}

#[test]
fn official_checkbox_faces_cover_open_active_waiting_and_done_states() {
    let text = "- [ ] [-] [?] [X] [#A]";
    let style = line_style(text, &mut CodeContext::default());
    let theme = current_theme();
    let runs = runs(
        Path::new("a.org"),
        text,
        base_run(text.len()),
        &style,
        None,
        theme,
    );
    for (token, expected) in [
        ("[ ]", theme.todo),
        ("[-]", theme.todo_active),
        ("[?]", theme.waiting),
        ("[X]", theme.done),
        ("[#A]", theme.keyword),
    ] {
        assert_eq!(
            run_at(&runs, text.find(token).unwrap()).color,
            rgb(expected).into(),
            "wrong face for {token}"
        );
    }
}

#[test]
fn org_code_content_does_not_receive_org_inline_semantics() {
    let text = "*bold* [[target]] TODO :tag:";
    let style = EditorLineStyle {
        source_range: ByteRange::new(0, text.len() as u64),
        id: EditorStyleId::Code,
        code_language: None,
        block: None,
        todo: None,
        metrics: metrics_for(EditorStyleId::Code),
    };
    let runs = runs(
        Path::new("a.org"),
        text,
        base_run(text.len()),
        &style,
        None,
        current_theme(),
    );

    assert_eq!(runs.len(), 1);
    assert_eq!(runs[0].len, text.len());
    assert!(runs[0].underline.is_none());
    assert!(runs[0].strikethrough.is_none());
}

#[test]
fn editor_code_runs_use_the_declared_source_language() {
    for (path, language, source) in [
        (
            "a.org",
            "rust",
            "#+begin_src rust\nfn main() {}\n#+end_src\n",
        ),
        (
            "a.org",
            "typst",
            "#+begin_src typst\n#let message = text(\"你好😀\")\n#+end_src\n",
        ),
        (
            "a.md",
            "typ",
            "```typ\n#let message = text(\"你好😀\")\n```\n",
        ),
    ] {
        let snapshot = DocumentSnapshot::from_utf8(source.as_bytes().to_vec()).unwrap();
        let styles = EditorStyleSnapshot::for_lines(
            Path::new(path),
            &snapshot,
            0..3,
            &EditorSyntaxService::default(),
        );
        let text = snapshot.copy_range(styles.lines[1].source_range);
        let theme = current_theme();
        let runs = runs(
            Path::new(path),
            &text,
            base_run(text.len()),
            &styles.lines[1],
            None,
            theme,
        );
        let keyword_color: gpui::Hsla = rgb(theme.keyword).into();
        let function_color: gpui::Hsla = rgb(theme.function).into();

        assert_eq!(styles.lines[1].code_language.as_deref(), Some(language));
        assert!(runs.iter().any(|run| run.color == keyword_color));
        assert!(runs.iter().any(|run| run.color == function_color));
    }
}

#[test]
fn swatch_glyphs_switch_to_a_readable_ink_on_every_theme() {
    let text = "light #ffffff dark #000000";
    let style = line_style(text, &mut CodeContext::default());
    let spans = semantic_spans(Path::new("a.org"), text, &style);
    for theme in [
        &crate::theme::ORG_STUDIO_LIGHT,
        &crate::theme::ORG_STUDIO_DARK,
    ] {
        let mut runs = runs_from_spans(&spans, base_run(text.len()), &style, None, theme);
        apply_swatch_text(&mut runs, &spans, theme);
        assert_eq!(
            run_at(&runs, text.find("#ffffff").unwrap()).color,
            rgb(0x000000).into(),
            "a white swatch needs dark ink"
        );
        assert_eq!(
            run_at(&runs, text.find("#000000").unwrap()).color,
            rgb(0xffffff).into(),
            "a black swatch needs light ink"
        );
        // Text outside the literals keeps the base color.
        assert_eq!(run_at(&runs, 0).color, rgb(0).into());
    }
}

#[test]
fn supported_code_language_produces_neutral_highlight_tokens() {
    let text = "fn main() { let message = \"你好😀\"; } // cafe\u{301}";
    let style = EditorLineStyle {
        source_range: ByteRange::new(0, text.len() as u64),
        id: EditorStyleId::Code,
        code_language: Some(Arc::from("rust")),
        block: None,
        todo: None,
        metrics: metrics_for(EditorStyleId::Code),
    };
    let spans = semantic_spans(Path::new("code.org"), text, &style);

    for token in [
        EditorColorToken::Keyword,
        EditorColorToken::Function,
        EditorColorToken::String,
        EditorColorToken::Comment,
    ] {
        assert!(
            spans.iter().any(|span| span.color == Some(token)),
            "missing {token:?}"
        );
    }
    assert!(spans.iter().all(|span| {
        text.is_char_boundary(span.bytes.start) && text.is_char_boundary(span.bytes.end)
    }));
}

#[test]
fn unsupported_code_languages_keep_only_the_code_base_style() {
    for language in ["unknown", "plantuml"] {
        let text = "#show: 中文😀\nAlice -> Bob";
        let style = EditorLineStyle {
            source_range: ByteRange::new(0, text.len() as u64),
            id: EditorStyleId::Code,
            code_language: Some(Arc::from(language)),
            block: None,
            todo: None,
            metrics: metrics_for(EditorStyleId::Code),
        };

        assert!(semantic_spans(Path::new("code.org"), text, &style).is_empty());
    }
}

#[test]
fn long_code_line_highlights_without_invalid_or_truncated_semantic_ranges() {
    let text = format!("let payload = \"{}😀\"; // 尾部", "x".repeat(16_384));
    let style = EditorLineStyle {
        source_range: ByteRange::new(0, u64::try_from(text.len()).expect("line length fits u64")),
        id: EditorStyleId::Code,
        code_language: Some(Arc::from("rust")),
        block: None,
        todo: None,
        metrics: metrics_for(EditorStyleId::Code),
    };
    let spans = semantic_spans(Path::new("long.org"), &text, &style);

    assert!(
        spans
            .iter()
            .any(|span| span.color == Some(EditorColorToken::String))
    );
    assert!(
        spans
            .iter()
            .any(|span| span.color == Some(EditorColorToken::Comment))
    );
    assert!(spans.iter().all(|span| {
        span.bytes.end <= text.len()
            && text.is_char_boundary(span.bytes.start)
            && text.is_char_boundary(span.bytes.end)
    }));
}

#[test]
fn cpp_source_uses_the_official_feature_illustration_faces() {
    let snapshot = DocumentSnapshot::from_utf8(
        b"#+begin_src cpp\n#include <iostream>\nint main() { std::cout << \"You\"; return 0; }\n#+end_src\n".to_vec(),
    )
    .unwrap();
    let styles = EditorStyleSnapshot::for_lines(
        Path::new("a.org"),
        &snapshot,
        0..4,
        &EditorSyntaxService::default(),
    );
    let theme = current_theme();

    let boundary_text = snapshot.copy_range(styles.lines[0].source_range);
    let boundary_runs = runs(
        Path::new("a.org"),
        &boundary_text,
        base_run(boundary_text.len()),
        &styles.lines[0],
        None,
        theme,
    );
    assert_eq!(boundary_runs[0].font.style, FontStyle::Normal);
    assert_eq!(boundary_runs[0].color, rgb(theme.meta).into());

    let include_text = snapshot.copy_range(styles.lines[1].source_range);
    let include_runs = runs(
        Path::new("a.org"),
        &include_text,
        base_run(include_text.len()),
        &styles.lines[1],
        None,
        theme,
    );
    assert_eq!(
        run_at(&include_runs, include_text.find("#include").unwrap()).color,
        rgb(theme.comment).into()
    );
    assert_eq!(
        run_at(&include_runs, include_text.find("<iostream>").unwrap()).color,
        rgb(theme.string).into()
    );

    let body_text = snapshot.copy_range(styles.lines[2].source_range);
    let body_runs = runs(
        Path::new("a.org"),
        &body_text,
        base_run(body_text.len()),
        &styles.lines[2],
        None,
        theme,
    );
    for (token, color) in [
        ("int", theme.type_name),
        ("main", theme.function),
        ("std", theme.variable),
        ("cout", theme.foreground),
        ("\"You\"", theme.string),
        ("return", theme.keyword),
        ("0", theme.number),
    ] {
        assert_eq!(
            run_at(&body_runs, body_text.find(token).unwrap()).color,
            rgb(color).into(),
            "unexpected face for {token}"
        );
    }
}
