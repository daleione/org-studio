use super::*;

fn table_delimiter_columns(text: &str, format: DocumentFormat) -> Vec<usize> {
    let geometry = TableRowGeometry::from_source(text, format).unwrap();
    table_fragments(text, &geometry)
        .into_iter()
        .filter_map(|(range, column)| {
            matches!(text.as_bytes().get(range.start), Some(b'|' | b'+')).then_some(column)
        })
        .collect()
}

#[test]
fn table_fragments_align_mixed_width_rows_in_logical_columns() {
    assert_eq!(
        table_delimiter_columns("| English | 中文    |", DocumentFormat::Org),
        vec![0, 10, 20]
    );
    assert_eq!(
        table_delimiter_columns("| 中文    | English |", DocumentFormat::Org),
        vec![0, 10, 20]
    );
    assert_eq!(
        table_delimiter_columns("|a| bbbb |", DocumentFormat::Org),
        vec![0, 2, 9],
        "an unaligned source row must keep its original logical delimiter positions"
    );
}

fn semantic_span(byte: usize) -> crate::editor::syntax::EditorSemanticSpan {
    crate::editor::syntax::EditorSemanticSpan {
        bytes: byte..byte + 1,
        color: Some(crate::editor::syntax::EditorColorToken::Keyword),
        weight: Some(FontWeight::BOLD),
        ..Default::default()
    }
}

#[test]
fn rich_span_budget_degrades_the_whole_row_after_the_row_limit() {
    let text = "a".repeat(MAX_RICH_SPANS_PER_ROW + 1);
    let spans = (0..text.len()).map(semantic_span).collect();
    let mut budget = RichSpanBudget::default();
    let adapted = budget.adapt(&text, spans, crate::theme::current_theme());

    assert!(adapted.is_empty());
    assert_eq!(budget.degraded_rows(), 1);
}

#[test]
fn rich_span_budget_degrades_rows_after_the_request_limit() {
    let mut budget = RichSpanBudget::default();
    let theme = crate::theme::current_theme();
    for _ in 0..MAX_RICH_SPANS_PER_REQUEST {
        assert_eq!(budget.adapt("a", vec![semantic_span(0)], theme).len(), 1);
    }
    assert!(budget.adapt("a", vec![semantic_span(0)], theme).is_empty());
}

#[test]
fn hidden_spans_keep_transparency_and_text_when_styles_are_degraded() {
    use crate::editor::syntax::{EditorColorToken, EditorSemanticSpan};
    let source = "*** Heading";
    for remaining in [0, MAX_RICH_SPANS_PER_REQUEST] {
        for invalid in [false, true] {
            let mut budget = RichSpanBudget {
                remaining,
                degraded_rows: 0,
            };
            let spans = budget.adapt(
                source,
                vec![
                    EditorSemanticSpan {
                        bytes: 0..2,
                        color: Some(EditorColorToken::Hidden),
                        ..Default::default()
                    },
                    EditorSemanticSpan {
                        bytes: 4..source.len() + usize::from(invalid),
                        color: Some(EditorColorToken::Foreground),
                        ..Default::default()
                    },
                ],
                crate::theme::current_theme(),
            );
            let segments = rich_text_segments(source, &spans, &cosmic_text::Attrs::new(), None);
            assert_eq!(
                segments.iter().map(|(text, _)| *text).collect::<String>(),
                source
            );
            assert_eq!(segments[0].0, "**");
            assert_eq!(
                segments[0].1.color_opt,
                Some(cosmic_text::Color::rgba(0, 0, 0, 0))
            );
            assert_eq!(budget.degraded_rows(), u64::from(remaining == 0 || invalid));
            let mut pixels = [255; 4];
            crate::minimap::paint_text_pixels(
                &mut pixels,
                1,
                1,
                0,
                0,
                1,
                1,
                segments[0].1.color_opt.unwrap(),
            );
            assert_eq!(pixels[3], 0);
        }
    }
}

#[test]
fn rich_text_segments_preserve_color_emphasis_and_utf8_boundaries() {
    let text = "甲😀b";
    let spans = [
        TextSpan {
            bytes: 3..7,
            color: Some(cosmic_text::Color::rgb(0x12, 0x34, 0x56)),
            weight: Some(FontWeight::BOLD),
            italic: true,
            underline: true,
            strikethrough: true,
        },
        TextSpan {
            bytes: 7..8,
            color: None,
            weight: Some(FontWeight::NORMAL),
            italic: false,
            underline: false,
            strikethrough: false,
        },
    ];
    let base = cosmic_text::Attrs::new()
        .family(cosmic_text::Family::Monospace)
        .weight(cosmic_text::Weight::BOLD);
    let segments = rich_text_segments(text, &spans, &base, None);

    assert_eq!(
        segments.iter().map(|(text, _)| *text).collect::<String>(),
        text
    );
    assert_eq!(segments.len(), 3);
    assert_eq!(segments[0].1.weight, cosmic_text::Weight::BOLD);
    assert_eq!(segments[2].1.weight, cosmic_text::Weight::NORMAL);
    assert_eq!(segments[1].0, "😀");
    assert_eq!(
        segments[1].1.color_opt,
        Some(cosmic_text::Color::rgb(0x12, 0x34, 0x56))
    );
    assert_eq!(segments[1].1.weight, cosmic_text::Weight::BOLD);
    assert_eq!(segments[1].1.style, cosmic_text::Style::Italic);
    assert!(segments[1].1.text_decoration.strikethrough);
    assert_eq!(
        segments[1].1.text_decoration.underline,
        cosmic_text::UnderlineStyle::Single
    );
}
