use std::{collections::HashSet, path::PathBuf, time::Duration};

use gpui::{Context, Task};

use super::{ContentRoute, WorkspaceWindow};
use crate::{file_watcher::DirectorySubscriptions, navigation::NavigationCause};

pub(super) struct DirectoryWatch {
    // Close OS notifications before cancelling their foreground consumer.
    subscriptions: DirectorySubscriptions,
    _task: Task<()>,
    directories: HashSet<PathBuf>,
}

impl WorkspaceWindow {
    pub(crate) fn sync_file_directory_watches(&mut self, cx: &mut Context<Self>) {
        let mut active = HashSet::new();
        if self.content_route == ContentRoute::FileManager {
            if let Some(session) = &self.file_manager.session {
                active.insert(session.directory().to_path_buf());
            }
            if let Some(load) = &self.file_manager.dired_load {
                active.insert(load.intent.target.clone());
            }
        }
        if self.content_route == ContentRoute::Document && self.file_manager.sidebar_visible {
            active.extend(self.file_manager.tree.model.active_directories());
        }
        self.file_manager.directories.retain_active(&active);
        self.file_manager
            .directory_tasks
            .retain(|directory, _| active.contains(directory));
        if self.file_manager.directory_watch.is_none() && !active.is_empty() {
            let (subscriptions, events) = match DirectorySubscriptions::new() {
                Ok(pair) => pair,
                Err(error) => {
                    self.file_manager.watch_error = Some(error.to_string().into());
                    return;
                }
            };
            let task = cx.spawn(async move |this, cx| {
                while let Some(mut changes) = events.changed().await {
                    cx.background_executor()
                        .timer(Duration::from_millis(100))
                        .await;
                    if this
                        .update(cx, |this, cx| {
                            if let Some(error) = changes.error.take() {
                                this.file_manager.watch_error = Some(
                                    format!(
                                        "{}: {error}",
                                        this.language.text("files.watch_failed")
                                    )
                                    .into(),
                                );
                            }
                            for directory in changes.directories {
                                if !this
                                    .file_manager
                                    .directory_watch
                                    .as_ref()
                                    .is_some_and(|watch| watch.directories.contains(&directory))
                                {
                                    continue;
                                }
                                this.file_manager.directories.invalidate(&directory);
                                if this.content_route == ContentRoute::FileManager
                                    && this
                                        .file_manager
                                        .session
                                        .as_ref()
                                        .is_some_and(|session| session.directory() == directory)
                                {
                                    this.refresh_file_manager(NavigationCause::FileSystemDelta, cx);
                                } else {
                                    this.request_file_directory(directory, true, cx);
                                }
                            }
                            cx.notify();
                        })
                        .is_err()
                    {
                        break;
                    }
                }
            });
            self.file_manager.directory_watch = Some(DirectoryWatch {
                subscriptions,
                _task: task,
                directories: HashSet::new(),
            });
            self.file_manager.watch_error = None;
        }
        if let Some(watch) = &mut self.file_manager.directory_watch
            && watch.directories != active
        {
            watch.subscriptions.set_directories(active.clone());
            watch.directories = active;
        }
    }
}
