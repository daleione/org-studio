//! Runtime system symbols, cached by their actual raster size and tint.
//! Callers select a semantic icon, never a glyph, font, or asset path.

use std::{cell::RefCell, collections::HashMap, sync::Arc};

use gpui::{AnyElement, ObjectFit, Pixels, RenderImage, Window, div, img, prelude::*};
use objc2_app_kit::{NSImage, NSImageSymbolConfiguration};
use objc2_foundation::NSString;

use crate::file_manager::EntryKind;

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(crate) enum Icon {
    Folder,
    Document,
    Text,
    PlainText,
    RichText,
    Code,
    Configuration,
    Table,
    Image,
    Audio,
    Video,
    Archive,
    Link,
    ChevronRight,
    ChevronDown,
    Plus,
    More,
    Warning,
    ArrowUp,
    ArrowDown,
    Check,
}

impl Icon {
    fn symbol(self) -> &'static str {
        match self {
            Self::Folder => "folder",
            Self::Document => "doc",
            Self::Text => "doc.text",
            Self::PlainText => "doc.plaintext",
            Self::RichText => "doc.richtext",
            Self::Code => "curlybraces",
            Self::Configuration => "slider.horizontal.3",
            Self::Table => "tablecells",
            Self::Image => "photo",
            Self::Audio => "waveform",
            Self::Video => "film",
            Self::Archive => "archivebox",
            Self::Link => "link",
            Self::ChevronRight => "chevron.right",
            Self::ChevronDown => "chevron.down",
            Self::Plus => "plus",
            Self::More => "ellipsis",
            Self::Warning => "exclamationmark.triangle",
            Self::ArrowUp => "arrow.up",
            Self::ArrowDown => "arrow.down",
            Self::Check => "checkmark",
        }
    }

    pub(crate) fn for_file(kind: EntryKind) -> Self {
        match kind {
            EntryKind::Parent => Self::ArrowUp,
            EntryKind::Directory => Self::Folder,
            EntryKind::OrgFile | EntryKind::Markdown => Self::Text,
            EntryKind::Text => Self::PlainText,
            EntryKind::Pdf => Self::RichText,
            EntryKind::CodeFile => Self::Code,
            EntryKind::Configuration => Self::Configuration,
            EntryKind::Data => Self::Table,
            EntryKind::Image => Self::Image,
            EntryKind::Audio => Self::Audio,
            EntryKind::Video => Self::Video,
            EntryKind::Archive => Self::Archive,
            EntryKind::Symlink => Self::Document,
            EntryKind::RegularFile => Self::Document,
        }
    }
}

pub(crate) fn file_icon(kind: EntryKind, size: Pixels, color: u32, window: &Window) -> gpui::Div {
    div()
        .relative()
        .flex_none()
        .size(size)
        .child(icon(Icon::for_file(kind), size, color, window))
        .when(kind == EntryKind::Symlink, |view| {
            view.child(div().absolute().bottom_0().left_0().child(icon(
                Icon::Link,
                size * 0.55,
                color,
                window,
            )))
        })
}

type SymbolCache = HashMap<(Icon, u16, u32), Arc<RenderImage>>;
thread_local! { static SYMBOLS: RefCell<SymbolCache> = RefCell::default(); }

pub(crate) fn icon(symbol: Icon, size: Pixels, color: u32, window: &Window) -> AnyElement {
    let raster_size = (f32::from(size) * window.scale_factor()).ceil().max(1.) as u16;
    let key = (symbol, raster_size, color);
    let image = SYMBOLS.with(|cache| {
        let mut cache = cache.borrow_mut();
        if let Some(image) = cache.get(&key) {
            return image.clone();
        }
        if cache.len() >= 256 {
            cache.clear();
        }
        let image = render_symbol(symbol, raster_size, color);
        cache.insert(key, image.clone());
        image
    });
    img(image)
        .size(size)
        .object_fit(ObjectFit::Contain)
        .into_any_element()
}

fn render_symbol(symbol: Icon, size: u16, color: u32) -> Arc<RenderImage> {
    let name = NSString::from_str(symbol.symbol());
    let original = NSImage::imageWithSystemSymbolName_accessibilityDescription(&name, None)
        .or_else(|| {
            NSImage::imageWithSystemSymbolName_accessibilityDescription(
                &NSString::from_str(if symbol == Icon::Folder {
                    "folder"
                } else {
                    "doc"
                }),
                None,
            )
        })
        .expect("macOS 13 provides the standard document and folder symbols");
    let configuration =
        NSImageSymbolConfiguration::configurationWithPointSize_weight(f64::from(size), 0.);
    let image = original
        .imageWithSymbolConfiguration(&configuration)
        .unwrap_or(original);
    let data = image
        .TIFFRepresentation()
        .expect("system symbol rasterization");
    let mut pixels = image::load_from_memory(&data.to_vec())
        .expect("AppKit TIFF image")
        .into_rgba8();
    for pixel in pixels.pixels_mut() {
        // RenderImage uploads BGRA pixels. AppKit/image decode yields RGBA;
        // tint in upload order so the system blue accent stays blue.
        pixel.0[0] = color as u8;
        pixel.0[1] = (color >> 8) as u8;
        pixel.0[2] = (color >> 16) as u8;
    }
    Arc::new(RenderImage::new(smallvec::smallvec![image::Frame::new(
        pixels
    )]))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn system_symbols_upload_the_requested_color_in_bgra_order() {
        let image = render_symbol(Icon::Folder, 32, 0x1256ab);
        let pixels = image.as_bytes(0).unwrap();
        assert!(pixels.as_chunks::<4>().0.iter().any(|pixel| pixel[3] > 0));
        for pixel in pixels
            .as_chunks::<4>()
            .0
            .iter()
            .filter(|pixel| pixel[3] > 0)
        {
            assert_eq!(&pixel[..3], &[0xab, 0x56, 0x12]);
        }
    }
}
