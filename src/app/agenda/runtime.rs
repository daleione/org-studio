use super::worker::LatestRequestWorker;
use crate::{
    agenda::{
        AgendaIndexSnapshot, FileAgendaShard, FileId, discover_sources, shard_from_disk,
        shard_from_live,
    },
    app::WorkspaceWindow,
    document::{DocumentSnapshot, TextSnapshot},
};
use gpui::{Context, Task};
use std::{path::PathBuf, sync::Arc, time::Duration};

pub(super) struct ScanRequest {
    pub identities: std::collections::BTreeMap<PathBuf, FileId>,
    pub roots: Vec<PathBuf>,
    pub previous: Arc<AgendaIndexSnapshot>,
    pub generation: u64,
    pub live: Option<(PathBuf, DocumentSnapshot)>,
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn scan_preserves_live_text_and_file_identity_after_removal() {
        let root = std::env::temp_dir().join(format!("agenda-runtime-{}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        let path = root.join("a.org");
        std::fs::write(&path, "* TODO Disk\n").unwrap();
        let first = scan(ScanRequest {
            identities: Default::default(),
            roots: vec![root.clone()],
            previous: Arc::default(),
            generation: 1,
            live: Some((
                path.clone(),
                DocumentSnapshot::from_utf8(b"* TODO Unsaved\n".to_vec()).unwrap(),
            )),
        });
        assert_eq!(first.shards[0].tasks[0].title.as_ref(), "Unsaved");
        let id = first.shards[0].file;
        let mut index = crate::agenda::AgendaIndex::default();
        for shard in first.shards {
            index.replace(shard);
        }
        std::fs::remove_file(&path).unwrap();
        let other = root.join("b.org");
        std::fs::write(&other, "* TODO Other\n").unwrap();
        let second = scan(ScanRequest {
            identities: first.identities,
            roots: vec![root.clone()],
            previous: index.snapshot(),
            generation: 2,
            live: None,
        });
        assert_eq!(second.shards.len(), 1);
        assert_ne!(second.shards[0].file, id);
        assert_eq!(
            second.identities[&std::fs::canonicalize(&root).unwrap().join("a.org")],
            id
        );
        std::fs::remove_file(other).unwrap();
        std::fs::remove_dir(root).unwrap();
    }
}

pub(super) struct ScanResult {
    pub identities: std::collections::BTreeMap<PathBuf, FileId>,
    pub shards: Vec<FileAgendaShard>,
    pub errors: Vec<Arc<str>>,
}

pub(super) fn scan(mut request: ScanRequest) -> ScanResult {
    if let Some((path, _)) = &mut request.live {
        *path = std::fs::canonicalize(&*path).unwrap_or_else(|_| path.clone());
    }
    request.roots = request
        .roots
        .into_iter()
        .map(|path| std::fs::canonicalize(&path).unwrap_or(path))
        .collect();
    let mut sources = match discover_sources(&request.roots) {
        Ok(sources) => sources,
        Err(error) => {
            return ScanResult {
                identities: request.identities,
                shards: request
                    .previous
                    .files
                    .iter()
                    .map(|shard| shard.as_ref().clone())
                    .collect(),
                errors: vec![Arc::from(error.to_string())],
            };
        }
    };
    if let Some((path, _)) = &request.live
        && path
            .extension()
            .is_some_and(|extension| extension.eq_ignore_ascii_case("org"))
        && request.roots.iter().any(|root| path.starts_with(root))
        && !sources.iter().any(|source| source.path == *path)
    {
        sources.push(crate::agenda::DiscoveredSource { path: path.clone() });
    }
    let mut next = request
        .identities
        .values()
        .map(|id| id.0)
        .max()
        .unwrap_or(0)
        + 1;
    let mut result = ScanResult {
        identities: request.identities,
        shards: Vec::new(),
        errors: Vec::new(),
    };
    for source in sources {
        let file = *result
            .identities
            .entry(source.path.clone())
            .or_insert_with(|| {
                let file = FileId(next);
                next += 1;
                file
            });
        if let Some((path, snapshot)) = &request.live
            && *path == source.path
        {
            if let Some(previous) = request
                .previous
                .files
                .iter()
                .find(|shard| shard.file == file)
                && matches!(previous.version, crate::agenda::SourceVersion::Live { document, revision } if document == snapshot.document_id() && revision == snapshot.revision())
            {
                result.shards.push(previous.as_ref().clone());
                continue;
            }
            let analysis = crate::org_semantic::analyze(
                snapshot,
                Arc::new(crate::org_syntax::parse(snapshot)),
            );
            result.shards.push(shard_from_live(
                file,
                request.generation,
                Arc::new(path.clone()),
                &analysis,
            ));
        } else {
            if let Some(previous) = request
                .previous
                .files
                .iter()
                .find(|shard| shard.file == file)
                && let crate::agenda::SourceVersion::Disk(expected) = &previous.version
                && crate::document::FileStamp::read(&source.path)
                    .is_ok_and(|actual| actual == **expected)
            {
                result.shards.push(previous.as_ref().clone());
                continue;
            }
            match shard_from_disk(file, request.generation, source.path.clone()) {
                Ok(shard) => result.shards.push(shard),
                Err(error) => result
                    .errors
                    .push(Arc::from(format!("{}: {error}", source.path.display()))),
            }
        }
    }
    result
}

pub(super) struct AgendaRuntime {
    pub initialized: bool,
    pub identities: std::collections::BTreeMap<PathBuf, FileId>,
    pub worker: LatestRequestWorker<ScanRequest, ScanResult>,
    pub live: Option<(PathBuf, DocumentSnapshot)>,
    pub pump: Option<Task<()>>,
    pub watch: Option<Task<()>>,
}

impl Default for AgendaRuntime {
    fn default() -> Self {
        Self {
            initialized: false,
            identities: Default::default(),
            worker: LatestRequestWorker::spawn(scan),
            live: None,
            pump: None,
            watch: None,
        }
    }
}

impl WorkspaceWindow {
    pub(crate) fn sync_agenda_document(&mut self, cx: &mut Context<Self>) {
        let live = self.document_session().map(|session| {
            let session = session.read(cx);
            (session.path().to_path_buf(), session.snapshot())
        });
        let identity = |value: &Option<(PathBuf, DocumentSnapshot)>| {
            value
                .as_ref()
                .map(|(path, snapshot)| (path.clone(), snapshot.document_id(), snapshot.revision()))
        };
        if identity(&live) != identity(&self.agenda.runtime.live) {
            self.agenda.runtime.live = live;
            self.agenda.refresh_sources();
        }
    }

    pub(crate) fn ensure_agenda_runtime(&mut self, cx: &mut Context<Self>) {
        if self.agenda.search_input.is_none() {
            let workspace = cx.entity().downgrade();
            self.agenda.search_input =
                Some(cx.new(|cx| super::search::AgendaSearch::new(workspace, cx)));
        }
        if let Some(input) = &self.agenda.search_input {
            input.update(cx, |input, cx| {
                input.sync(&self.agenda.state.search, cx);
                if self.agenda.language != self.language {
                    cx.notify();
                }
            });
        }
        self.agenda.language = self.language;
        self.sync_agenda_document(cx);
        if self.agenda.runtime.pump.is_some() {
            return;
        }
        let executor = cx.background_executor().clone();
        self.agenda.runtime.pump = Some(cx.spawn(async move |this, cx| {
            loop {
                executor.timer(Duration::from_millis(40)).await;
                if this
                    .update(cx, |this, cx| {
                        if let Some(result) = this.agenda.runtime.worker.try_latest() {
                            this.agenda.runtime.identities = result.identities;
                            let previous = this.agenda.index.snapshot();
                            for shard in previous.files.iter() {
                                if !result.shards.iter().any(|new| new.file == shard.file) {
                                    this.agenda.index.remove(shard.file);
                                }
                            }
                            for shard in result.shards {
                                this.agenda.index.replace(shard);
                            }
                            if !result.errors.is_empty() {
                                this.agenda.state.workflow_message = Some(Arc::from(
                                    result
                                        .errors
                                        .iter()
                                        .map(|error| error.as_ref())
                                        .collect::<Vec<_>>()
                                        .join("\n"),
                                ));
                            }
                            this.agenda.scan_progress.finish();
                            this.agenda.requery();
                            if !this.agenda.runtime.initialized {
                                this.agenda.runtime.initialized = true;
                                this.agenda.apply_initial_view();
                            }
                            this.flush_agenda_clock(cx);
                            cx.notify();
                        }
                    })
                    .is_err()
                {
                    break;
                }
            }
        }));
        let roots = self.agenda.config.sources.clone();
        if roots.is_empty() {
            return;
        }
        let setup = cx
            .background_executor()
            .spawn(async move { crate::file_watcher::AgendaWatch::new(roots) });
        let executor = cx.background_executor().clone();
        self.agenda.runtime.watch = Some(cx.spawn(async move |this, cx| {
            let watcher = match setup.await {
                Ok(watcher) => watcher,
                Err(error) => {
                    let _ = this.update(cx, |this, cx| {
                        this.agenda.state.workflow_message =
                            Some(Arc::from(format!("文件监听失败：{error}")));
                        cx.notify();
                    });
                    return;
                }
            };
            loop {
                let change = watcher.changed().await;
                executor.timer(Duration::from_millis(80)).await;
                if this
                    .update(cx, |this, cx| {
                        if let Err(error) = change {
                            this.agenda.state.workflow_message =
                                Some(Arc::from(format!("文件监听错误：{error}")));
                        }
                        this.agenda.refresh_sources();
                        cx.notify();
                    })
                    .is_err()
                {
                    break;
                }
            }
        }));
    }
}
use gpui::AppContext;
