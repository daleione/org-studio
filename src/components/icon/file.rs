//! File identity is a presentation concern; reuse syntax language detection for source files.
use gpui::{App, DevicePixels, ObjectFit, Pixels, SvgSize, Window, div, img, prelude::*, size};

use super::{Icon, ImageKey, cached_image, icon, raster_size};
use crate::file_manager::{EntryKind, FileEntry};

macro_rules! artwork {
    ($($variant:ident => $name:literal),* $(,)?) => {
        #[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
        pub(super) enum Artwork { $($variant,)* }

        impl Artwork {
            fn data(self, dark: bool) -> &'static [u8] {
                if !dark {
                    match self {
                        Self::Org => return include_bytes!("../../../assets/icons/files/org_light.svg"),
                        Self::Markdown => return include_bytes!("../../../assets/icons/files/markdown_light.svg"),
                        Self::Json => return include_bytes!("../../../assets/icons/files/json_light.svg"),
                        Self::Toml => return include_bytes!("../../../assets/icons/files/toml_light.svg"),
                        _ => {}
                    }
                }
                match self {
                    $(Self::$variant => include_bytes!(concat!("../../../assets/icons/files/", $name, ".svg")),)*
                }
            }
        }
    };
}

artwork! {
    Org => "org",
    Markdown => "markdown",
    Rust => "rust",
    Go => "go_gopher",
    Json => "json",
    Python => "python",
    JavaScript => "javascript",
    TypeScript => "typescript",
    Jsx => "react",
    Tsx => "react_ts",
    C => "c",
    Cpp => "cpp",
    Shell => "console",
    Sql => "database",
    Typst => "typst",
    Yaml => "yaml",
    Toml => "toml",
    Git => "git",
    Html => "html",
    Css => "css",
}

impl Artwork {
    fn for_entry(entry: &FileEntry) -> Option<Self> {
        match entry.kind {
            EntryKind::Parent | EntryKind::Directory | EntryKind::Symlink => return None,
            EntryKind::OrgFile => return Some(Self::Org),
            EntryKind::Markdown => return Some(Self::Markdown),
            _ => {}
        }
        if matches!(
            entry.path.file_name()?.to_str()?,
            ".gitignore" | ".gitattributes" | ".gitmodules"
        ) {
            return Some(Self::Git);
        }
        let extension = entry.path.extension()?.to_str()?.to_ascii_lowercase();
        let language =
            crate::syntax_highlighting::language_for_path(&entry.path).unwrap_or(&extension);
        Some(match language {
            "rust" => Self::Rust,
            "go" => Self::Go,
            "python" => Self::Python,
            "javascript" | "mjs" | "cjs" => Self::JavaScript,
            "typescript" | "mts" | "cts" => Self::TypeScript,
            "jsx" => Self::Jsx,
            "tsx" => Self::Tsx,
            "c" => Self::C,
            "cpp" => Self::Cpp,
            "bash" => Self::Shell,
            "sql" => Self::Sql,
            "typst" => Self::Typst,
            "json" | "jsonc" | "json5" | "jsonl" => Self::Json,
            "yaml" | "yml" => Self::Yaml,
            "toml" => Self::Toml,
            "html" | "htm" => Self::Html,
            "css" | "scss" | "sass" | "less" => Self::Css,
            _ => return None,
        })
    }
}

pub(crate) fn file_icon(
    entry: &FileEntry,
    extent: Pixels,
    color: u32,
    selected: bool,
    window: &Window,
    cx: &App,
) -> gpui::Div {
    let image = if let Some(artwork) = Artwork::for_entry(entry) {
        // Selection uses a dark background even in a light window. Preserve
        // mascot details and use the light-ink variants of monochrome marks.
        let dark = selected || crate::theme::effective_theme_is_dark();
        let raster = raster_size(extent, window);
        let image = cached_image(ImageKey::File(artwork, raster, dark), || {
            let renderer = cx.svg_renderer();
            let svg = renderer
                .parse_svg(artwork.data(dark))
                .expect("bundled file icon SVG");
            renderer
                .render_parsed(
                    &svg,
                    SvgSize::Size(size(
                        DevicePixels(i32::from(raster)),
                        DevicePixels(i32::from(raster)),
                    )),
                )
                .expect("file icon rasterization")
        });
        img(image)
            .size(extent)
            .object_fit(ObjectFit::Contain)
            .into_any_element()
    } else {
        icon(Icon::for_file(entry.kind), extent, color, window)
    };
    div().relative().flex_none().size(extent).child(image).when(
        entry.kind == EntryKind::Symlink,
        |view| {
            view.child(div().absolute().bottom_0().left_0().child(icon(
                Icon::Link,
                extent * 0.55,
                color,
                window,
            )))
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[gpui::test]
    fn file_artwork_distinguishes_formats_and_renders_in_both_appearances(
        cx: &mut gpui::TestAppContext,
    ) {
        for (name, kind, expected) in [
            ("notes.org", EntryKind::OrgFile, Artwork::Org),
            ("readme.md", EntryKind::Markdown, Artwork::Markdown),
            ("main.RS", EntryKind::CodeFile, Artwork::Rust),
            ("server.go", EntryKind::CodeFile, Artwork::Go),
            ("settings.JSON", EntryKind::Configuration, Artwork::Json),
            ("settings.jsonc", EntryKind::Configuration, Artwork::Json),
            ("worker.py", EntryKind::CodeFile, Artwork::Python),
            ("index.mjs", EntryKind::RegularFile, Artwork::JavaScript),
            ("types.ts", EntryKind::CodeFile, Artwork::TypeScript),
            ("view.jsx", EntryKind::CodeFile, Artwork::Jsx),
            ("view.tsx", EntryKind::CodeFile, Artwork::Tsx),
            ("main.c", EntryKind::CodeFile, Artwork::C),
            ("main.cpp", EntryKind::CodeFile, Artwork::Cpp),
            ("run.zsh", EntryKind::CodeFile, Artwork::Shell),
            ("query.sql", EntryKind::CodeFile, Artwork::Sql),
            ("paper.typ", EntryKind::CodeFile, Artwork::Typst),
            ("config.yml", EntryKind::Configuration, Artwork::Yaml),
            ("config.toml", EntryKind::Configuration, Artwork::Toml),
            (".gitignore", EntryKind::Configuration, Artwork::Git),
            ("index.html", EntryKind::RegularFile, Artwork::Html),
            ("style.css", EntryKind::RegularFile, Artwork::Css),
        ] {
            let mut entry = FileEntry::directory(name.into());
            entry.kind = kind;
            assert_eq!(Artwork::for_entry(&entry), Some(expected), "{name}");
            for dark in [false, true] {
                cx.update(|cx| {
                    let renderer = cx.svg_renderer();
                    let svg = renderer.parse_svg(expected.data(dark)).unwrap();
                    let image = renderer
                        .render_parsed(
                            &svg,
                            SvgSize::ExactSize(size(DevicePixels(32), DevicePixels(32))),
                        )
                        .unwrap();
                    assert!(
                        image
                            .as_bytes(0)
                            .unwrap()
                            .as_chunks::<4>()
                            .0
                            .iter()
                            .any(|pixel| pixel[3] > 0),
                        "{name}, dark={dark}"
                    );
                });
            }
        }
        let mut directory = FileEntry::directory("folder.rs".into());
        assert_eq!(Artwork::for_entry(&directory), None);
        directory.kind = EntryKind::Symlink;
        assert_eq!(Artwork::for_entry(&directory), None);
        directory.kind = EntryKind::RegularFile;
        directory.path = std::path::Path::new("unknown.bin").into();
        assert_eq!(Artwork::for_entry(&directory), None);
    }
}
