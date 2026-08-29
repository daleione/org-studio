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
mod theme;
mod typst;

pub use diagnostic::{ExportDiagnostic, ExportSeverity};
pub use model::{ExportDocument, ExportMeta};
pub use theme::{Theme, themes};
pub use typst::{CompileOutput, TypstEngine};

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
    Theme,
    A4,
    A5,
    B5,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Orientation {
    Theme,
    Portrait,
    Landscape,
}

#[derive(Clone, Debug)]
pub struct ExportOptions {
    pub format: ExportFormat,
    pub theme_id: String,
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
            theme_id: "minimal-blue".into(),
            layout: LayoutMode::Paged,
            per_page: false,
            png_ppi: 144.0,
            include_org_task_metadata: true,
            paper: PaperSize::A4,
            orientation: Orientation::Theme,
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
    UnknownTheme(String),
    Compile(Vec<ExportDiagnostic>),
    Render(String),
    Io { path: PathBuf, message: String },
}

impl std::fmt::Display for ExportError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Parse(message) | Self::Render(message) => f.write_str(message),
            Self::UnknownTheme(theme) => write!(f, "unknown export theme `{theme}`"),
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
    let theme = theme::theme(&options.theme_id)
        .ok_or_else(|| ExportError::UnknownTheme(options.theme_id.clone()))?;
    let emitted = emit::emit(&document, theme, options, &mut diagnostics);
    let inputs = template_inputs(options);
    let output = engine.compile_with_inputs(
        emitted,
        options.format,
        options.per_page,
        options.png_ppi,
        document_path.parent(),
        &inputs,
    )?;
    diagnostics.extend(output.diagnostics);
    Ok(ExportArtifacts {
        files: output.pages,
        diagnostics,
    })
}

fn template_inputs(options: &ExportOptions) -> BTreeMap<String, String> {
    let mut inputs = BTreeMap::new();
    let print_layout = options.paper != PaperSize::Theme;
    inputs.insert(
        "paged".into(),
        (options.layout == LayoutMode::Paged).to_string(),
    );
    inputs.insert(
        "paper".into(),
        match options.paper {
            PaperSize::Theme => "",
            PaperSize::A4 => "a4",
            PaperSize::A5 => "a5",
            PaperSize::B5 => "iso-b5",
        }
        .into(),
    );
    inputs.insert(
        "orientation".into(),
        match options.orientation {
            Orientation::Theme => "",
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
            let allowed = if raw.starts_with("http://") || raw.starts_with("https://") {
                false
            } else if path.is_absolute() {
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

pub fn shared_engine() -> &'static TypstEngine {
    static ENGINE: std::sync::OnceLock<TypstEngine> = std::sync::OnceLock::new();
    ENGINE.get_or_init(|| TypstEngine::new(&platform_font_data()))
}

fn platform_font_data() -> Vec<Vec<u8>> {
    #[cfg(target_os = "macos")]
    {
        use std::collections::BTreeSet;

        let mut database = fontdb::Database::new();
        database.load_system_fonts();
        let paths = database
            .faces()
            .filter_map(|face| match &face.source {
                fontdb::Source::File(path) | fontdb::Source::SharedFile(path, _) => {
                    Some(path.clone())
                }
                fontdb::Source::Binary(_) => None,
            })
            .collect::<BTreeSet<_>>();
        paths
            .into_iter()
            .filter_map(|path| std::fs::read(path).ok())
            .collect()
    }
    #[cfg(not(target_os = "macos"))]
    {
        Vec::new()
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
            paper: PaperSize::Theme,
            layout: LayoutMode::Continuous,
            ..ExportOptions::default()
        });
        assert_eq!(long_image["layout-scale"], "1.0");
        assert!(!long_image.contains_key("theme-size-body"));
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn macos_system_font_search_loads_installed_families() {
        let data = platform_font_data();
        assert!(!data.is_empty());
        let families = TypstEngine::new(&data).font_families();
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
        for theme in themes() {
            let source = emit::emit(&document, theme, &options, &mut diagnostics);
            let mut inputs = BTreeMap::new();
            inputs.insert("paged".into(), "true".into());
            for format in [ExportFormat::Pdf, ExportFormat::Png, ExportFormat::Svg] {
                if let Err(error) =
                    engine.compile_with_inputs(source.clone(), format, false, 72.0, None, &inputs)
                {
                    failures.push(format!("{} {format:?}: {error}", theme.id));
                }
            }
        }
        assert!(failures.is_empty(), "{}", failures.join("\n"));
    }
}
