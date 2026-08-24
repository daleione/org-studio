use std::path::{Path, PathBuf};

use notify::{Event, RecommendedWatcher, RecursiveMode, Watcher};

pub struct FileWatch {
    _watcher: RecommendedWatcher,
    receiver: async_channel::Receiver<notify::Result<Event>>,
    target: PathBuf,
}

impl FileWatch {
    pub fn new(target: PathBuf) -> notify::Result<Self> {
        let (sender, receiver) = async_channel::unbounded();
        let mut watcher = notify::recommended_watcher(move |event| {
            let _ = sender.try_send(event);
        })?;
        let parent = target.parent().unwrap_or_else(|| Path::new("."));
        watcher.watch(parent, RecursiveMode::NonRecursive)?;
        Ok(Self { _watcher: watcher, receiver, target })
    }

    pub async fn changed(&self) -> bool {
        while let Ok(event) = self.receiver.recv().await {
            if event.is_ok_and(|event| event.paths.iter().any(|path| same_target(path, &self.target))) {
                return true;
            }
        }
        false
    }

    pub fn drain(&self) {
        while self.receiver.try_recv().is_ok() {}
    }
}

fn same_target(event_path: &Path, target: &Path) -> bool {
    event_path == target
        || (event_path.file_name() == target.file_name()
            && event_path.parent() == target.parent())
}

#[cfg(test)]
mod tests {
    use super::same_target;
    use std::path::Path;

    #[test]
    fn matches_direct_and_atomic_replace_target_paths() {
        assert!(same_target(Path::new("/tmp/note.md"), Path::new("/tmp/note.md")));
        assert!(!same_target(Path::new("/tmp/.note.md.tmp"), Path::new("/tmp/note.md")));
        assert!(!same_target(Path::new("/other/note.md"), Path::new("/tmp/note.md")));
    }
}
