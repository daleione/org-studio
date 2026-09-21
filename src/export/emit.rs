use std::fmt::Write;

use super::{
    ExportDiagnostic, ExportOptions, ExportTemplate, LayoutMode,
    model::{ExportBlock, ExportDocument, ExportInline},
};

pub(super) fn emit(
    document: &ExportDocument,
    template: &ExportTemplate,
    options: &ExportOptions,
    _diagnostics: &mut Vec<ExportDiagnostic>,
) -> String {
    let mut output = String::with_capacity(template.source.len() + document.blocks.len() * 80);
    let paged = matches!(options.layout, LayoutMode::Paged);
    let _ = writeln!(output, "#let divider() = line(length: 100%)");
    let _ = writeln!(output, "#let md-toc() = none");
    // Inline code must go through `raw` so each theme's own
    // `show raw.where(block: false)` rules style it. The old stub painted a
    // fixed `luma(245)` chip and left the text colour inherited, which made
    // inline code invisible on every dark theme.
    let _ = writeln!(
        output,
        "#let inline-code(body) = raw(body.text, block: false)"
    );
    let _ = writeln!(
        output,
        "#let signature-row(author: \"\", date: \"\") = none"
    );
    output.push_str(template.source);
    let _ = writeln!(output, "\n#show: conf\n");

    if let Some(title) = &document.meta.title {
        let _ = writeln!(output, "#poster-title(text({}))\n", string_literal(title));
    }
    if options.toc.or(document.meta.toc).unwrap_or(false) {
        output.push_str("#md-toc()\n\n");
    }
    let heading_offset = u8::from(document.meta.title.is_some());
    for (index, block) in document.blocks.iter().enumerate() {
        if index == 0 && heading_repeats_title(block, document.meta.title.as_deref()) {
            continue;
        }
        emit_block(&mut output, block, heading_offset);
        output.push_str("\n\n");
    }
    if options.byline.unwrap_or(true)
        && (document.meta.author.is_some() || document.meta.date.is_some())
    {
        let _ = writeln!(
            output,
            "#signature-row(author: {}, date: {})",
            string_literal(document.meta.author.as_deref().unwrap_or("")),
            string_literal(document.meta.date.as_deref().unwrap_or(""))
        );
    }
    // Themes consume this through sys.inputs in the final implementation.
    // Keep the value in source for the built-in bootstrap template as well.
    if !paged {
        output.insert_str(0, "#set page(height: auto)\n");
    }
    output
}

fn heading_repeats_title(block: &ExportBlock, title: Option<&str>) -> bool {
    let (
        Some(title),
        ExportBlock::Heading {
            level: 1, content, ..
        },
    ) = (title, block)
    else {
        return false;
    };
    let mut heading = String::new();
    plain_inlines(content, &mut heading);
    heading.trim().eq_ignore_ascii_case(title.trim())
}

fn plain_inlines(inlines: &[ExportInline], output: &mut String) {
    for inline in inlines {
        match inline {
            ExportInline::Text(text) | ExportInline::Code(text) => output.push_str(text),
            ExportInline::Strong(children)
            | ExportInline::Emphasis(children)
            | ExportInline::Underline(children)
            | ExportInline::Strike(children) => plain_inlines(children, output),
            ExportInline::Link { label, .. } => plain_inlines(label, output),
            ExportInline::LineBreak => output.push(' '),
        }
    }
}

fn emit_block(output: &mut String, block: &ExportBlock, heading_offset: u8) {
    match block {
        ExportBlock::Heading { level, content, .. } => {
            let level = level.saturating_add(heading_offset).min(6);
            let _ = write!(output, "#heading(level: {level})[");
            emit_inlines(output, content);
            output.push(']');
        }
        ExportBlock::Paragraph { content, .. } => emit_inlines(output, content),
        ExportBlock::List { ordered, items, .. } => {
            let _ = write!(output, "#{}(", if *ordered { "enum" } else { "list" });
            for item in items {
                output.push('[');
                for block in item {
                    emit_block(output, block, heading_offset);
                }
                output.push_str("],");
            }
            output.push(')');
        }
        ExportBlock::Quote { blocks, .. } => {
            output.push_str("#quote(block: true)[");
            for block in blocks {
                emit_block(output, block, heading_offset);
                output.push_str("\n\n");
            }
            output.push(']');
        }
        ExportBlock::Code { language, code, .. } => {
            let _ = write!(output, "#raw({}", string_literal(code));
            if let Some(language) = language {
                let _ = write!(output, ", lang: {}", string_literal(language));
            }
            output.push_str(", block: true)");
        }
        ExportBlock::Table { rows, .. } => {
            let columns = rows.first().map(Vec::len).unwrap_or(1).max(1);
            let _ = write!(output, "#table(columns: (1fr,) * {columns},");
            for row in rows {
                for cell in row {
                    output.push('[');
                    emit_inlines(output, cell);
                    output.push_str("],");
                }
            }
            output.push(')');
        }
        ExportBlock::Image {
            path, alt, caption, ..
        } => {
            let path = path.to_string_lossy();
            let _ = write!(output, "#figure(image({})", string_literal(&path));
            if let Some(caption) = caption.as_ref().or(alt.as_ref()) {
                let _ = write!(output, ", caption: text({})", string_literal(caption));
            }
            output.push(')');
        }
        ExportBlock::Rule { .. } => output.push_str("#divider()"),
    }
}

fn emit_inlines(output: &mut String, inlines: &[ExportInline]) {
    for inline in inlines {
        match inline {
            ExportInline::Text(text) => {
                let _ = write!(output, "#text({})", string_literal(text));
            }
            ExportInline::Strong(children) => {
                output.push_str("#strong[");
                emit_inlines(output, children);
                output.push(']');
            }
            ExportInline::Emphasis(children) => {
                output.push_str("#emph[");
                emit_inlines(output, children);
                output.push(']');
            }
            ExportInline::Underline(children) => {
                output.push_str("#underline[");
                emit_inlines(output, children);
                output.push(']');
            }
            ExportInline::Strike(children) => {
                output.push_str("#strike[");
                emit_inlines(output, children);
                output.push(']');
            }
            ExportInline::Code(text) => {
                let _ = write!(output, "#inline-code(text({}))", string_literal(text));
            }
            ExportInline::Link { target, label } => {
                let _ = write!(output, "#link({})[", string_literal(target));
                emit_inlines(output, label);
                output.push(']');
            }
            ExportInline::LineBreak => output.push_str("#linebreak()"),
        }
    }
}

fn string_literal(text: &str) -> String {
    let mut output = String::with_capacity(text.len() + 2);
    output.push('"');
    for character in text.chars() {
        match character {
            '\\' => output.push_str("\\\\"),
            '"' => output.push_str("\\\""),
            '\n' => output.push_str("\\n"),
            '\r' => output.push_str("\\r"),
            '\t' => output.push_str("\\t"),
            character => output.push(character),
        }
    }
    output.push('"');
    output
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::export::{ExportMeta, PaperSize, export_templates};

    #[test]
    fn escapes_typst_string_literals() {
        assert_eq!(string_literal("a\"b\\c\n"), "\"a\\\"b\\\\c\\n\"");
    }

    #[test]
    fn metadata_title_owns_level_one_heading() {
        let document = ExportDocument {
            meta: ExportMeta {
                title: Some("Document title".into()),
                ..ExportMeta::default()
            },
            blocks: vec![ExportBlock::Heading {
                level: 1,
                content: vec![ExportInline::Text("First section".into())],
                source: 0..1,
            }],
        };
        let mut diagnostics = Vec::new();
        let source = emit(
            &document,
            export_templates().first().unwrap(),
            &ExportOptions {
                paper: PaperSize::A4,
                ..ExportOptions::default()
            },
            &mut diagnostics,
        );
        assert!(source.contains("#poster-title(text(\"Document title\"))"));
        assert!(source.contains("#heading(level: 2)"));
        assert!(!source.contains("#heading(level: 1)"));
    }

    #[test]
    fn repeated_first_heading_is_not_exported_twice() {
        let document = ExportDocument {
            meta: ExportMeta {
                title: Some("Org Mode".into()),
                ..ExportMeta::default()
            },
            blocks: vec![ExportBlock::Heading {
                level: 1,
                content: vec![ExportInline::Text("Org Mode".into())],
                source: 0..1,
            }],
        };
        let mut diagnostics = Vec::new();
        let source = emit(
            &document,
            export_templates().first().unwrap(),
            &ExportOptions::default(),
            &mut diagnostics,
        );
        assert_eq!(source.matches("Org Mode").count(), 1);
    }
}
