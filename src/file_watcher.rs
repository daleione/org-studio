mod directories;
pub(crate) use directories::DirectorySubscriptions;

use std::{
    path::{Path, PathBuf},
    sync::{Arc, RwLock},
};

use notify::{Event, RecommendedWatcher, RecursiveMode, Watcher};

/// Recursive, multi-root invalidation watcher used by Agenda. The callback
/// only coalesces wakeups; discovery and file IO stay on the analysis worker.
pub struct AgendaWatch {
    _watcher: RecommendedWatcher,
    receiver: async_channel::Receiver<notify::Result<Event>>,
    roots: Arc<[PathBuf]>,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct AgendaWatchBatch {
    pub paths: Arc<[PathBuf]>,
    pub rescan_roots: bool,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct InitialScanProgress {
    pub discovered: usize,
    pub indexed: usize,
    pub finished: bool,
}

impl InitialScanProgress {
    pub fn discovered(&mut self) {
        self.discovered = self.discovered.saturating_add(1);
    }
    pub fn indexed(&mut self) {
        self.indexed = self.indexed.saturating_add(1);
    }
    pub fn finish(&mut self) {
        self.finished = true;
    }
    pub fn percent(self) -> u8 {
        if self.finished {
            100
        } else {
            self.indexed
                .min(self.discovered)
                .checked_mul(100)
                .and_then(|value| value.checked_div(self.discovered))
                .unwrap_or(0) as u8
        }
    }
}

impl AgendaWatch {
    pub fn new(roots: impl IntoIterator<Item = PathBuf>) -> notify::Result<Self> {
        let roots = roots
            .into_iter()
            .map(|path| std::fs::canonicalize(&path).unwrap_or(path))
            .collect::<Vec<_>>();
        let (sender, receiver) = async_channel::bounded(64);
        let mut watcher = notify::recommended_watcher(move |event: notify::Result<Event>| {
            if !event
                .as_ref()
                .is_ok_and(|event| matches!(event.kind, notify::EventKind::Access(_)))
            {
                let _ = sender.try_send(event);
            }
        })?;
        for root in &roots {
            let mut target = if root.is_file() {
                root.parent().unwrap_or(root).to_path_buf()
            } else {
                root.clone()
            };
            while !target.exists() {
                if !target.pop() {
                    break;
                }
            }
            watcher.watch(&target, RecursiveMode::Recursive)?;
        }
        Ok(Self {
            _watcher: watcher,
            receiver,
            roots: roots.into(),
        })
    }

    pub fn roots(&self) -> &[PathBuf] {
        &self.roots
    }

    /// Drain events without registering a waker on the file system's callback thread.
    pub fn try_changed(&self) -> notify::Result<Option<AgendaWatchBatch>> {
        let first = match self.receiver.try_recv() {
            Ok(event) => event?,
            Err(async_channel::TryRecvError::Empty) => return Ok(None),
            Err(async_channel::TryRecvError::Closed) => {
                return Err(notify::Error::generic("Agenda file watcher stopped"));
            }
        };
        let mut paths = first.paths;
        let mut rescan_roots = event_requires_discovery(&first.kind);
        while let Ok(event) = self.receiver.try_recv() {
            let event = event?;
            rescan_roots |= event_requires_discovery(&event.kind);
            paths.extend(event.paths);
        }
        paths.retain(|path| self.roots.iter().any(|root| path.starts_with(root)));
        paths.sort();
        paths.dedup();
        Ok(Some(AgendaWatchBatch {
            paths: paths.into(),
            rescan_roots,
        }))
    }
}

fn event_requires_discovery(kind: &notify::EventKind) -> bool {
    matches!(
        kind,
        notify::EventKind::Create(_)
            | notify::EventKind::Remove(_)
            | notify::EventKind::Modify(notify::event::ModifyKind::Name(_))
    )
}

pub struct FileWatch {
    _watcher: RecommendedWatcher,
    receiver: async_channel::Receiver<notify::Result<Event>>,
    target: FileWatchTarget,
}

#[derive(Clone)]
pub struct FileWatchTarget(Arc<RwLock<PathBuf>>);

impl FileWatchTarget {
    pub fn new(path: PathBuf) -> Self {
        Self(Arc::new(RwLock::new(path)))
    }

    pub fn set(&self, path: PathBuf) {
        *self.0.write().expect("file watch target poisoned") = path;
    }

    fn get(&self) -> PathBuf {
        self.0.read().expect("file watch target poisoned").clone()
    }
}

impl FileWatch {
    pub fn new(target: FileWatchTarget) -> notify::Result<Self> {
        let (sender, receiver) = async_channel::unbounded();
        let mut watcher = notify::recommended_watcher(move |event| {
            let _ = sender.try_send(event);
        })?;
        let watched_path = target.get();
        let parent = watched_path.parent().unwrap_or_else(|| Path::new("."));
        watcher.watch(parent, RecursiveMode::NonRecursive)?;
        Ok(Self {
            _watcher: watcher,
            receiver,
            target,
        })
    }

    pub async fn changed(&self) -> Option<PathBuf> {
        while let Ok(event) = self.receiver.recv().await {
            let target = self.target.get();
            if event.is_ok_and(|event| event.paths.iter().any(|path| same_target(path, &target))) {
                return Some(target);
            }
        }
        None
    }

    pub fn drain(&self) {
        while self.receiver.try_recv().is_ok() {}
    }
}

fn same_target(event_path: &Path, target: &Path) -> bool {
    event_path == target
        || (event_path.file_name() == target.file_name() && event_path.parent() == target.parent())
}

#[cfg(test)]
mod tests {
    #[test]
    fn agenda_file_watch_survives_atomic_replacement() {
        let root = std::env::temp_dir().join(format!("agenda-watch-{}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        let path = root.join("task.org");
        std::fs::write(&path, "* TODO Original\n").unwrap();
        let watch = super::AgendaWatch::new([path.clone()]).unwrap();
        for name in ["first", "second"] {
            let temporary = root.join(format!("{name}.org"));
            std::fs::write(&temporary, format!("* TODO {name}\n")).unwrap();
            std::fs::rename(&temporary, &path).unwrap();
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(3);
            loop {
                if watch
                    .try_changed()
                    .unwrap()
                    .is_some_and(|batch| batch.paths.contains(&watch.roots()[0]))
                {
                    break;
                }
                assert!(
                    std::time::Instant::now() < deadline,
                    "Agenda watcher missed atomic replacement"
                );
                std::thread::sleep(std::time::Duration::from_millis(10));
            }
            while watch.receiver.try_recv().is_ok() {}
        }
        drop(watch);
        std::fs::remove_file(path).unwrap();
        std::fs::remove_dir(root).unwrap();
    }
    use super::{FileWatchTarget, InitialScanProgress, event_requires_discovery, same_target};
    use std::path::{Path, PathBuf};

    #[test]
    fn file_watch_target_can_retarget_a_shared_directory_watch() {
        let target = FileWatchTarget::new(PathBuf::from("/tmp/first.org"));
        let watcher_target = target.clone();
        target.set(PathBuf::from("/tmp/second.org"));
        assert_eq!(watcher_target.get(), PathBuf::from("/tmp/second.org"));
    }

    #[test]
    fn initial_scan_progress_is_bounded_and_finishes_at_one_hundred() {
        let mut progress = InitialScanProgress::default();
        progress.discovered();
        progress.discovered();
        progress.indexed();
        assert_eq!(progress.percent(), 50);
        progress.finish();
        assert_eq!(progress.percent(), 100);
    }

    #[test]
    fn matches_direct_and_atomic_replace_target_paths() {
        assert!(same_target(
            Path::new("/tmp/note.md"),
            Path::new("/tmp/note.md")
        ));
        assert!(!same_target(
            Path::new("/tmp/.note.md.tmp"),
            Path::new("/tmp/note.md")
        ));
        assert!(!same_target(
            Path::new("/other/note.md"),
            Path::new("/tmp/note.md")
        ));
    }

    #[test]
    fn agenda_discovery_events_include_create_remove_and_rename() {
        use notify::EventKind;
        use notify::event::{CreateKind, ModifyKind, RemoveKind, RenameMode};
        assert!(event_requires_discovery(&EventKind::Create(
            CreateKind::File
        )));
        assert!(event_requires_discovery(&EventKind::Remove(
            RemoveKind::File
        )));
        assert!(event_requires_discovery(&EventKind::Modify(
            ModifyKind::Name(RenameMode::Both)
        )));
        assert!(!event_requires_discovery(&EventKind::Modify(
            ModifyKind::Data(notify::event::DataChange::Content)
        )));
    }
}
