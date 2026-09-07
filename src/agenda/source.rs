use std::{
    fs, io,
    path::{Path, PathBuf},
    sync::Arc,
};

use crate::{
    document::{DocumentSnapshot, FileStamp},
    org_semantic::{OrgAnalysisSnapshot, analyze},
    org_syntax,
};

use super::{
    FileAgendaShard, FileId, HeadingFingerprint, OrgAnchor, SourceLocator, SourceVersion, TaskKey,
    TaskRecord,
};

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct DiscoveredSource {
    pub(crate) path: PathBuf,
}

pub(crate) fn discover_sources(roots: &[PathBuf]) -> io::Result<Vec<DiscoveredSource>> {
    let mut result = Vec::new();
    for root in roots {
        discover(root, &mut result)?;
    }
    result.sort_by(|left, right| left.path.cmp(&right.path));
    result.dedup_by(|left, right| left.path == right.path);
    Ok(result)
}

fn discover(path: &Path, result: &mut Vec<DiscoveredSource>) -> io::Result<()> {
    let metadata = match fs::metadata(path) {
        Ok(value) => value,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error),
    };
    if metadata.is_file() {
        if path
            .extension()
            .is_some_and(|extension| extension.eq_ignore_ascii_case("org"))
        {
            result.push(DiscoveredSource {
                path: fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf()),
            });
        }
        return Ok(());
    }
    let mut entries = fs::read_dir(path)?.collect::<Result<Vec<_>, _>>()?;
    entries.sort_by_key(|entry| entry.path());
    for entry in entries {
        let file_type = entry.file_type()?;
        if file_type.is_symlink() {
            continue;
        }
        if file_type.is_dir() || file_type.is_file() {
            discover(&entry.path(), result)?;
        }
    }
    Ok(())
}

pub(crate) fn shard_from_disk(
    file: FileId,
    generation: u64,
    path: PathBuf,
) -> io::Result<FileAgendaShard> {
    let bytes = fs::read(&path)?;
    let diagnostics = super::compatibility_diagnostics(&String::from_utf8_lossy(&bytes))
        .into_iter()
        .map(|message| super::AgendaDiagnostic {
            path: Some(Arc::new(path.clone())),
            message,
        })
        .collect();
    let stamp = Arc::new(FileStamp::from_loaded(&path, &bytes)?);
    let snapshot = DocumentSnapshot::from_utf8(bytes)
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, format!("{error:?}")))?;
    let analysis = analyze(&snapshot, Arc::new(org_syntax::parse(&snapshot)));
    Ok(shard_from_analysis(
        file,
        generation,
        Arc::new(path),
        SourceVersion::Disk(stamp),
        &analysis,
    )
    .with_diagnostics(diagnostics))
}

pub(crate) fn shard_from_live(
    file: FileId,
    generation: u64,
    path: Arc<PathBuf>,
    analysis: &OrgAnalysisSnapshot,
) -> FileAgendaShard {
    shard_from_analysis(
        file,
        generation,
        path,
        SourceVersion::Live {
            document: analysis.document_id,
            revision: analysis.revision,
        },
        analysis,
    )
}

fn shard_from_analysis(
    file: FileId,
    generation: u64,
    path: Arc<PathBuf>,
    version: SourceVersion,
    analysis: &OrgAnalysisSnapshot,
) -> FileAgendaShard {
    let mut syntax_to_task = std::collections::HashMap::new();
    let heading_titles = analysis
        .headings
        .iter()
        .map(|heading| (heading.syntax_id, heading.title.clone()))
        .collect::<std::collections::HashMap<_, _>>();
    let allowed_todo_states = analysis.config.todo_keywords();
    let mut tasks = Vec::new();
    for heading in analysis.headings.iter() {
        let Some(todo) = heading.todo.as_ref() else {
            continue;
        };
        let local = tasks.len() as u32;
        let key = TaskKey {
            file,
            local,
            shard_generation: generation,
        };
        let parent = heading
            .parent
            .and_then(|syntax| syntax_to_task.get(&syntax).copied());
        syntax_to_task.insert(heading.syntax_id, key);
        let locator = SourceLocator {
            file,
            path: path.clone(),
            version: version.clone(),
            heading_range: heading.source,
            title_range: heading.content,
            anchor: extract_anchor_from_heading(heading),
            fingerprint: HeadingFingerprint {
                level: heading.level,
                title: heading.title.clone(),
                parent_title: heading
                    .parent
                    .and_then(|parent| heading_titles.get(&parent).cloned()),
            },
        };
        tasks.push(TaskRecord {
            key,
            source: locator,
            level: heading.level,
            title: heading.title.clone(),
            todo: todo.keyword.clone(),
            todo_kind: todo.kind,
            priority: heading.priority,
            effective_tags: heading.effective_tags.clone(),
            category: analysis.config.category.clone(),
            properties: heading.properties.clone(),
            parent,
            timestamps: heading.timestamps.clone(),
            allowed_todo_states: allowed_todo_states.clone(),
        });
    }
    FileAgendaShard::new(file, path, version, tasks)
}

fn extract_anchor_from_heading(heading: &crate::org_semantic::OrgHeading) -> Option<OrgAnchor> {
    for wanted in ["ID", "CUSTOM_ID"] {
        if let Some((kind, value)) = heading
            .properties
            .iter()
            .find(|(key, _)| key.eq_ignore_ascii_case(wanted))
        {
            return Some(OrgAnchor {
                kind: kind.clone(),
                value: value.clone(),
            });
        }
    }
    None
}
