//! One non-recursive notify watcher per workspace, shared by all directory views.

use std::{
    collections::{HashMap, HashSet},
    path::PathBuf,
    sync::{Arc, Mutex, mpsc},
};

use notify::{RecursiveMode, Watcher};

#[derive(Default)]
pub(crate) struct DirectoryChanges {
    pub(crate) directories: HashSet<PathBuf>,
    pub(crate) error: Option<String>,
}

pub(crate) struct DirectorySubscriptions {
    sender: mpsc::Sender<HashSet<PathBuf>>,
    signal: async_channel::Sender<()>,
    pending: Arc<Mutex<DirectoryChanges>>,
}

impl Drop for DirectorySubscriptions {
    fn drop(&mut self) {
        // Close on the owning executor before dropping its consumer task.
        // Serialize with publish so no OS callback can retain/wake a local
        // task after the window (or its deterministic test executor) exits.
        let _pending = self.pending.lock().expect("directory changes lock");
        self.signal.close();
    }
}

pub(crate) struct DirectoryEvents {
    receiver: async_channel::Receiver<()>,
    pending: Arc<Mutex<DirectoryChanges>>,
}

impl DirectoryEvents {
    pub(crate) async fn changed(&self) -> Option<DirectoryChanges> {
        self.receiver.recv().await.ok()?;
        Some(std::mem::take(
            &mut *self.pending.lock().expect("directory changes lock"),
        ))
    }
}

fn publish(
    pending: &Mutex<DirectoryChanges>,
    signal: &async_channel::Sender<()>,
    directories: impl IntoIterator<Item = PathBuf>,
    error: Option<String>,
) {
    let mut changes = pending.lock().expect("directory changes lock");
    changes.directories.extend(directories);
    if error.is_some() {
        changes.error = error;
    }
    let _ = signal.try_send(());
}

impl DirectorySubscriptions {
    pub(crate) fn new() -> std::io::Result<(Self, DirectoryEvents)> {
        let (sender, commands) = mpsc::channel::<HashSet<PathBuf>>();
        let (signal, receiver) = async_channel::bounded(1);
        let pending = Arc::new(Mutex::new(DirectoryChanges::default()));
        let events = DirectoryEvents {
            receiver,
            pending: pending.clone(),
        };
        let subscriptions = Self {
            sender,
            signal: signal.clone(),
            pending: pending.clone(),
        };
        std::thread::Builder::new()
            .name("org-studio-directories".into())
            .spawn(move || {
                let targets = Arc::new(Mutex::new(HashMap::<PathBuf, HashSet<PathBuf>>::new()));
                let callback_targets = targets.clone();
                let callback_pending = pending.clone();
                let callback_signal = signal.clone();
                let watcher =
                    notify::recommended_watcher(move |event: notify::Result<notify::Event>| {
                        if event
                            .as_ref()
                            .is_ok_and(|event| matches!(event.kind, notify::EventKind::Access(_)))
                        {
                            return;
                        }
                        let targets = callback_targets.lock().expect("directory targets lock");
                        let (directories, error) = match event {
                            Ok(event) => (
                                targets
                                    .iter()
                                    .filter(|(canonical, _)| {
                                        event.paths.iter().any(|path| {
                                            path == *canonical
                                                || path.parent() == Some(canonical.as_path())
                                        })
                                    })
                                    .flat_map(|(_, aliases)| aliases.iter().cloned())
                                    .collect::<HashSet<_>>(),
                                None,
                            ),
                            Err(error) => (
                                targets
                                    .values()
                                    .flat_map(|aliases| aliases.iter().cloned())
                                    .collect(),
                                Some(error.to_string()),
                            ),
                        };
                        if !directories.is_empty() || error.is_some() {
                            publish(&callback_pending, &callback_signal, directories, error);
                        }
                    });
                let mut watcher = match watcher {
                    Ok(watcher) => watcher,
                    Err(error) => {
                        publish(&pending, &signal, [], Some(error.to_string()));
                        return;
                    }
                };
                while let Ok(mut desired) = commands.recv() {
                    while let Ok(newer) = commands.try_recv() {
                        desired = newer;
                    }
                    let mut next = HashMap::<PathBuf, HashSet<PathBuf>>::new();
                    for display in desired {
                        let canonical =
                            std::fs::canonicalize(&display).unwrap_or_else(|_| display.clone());
                        next.entry(canonical).or_default().insert(display);
                    }
                    let old = targets.lock().expect("directory targets lock").clone();
                    // Publish the new routing first so an event fired during watch()
                    // is routed to the right displayed directory instances.
                    *targets.lock().expect("directory targets lock") = next.clone();
                    for path in old.keys().filter(|path| !next.contains_key(*path)) {
                        let _ = watcher.unwatch(path);
                    }
                    for (path, aliases) in next.iter().filter(|(path, _)| !old.contains_key(*path))
                    {
                        let error = watcher
                            .watch(path, RecursiveMode::NonRecursive)
                            .err()
                            .map(|error| error.to_string());
                        publish(&pending, &signal, aliases.iter().cloned(), error);
                    }
                }
            })?;
        Ok((subscriptions, events))
    }

    pub(crate) fn set_directories(&self, directories: HashSet<PathBuf>) -> bool {
        self.sender.send(directories).is_ok()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn subscriptions_follow_multiple_directories_and_deliver_displayed_aliases() {
        let root = tempfile::tempdir().unwrap();
        let first = root.path().join("first");
        let second = root.path().join("second");
        std::fs::create_dir(&first).unwrap();
        std::fs::create_dir(&second).unwrap();
        let (subscriptions, events) = DirectorySubscriptions::new().unwrap();
        assert!(subscriptions.set_directories(HashSet::from([first.clone(), second.clone()])));
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        let mut watching = HashSet::new();
        while watching.len() < 2 && std::time::Instant::now() < deadline {
            if events.receiver.try_recv().is_ok() {
                let changes = std::mem::take(&mut *events.pending.lock().unwrap());
                assert!(changes.error.is_none(), "{:?}", changes.error);
                watching.extend(changes.directories);
            }
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        assert_eq!(
            watching.len(),
            2,
            "both displayed directories must be subscribed"
        );
        std::fs::write(first.join("new.txt"), "value").unwrap();
        let mut received = false;
        while std::time::Instant::now() < deadline {
            if events.receiver.try_recv().is_ok() {
                let changes = std::mem::take(&mut *events.pending.lock().unwrap());
                if changes.directories.contains(&first) {
                    received = true;
                    break;
                }
            }
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        assert!(
            received,
            "a real child creation must invalidate the displayed directory"
        );
    }
}
