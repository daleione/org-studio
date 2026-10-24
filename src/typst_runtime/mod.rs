mod packages;
mod render;
mod world;

use std::{
    collections::BTreeMap,
    ops::Range,
    path::Path,
    sync::{Mutex, OnceLock},
};

use typst_layout::PagedDocument;

use world::{OrgStudioWorld, SharedResources};

pub(crate) fn run_with_cache_cleanup<T>(operation: impl FnOnce() -> T) -> T {
    // comemo is shared by every Typst consumer, including diagrams. Eviction must not
    // run in the middle of another compilation.
    static COMPILER: Mutex<()> = Mutex::new(());
    let _guard = COMPILER.lock().unwrap_or_else(|error| error.into_inner());
    let result = operation();
    ::typst::comemo::evict(0);
    result
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum OutputFormat {
    Pdf,
    Png,
    Svg,
}

impl OutputFormat {
    pub fn infer_from_path(path: &Path) -> Option<Self> {
        match path.extension()?.to_str()?.to_ascii_lowercase().as_str() {
            "pdf" => Some(Self::Pdf),
            "png" => Some(Self::Png),
            "svg" => Some(Self::Svg),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DiagnosticSeverity {
    Warning,
    Error,
}

#[derive(Clone, Debug)]
pub struct CompileDiagnostic {
    pub severity: DiagnosticSeverity,
    pub message: String,
    pub source: Option<Range<usize>>,
}

#[derive(Debug)]
pub enum CompileError {
    Compile(Vec<CompileDiagnostic>),
    Render(String),
}

impl std::fmt::Display for CompileError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Compile(diagnostics) => formatter.write_str(
                &diagnostics
                    .iter()
                    .map(format_diagnostic)
                    .collect::<Vec<_>>()
                    .join("; "),
            ),
            Self::Render(message) => formatter.write_str(message),
        }
    }
}

impl std::error::Error for CompileError {}

#[derive(Debug)]
pub struct CompileOutput {
    pub pages: Vec<Vec<u8>>,
    pub diagnostics: Vec<CompileDiagnostic>,
}

/// A single auto-sized page. Measurements use Typst points, before rasterization.
pub(crate) struct CompiledFragment {
    pub(crate) svg: Vec<u8>,
    pub(crate) width: f32,
    pub(crate) height: f32,
    pub(crate) baseline: f32,
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
        format: OutputFormat,
        per_page: bool,
        png_ppi: f32,
        root: Option<&Path>,
    ) -> Result<CompileOutput, CompileError> {
        self.compile_with_inputs(source, format, per_page, png_ppi, root, &BTreeMap::new())
    }

    pub(crate) fn compile_with_inputs(
        &self,
        source: String,
        format: OutputFormat,
        per_page: bool,
        png_ppi: f32,
        root: Option<&Path>,
        inputs: &BTreeMap<String, String>,
    ) -> Result<CompileOutput, CompileError> {
        if per_page && format == OutputFormat::Pdf {
            return Err(CompileError::Render(
                "per-page output is only supported for PNG and SVG".into(),
            ));
        }
        self.with_document(source, root, inputs, |document, warnings| {
            validate_requested_paper(document, inputs)?;
            let pages = render::render(document, format, per_page, png_ppi)?;
            Ok(CompileOutput {
                pages,
                diagnostics: warnings,
            })
        })
    }

    /// Compile several independent fragments in one invocation, each on its own page.
    /// The caller supplies auto-sized, marginless pages containing one inline box.
    pub(crate) fn compile_fragments(
        &self,
        source: String,
    ) -> Result<Vec<CompiledFragment>, CompileError> {
        self.with_document(source, None, &BTreeMap::new(), |document, _| {
            let pages = render::render(document, OutputFormat::Svg, true, 192.0)?;
            document
                .pages()
                .iter()
                .zip(pages)
                .map(|(page, svg)| {
                    // Pages have no baseline of their own. The paragraph's baseline is
                    // explicit in its content frame; no SVG/glyph-position guessing.
                    let baseline = page
                        .frame
                        .items()
                        .find_map(|(position, item)| {
                            if let ::typst::layout::FrameItem::Group(group) = item
                                && group.frame.has_baseline()
                            {
                                Some(position.y + group.frame.baseline())
                            } else {
                                None
                            }
                        })
                        .ok_or_else(|| {
                            CompileError::Render("fragment has no text baseline".into())
                        })?;
                    Ok(CompiledFragment {
                        svg,
                        width: page.frame.width().to_pt() as f32,
                        height: page.frame.height().to_pt() as f32,
                        baseline: baseline.to_pt() as f32,
                    })
                })
                .collect()
        })
    }

    fn with_document<T>(
        &self,
        source: String,
        root: Option<&Path>,
        inputs: &BTreeMap<String, String>,
        consume: impl FnOnce(&PagedDocument, Vec<CompileDiagnostic>) -> Result<T, CompileError>,
    ) -> Result<T, CompileError> {
        run_with_cache_cleanup(|| {
            let world = OrgStudioWorld::new(&self.shared, source, root, inputs);
            let compiled = ::typst::compile::<PagedDocument>(&world);
            let warnings = diagnostics(&world, &compiled.warnings, DiagnosticSeverity::Warning);
            let document = compiled.output.map_err(|errors| {
                let mut all = diagnostics(&world, &errors, DiagnosticSeverity::Error);
                all.extend(warnings.clone());
                CompileError::Compile(all)
            })?;
            consume(&document, warnings)
        })
    }
}

/// Typst string escaping, shared by generated formula, chart and document sources.
pub(crate) fn string_literal(text: &str) -> String {
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

fn validate_requested_paper(
    document: &PagedDocument,
    inputs: &BTreeMap<String, String>,
) -> Result<(), CompileError> {
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
            return Err(CompileError::Render(format!(
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
    world: &OrgStudioWorld<'_>,
    source: &[::typst::diag::SourceDiagnostic],
    severity: DiagnosticSeverity,
) -> Vec<CompileDiagnostic> {
    use ::typst::{World as _, WorldExt as _};

    let mut seen = std::collections::BTreeSet::new();
    source
        .iter()
        .filter(|diagnostic| {
            severity == DiagnosticSeverity::Error || seen.insert(diagnostic.message.to_string())
        })
        .map(|diagnostic| CompileDiagnostic {
            severity,
            message: diagnostic.message.to_string(),
            source: (diagnostic.span.id() == Some(world.main()))
                .then(|| world.range(diagnostic.span))
                .flatten(),
        })
        .collect()
}

fn format_diagnostic(diagnostic: &CompileDiagnostic) -> String {
    diagnostic.source.as_ref().map_or_else(
        || diagnostic.message.clone(),
        |range| format!("byte {}: {}", range.start, diagnostic.message),
    )
}

pub fn shared_engine() -> &'static TypstEngine {
    static ENGINE: OnceLock<TypstEngine> = OnceLock::new();
    ENGINE.get_or_init(TypstEngine::default)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compiles_pdf_png_and_svg() {
        let engine = TypstEngine::default();
        let pdf = engine
            .compile("Hello".into(), OutputFormat::Pdf, false, 144.0, None)
            .unwrap();
        assert!(pdf.pages[0].starts_with(b"%PDF-"));

        let png = engine
            .compile("Hello".into(), OutputFormat::Png, false, 96.0, None)
            .unwrap();
        assert!(png.pages[0].starts_with(&[0x89, b'P', b'N', b'G']));

        let svg = engine
            .compile("Hello".into(), OutputFormat::Svg, false, 144.0, None)
            .unwrap();
        assert!(String::from_utf8_lossy(&svg.pages[0]).contains("<svg"));
    }

    #[test]
    fn bundled_fonts_supply_the_typst_default_families() {
        // The compiler's default text and math families ship with the binary, not
        // with the host. Losing them silently changes every document's metrics and
        // degrades math layout, so pin them on the shared engine.
        let families = shared_engine().font_families();
        for family in ["Libertinus Serif", "New Computer Modern Math"] {
            assert!(
                families.iter().any(|available| available == family),
                "bundled Typst family `{family}` is missing; the default text and math \
                 families must come from typst-assets"
            );
        }
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
                OutputFormat::Pdf,
                false,
                144.0,
                Some(&root),
            )
            .unwrap_err();
        assert!(matches!(error, CompileError::Compile(_)));
        std::fs::remove_dir_all(base).unwrap();
    }

    #[test]
    fn compile_inputs_can_control_and_validate_paper_size() {
        let engine = TypstEngine::default();
        for paper in ["a4", "a5", "iso-b5"] {
            let inputs = BTreeMap::from([("paper".to_owned(), paper.to_owned())]);
            engine
                .compile_with_inputs(
                    "#set page(paper: sys.inputs.paper)\nHello".into(),
                    OutputFormat::Pdf,
                    false,
                    144.0,
                    None,
                    &inputs,
                )
                .unwrap();
        }
    }
}
