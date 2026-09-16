//! Shared interaction colors and compact toolbar metrics.
// Interaction colors route through the global theme so selection states stay
// legible in both palettes. Same-name functions keep existing call sites
// readable while resolving light/dark at paint time.
#[allow(non_snake_case)]
pub(crate) fn HOVER_BACKGROUND() -> u32 {
    palette(0xeaf1ff, 0x253444)
}

#[allow(non_snake_case)]
pub(crate) fn SELECTED_BACKGROUND() -> u32 {
    palette(0x3f78f2, 0x4da3ff)
}

#[allow(non_snake_case)]
pub(crate) fn SELECTED_HOVER_BACKGROUND() -> u32 {
    palette(0x3269df, 0x66b3ff)
}

fn palette(light: u32, dark: u32) -> u32 {
    if crate::theme::effective_theme_is_dark() {
        dark
    } else {
        light
    }
}

pub(crate) const QUICK_BAR_HEIGHT: f32 = 36.;
