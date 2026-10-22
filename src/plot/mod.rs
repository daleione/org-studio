mod data;
mod document;
mod model;
pub(crate) mod output;
mod parse;
mod plan;
mod render;
mod service;
mod time;
mod view;

pub(crate) use document::{PlotDocument, PlotEntry};
pub(crate) use model::*;
pub(crate) use parse::{bind_images, collect_plots};
pub(crate) use render::render_plot;
#[cfg(test)]
pub(crate) use service::PlotRendered;
pub(crate) use service::{PlotDemand, PlotRenderState, PlotService};
pub(crate) use view::{
    PlotHost, PlotPresentation, plot_image_size, render_plot_output_error, render_plot_surface,
};

pub(crate) fn may_have_plots(snapshot: &dyn crate::document::TextSnapshot) -> bool {
    let mut offset = 0;
    let mut tail = Vec::new();
    while offset < snapshot.len_bytes() {
        let Some(chunk) = snapshot.chunk_at(crate::document::ByteOffset(offset)) else {
            break;
        };
        let bytes = &chunk.text.as_bytes()[(offset - chunk.start.0) as usize..];
        if bytes.is_empty() {
            break;
        }
        let boundary = tail
            .iter()
            .copied()
            .chain(bytes.iter().take(6).copied())
            .collect::<Vec<_>>();
        if boundary
            .windows(7)
            .chain(bytes.windows(7))
            .any(|word| word.eq_ignore_ascii_case(b"#+PLOT:"))
        {
            return true;
        }
        tail = tail
            .into_iter()
            .chain(bytes.iter().copied())
            .rev()
            .take(6)
            .collect();
        tail.reverse();
        offset = chunk.start.0 + chunk.text.len() as u64;
    }
    false
}

pub(crate) fn prepare_plot(
    snapshot: &dyn crate::document::TextSnapshot,
    candidate: &PlotCandidate,
) -> PlotPreparation {
    match data::prepare_input(snapshot, candidate).and_then(plan::build_plan) {
        Ok(Some(plot)) => PlotPreparation::Ready(std::sync::Arc::new(plot)),
        Ok(None) => PlotPreparation::Empty,
        Err(error) => PlotPreparation::Invalid(std::sync::Arc::new(error)),
    }
}

#[cfg(test)]
mod tests;
