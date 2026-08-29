use ::typst::{layout::Abs, utils::Scalar, visualize::Color};
use typst_layout::PagedDocument;

use crate::export::{ExportError, ExportFormat};

pub(super) fn render(
    document: &PagedDocument,
    format: ExportFormat,
    per_page: bool,
    ppi: f32,
) -> Result<Vec<Vec<u8>>, ExportError> {
    if document.pages().is_empty() {
        return Err(ExportError::Render(
            "empty document: no pages to render".into(),
        ));
    }
    match format {
        ExportFormat::Pdf => typst_pdf::pdf(document, &typst_pdf::PdfOptions::default())
            .map(|bytes| vec![bytes])
            .map_err(|errors| {
                ExportError::Render(
                    errors
                        .iter()
                        .map(|error| error.message.to_string())
                        .collect::<Vec<_>>()
                        .join("; "),
                )
            }),
        ExportFormat::Png => {
            if !ppi.is_finite() || !(36.0..=600.0).contains(&ppi) {
                return Err(ExportError::Render(
                    "PNG PPI must be between 36 and 600".into(),
                ));
            }
            let pixel_per_pt = f64::from(ppi) / 72.0;
            let options = typst_render::RenderOptions {
                pixel_per_pt: Scalar::new(pixel_per_pt),
                render_bleed: false,
            };
            if per_page {
                document
                    .pages()
                    .iter()
                    .map(|page| {
                        typst_render::render(page, &options)
                            .encode_png()
                            .map_err(|error| ExportError::Render(format!("PNG encoding: {error}")))
                    })
                    .collect()
            } else {
                typst_render::render_merged(document, &options, Abs::pt(0.0), Some(Color::WHITE))
                    .encode_png()
                    .map(|bytes| vec![bytes])
                    .map_err(|error| ExportError::Render(format!("PNG encoding: {error}")))
            }
        }
        ExportFormat::Svg => {
            let options = typst_svg::SvgOptions::default();
            if per_page {
                Ok(document
                    .pages()
                    .iter()
                    .map(|page| typst_svg::svg(page, &options).into_bytes())
                    .collect())
            } else {
                Ok(vec![
                    typst_svg::svg_merged(document, &options, Abs::pt(0.0)).into_bytes(),
                ])
            }
        }
    }
}
