use std::{
    path::{Path, PathBuf},
    sync::{Arc, RwLock},
};

use notify::{Event, RecommendedWatcher, RecursiveMode, Watcher};

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

/// Watches the immediate children of one Dired directory.
///
/// Dired currently presents one flat directory at a time, so a non-recursive
/// watch has the same scope as the active session and avoids paying for an
/// unrelated workspace tree.
pub struct DirectoryWatch {
    _watcher: RecommendedWatcher,
    receiver: async_channel::Receiver<notify::Result<Event>>,
    target: PathBuf,
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

impl DirectoryWatch {
    pub fn new(target: PathBuf) -> notify::Result<Self> {
        let target = std::fs::canonicalize(&target).unwrap_or(target);
        // Dired only needs to know that the directory became dirty. Capacity
        // one avoids retaining one Event per filesystem write.
        let (sender, receiver) = async_channel::bounded(1);
        let mut watcher = notify::recommended_watcher(move |event| {
            let _ = sender.try_send(event);
        })?;
        watcher.watch(&target, RecursiveMode::NonRecursive)?;
        Ok(Self {
            _watcher: watcher,
            receiver,
            target,
        })
    }

    pub async fn changed(&self) -> notify::Result<bool> {
        loop {
            match self.receiver.recv().await {
                Ok(Ok(event)) => {
                    if event
                        .paths
                        .iter()
                        .any(|path| belongs_to_directory(path, &self.target))
                    {
                        return Ok(true);
                    }
                }
                Ok(Err(error)) => return Err(error),
                Err(_) => return Ok(false),
            }
        }
    }

    pub fn drain(&self) {
        while self.receiver.try_recv().is_ok() {}
    }
}

fn same_target(event_path: &Path, target: &Path) -> bool {
    event_path == target
        || (event_path.file_name() == target.file_name() && event_path.parent() == target.parent())
}

fn belongs_to_directory(event_path: &Path, directory: &Path) -> bool {
    event_path == directory || event_path.parent() == Some(directory)
}

#[cfg(test)]
mod tests {
    use super::{FileWatchTarget, belongs_to_directory, same_target};
    use std::path::{Path, PathBuf};

    #[test]
    fn file_watch_target_can_retarget_a_shared_directory_watch() {
        let target = FileWatchTarget::new(PathBuf::from("/tmp/first.org"));
        let watcher_target = target.clone();
        target.set(PathBuf::from("/tmp/second.org"));
        assert_eq!(watcher_target.get(), PathBuf::from("/tmp/second.org"));
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
    fn directory_watch_matches_only_the_directory_and_its_immediate_children() {
        let directory = Path::new("/tmp/notes");
        assert!(belongs_to_directory(directory, directory));
        assert!(belongs_to_directory(
            Path::new("/tmp/notes/new.org"),
            directory
        ));
        assert!(!belongs_to_directory(
            Path::new("/tmp/notes/archive/old.org"),
            directory
        ));
        assert!(!belongs_to_directory(
            Path::new("/tmp/other/new.org"),
            directory
        ));
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn directory_watch_receives_a_real_child_creation() {
        let unique = std::time::SystemTime::now()
            .duration_since(std::time::SystemTime::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let directory = std::env::temp_dir().join(format!(
            "org-studio-directory-watch-{}-{unique}",
            std::process::id()
        ));
        std::fs::create_dir_all(&directory).unwrap();
        let watch = super::DirectoryWatch::new(directory.clone()).unwrap();
        assert_eq!(watch.receiver.capacity(), Some(1));
        let canonical_directory = std::fs::canonicalize(&directory).unwrap();
        let (sender, receiver) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let result = watch.receiver.recv_blocking();
            let _ = sender.send(result);
        });
        std::fs::write(directory.join("new.org"), "* New\n").unwrap();
        let event = receiver
            .recv_timeout(std::time::Duration::from_secs(3))
            .expect("notify should publish a directory event")
            .expect("directory channel should stay open")
            .expect("directory event should be valid");
        assert!(
            event
                .paths
                .iter()
                .any(|path| super::belongs_to_directory(path, &canonical_directory))
        );
        std::fs::remove_dir_all(directory).unwrap();
    }
}
