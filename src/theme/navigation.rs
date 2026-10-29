use std::cell::Cell;

use block2::StackBlock;
use objc2_app_kit::{
    NSAppearance, NSAppearanceNameAqua, NSAppearanceNameDarkAqua, NSColor, NSColorSpace,
};

/// Shared UI semantics; syntax highlighting is deliberately not an input.
#[derive(Clone, Copy)]
pub(crate) struct NavigationColors {
    pub accent: u32,
    pub selected: u32,
    pub selected_foreground: u32,
    pub inactive: u32,
    pub secondary: u32,
    pub focus: u32,
}

fn rgb(color: &NSColor) -> u32 {
    let color = color
        .colorUsingColorSpace(&NSColorSpace::sRGBColorSpace())
        .expect("RGB system color");
    let (mut r, mut g, mut b, mut a) = (0., 0., 0., 0.);
    // All output pointers are distinct valid CGFloat values, and the color
    // was explicitly converted to the RGB color space above.
    unsafe {
        color.getRed_green_blue_alpha(&mut r, &mut g, &mut b, &mut a);
    }
    let channel = |value: f64| (value.clamp(0., 1.) * 255.).round() as u32;
    (channel(r) << 16) | (channel(g) << 8) | channel(b)
}

pub(crate) fn navigation_colors() -> NavigationColors {
    let dark = super::effective_theme_is_dark();
    let palette = Cell::new(NavigationColors {
        accent: super::current_theme().accent,
        selected: super::current_theme().accent_bg,
        selected_foreground: super::current_theme().foreground,
        inactive: if dark { 0x263748 } else { 0xe5e9ef },
        secondary: if dark { 0xa7b4c4 } else { 0x5f6772 },
        focus: super::current_theme().accent,
    });
    // These immutable names are provided by AppKit on every supported macOS.
    let name = unsafe {
        if dark {
            NSAppearanceNameDarkAqua
        } else {
            NSAppearanceNameAqua
        }
    };
    let appearance = NSAppearance::appearanceNamed(name).expect("standard macOS appearance");
    appearance.performAsCurrentDrawingAppearance(&StackBlock::new(|| {
        palette.set(NavigationColors {
            accent: rgb(&NSColor::controlAccentColor()),
            selected: rgb(&NSColor::selectedContentBackgroundColor()),
            selected_foreground: rgb(&NSColor::alternateSelectedControlTextColor()),
            focus: rgb(&NSColor::keyboardFocusIndicatorColor()),
            ..palette.get()
        });
    }));
    let mut colors = palette.get();
    colors.selected_foreground = readable_text(colors.selected_foreground, colors.selected);
    colors.accent = contrasting_graphic(colors.accent, super::current_theme().surface);
    colors.focus = contrasting_graphic(colors.focus, super::current_theme().surface);
    colors
}

fn luminance(rgb: u32) -> f64 {
    let channel = |shift: u32| {
        let value = ((rgb >> shift) & 255_u32) as f64 / 255.;
        if value <= 0.04045 {
            value / 12.92
        } else {
            ((value + 0.055) / 1.055).powf(2.4)
        }
    };
    0.2126 * channel(16) + 0.7152 * channel(8) + 0.0722 * channel(0)
}

fn contrast(a: u32, b: u32) -> f64 {
    let (a, b) = (luminance(a), luminance(b));
    (a.max(b) + 0.05) / (a.min(b) + 0.05)
}

fn readable_text(foreground: u32, background: u32) -> u32 {
    if contrast(foreground, background) >= 4.5 {
        foreground
    } else if contrast(0, background) >= contrast(0xffffff, background) {
        0
    } else {
        0xffffff
    }
}

fn contrasting_graphic(color: u32, background: u32) -> u32 {
    let target = readable_text(color, background);
    for step in 0..=20 {
        let weight = step as f64 / 20.;
        let channel = |shift: u32| {
            let original = ((color >> shift) & 255_u32) as f64;
            let target = ((target >> shift) & 255_u32) as f64;
            (original * (1. - weight) + target * weight).round() as u32
        };
        let candidate = (channel(16) << 16) | (channel(8) << 8) | channel(0);
        if contrast(candidate, background) >= 3. {
            return candidate;
        }
    }
    target
}
