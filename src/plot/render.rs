use super::{MAX_ARTIFACT_BYTES, PlotArtifact, PlotDiagnostic, PlotRenderRequest};
use crate::typst_runtime::{OutputFormat, TypstEngine};
use std::{
    borrow::Cow,
    collections::BTreeMap,
    sync::{Mutex, OnceLock},
};

pub(crate) fn render_plot(
    engine: &TypstEngine,
    request: &PlotRenderRequest,
) -> Result<PlotArtifact, PlotDiagnostic> {
    let svg = render_file(engine, request, OutputFormat::Svg)?;
    let (width, height) = request.key.size;
    Ok(PlotArtifact {
        key: request.key.clone(),
        svg: svg.into(),
        width: width as f32,
        height: height as f32,
    })
}

pub(crate) fn render_file(
    engine: &TypstEngine,
    request: &PlotRenderRequest,
    format: OutputFormat,
) -> Result<Vec<u8>, PlotDiagnostic> {
    static COMPILER: OnceLock<Mutex<()>> = OnceLock::new();
    let _guard = COMPILER
        .get_or_init(Mutex::default)
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    let mut style = request.key.style.clone();
    let available = engine.font_families();
    style.fonts.retain(|family| {
        available
            .iter()
            .any(|known| known.eq_ignore_ascii_case(family))
    });
    let (width, height) = request.key.size;
    let mut plan = Cow::Borrowed(&request.plot.plan);
    if plan.time_axis.is_some() {
        super::time::reflow(plan.to_mut(), width, style.font_size_100 as f32 / 100.0)?;
    }
    let input = serde_json::json!({"plan": plan.as_ref(), "style": style, "width": width, "height": height});
    let inputs = BTreeMap::from([("chart-json".to_owned(), input.to_string())]);
    let body = match request.plot.plan.layout {
        super::PlotLayout::Cartesian => include_str!("../../assets/typst/plot/cartesian.typ"),
        super::PlotLayout::Radar => include_str!("../../assets/typst/plot/radar.typ"),
        super::PlotLayout::Grid => include_str!("../../assets/typst/plot/grid.typ"),
    };
    let source = [include_str!("../../assets/typst/plot/chart.typ"), body].concat();
    let output = engine
        .compile_with_inputs(
            source,
            format,
            format != OutputFormat::Pdf,
            192.0,
            None,
            &inputs,
        )
        .map_err(|error| {
            tracing::warn!(error = %error,"Org Plot rendering failed");
            let message = if let crate::typst_runtime::CompileError::Compile(diagnostics) = &error {
                diagnostics
                    .iter()
                    .find(|diagnostic| {
                        diagnostic.message.contains("shorten")
                            || diagnostic.message.contains("too narrow")
                    })
                    .map(|diagnostic| diagnostic.message.clone())
            } else {
                None
            };
            PlotDiagnostic::new(
                "compile-failed",
                message.unwrap_or_else(|| "Could not render the chart. Click to retry.".into()),
            )
        })?;
    if output.pages.len() != 1 {
        return Err(PlotDiagnostic::new(
            "invalid-artifact",
            "A chart must render as exactly one page",
        ));
    }
    let bytes = output
        .pages
        .into_iter()
        .next()
        .expect("single page checked");
    if bytes.len() > MAX_ARTIFACT_BYTES {
        return Err(PlotDiagnostic::new(
            "limit-exceeded",
            "The chart output exceeds 8 MiB; reduce the number of points",
        ));
    }
    Ok(bytes)
}
