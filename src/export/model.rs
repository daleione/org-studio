use std::{ops::Range, path::PathBuf, sync::Arc};

pub type SourceRange = Range<usize>;

#[derive(Clone, Debug, Default)]
pub struct ExportDocument {
    pub meta: ExportMeta,
    pub blocks: Vec<ExportBlock>,
}

#[derive(Clone, Debug, Default)]
pub struct ExportMeta {
    pub title: Option<String>,
    pub author: Option<String>,
    pub date: Option<String>,
    pub language: Option<String>,
    pub toc: Option<bool>,
}

#[derive(Clone, Debug)]
pub enum ExportBlock {
    Heading {
        level: u8,
        content: Vec<ExportInline>,
        source: SourceRange,
    },
    Paragraph {
        content: Vec<ExportInline>,
        source: SourceRange,
    },
    List {
        ordered: bool,
        items: Vec<Vec<ExportBlock>>,
        source: SourceRange,
    },
    Quote {
        blocks: Vec<ExportBlock>,
        source: SourceRange,
    },
    Code {
        language: Option<String>,
        code: String,
        source: SourceRange,
    },
    Table {
        rows: Vec<Vec<Vec<ExportInline>>>,
        source: SourceRange,
    },
    Image {
        path: PathBuf,
        alt: Option<String>,
        caption: Option<String>,
        source: SourceRange,
    },
    GeneratedImage {
        svg: Arc<[u8]>,
        alt: String,
        size: (f32, f32),
        width: Option<ExportImageWidth>,
        source: SourceRange,
    },
    Rule {
        source: SourceRange,
    },
}

#[derive(Clone, Copy, Debug)]
pub enum ExportImageWidth {
    Pixels(f32),
    Percent(f32),
    Em(f32),
}

#[derive(Clone, Debug)]
pub enum ExportInline {
    Text(String),
    Strong(Vec<ExportInline>),
    Emphasis(Vec<ExportInline>),
    Underline(Vec<ExportInline>),
    Strike(Vec<ExportInline>),
    Code(String),
    Math(crate::org_syntax::math::Formula),
    Link {
        target: String,
        label: Vec<ExportInline>,
    },
    LineBreak,
}

impl ExportBlock {
    pub(super) fn has_math(&self) -> bool {
        match self {
            Self::Heading { content, .. } | Self::Paragraph { content, .. } => {
                content.iter().any(ExportInline::has_math)
            }
            Self::List { items, .. } => items.iter().flatten().any(Self::has_math),
            Self::Quote { blocks, .. } => blocks.iter().any(Self::has_math),
            Self::Table { rows, .. } => rows.iter().flatten().flatten().any(ExportInline::has_math),
            _ => false,
        }
    }
    pub fn source(&self) -> &SourceRange {
        match self {
            Self::Heading { source, .. }
            | Self::Paragraph { source, .. }
            | Self::List { source, .. }
            | Self::Quote { source, .. }
            | Self::Code { source, .. }
            | Self::Table { source, .. }
            | Self::Image { source, .. }
            | Self::GeneratedImage { source, .. }
            | Self::Rule { source } => source,
        }
    }
}

impl ExportInline {
    fn has_math(&self) -> bool {
        match self {
            Self::Math(_) => true,
            Self::Strong(children)
            | Self::Emphasis(children)
            | Self::Underline(children)
            | Self::Strike(children)
            | Self::Link {
                label: children, ..
            } => children.iter().any(Self::has_math),
            _ => false,
        }
    }
}
