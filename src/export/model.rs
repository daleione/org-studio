use std::{ops::Range, path::PathBuf};

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
    Rule {
        source: SourceRange,
    },
}

#[derive(Clone, Debug)]
pub enum ExportInline {
    Text(String),
    Strong(Vec<ExportInline>),
    Emphasis(Vec<ExportInline>),
    Underline(Vec<ExportInline>),
    Strike(Vec<ExportInline>),
    Code(String),
    Link {
        target: String,
        label: Vec<ExportInline>,
    },
    LineBreak,
}

impl ExportBlock {
    pub fn source(&self) -> &SourceRange {
        match self {
            Self::Heading { source, .. }
            | Self::Paragraph { source, .. }
            | Self::List { source, .. }
            | Self::Quote { source, .. }
            | Self::Code { source, .. }
            | Self::Table { source, .. }
            | Self::Image { source, .. }
            | Self::Rule { source } => source,
        }
    }
}
