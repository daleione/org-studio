use std::{
    collections::{HashMap, VecDeque},
    sync::{Arc, Mutex, OnceLock},
};

use gpui::{Image, ImageFormat};

use crate::{
    document::{ByteRange, TextSnapshot},
    org_syntax::BlockId,
};

use super::{
    CodeRowRole,
    markdown::{MarkdownBlock, MarkdownKind},
};

#[derive(Clone, Debug)]
pub(crate) enum DiagramProjection {
    Ready {
        image: Arc<Image>,
        dimensions: (f32, f32),
        warnings: Arc<[Arc<str>]>,
    },
    Error {
        diagnostics: Arc<[Arc<str>]>,
    },
}

impl DiagramProjection {
    pub(crate) fn dimensions(&self) -> Option<(f32, f32)> {
        match self {
            Self::Ready { dimensions, .. } => Some(*dimensions),
            Self::Error { diagnostics } => {
                let lines = 1 + diagnostics.len().min(3);
                let body_height = 24.0 + lines as f32 * 18.0 + (lines - 1) as f32 * 4.0;
                Some((640.0, body_height.max(72.0)))
            }
        }
    }

    pub(crate) fn layout_extra_height(&self) -> f32 {
        match self {
            Self::Ready { warnings, .. } => {
                // Language/copy label, card padding, and the optional warning line.
                46.0 + if warnings.is_empty() { 0.0 } else { 22.0 }
            }
            Self::Error { .. } => 22.0,
        }
    }
}

const DIAGRAM_CACHE_CAPACITY: usize = 64;

#[derive(Default)]
struct DiagramCache {
    entries: HashMap<Arc<str>, DiagramProjection>,
    order: VecDeque<Arc<str>>,
}

impl DiagramCache {
    fn get(&mut self, source: &str) -> Option<DiagramProjection> {
        let projection = self.entries.get(source)?.clone();
        self.order.retain(|candidate| candidate.as_ref() != source);
        self.order.push_back(Arc::from(source));
        Some(projection)
    }

    fn insert(&mut self, source: &str, projection: DiagramProjection) {
        self.order.retain(|candidate| candidate.as_ref() != source);
        let source: Arc<str> = Arc::from(source);
        self.entries.insert(source.clone(), projection);
        self.order.push_back(source);
        while self.entries.len() > DIAGRAM_CACHE_CAPACITY {
            if let Some(expired) = self.order.pop_front() {
                self.entries.remove(expired.as_ref());
            }
        }
    }
}

fn diagram_cache() -> &'static Mutex<DiagramCache> {
    static CACHE: OnceLock<Mutex<DiagramCache>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(DiagramCache::default()))
}

pub(crate) fn is_plantuml_language(language: Option<&str>) -> bool {
    language.is_some_and(|language| {
        matches!(
            language.trim().to_ascii_lowercase().as_str(),
            "plantuml" | "puml" | "uml"
        )
    })
}

pub(crate) fn build_markdown_diagrams(
    text: &dyn TextSnapshot,
    blocks: &[MarkdownBlock],
) -> HashMap<BlockId, DiagramProjection> {
    markdown_diagram_ranges(blocks)
        .into_iter()
        .map(|(block_id, _, body)| (block_id as BlockId, render(&text.copy_range(body))))
        .collect()
}

fn markdown_diagram_ranges(blocks: &[MarkdownBlock]) -> Vec<(usize, usize, ByteRange)> {
    let mut diagrams = Vec::new();
    let mut index = 0;
    while index < blocks.len() {
        let MarkdownKind::Code {
            language,
            role: CodeRowRole::Open,
        } = &blocks[index].kind
        else {
            index += 1;
            continue;
        };
        if !is_plantuml_language(language.as_deref()) {
            index += 1;
            continue;
        }
        let open = index;
        let mut close = open;
        let mut body_start = None;
        let mut body_end = blocks[open].source.end;
        index += 1;
        while index < blocks.len() {
            match &blocks[index].kind {
                MarkdownKind::Code {
                    role: CodeRowRole::Body,
                    ..
                } => {
                    body_start.get_or_insert(blocks[index].source.start);
                    body_end = blocks[index].source.end;
                    close = index;
                }
                MarkdownKind::Code {
                    role: CodeRowRole::Close,
                    ..
                } => {
                    close = index;
                    index += 1;
                    break;
                }
                _ => break,
            }
            index += 1;
        }
        diagrams.push((
            open,
            close,
            ByteRange {
                start: body_start.unwrap_or(body_end),
                end: body_end,
            },
        ));
    }
    diagrams
}

fn render(source: &str) -> DiagramProjection {
    if let Ok(mut cache) = diagram_cache().lock()
        && let Some(projection) = cache.get(source)
    {
        return projection;
    }

    let projection = match typstuml::render::render_source(source, typstuml::render::Format::Svg) {
        Ok(rendered) => {
            let dimensions = rendered_dimensions(rendered.size, &rendered.page_sizes);
            DiagramProjection::Ready {
                image: Arc::new(Image::from_bytes(ImageFormat::Svg, rendered.bytes)),
                dimensions,
                warnings: rendered
                    .warnings
                    .into_iter()
                    .map(|warning| Arc::<str>::from(warning.to_string()))
                    .collect::<Vec<_>>()
                    .into(),
            }
        }
        Err(error) => DiagramProjection::Error {
            diagnostics: error
                .to_diagnostics()
                .into_iter()
                .map(|diagnostic| Arc::<str>::from(diagnostic.to_string()))
                .collect::<Vec<_>>()
                .into(),
        },
    };
    if let Ok(mut cache) = diagram_cache().lock() {
        cache.insert(source, projection.clone());
    }
    projection
}

fn rendered_dimensions(
    size: Option<typstuml::render::RenderSize>,
    page_sizes: &[typstuml::render::RenderSize],
) -> (f32, f32) {
    let size = size.or_else(|| {
        (!page_sizes.is_empty()).then(|| typstuml::render::RenderSize {
            width_pt: page_sizes
                .iter()
                .map(|page| page.width_pt)
                .fold(0.0, f32::max),
            // TypstUML merges multiple SVG pages vertically with a 2 pt gap.
            height_pt: page_sizes.iter().map(|page| page.height_pt).sum::<f32>()
                + 2.0 * page_sizes.len().saturating_sub(1) as f32,
        })
    });
    size.map_or((640.0, 360.0), |size| {
        // Typst reports points, while GPUI lays SVG images out in CSS pixels at 96 DPI.
        (size.width_pt * 4.0 / 3.0, size.height_pt * 4.0 / 3.0)
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::DocumentSnapshot;

    #[test]
    fn recognizes_common_plantuml_language_aliases() {
        for language in ["plantuml", "PlantUML", "puml", "uml"] {
            assert!(is_plantuml_language(Some(language)));
        }
        assert!(!is_plantuml_language(Some("rust")));
        assert!(!is_plantuml_language(None));
    }

    #[test]
    fn renders_a_markdown_plantuml_fence_to_memory_svg() {
        let text = DocumentSnapshot::from_utf8(
            b"```plantuml\n@startuml\nAlice -> Bob: hello\n@enduml\n```\n".to_vec(),
        )
        .unwrap();
        let (blocks, _) = crate::preview::markdown::parse_markdown(&text);
        let diagrams = build_markdown_diagrams(&text, &blocks);

        let DiagramProjection::Ready {
            image, dimensions, ..
        } = diagrams.get(&0).expect("PlantUML block is projected")
        else {
            panic!("valid PlantUML should render")
        };
        assert_eq!(image.format(), ImageFormat::Svg);
        assert!(String::from_utf8_lossy(image.bytes()).contains("<svg"));
        assert!(dimensions.0 > 0.0 && dimensions.1 > 0.0);
    }

    #[test]
    fn keeps_render_errors_as_inline_diagnostics() {
        let projection = render("not a PlantUML diagram");
        let DiagramProjection::Error { diagnostics } = projection else {
            panic!("invalid PlantUML should not render")
        };
        assert!(!diagnostics.is_empty());
    }

    #[test]
    fn merged_pages_use_the_full_svg_extent() {
        let dimensions = rendered_dimensions(
            None,
            &[
                typstuml::render::RenderSize {
                    width_pt: 100.0,
                    height_pt: 80.0,
                },
                typstuml::render::RenderSize {
                    width_pt: 120.0,
                    height_pt: 60.0,
                },
            ],
        );
        assert_eq!(dimensions, (160.0, 142.0 * 4.0 / 3.0));
    }
}
