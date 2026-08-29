mod render;
mod world;

use std::{collections::BTreeMap, path::Path};

use typst_layout::PagedDocument;

use super::{ExportDiagnostic, ExportError, ExportFormat, ExportSeverity};
use world::{OrgStudioWorld, SharedResources};

#[derive(Debug)]
pub struct CompileOutput {
    pub pages: Vec<Vec<u8>>,
    pub diagnostics: Vec<ExportDiagnostic>,
}

pub struct TypstEngine {
    shared: SharedResources,
}

impl Default for TypstEngine {
    fn default() -> Self {
        Self::new(&[])
    }
}

impl TypstEngine {
    pub fn new(custom_fonts: &[Vec<u8>]) -> Self {
        Self {
            shared: SharedResources::new(custom_fonts),
        }
    }

    pub fn font_families(&self) -> Vec<String> {
        self.shared.family_names()
    }

    pub fn compile(
        &self,
        source: String,
        format: ExportFormat,
        per_page: bool,
        png_ppi: f32,
        root: Option<&Path>,
    ) -> Result<CompileOutput, ExportError> {
        self.compile_with_inputs(source, format, per_page, png_ppi, root, &BTreeMap::new())
    }

    pub(super) fn compile_with_inputs(
        &self,
        source: String,
        format: ExportFormat,
        per_page: bool,
        png_ppi: f32,
        root: Option<&Path>,
        inputs: &BTreeMap<String, String>,
    ) -> Result<CompileOutput, ExportError> {
        if per_page && format == ExportFormat::Pdf {
            return Err(ExportError::Render(
                "per-page output is only supported for PNG and SVG".into(),
            ));
        }
        let world = OrgStudioWorld::new(&self.shared, source, root, inputs);
        let result = (|| {
            let compiled = ::typst::compile::<PagedDocument>(&world);
            let warnings = diagnostics(&compiled.warnings, ExportSeverity::Warning);
            let document = compiled.output.map_err(|errors| {
                let mut all = diagnostics(&errors, ExportSeverity::Error);
                all.extend(warnings.clone());
                ExportError::Compile(all)
            })?;
            validate_requested_paper(&document, inputs)?;
            let pages = render::render(&document, format, per_page, png_ppi)?;
            Ok(CompileOutput {
                pages,
                diagnostics: warnings,
            })
        })();
        ::typst::comemo::evict(0);
        result
    }
}

fn validate_requested_paper(
    document: &PagedDocument,
    inputs: &BTreeMap<String, String>,
) -> Result<(), ExportError> {
    let Some((paper, expected_width, expected_height)) =
        inputs.get("paper").and_then(|paper| match paper.as_str() {
            "a4" => Some(("A4", 595.276, 841.890)),
            "a5" => Some(("A5", 419.528, 595.276)),
            "iso-b5" => Some(("B5", 498.898, 708.661)),
            _ => None,
        })
    else {
        return Ok(());
    };
    let landscape = inputs
        .get("orientation")
        .is_some_and(|orientation| orientation == "landscape");
    let (expected_width, expected_height) = if landscape {
        (expected_height, expected_width)
    } else {
        (expected_width, expected_height)
    };
    for (index, page) in document.pages().iter().enumerate() {
        let size = page.frame.size();
        if (size.x.to_pt() - expected_width).abs() > 0.2
            || (size.y.to_pt() - expected_height).abs() > 0.2
        {
            return Err(ExportError::Render(format!(
                "page {} does not use the selected {paper} paper size (actual size: {:.1} x {:.1} pt)",
                index + 1,
                size.x.to_pt(),
                size.y.to_pt()
            )));
        }
    }
    Ok(())
}

fn diagnostics(
    source: &[::typst::diag::SourceDiagnostic],
    severity: ExportSeverity,
) -> Vec<ExportDiagnostic> {
    let mut seen = std::collections::BTreeSet::new();
    source
        .iter()
        .filter(|diagnostic| {
            severity == ExportSeverity::Error || seen.insert(diagnostic.message.to_string())
        })
        .map(|diagnostic| ExportDiagnostic {
            severity,
            code: if severity == ExportSeverity::Error {
                "typst-error"
            } else {
                "typst-warning"
            },
            message: diagnostic.message.to_string(),
            source: None,
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::export::{ExportOptions, PaperSize, emit, markdown, template_inputs, themes};
    use typst::layout::{Frame, FrameItem};

    fn frame_text(frame: &Frame, output: &mut String) {
        for (_, item) in frame.items() {
            match item {
                FrameItem::Text(text) => output.push_str(&text.text),
                FrameItem::Group(group) => frame_text(&group.frame, output),
                _ => {}
            }
        }
    }

    #[test]
    fn compiles_pdf_png_and_svg() {
        let engine = TypstEngine::default();
        let pdf = engine
            .compile("Hello".into(), ExportFormat::Pdf, false, 144.0, None)
            .unwrap();
        assert!(pdf.pages[0].starts_with(b"%PDF-"));

        let png = engine
            .compile("Hello".into(), ExportFormat::Png, false, 96.0, None)
            .unwrap();
        assert!(png.pages[0].starts_with(&[0x89, b'P', b'N', b'G']));

        let svg = engine
            .compile("Hello".into(), ExportFormat::Svg, false, 144.0, None)
            .unwrap();
        assert!(String::from_utf8_lossy(&svg.pages[0]).contains("<svg"));
    }

    #[test]
    fn source_cannot_read_outside_the_document_root() {
        let base =
            std::env::temp_dir().join(format!("org-studio-world-test-{}", std::process::id()));
        let root = base.join("document");
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(base.join("secret.txt"), "secret").unwrap();
        let error = TypstEngine::default()
            .compile(
                "#read(\"../secret.txt\")".into(),
                ExportFormat::Pdf,
                false,
                144.0,
                Some(&root),
            )
            .unwrap_err();
        assert!(matches!(error, ExportError::Compile(_)));
        std::fs::remove_dir_all(base).unwrap();
    }

    #[test]
    fn selected_paper_size_controls_physical_page_dimensions() {
        let (document, _) = markdown::parse("# Paper size\n\nBody");
        let document = document.unwrap();
        let engine = TypstEngine::default();
        for theme in themes() {
            for (paper, expected) in [
                (PaperSize::A4, (595.28, 841.89)),
                (PaperSize::A5, (419.53, 595.28)),
                (PaperSize::B5, (498.90, 708.66)),
            ] {
                let options = ExportOptions {
                    paper,
                    theme_id: theme.id.into(),
                    ..ExportOptions::default()
                };
                let mut diagnostics = Vec::new();
                let source = emit::emit(&document, theme, &options, &mut diagnostics);
                let inputs = template_inputs(&options);
                let world = OrgStudioWorld::new(&engine.shared, source, None, &inputs);
                let compiled = ::typst::compile::<PagedDocument>(&world);
                let output = compiled
                    .output
                    .unwrap_or_else(|errors| panic!("{} {paper:?}: {errors:?}", theme.id));
                for page in output.pages() {
                    let size = page.frame.size();
                    assert!(
                        (size.x.to_pt() - expected.0).abs() < 0.2,
                        "{} {paper:?} width was {}",
                        theme.id,
                        size.x.to_pt()
                    );
                    assert!(
                        (size.y.to_pt() - expected.1).abs() < 0.2,
                        "{} {paper:?} height was {}",
                        theme.id,
                        size.y.to_pt()
                    );
                }
            }
        }
    }

    #[test]
    fn inline_code_survives_theme_transformation() {
        let (document, _) = markdown::parse("- `overlays-at`, `overlays-in`;\n");
        let document = document.unwrap();
        let options = ExportOptions::default();
        let theme = themes().first().unwrap();
        let mut diagnostics = Vec::new();
        let source = emit::emit(&document, theme, &options, &mut diagnostics);
        let inputs = template_inputs(&options);
        let engine = TypstEngine::default();
        assert!(
            engine
                .font_families()
                .iter()
                .any(|family| family == "DejaVu Sans Mono"),
            "fonts: {:?}",
            engine.font_families()
        );
        let world = OrgStudioWorld::new(&engine.shared, source, None, &inputs);
        let compiled = ::typst::compile::<PagedDocument>(&world);
        let document = compiled.output.unwrap();
        let mut text = String::new();
        for page in document.pages() {
            frame_text(&page.frame, &mut text);
        }
        assert!(text.contains("overlays-at"), "rendered text: {text}");
        assert!(text.contains("overlays-in"), "rendered text: {text}");
    }
}
