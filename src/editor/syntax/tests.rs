use super::*;
use crate::{
    document::DocumentSnapshot,
    theme::{Theme, current_theme},
};
use gpui::{FontStyle, FontWeight, TextRun, rgb};

mod cache;
mod classification;
mod faces;
mod markup;

fn runs(
    path: &Path,
    text: &str,
    base: TextRun,
    line_style: &EditorLineStyle,
    marked: Option<Range<usize>>,
    theme: &Theme,
) -> Vec<TextRun> {
    let spans = semantic_spans(path, text, line_style);
    runs_from_spans(&spans, base, line_style, marked, theme)
}

fn base_run(len: usize) -> TextRun {
    TextRun {
        len,
        font: Default::default(),
        color: rgb(0).into(),
        background_color: None,
        underline: None,
        strikethrough: None,
    }
}

fn run_at(runs: &[TextRun], offset: usize) -> &TextRun {
    let mut cursor = 0;
    runs.iter()
        .find(|run| {
            let contains = cursor <= offset && offset < cursor + run.len;
            cursor += run.len;
            contains
        })
        .expect("offset belongs to a paint run")
}

fn line_style(text: &str, context: &mut CodeContext) -> EditorLineStyle {
    let todo = heading_todo_span(Language::Org, text, context);
    let block_before = context.block_kind.clone();
    let block_body_line_before = context.block_body_line;
    let id = classify_line(Language::Org, text, context);
    EditorLineStyle {
        source_range: ByteRange::new(0, text.len() as u64),
        id,
        code_language: None,
        block: block_decoration(
            id,
            block_before,
            context.block_kind.clone(),
            block_body_line_before,
            context.block_body_line,
        ),
        todo,
        metrics: metrics_for(id),
    }
}
