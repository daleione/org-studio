//! Offline Markdown/Org export to PDF, PNG and SVG.
//!
//! The Typst kernel in this module was distilled from Yunli's `be-typst`
//! implementation. It deliberately has no dependency on Yunli's CDM or
//! design system; Org Studio owns the copied code and its compatibility.

mod diagnostic;
mod emit;
mod markdown;
mod model;
mod org;
mod output;
mod template;

pub use crate::typst_runtime::{CompileOutput, TypstEngine, shared_engine};
pub use diagnostic::{ExportDiagnostic, ExportSeverity};
pub use model::{ExportDocument, ExportMeta};
pub use template::{ExportTemplate, export_templates};

use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
};

use crate::document::TextSnapshot;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ExportSourceFormat {
    Org,
    Markdown,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ExportFormat {
    Pdf,
    Png,
    Svg,
}

impl From<ExportFormat> for crate::typst_runtime::OutputFormat {
    fn from(format: ExportFormat) -> Self {
        match format {
            ExportFormat::Pdf => Self::Pdf,
            ExportFormat::Png => Self::Png,
            ExportFormat::Svg => Self::Svg,
        }
    }
}

impl ExportFormat {
    pub fn extension(self) -> &'static str {
        match self {
            Self::Pdf => "pdf",
            Self::Png => "png",
            Self::Svg => "svg",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LayoutMode {
    Paged,
    Continuous,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PaperSize {
    TemplateDefault,
    A4,
    A5,
    B5,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Orientation {
    TemplateDefault,
    Portrait,
    Landscape,
}

#[derive(Clone, Debug)]
pub struct ExportOptions {
    pub format: ExportFormat,
    pub template_id: String,
    pub layout: LayoutMode,
    pub per_page: bool,
    pub png_ppi: f32,
    pub include_org_task_metadata: bool,
    pub paper: PaperSize,
    pub orientation: Orientation,
    pub font_scale: f32,
    pub margin_scale: f32,
    pub line_height_scale: f32,
    pub toc: Option<bool>,
    pub byline: Option<bool>,
    pub page_numbers: Option<bool>,
    pub footer: String,
}

impl Default for ExportOptions {
    fn default() -> Self {
        Self {
            format: ExportFormat::Pdf,
            template_id: "minimal-blue".into(),
            layout: LayoutMode::Paged,
            per_page: false,
            png_ppi: 144.0,
            include_org_task_metadata: true,
            paper: PaperSize::A4,
            orientation: Orientation::TemplateDefault,
            font_scale: 1.0,
            margin_scale: 1.0,
            line_height_scale: 1.0,
            toc: None,
            byline: None,
            page_numbers: None,
            footer: String::new(),
        }
    }
}

#[derive(Debug)]
pub struct ExportArtifacts {
    pub files: Vec<Vec<u8>>,
    pub diagnostics: Vec<ExportDiagnostic>,
}

#[derive(Debug)]
pub enum ExportError {
    Parse(String),
    UnknownTemplate(String),
    Compile(Vec<ExportDiagnostic>),
    Render(String),
    Io { path: PathBuf, message: String },
}

impl std::fmt::Display for ExportError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Parse(message) | Self::Render(message) => f.write_str(message),
            Self::UnknownTemplate(template) => {
                write!(f, "unknown export template `{template}`")
            }
            Self::Compile(diagnostics) => write!(
                f,
                "{}",
                diagnostics
                    .iter()
                    .map(|d| d.message.as_str())
                    .collect::<Vec<_>>()
                    .join("; ")
            ),
            Self::Io { path, message } => write!(f, "{}: {message}", path.display()),
        }
    }
}

impl std::error::Error for ExportError {}

pub fn export_snapshot(
    engine: &TypstEngine,
    snapshot: &dyn TextSnapshot,
    document_format: ExportSourceFormat,
    document_path: &Path,
    options: &ExportOptions,
) -> Result<ExportArtifacts, ExportError> {
    let source = snapshot.copy_range(crate::document::ByteRange::new(0, snapshot.len_bytes()));
    let (document, mut diagnostics) = match document_format {
        ExportSourceFormat::Markdown => markdown::parse(&source),
        ExportSourceFormat::Org => org::parse(&source, options.include_org_task_metadata),
    };
    let mut document = document.map_err(ExportError::Parse)?;
    filter_unavailable_assets(
        &mut document.blocks,
        document_path.parent(),
        &mut diagnostics,
    );
    let template = template::export_template(&options.template_id)
        .ok_or_else(|| ExportError::UnknownTemplate(options.template_id.clone()))?;
    let emitted = emit::emit(&document, template, options, &mut diagnostics);
    let inputs = template_inputs(options);
    let output = engine
        .compile_with_inputs(
            emitted,
            options.format.into(),
            options.per_page,
            options.png_ppi,
            document_path.parent(),
            &inputs,
        )
        .map_err(typst_error)?;
    diagnostics.extend(output.diagnostics.into_iter().map(typst_diagnostic));
    Ok(ExportArtifacts {
        files: output.pages,
        diagnostics,
    })
}

fn template_inputs(options: &ExportOptions) -> BTreeMap<String, String> {
    let mut inputs = BTreeMap::new();
    let print_layout = options.paper != PaperSize::TemplateDefault;
    inputs.insert(
        "paged".into(),
        (options.layout == LayoutMode::Paged).to_string(),
    );
    inputs.insert(
        "paper".into(),
        match options.paper {
            PaperSize::TemplateDefault => "",
            PaperSize::A4 => "a4",
            PaperSize::A5 => "a5",
            PaperSize::B5 => "iso-b5",
        }
        .into(),
    );
    inputs.insert(
        "orientation".into(),
        match options.orientation {
            Orientation::TemplateDefault => "",
            Orientation::Portrait => "portrait",
            Orientation::Landscape => "landscape",
        }
        .into(),
    );
    inputs.insert(
        "font-scale".into(),
        (options.font_scale.clamp(0.8, 1.6) * if print_layout { 0.7 } else { 1.0 }).to_string(),
    );
    inputs.insert(
        "layout-scale".into(),
        if print_layout { "0.65" } else { "1.0" }.into(),
    );
    inputs.insert(
        "margin-scale".into(),
        options.margin_scale.clamp(0.7, 1.6).to_string(),
    );
    inputs.insert(
        "line-height-scale".into(),
        options.line_height_scale.clamp(0.8, 1.6).to_string(),
    );
    if print_layout {
        for (name, size) in [
            ("theme-size-h1", "28"),
            ("theme-size-h2", "22"),
            ("theme-size-h3", "18"),
            ("theme-size-h4", "16"),
            ("theme-size-h5", "15"),
            ("theme-size-h6", "14"),
            ("theme-size-body", "15"),
            ("theme-size-code", "13"),
            ("theme-size-caption", "12"),
        ] {
            inputs.insert(name.into(), size.into());
        }
    }
    if let Some(value) = options.page_numbers {
        inputs.insert("page-number".into(), value.to_string());
    }
    if !options.footer.is_empty() {
        inputs.insert("footer-text".into(), options.footer.clone());
    }
    inputs
}

fn filter_unavailable_assets(
    blocks: &mut Vec<model::ExportBlock>,
    root: Option<&Path>,
    diagnostics: &mut Vec<ExportDiagnostic>,
) {
    blocks.retain_mut(|block| match block {
        model::ExportBlock::Image { path, .. } => {
            let raw = path.to_string_lossy();
            let allowed = if raw.starts_with("http://")
                || raw.starts_with("https://")
                || path.is_absolute()
            {
                false
            } else if let Some(root) = root {
                let canonical_root = root.canonicalize().ok();
                root.join(path.as_path())
                    .canonicalize()
                    .ok()
                    .zip(canonical_root)
                    .is_some_and(|(asset, root)| asset.starts_with(root) && asset.is_file())
            } else {
                false
            };
            if !allowed {
                diagnostics.push(ExportDiagnostic::warning(
                    "image-unavailable",
                    format!("image `{}` was omitted", path.display()),
                ));
            }
            allowed
        }
        model::ExportBlock::List { items, .. } => {
            for item in items {
                filter_unavailable_assets(item, root, diagnostics);
            }
            true
        }
        model::ExportBlock::Quote { blocks, .. } => {
            filter_unavailable_assets(blocks, root, diagnostics);
            true
        }
        _ => true,
    });
}

pub fn write_artifacts(
    destination: &Path,
    format: ExportFormat,
    artifacts: &[Vec<u8>],
) -> Result<Vec<PathBuf>, ExportError> {
    output::write(destination, format, artifacts)
}

fn typst_diagnostic(diagnostic: crate::typst_runtime::CompileDiagnostic) -> ExportDiagnostic {
    ExportDiagnostic {
        severity: match diagnostic.severity {
            crate::typst_runtime::DiagnosticSeverity::Warning => ExportSeverity::Warning,
            crate::typst_runtime::DiagnosticSeverity::Error => ExportSeverity::Error,
        },
        code: match diagnostic.severity {
            crate::typst_runtime::DiagnosticSeverity::Warning => "typst-warning",
            crate::typst_runtime::DiagnosticSeverity::Error => "typst-error",
        },
        message: diagnostic.message,
        source: diagnostic.source,
    }
}

fn typst_error(error: crate::typst_runtime::CompileError) -> ExportError {
    match error {
        crate::typst_runtime::CompileError::Compile(diagnostics) => {
            ExportError::Compile(diagnostics.into_iter().map(typst_diagnostic).collect())
        }
        crate::typst_runtime::CompileError::Render(message) => ExportError::Render(message),
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use super::*;

    #[test]
    fn common_paper_sizes_map_to_typst_names() {
        for (paper, expected) in [
            (PaperSize::A4, "a4"),
            (PaperSize::A5, "a5"),
            (PaperSize::B5, "iso-b5"),
        ] {
            let options = ExportOptions {
                paper,
                ..ExportOptions::default()
            };
            assert_eq!(template_inputs(&options)["paper"], expected);
        }
        let print = template_inputs(&ExportOptions::default());
        assert_eq!(print["layout-scale"], "0.65");
        assert_eq!(print["theme-size-body"], "15");
        assert_eq!(print["font-scale"], "0.7");

        let long_image = template_inputs(&ExportOptions {
            format: ExportFormat::Png,
            paper: PaperSize::TemplateDefault,
            layout: LayoutMode::Continuous,
            ..ExportOptions::default()
        });
        assert_eq!(long_image["layout-scale"], "1.0");
        assert!(!long_image.contains_key("theme-size-body"));
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn macos_system_font_search_loads_installed_families() {
        let families = shared_engine().font_families();
        assert!(families.iter().any(|family| family == "Menlo"));
        assert!(families.iter().any(|family| family.contains("Hiragino")));
    }

    #[test]
    fn every_bundled_theme_compiles_all_formats() {
        let markdown = "# 中英标题 Export\n\n正文 with **bold** and `code`.\n\n- first\n- 第二项\n\n> quote\n\n| A | B |\n|---|---|\n| 1 | 2 |\n\n```rust\nfn main() {}\n```\n";
        let (document, mut diagnostics) = markdown::parse(markdown);
        let document = document.unwrap();
        let engine = TypstEngine::default();
        let options = ExportOptions::default();
        let mut failures = Vec::new();
        for template in export_templates() {
            let source = emit::emit(&document, template, &options, &mut diagnostics);
            let mut inputs = BTreeMap::new();
            inputs.insert("paged".into(), "true".into());
            for format in [ExportFormat::Pdf, ExportFormat::Png, ExportFormat::Svg] {
                if let Err(error) = engine.compile_with_inputs(
                    source.clone(),
                    format.into(),
                    false,
                    72.0,
                    None,
                    &inputs,
                ) {
                    failures.push(format!("{} {format:?}: {error}", template.id));
                }
            }
        }
        assert!(failures.is_empty(), "{}", failures.join("\n"));
    }

    #[test]
    fn inline_code_survives_theme_transformation() {
        let (document, _) = markdown::parse("- `overlays-at`, `overlays-in`;\n");
        let document = document.unwrap();
        let options = ExportOptions::default();
        let template = export_templates().first().unwrap();
        let mut diagnostics = Vec::new();
        let source = emit::emit(&document, template, &options, &mut diagnostics);
        assert!(source.contains("overlays-at"), "generated Typst: {source}");
        assert!(source.contains("overlays-in"), "generated Typst: {source}");
        let inputs = template_inputs(&options);
        TypstEngine::default()
            .compile_with_inputs(source, options.format.into(), false, 144.0, None, &inputs)
            .unwrap();
    }
}
