use super::{
    headers,
    value::{Data, Value},
};
use crate::{
    document::{DocumentSnapshot, FileStamp, TextSnapshot},
    org_syntax::{BlockArena, BlockKind, parse},
};
use std::{
    collections::HashMap,
    path::{Path, PathBuf},
};

pub(super) struct Document {
    pub snapshot: DocumentSnapshot,
    pub path: PathBuf,
    pub blocks: BlockArena,
    names: HashMap<String, usize>,
}

#[derive(Clone)]
pub(crate) struct DocumentVersion {
    pub(crate) path: PathBuf,
    snapshot: DocumentSnapshot,
    stamp: FileStamp,
}

impl DocumentVersion {
    pub(crate) fn document_id(&self) -> crate::document::DocumentId {
        self.snapshot.document_id()
    }
    pub(crate) fn path_matches(&self, path: &Path) -> bool {
        same_path(path, &self.path)
    }

    pub(crate) fn matches(&self, current: Option<&DocumentSnapshot>) -> bool {
        match current {
            Some(current) => {
                current.document_id() == self.snapshot.document_id()
                    && current.revision() == self.snapshot.revision()
            }
            None => std::fs::read(&self.path).is_ok_and(|bytes| {
                let bytes = bytes.strip_prefix(b"\xef\xbb\xbf").unwrap_or(&bytes);
                FileStamp::from_loaded(&self.path, bytes).is_ok_and(|stamp| {
                    stamp.resource_id == self.stamp.resource_id
                        && stamp.len == self.stamp.len
                        && stamp.content_hash == self.stamp.content_hash
                })
            }),
        }
    }
}

pub(super) fn same_path(left: &Path, right: &Path) -> bool {
    path_key(left) == path_key(right)
        || FileStamp::from_loaded(left, &[])
            .ok()
            .zip(FileStamp::from_loaded(right, &[]).ok())
            .is_some_and(|(a, b)| a.resource_id.is_some() && a.resource_id == b.resource_id)
}

pub(super) fn path_key(path: &Path) -> PathBuf {
    std::fs::canonicalize(path).unwrap_or_else(|_| {
        if path.is_absolute() {
            path.to_owned()
        } else {
            std::env::current_dir().unwrap_or_default().join(path)
        }
    })
}

impl Document {
    pub fn new(path: PathBuf, snapshot: DocumentSnapshot) -> Result<Self, String> {
        let blocks = parse(&snapshot);
        let mut names = HashMap::new();
        for (index, node) in blocks.nodes().iter().enumerate() {
            if (matches!(
                node.kind,
                BlockKind::SourceBlock { .. } | BlockKind::TableRow
            ) || is_call(&snapshot, &node.kind, node.content))
                && let Some(name) = headers::source_name(&snapshot, &blocks, index)
                && names.insert(name.clone(), index).is_some()
            {
                return Err(format!("Duplicate Babel name: {name}"));
            }
        }
        Ok(Self {
            snapshot,
            path,
            blocks,
            names,
        })
    }

    pub fn named(&self, name: &str) -> Result<usize, String> {
        self.names
            .get(name)
            .copied()
            .ok_or_else(|| format!("Unknown Babel reference: {name}"))
    }

    pub fn version(&self) -> DocumentVersion {
        let bytes = self.snapshot.copy_range(crate::document::ByteRange::new(
            0,
            self.snapshot.len_bytes(),
        ));
        DocumentVersion {
            path: self.path.clone(),
            snapshot: self.snapshot.clone(),
            stamp: FileStamp::from_loaded(&self.path, bytes.as_bytes())
                .unwrap_or_else(|_| FileStamp::detached(bytes.as_bytes())),
        }
    }

    pub fn table(&self, index: usize) -> Result<Data, String> {
        let mut rows = Vec::new();
        for node in &self.blocks.nodes()[index..] {
            if !matches!(node.kind, BlockKind::TableRow) {
                break;
            }
            let line = self.snapshot.copy_range(node.content);
            let parsed =
                crate::document::table::parse_line(&line, crate::document::DocumentFormat::Org);
            if parsed.separator {
                rows.push(Value::Hline);
                continue;
            }
            let cells = parsed
                .cells
                .iter()
                .map(|c| {
                    let text = &line[c.text_range.clone()];
                    // Org table reads infer numbers but keep source markup/entities intact.
                    if text.parse::<f64>().is_ok() {
                        Value::literal(text).map(|v| v.unwrap_or_else(|| Value::Text(text.into())))
                    } else {
                        Ok(Value::Text(text.into()))
                    }
                })
                .collect::<Result<_, String>>()?;
            rows.push(Value::List(cells));
        }
        Ok(Data {
            value: Value::List(rows),
            ..Data::default()
        })
    }
}

pub(super) fn is_call(
    snapshot: &DocumentSnapshot,
    kind: &BlockKind,
    content: crate::document::ByteRange,
) -> bool {
    matches!(kind, BlockKind::Keyword)
        && headers::keyword(&snapshot.copy_range(content))
            .is_some_and(|(key, _)| key.eq_ignore_ascii_case("CALL"))
}
