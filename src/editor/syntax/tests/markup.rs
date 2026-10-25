use super::*;

#[test]
fn inline_delimiters_follow_only_the_active_code_span() {
    let theme = &crate::theme::ORG_STUDIO_LIGHT;
    for (path, literal, width) in [
        ("notes.org", "=示例=", 1),
        ("notes.org", "~snippet~", 1),
        ("notes.md", "`sample`", 1),
        ("notes.md", "`` a`b ``", 2),
    ] {
        let text = format!("{literal} {literal}");
        let style = line_style(&text, &mut CodeContext::default());
        let mut spans = semantic_spans(Path::new(path), &text, &style);
        let plain = runs_from_spans(&spans, base_run(text.len()), &style, None, theme);
        assert_eq!(emphasize_inline_delimiters(&mut spans, None), None);
        assert_eq!(
            emphasize_inline_delimiters(&mut spans, Some(text.len())),
            None
        );
        assert_eq!(
            emphasize_inline_delimiters(&mut spans, Some(width)),
            Some(0..literal.len())
        );
        let active = runs_from_spans(&spans, base_run(text.len()), &style, None, theme);
        assert_eq!(active.iter().map(|run| run.len).sum::<usize>(), text.len());
        let muted = rgb(EditorColorToken::InlineDelimiter.resolve(theme)).into();
        for offset in (0..width).chain(literal.len() - width..literal.len()) {
            assert_eq!(run_at(&plain, offset).color, muted);
            assert_eq!(run_at(&active, offset).color, run_at(&plain, width).color);
            assert_eq!(run_at(&active, literal.len() + 1 + offset).color, muted);
        }
    }
}

#[test]
fn editor_uses_parser_spans_for_org_emphasis() {
    let text = "*=render_document= returns formatted output.* \
                Then call =load_config= to read settings.";
    let style = line_style(text, &mut CodeContext::default());
    let theme = current_theme();
    let markup_runs = runs(
        Path::new("a.org"),
        text,
        base_run(text.len()),
        &style,
        None,
        theme,
    );
    for (offset, _) in text.match_indices('_') {
        assert!(
            run_at(&markup_runs, offset).underline.is_none(),
            "identifier underscore at byte {offset} was treated as Org underline"
        );
    }
    assert_eq!(
        run_at(&markup_runs, text.find("render_document").unwrap())
            .font
            .weight,
        FontWeight::BOLD
    );
    assert_eq!(
        run_at(&markup_runs, text.find("load_config").unwrap()).color,
        rgb(theme.verbatim).into()
    );

    let underline_text = "_real underline_";
    let underline_style = line_style(underline_text, &mut CodeContext::default());
    let underline_runs = runs(
        Path::new("a.org"),
        underline_text,
        base_run(underline_text.len()),
        &underline_style,
        None,
        theme,
    );
    assert!(
        run_at(&underline_runs, underline_text.find("real").unwrap())
            .underline
            .is_some()
    );
}

#[test]
fn table_inline_syntax_cannot_cross_cell_boundaries() {
    for (path, text) in [
        ("result.org", "| /left | right/ | *left | right* |"),
        ("result.md", "| *left | right* |"),
    ] {
        let mut style = line_style(text, &mut CodeContext::default());
        style.id = EditorStyleId::Table;
        let spans = semantic_spans(Path::new(path), text, &style);
        assert!(spans.iter().all(|span| !span.italic && !span.underline));
        assert!(
            spans
                .iter()
                .all(|span| span.weight == EditorSemanticWeight::Inherit)
        );
        assert!(
            spans
                .iter()
                .all(|span| span.color != Some(EditorColorToken::Verbatim))
        );
    }
}

#[test]
fn org_tokens_inside_literal_spans_keep_the_literal_face() {
    let text = "=SCHEDULED: [X] :tag:= outside [X]";
    let style = line_style(text, &mut CodeContext::default());
    let theme = current_theme();
    let markup_runs = runs(
        Path::new("a.org"),
        text,
        base_run(text.len()),
        &style,
        None,
        theme,
    );
    assert_eq!(
        run_at(&markup_runs, text.find("SCHEDULED:").unwrap()).color,
        rgb(theme.verbatim).into()
    );
    assert_eq!(
        run_at(&markup_runs, text.find("[X]").unwrap()).color,
        rgb(theme.verbatim).into()
    );
    assert_eq!(
        run_at(&markup_runs, text.rfind("[X]").unwrap()).color,
        rgb(theme.done).into()
    );
}

#[test]
fn org_links_classify_by_type_keeping_the_raw_source() {
    let text = "[[https://x.org][web]] [[file:notes.org][notes]] [[#cid][jump]] [[mailto:a@b.c][mail]] [[shell:ls][danger]] [[irc:chan][chat]]";
    let style = line_style(text, &mut CodeContext::default());
    let spans = semantic_spans(Path::new("a.org"), text, &style);
    let links: Vec<(EditorColorToken, &str)> = spans
        .iter()
        .filter_map(|span| {
            span.link
                .as_ref()
                .map(|meta| (span.color.expect("links are colored"), meta.raw.as_ref()))
        })
        .collect();
    assert_eq!(
        links,
        vec![
            (EditorColorToken::LinkExternal, "https://x.org"),
            (EditorColorToken::LinkFile, "file:notes.org"),
            (EditorColorToken::LinkInternal, "#cid"),
            (EditorColorToken::LinkMail, "mailto:a@b.c"),
            (EditorColorToken::LinkWarn, "shell:ls"),
            (EditorColorToken::LinkOther, "irc:chan"),
        ]
    );
    // Raw source bytes are all preserved by the paint runs.
    let theme = current_theme();
    let runs = runs(
        Path::new("a.org"),
        text,
        base_run(text.len()),
        &style,
        None,
        theme,
    );
    assert_eq!(runs.iter().map(|run| run.len).sum::<usize>(), text.len());
}

#[test]
fn bare_urls_are_scanned_as_external_links() {
    let text = "visit https://orgmode.org now and www.example.com/a!";
    let style = line_style(text, &mut CodeContext::default());
    let spans = semantic_spans(Path::new("a.org"), text, &style);
    let external: Vec<&str> = spans
        .iter()
        .filter(|span| {
            matches!(
                span.link,
                Some(crate::links::LinkInfo {
                    kind: crate::links::LinkKind::External,
                    ..
                })
            )
        })
        .map(|span| &text[span.bytes.clone()])
        .collect();
    assert_eq!(external, vec!["https://orgmode.org", "www.example.com/a"]);
}

#[test]
fn org_tag_ranges_split_clusters_into_individual_tags_with_colons() {
    // Adjacent tags share the separator colon, so the ranges overlap on it.
    assert_eq!(org_tag_ranges("* H :alpha:beta:"), vec![4..11, 10..16]);
    assert_eq!(
        org_tag_ranges("** TODO Ship it :ui:mac:"),
        vec![16..20, 19..24]
    );
    assert_eq!(org_tag_ranges("body text"), Vec::<Range<usize>>::new());
    assert_eq!(org_tag_ranges("* H :alpha"), Vec::<Range<usize>>::new());
    assert_eq!(org_tag_ranges("* H alpha:"), Vec::<Range<usize>>::new());
    assert_eq!(org_tag_ranges("* H :"), Vec::<Range<usize>>::new());
    assert_eq!(org_tag_ranges("* H :a@b#c_d:"), vec![4..13]);
    for (text, ranges) in [
        ("* H :alpha:beta:", org_tag_ranges("* H :alpha:beta:")),
        (
            "** TODO Ship it :ui:mac:",
            org_tag_ranges("** TODO Ship it :ui:mac:"),
        ),
    ] {
        for range in &ranges {
            assert!(
                text.is_char_boundary(range.start) && text.is_char_boundary(range.end),
                "{range:?} is not on char boundaries in {text:?}"
            );
        }
    }
}

#[test]
fn semantic_spans_mark_each_org_tag_as_a_pill() {
    let text = "** TODO Ship it :ui:mac:";
    let style = line_style(text, &mut CodeContext::default());
    let spans = semantic_spans(Path::new("a.org"), text, &style);
    let pills: Vec<&str> = spans
        .iter()
        .filter(|span| span.pill)
        .map(|span| &text[span.bytes.clone()])
        .collect();
    assert_eq!(pills, vec![":ui:", ":mac:"]);
    for span in spans.iter().filter(|span| span.pill) {
        assert_eq!(span.color, Some(EditorColorToken::Attribute));
        assert!(
            text.is_char_boundary(span.bytes.start) && text.is_char_boundary(span.bytes.end),
            "pill range is not on char boundaries: {:?}",
            span.bytes
        );
    }
}

#[test]
fn hex_literals_get_a_swatch_span_without_recoloring_the_run() {
    let theme = &crate::theme::ORG_STUDIO_LIGHT;
    let text = "palette: #ff0000 and #11223344 done";
    let style = line_style(text, &mut CodeContext::default());
    let spans = semantic_spans(Path::new("a.org"), text, &style);
    let swatches: Vec<(Range<usize>, u32)> = spans
        .iter()
        .filter_map(|span| span.swatch.map(|rgba| (span.bytes.clone(), rgba)))
        .collect();
    assert_eq!(swatches, vec![(9..16, 0xff0000ff), (21..30, 0x11223344)]);

    // The swatch is geometry only: it must not tint or underline the run.
    let runs = runs_from_spans(&spans, base_run(text.len()), &style, None, theme);
    let run = run_at(&runs, text.find("#ff0000").unwrap());
    assert_eq!(run.color, rgb(0).into());
    assert!(run.background_color.is_none());
}

#[test]
fn hex_literals_are_swatched_inside_code_verbatim_and_source_blocks() {
    // Verbatim/code markup deliberately hides nothing, so a literal inside
    // it still gets a swatch.
    let text = "=#ff0000= and ~#00ff00~";
    let style = line_style(text, &mut CodeContext::default());
    let spans = semantic_spans(Path::new("a.org"), text, &style);
    let mut ranges: Vec<Range<usize>> = spans
        .iter()
        .filter_map(|span| span.swatch.map(|_| span.bytes.clone()))
        .collect();
    ranges.sort_by_key(|range| range.start);
    assert_eq!(ranges, vec![1..8, 15..22]);

    let text = "let fill = #ff0000;";
    let style = EditorLineStyle {
        source_range: ByteRange::new(0, text.len() as u64),
        id: EditorStyleId::Code,
        code_language: Some(Arc::from("rust")),
        block: None,
        todo: None,
        metrics: metrics_for(Language::Org, EditorStyleId::Code),
    };
    let spans = semantic_spans(Path::new("code.org"), text, &style);
    assert!(spans.iter().any(|span| span.swatch == Some(0xff0000ff)));
}

#[test]
fn hashed_org_tags_and_keywords_do_not_gain_a_swatch() {
    let text = "* H :#ff0000:";
    let style = line_style(text, &mut CodeContext::default());
    let spans = semantic_spans(Path::new("a.org"), text, &style);
    assert!(spans.iter().any(|span| span.pill));
    assert!(spans.iter().all(|span| span.swatch.is_none()));

    for text in ["#+begin_src rust", "#+attr_html: :style bg"] {
        let style = line_style(text, &mut CodeContext::default());
        assert!(
            semantic_spans(Path::new("a.org"), text, &style)
                .iter()
                .all(|span| span.swatch.is_none()),
            "{text}"
        );
    }
}

#[test]
fn org_tags_inside_inline_code_are_not_pilled() {
    let text = "see =a:b:= then :real:";
    let style = line_style(text, &mut CodeContext::default());
    let spans = semantic_spans(Path::new("a.org"), text, &style);
    let pills: Vec<&str> = spans
        .iter()
        .filter(|span| span.pill)
        .map(|span| &text[span.bytes.clone()])
        .collect();
    assert_eq!(pills, vec![":real:"]);
}

#[test]
fn semantic_spans_are_shared_tokens_with_valid_complex_utf8_ranges() {
    let text = "前缀 [[https://例子.invalid][链接😀]] cafe\u{301} שלום *粗体*";
    let style = EditorLineStyle {
        source_range: ByteRange::new(0, text.len() as u64),
        id: EditorStyleId::Plain,
        code_language: None,
        block: None,
        todo: None,
        metrics: metrics_for(Language::Org, EditorStyleId::Plain),
    };
    let spans = semantic_spans(Path::new("unicode.org"), text, &style);

    assert!(!spans.is_empty());
    assert!(spans.iter().any(|span| {
        matches!(
            span.color,
            Some(EditorColorToken::LinkExternal | EditorColorToken::LinkInternal)
        )
    }));
    assert!(
        spans
            .iter()
            .filter(|span| span.link.is_some())
            .all(|span| { span.link.as_ref().is_some_and(|meta| !meta.raw.is_empty()) })
    );
    assert!(spans.iter().all(|span| {
        span.bytes.start < span.bytes.end
            && span.bytes.end <= text.len()
            && text.is_char_boundary(span.bytes.start)
            && text.is_char_boundary(span.bytes.end)
    }));
}
