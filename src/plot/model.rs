use std::{
    path::PathBuf,
    sync::{Arc, Mutex},
};

use serde::Serialize;

use crate::{
    document::ByteRange,
    org_syntax::{BlockId, SyntaxId},
};

pub(crate) const MAX_SERIES: usize = 8;
pub(crate) const MAX_POINTS: usize = 10_000;
pub(crate) const MAX_ARTIFACT_BYTES: usize = 8 * 1024 * 1024;
pub(crate) const PLOT_WIDTH: f32 = 480.0;
pub(crate) const PLOT_HEIGHT: f32 = 250.0;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) enum PlotOutputTarget {
    #[default]
    Memory,
    File(PathBuf),
}

#[derive(Debug, Default)]
pub(crate) struct PlotDisplayState {
    pub(crate) requested_width: Option<f32>,
    pub(crate) requested_height: Option<f32>,
    pub(crate) resizing: bool,
    pub(crate) selected: bool,
}

impl PlotDisplayState {
    pub(crate) fn attributes(
        &self,
        authored: &crate::org_syntax::attributes::ImageAttributeSpec,
    ) -> crate::org_syntax::attributes::ImageAttributeSpec {
        use crate::org_syntax::attributes::{ImageAttributeSpec, ImageLength};
        if self.requested_width.is_none() && self.requested_height.is_none() {
            return authored.clone();
        }
        ImageAttributeSpec {
            width: self.requested_width.map(ImageLength::Px).or(authored.width),
            height: self
                .requested_height
                .map(ImageLength::Px)
                .or(authored.height),
            scale: None,
        }
    }
}

pub(crate) type PlotDisplay = Arc<Mutex<PlotDisplayState>>;

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(crate) struct PlotId(pub(crate) u64);

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub(crate) enum PlotIdentity {
    Declaration(SyntaxId),
    NamedTable(Arc<str>),
    NamedResult(Arc<str>),
    SourceBlock(SyntaxId),
}

#[derive(Clone, Debug)]
pub(crate) struct PlotCandidate {
    pub(crate) identity: PlotIdentity,
    pub(crate) declaration: ByteRange,
    pub(crate) table: ByteRange,
    pub(crate) formulas: Option<ByteRange>,
    pub(crate) owner: BlockId,
    pub(crate) options: Vec<String>,
    pub(crate) image_link: Option<ByteRange>,
    pub(crate) image_attributes: crate::org_syntax::attributes::ImageAttributeSpec,
}

impl PlotCandidate {
    pub(crate) fn source_end(&self) -> crate::document::ByteOffset {
        self.formulas.map_or(self.table.end, |range| range.end)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum PlotKind {
    Lines,
    Points,
    LinesPoints,
    Histograms,
    Steps,
    FSteps,
    HiSteps,
    FilledCurves,
    Impulses,
    Boxes,
    BoxPlot,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize)]
pub(crate) enum HistogramStyle {
    #[default]
    Clustered,
    RowStacked,
    ColumnStacked,
}

#[derive(Clone, Copy, Debug, Serialize)]
pub(crate) struct BoxPlotStyle {
    pub(crate) range: f64,
    pub(crate) outliers: bool,
}

impl Default for BoxPlotStyle {
    fn default() -> Self {
        Self {
            range: 1.5,
            outliers: true,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum PlotLayout {
    #[default]
    Cartesian,
    Radar,
    Grid,
}

impl PlotLayout {
    pub(crate) fn dimensions(self) -> (u32, u32) {
        match self {
            Self::Radar => (360, 320),
            _ => (PLOT_WIDTH as u32, PLOT_HEIGHT as u32),
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Serialize)]
pub(crate) struct PlotAxisSpec {
    pub(crate) min: Option<f64>,
    pub(crate) max: Option<f64>,
}

#[derive(Clone, Debug, Default)]
pub(crate) struct PlotSpec {
    pub(crate) output: PlotOutputTarget,
    pub(crate) title: Option<String>,
    pub(crate) kind: Option<PlotKind>,
    pub(crate) x_column: usize,
    pub(crate) columns: Option<Vec<usize>>,
    pub(crate) labels: Option<Vec<Option<String>>>,
    pub(crate) layout: PlotLayout,
    pub(crate) transpose: bool,
    pub(crate) time: bool,
    pub(crate) time_format: Option<String>,
    pub(crate) x_axis: PlotAxisSpec,
    pub(crate) y_axis: PlotAxisSpec,
    pub(crate) ticks: Option<usize>,
    pub(crate) fill_to_zero: bool,
    pub(crate) histogram: Option<HistogramStyle>,
    pub(crate) box_width: Option<f64>,
    pub(crate) boxplot: Option<BoxPlotStyle>,
}

#[derive(Clone, Debug, Serialize)]
pub(crate) struct PlotSeries {
    pub(crate) column: usize,
    pub(crate) label: String,
    pub(crate) values: Vec<Option<f64>>,
}

#[derive(Clone, Debug, Serialize)]
pub(crate) enum PlotX {
    Numeric(Vec<Option<f64>>),
    Time(Vec<Option<f64>>),
    Category(Vec<String>),
}

#[derive(Clone, Debug, Serialize)]
pub(crate) struct PlotInput {
    pub(crate) layout: PlotLayout,
    pub(crate) kind: PlotKind,
    pub(crate) title: Option<String>,
    pub(crate) x: PlotX,
    pub(crate) series: Vec<PlotSeries>,
    pub(crate) warnings: Vec<String>,
    pub(crate) x_axis: PlotAxisSpec,
    pub(crate) y_axis: PlotAxisSpec,
    pub(crate) ticks: Option<usize>,
    pub(crate) fill_to_zero: bool,
    pub(crate) histogram: HistogramStyle,
    pub(crate) box_width: Option<f64>,
    pub(crate) boxplot: BoxPlotStyle,
}

#[derive(Clone, Debug)]
pub(crate) struct PlotDiagnostic {
    pub(crate) code: &'static str,
    pub(crate) message: String,
}

impl PlotDiagnostic {
    pub(crate) fn new(code: &'static str, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }
}

impl std::fmt::Display for PlotDiagnostic {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for PlotDiagnostic {}

#[derive(Clone, Debug, Serialize)]
pub(crate) struct PlotTick {
    pub(crate) position: f64,
    pub(crate) label: String,
}

#[derive(Clone, Debug, Serialize)]
pub(crate) struct PlotPlanSeries {
    pub(crate) label: String,
    pub(crate) points: Vec<Option<(f64, f64)>>,
    pub(crate) bases: Vec<Option<f64>>,
    pub(crate) boxplot: Option<PlotBoxSummary>,
}

#[derive(Clone, Debug, Serialize)]
pub(crate) struct PlotBoxSummary {
    pub(crate) position: f64,
    pub(crate) width: Option<f64>,
    // Lower whisker, first quartile, median, third quartile, upper whisker.
    pub(crate) statistics: Option<[f64; 5]>,
    pub(crate) outliers: Vec<f64>,
}

#[derive(Clone, Debug, Serialize)]
pub(crate) struct PlotPlan {
    pub(crate) layout: PlotLayout,
    pub(crate) kind: PlotKind,
    pub(crate) title: Option<String>,
    pub(crate) x_ticks: Vec<PlotTick>,
    pub(crate) y_ticks: Vec<PlotTick>,
    pub(crate) series: Vec<PlotPlanSeries>,
    pub(crate) zero: f64,
    pub(crate) rows: usize,
    pub(crate) warnings: Vec<String>,
    pub(crate) radar_ticks: Vec<Vec<PlotTick>>,
    pub(crate) box_bounds: Vec<Option<(f64, f64)>>,
    pub(crate) fill_to_zero: bool,
    pub(crate) clip: bool,
    pub(crate) stacked: bool,
    #[serde(skip)]
    pub(crate) time_axis: Option<PlotTimeAxis>,
    pub(crate) x_minor_ticks: Vec<f64>,
    pub(crate) x_context: Vec<PlotTick>,
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct PlotTimeAxis {
    pub(crate) min: f64,
    pub(crate) max: f64,
    pub(crate) ticks: Option<usize>,
}

#[derive(Clone, Debug)]
pub(crate) struct PreparedPlot {
    pub(crate) content_key: Arc<[u8]>,
    pub(crate) plan: PlotPlan,
    pub(crate) natural_size: (u32, u32),
}

impl PreparedPlot {
    pub(super) fn new(content_key: Arc<[u8]>, plan: PlotPlan) -> Self {
        let natural_size = super::layout::natural_size(&plan);
        Self {
            content_key,
            plan,
            natural_size,
        }
    }
}

#[derive(Clone, Debug)]
pub(crate) enum PlotPreparation {
    Ready(Arc<PreparedPlot>),
    Empty,
    Invalid(Arc<PlotDiagnostic>),
}

#[derive(Clone, Debug, Eq, Hash, PartialEq, Serialize)]
pub(crate) struct PlotStyle {
    pub(crate) background: u32,
    pub(crate) foreground: u32,
    pub(crate) muted: u32,
    pub(crate) grid: u32,
    pub(crate) colors: [u32; MAX_SERIES],
    pub(crate) fonts: Vec<String>,
    pub(crate) font_size_100: u32,
}

impl PlotStyle {
    pub(crate) fn for_theme(theme: &crate::theme::Theme, font_size: f32) -> Self {
        let dark = theme.editor_background == crate::theme::ORG_STUDIO_DARK.editor_background;
        let fonts = crate::typography::fonts();
        let mut families = Vec::new();
        families.extend(
            fonts
                .reading_family
                .iter()
                .chain(fonts.ui_family.iter())
                .cloned(),
        );
        families.extend(["Helvetica Neue".into(), "Arial".into()]);
        families.extend(
            fonts
                .reading_cjk_family
                .iter()
                .chain(fonts.ui_cjk_family.iter())
                .cloned(),
        );
        families.extend(fonts.fallbacks.iter().cloned());
        families.extend([
            "PingFang SC".into(),
            "Noto Sans CJK SC".into(),
            "Libertinus Serif".into(),
        ]);
        Self {
            background: if dark {
                theme.elevated
            } else {
                theme.editor_background
            },
            foreground: theme.foreground,
            muted: if dark { 0xa5b3c2 } else { 0x737780 },
            grid: if dark { 0x47596f } else { 0xd1d5dc },
            colors: if dark {
                [
                    0x66b3ff, 0xc4a4ff, 0x53d4c1, 0xf5b84b, 0xf58bb5, 0x9bc46b, 0x85c5e8, 0xc7a98b,
                ]
            } else {
                [
                    theme.accent,
                    0x8e5bef,
                    0x168a84,
                    0xa66c00,
                    0xc1477b,
                    0x547c25,
                    0x367b9d,
                    0x8e6345,
                ]
            },
            fonts: families,
            font_size_100: (font_size.clamp(8.0, 48.0) * 100.0).round() as u32,
        }
    }
}

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub(crate) struct PlotRenderKey {
    pub(crate) content: Arc<[u8]>,
    pub(crate) style: PlotStyle,
    pub(crate) size: (u32, u32),
}

#[derive(Clone, Debug)]
pub(crate) struct PlotRenderRequest {
    pub(crate) key: PlotRenderKey,
    pub(crate) plot: Arc<PreparedPlot>,
}

impl PlotRenderRequest {
    pub(crate) fn natural(plot: Arc<PreparedPlot>, style: PlotStyle) -> Self {
        let (width, height) = plot.natural_size;
        Self::sized(plot, style, width as f32, height as f32)
    }

    pub(crate) fn for_display(
        plot: Arc<PreparedPlot>,
        style: PlotStyle,
        size: (f32, f32),
        font_size: f32,
    ) -> Self {
        let scale = super::layout::display_scale(font_size);
        Self::sized(plot, style, size.0 / scale, size.1 / scale)
    }

    pub(crate) fn for_export(
        plot: Arc<PreparedPlot>,
        style: PlotStyle,
        attributes: &crate::org_syntax::attributes::ImageAttributeSpec,
    ) -> Self {
        use crate::org_syntax::attributes::ImageLength;
        if attributes.scale.is_some() {
            return Self::natural(plot, style);
        }
        // Relative lengths remain relative to the exported page. Dragged pixel
        // dimensions must reflow the SVG before it is embedded.
        let pixels = |length| match length {
            Some(ImageLength::Px(value)) => Some(value),
            _ => None,
        };
        let (width, height) = super::layout::canvas_size(
            pixels(attributes.width).unwrap_or(plot.natural_size.0 as f32),
            pixels(attributes.height).unwrap_or(plot.natural_size.1 as f32),
        );
        let height = (height as f32).max(super::layout::minimum_height(&plot.plan, width as f32));
        Self::sized(plot, style, width as f32, height)
    }

    pub(crate) fn sized(
        plot: Arc<PreparedPlot>,
        style: PlotStyle,
        width: f32,
        height: f32,
    ) -> Self {
        let key = PlotRenderKey {
            content: plot.content_key.clone(),
            style,
            size: super::layout::canvas_size(width, height),
        };
        Self { key, plot }
    }
}

#[derive(Clone, Debug)]
pub(crate) struct PlotArtifact {
    pub(crate) key: PlotRenderKey,
    pub(crate) svg: Arc<[u8]>,
    pub(crate) width: f32,
    pub(crate) height: f32,
}
