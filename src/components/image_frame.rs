use std::sync::Arc;

use gpui::{
    BorderStyle, Bounds, BoxShadow, Corners, Div, Hsla, ImageSource, ObjectFit, Pixels,
    RenderImage, Window, div, fill, outline, point, prelude::*, px, rgb, rgba,
};

use crate::document_preview::{self, RecordedBox};

pub(crate) const RADIUS: f32 = 12.0;
pub(crate) const CONTENT_RADIUS: f32 = RADIUS - 1.0;

pub(crate) struct ImageFrame {
    background: Hsla,
    border: Hsla,
    shadow: BoxShadow,
}

impl ImageFrame {
    pub(crate) fn new(background: u32, selected: Option<u32>) -> Self {
        let theme = crate::theme::current_theme();
        let dark = crate::theme::effective_theme_is_dark();
        Self {
            background: rgb(background).into(),
            border: rgb(selected.unwrap_or(if dark {
                theme.border_hover
            } else {
                theme.border
            }))
            .into(),
            shadow: BoxShadow {
                color: rgba(if dark { 0x00000033 } else { 0x0000001a }).into(),
                offset: point(px(0.0), px(2.0)),
                blur_radius: px(8.0),
                spread_radius: px(0.0),
                inset: false,
            },
        }
    }

    pub(crate) fn surface(&self) -> Div {
        div()
            .relative()
            .overflow_hidden()
            .rounded(px(RADIUS))
            .border_1()
            .border_color(self.border)
            .bg(self.background)
            .shadow(vec![self.shadow.clone()])
    }

    pub(crate) fn image(
        &self,
        source: impl Into<ImageSource>,
        width: f32,
        height: f32,
    ) -> RecordedBox {
        document_preview::record_box(
            self.surface().w(px(width)).h(px(height)).child(
                document_preview::img(source)
                    .size_full()
                    .rounded(px(CONTENT_RADIUS)),
            ),
        )
    }

    pub(crate) fn paint_image(
        &self,
        bounds: Bounds<Pixels>,
        image: Arc<RenderImage>,
        window: &mut Window,
    ) -> gpui::Result<()> {
        let radii = Corners::all(px(RADIUS)).clamp_radii_for_quad_size(bounds.size);
        document_preview::drop_shadow(window, bounds, radii, &self.shadow);
        document_preview::quad(window, fill(bounds, self.background).corner_radii(radii));
        // Paint beneath the border; insetting and fitting creates a second image edge.
        let result = if bounds.size.width > px(0.0) && bounds.size.height > px(0.0) {
            let fitted = ObjectFit::Contain.get_bounds(bounds, image.size(0));
            document_preview::image(window, bounds, fitted, radii, image, 0, false)
        } else {
            Ok(())
        };
        let mut border = outline(bounds, self.border, BorderStyle::Solid).corner_radii(radii);
        // Match the native style painter's RGB when antialiasing into transparency.
        border.background = Hsla {
            a: 0.0,
            ..self.border
        }
        .into();
        document_preview::quad(window, border);
        result
    }
}
