//! A fixed window-space backdrop, shared by every reveal/handoff region.
use crate::theme::{self, Theme};
use gpui::{Bounds, ContentMask, Corners, Pixels, RenderImage, Window, point, px, rgb, size};
use std::sync::{Arc, LazyLock};

pub(super) fn paint(region: Bounds<Pixels>, window: &mut Window) {
    static LIGHT: LazyLock<Arc<RenderImage>> = LazyLock::new(|| {
        Arc::new(RenderImage::new(smallvec::smallvec![image::Frame::new(
            raster(&theme::ORG_STUDIO_LIGHT)
        )]))
    });
    static DARK: LazyLock<Arc<RenderImage>> = LazyLock::new(|| {
        Arc::new(RenderImage::new(smallvec::smallvec![image::Frame::new(
            raster(&theme::ORG_STUDIO_DARK)
        )]))
    });
    let image = if theme::effective_theme_is_dark() {
        &*DARK
    } else {
        &*LIGHT
    };
    let viewport = window.viewport_size();
    let bounds = Bounds::new(
        point(px(0.), px(super::super::TITLEBAR_HEIGHT)),
        size(
            viewport.width,
            (viewport.height - px(super::super::TITLEBAR_HEIGHT)).max(px(0.)),
        ),
    );
    if bounds.is_empty() || region.is_empty() {
        return;
    }
    window.with_content_mask(Some(ContentMask { bounds: region }), |window| {
        // Crop with the content mask, not image bounds: each region must keep
        // the same UVs, including while the document or sidebar is moving.
        if window
            .paint_image(bounds, bounds, Corners::default(), image.clone(), 0, false)
            .is_err()
        {
            window.paint_quad(gpui::fill(
                bounds,
                rgb(theme::current_theme().overview_background),
            ));
        }
    });
}

/// Generate once per appearance. RenderImage expects BGRA, as do our minimaps.
pub(super) fn raster(theme: &Theme) -> image::RgbaImage {
    const SIZE: u32 = 512;
    const DITHER: [u8; 16] = [0, 8, 2, 10, 12, 4, 14, 6, 3, 11, 1, 9, 15, 7, 13, 5];
    let colors = [
        rgb(theme.overview_edge),
        rgb(theme.overview_background),
        rgb(theme.overview_highlight),
    ];
    image::RgbaImage::from_fn(SIZE, SIZE, |x, y| {
        let dx = ((x as f32 + 0.5) / SIZE as f32 - 0.5) / 0.85;
        let dy = ((y as f32 + 0.5) / SIZE as f32 - 0.18) / 0.8;
        let glow = (-2. * (dx * dx + dy * dy)).exp() * 2.;
        let index = usize::from(glow >= 1.);
        let t = glow - index as f32;
        let a = colors[index];
        let b = colors[index + 1];
        // Sub-channel ordered dithering avoids broad bands in dark gradients.
        let noise = (DITHER[((y % 4) * 4 + x % 4) as usize] as f32 + 0.5) / 16. - 0.5;
        let channel = |from: f32, to: f32| {
            ((from + (to - from) * t) * 255. + noise)
                .round()
                .clamp(0., 255.) as u8
        };
        image::Rgba([channel(a.b, b.b), channel(a.g, b.g), channel(a.r, b.r), 255])
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn backdrop_is_opaque_and_keeps_a_broad_upper_light() {
        for theme in [&theme::ORG_STUDIO_LIGHT, &theme::ORG_STUDIO_DARK] {
            let image = raster(theme);
            let light = image.get_pixel(256, 118);
            let edge = image.get_pixel(0, 511);
            let low = theme.overview_edge.to_le_bytes();
            let high = theme.overview_highlight.to_le_bytes();
            for pixel in image.pixels() {
                assert_eq!(pixel[3], 255);
                for channel in 0..3 {
                    assert!((low[channel]..=high[channel]).contains(&pixel[channel]));
                }
            }
            for channel in 0..3 {
                assert!(light[channel] > edge[channel]);
                assert!(light[channel].abs_diff(image.get_pixel(230, 118)[channel]) <= 1);
            }
        }
    }
}
