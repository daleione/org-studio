use super::{
    ExportDiagnostic, ExportError, TypstEngine,
    model::{ExportBlock, ExportDocument},
};
use crate::{
    document::TextSnapshot,
    plot::{PlotPreparation, PlotRenderRequest, PlotStyle},
};

pub(super) fn insert_charts(
    engine: &TypstEngine,
    snapshot: &dyn TextSnapshot,
    document_path: &std::path::Path,
    document: &mut ExportDocument,
    diagnostics: &mut Vec<ExportDiagnostic>,
) -> Result<(), ExportError> {
    if !crate::plot::may_have_plots(snapshot) {
        return Ok(());
    }
    let arena = crate::org_syntax::parse(snapshot);
    let mut candidates = crate::plot::collect_plots(snapshot, &arena);
    crate::plot::bind_images(snapshot, &arena, &mut candidates, Some(document_path));
    if candidates.is_empty() {
        return Ok(());
    }
    let mut blocks = Vec::with_capacity(document.blocks.len() + candidates.len());
    for block in std::mem::take(&mut document.blocks) {
        let candidate = if matches!(block, ExportBlock::Table { .. }) {
            candidates.iter().find(|candidate| {
                candidate.image_link.is_none()
                    && candidate.table.start.0 as usize <= block.source().start
                    && candidate.table.end.0 as usize >= block.source().end
            })
        } else if matches!(block, ExportBlock::Image { .. }) {
            candidates.iter().find(|candidate| {
                candidate.image_link.is_some_and(|link| {
                    link.start.0 as usize <= block.source().start
                        && link.end.0 as usize >= block.source().end
                })
            })
        } else {
            None
        };
        if candidate.is_none() || !matches!(block, ExportBlock::Image { .. }) {
            blocks.push(block);
        }
        let Some(candidate) = candidate else {
            continue;
        };
        let result = match crate::plot::prepare_plot(snapshot, candidate) {
            PlotPreparation::Ready(plot) => {
                for warning in &plot.plan.warnings {
                    diagnostics.push(ExportDiagnostic::warning("plot-precision", warning.clone()));
                }
                let alt = plot.plan.title.clone().unwrap_or_else(|| {
                    plot.plan
                        .series
                        .iter()
                        .map(|series| series.label.as_str())
                        .collect::<Vec<_>>()
                        .join(", ")
                });
                let request = PlotRenderRequest::natural(
                    plot,
                    PlotStyle::for_theme(&crate::theme::ORG_STUDIO_LIGHT, 12.0),
                );
                crate::plot::render_plot(engine, &request).map(|artifact| {
                    Some(ExportBlock::GeneratedImage {
                        svg: artifact.svg,
                        alt,
                        size: (artifact.width, artifact.height),
                        width: export_width(
                            &candidate.image_attributes,
                            (artifact.width, artifact.height),
                        ),
                        source: candidate
                            .image_link
                            .unwrap_or(candidate.declaration)
                            .as_usize(),
                    })
                })
            }
            PlotPreparation::Empty => {
                diagnostics.push(ExportDiagnostic::warning(
                    "plot-empty",
                    "The Org Plot table contains no plottable data",
                ));
                Ok(None)
            }
            PlotPreparation::Invalid(error) => Err((*error).clone()),
        };
        match result {
            Ok(Some(image)) => blocks.push(image),
            Ok(None) => {}
            Err(error) => {
                let mut diagnostic = ExportDiagnostic::error(error.code, error.message);
                diagnostic.source = Some(candidate.declaration.as_usize());
                return Err(ExportError::Compile(vec![diagnostic]));
            }
        }
    }
    document.blocks = blocks;
    Ok(())
}

fn export_width(
    spec: &crate::org_syntax::attributes::ImageAttributeSpec,
    dimensions: (f32, f32),
) -> Option<super::model::ExportImageWidth> {
    use super::model::ExportImageWidth;
    use crate::org_syntax::attributes::ImageLength;
    if let Some(scale) = spec.scale {
        return Some(ExportImageWidth::Pixels(dimensions.0 * scale));
    }
    let factor = if spec.width.is_some() {
        1.0
    } else {
        dimensions.0 / dimensions.1
    };
    spec.width.or(spec.height).map(|length| match length {
        ImageLength::Px(value) => ExportImageWidth::Pixels(value * factor),
        ImageLength::Percent(value) => ExportImageWidth::Percent(value * factor),
        ImageLength::Em(value) => ExportImageWidth::Em(value * factor),
    })
}
