use super::*;
use notify::Watcher;

impl WorkspaceWindow {
    pub(crate) fn ensure_buffer_watch(&mut self, cx: &mut Context<Self>) {
        let mut paths = self
            .buffer_sessions()
            .filter_map(|s| s.read(cx).file_path().map(PathBuf::from))
            .collect::<Vec<_>>();
        paths.sort();
        paths.dedup();
        if paths == self.buffers.watch_paths {
            return;
        }
        self.buffers.watch_paths = paths.clone();
        self.buffers.watch_task = None;
        if paths.is_empty() {
            return;
        }
        let (sender, receiver) = std::sync::mpsc::sync_channel(1);
        let (ready, setup) = std::sync::mpsc::sync_channel(1);
        // A single nonrecursive watcher shares directories across background documents.
        // Setup stays off the UI executor because FSEvents initialization can block.
        let started = std::thread::Builder::new()
            .name("buffer-watch".into())
            .spawn(move || {
                let result = (|| -> notify::Result<_> {
                    let mut watcher = notify::recommended_watcher(
                        move |event: notify::Result<notify::Event>| {
                            if !event
                                .as_ref()
                                .is_ok_and(|e| matches!(e.kind, notify::EventKind::Access(_)))
                            {
                                let _ = sender.try_send(());
                            }
                        },
                    )?;
                    let mut directories = paths
                        .iter()
                        .filter_map(|p| p.parent().map(PathBuf::from))
                        .collect::<Vec<_>>();
                    directories.sort();
                    directories.dedup();
                    for directory in directories {
                        watcher.watch(&directory, notify::RecursiveMode::NonRecursive)?;
                    }
                    Ok(watcher)
                })();
                let _ = ready.send(result);
            });
        if started.is_err() {
            return;
        }
        self.buffers.watch_task = Some(cx.spawn(async move |this, cx| {
            let _watcher = loop {
                match setup.try_recv() {
                    Ok(Ok(watcher)) => break watcher,
                    Ok(Err(_)) | Err(std::sync::mpsc::TryRecvError::Disconnected) => return,
                    Err(std::sync::mpsc::TryRecvError::Empty) => {
                        cx.background_executor()
                            .timer(std::time::Duration::from_millis(50))
                            .await
                    }
                }
            };
            loop {
                let requests = this.update(cx, |this, cx| {
                    this.buffers
                        .parked
                        .iter()
                        .filter_map(|d| {
                            let s = d.document.session.read(cx);
                            Some((
                                s.id(),
                                s.file_path()?.to_path_buf(),
                                s.sync_state().base().clone(),
                                s.reload_request().ok(),
                            ))
                        })
                        .collect::<Vec<_>>()
                });
                let Ok(requests) = requests else {
                    return;
                };
                let results = cx
                    .background_executor()
                    .spawn(async move {
                        requests
                            .into_iter()
                            .map(|(id, path, base, reload)| {
                                let observed = crate::document::FileStamp::read(&path);
                                let prepared = observed
                                    .as_ref()
                                    .is_ok_and(|s| *s != base)
                                    .then(|| {
                                        reload.and_then(|r| {
                                            std::fs::read(&path)
                                                .ok()
                                                .and_then(|bytes| r.prepare(bytes).ok())
                                        })
                                    })
                                    .flatten();
                                (id, path, observed, prepared)
                            })
                            .collect::<Vec<_>>()
                    })
                    .await;
                let _ = this.update(cx, |this, cx| {
                    for (id, path, observed, prepared) in results {
                        let Some(s) = this.buffer_session(id, cx) else {
                            continue;
                        };
                        if s.read(cx).file_path() != Some(path.as_path()) {
                            continue;
                        }
                        let stamp = match observed {
                            Ok(stamp) => Some(stamp),
                            Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
                            Err(_) => continue,
                        };
                        s.update(cx, |s, cx| {
                            if s.observe_disk(stamp) == crate::document::DiskChangeAction::Reload
                                && let Some(reload) = prepared
                            {
                                let _ = s.apply_reload(reload, cx);
                            }
                        });
                    }
                    cx.notify();
                });
                loop {
                    cx.background_executor()
                        .timer(std::time::Duration::from_millis(100))
                        .await;
                    match receiver.try_recv() {
                        Ok(()) => break,
                        Err(std::sync::mpsc::TryRecvError::Disconnected) => return,
                        Err(std::sync::mpsc::TryRecvError::Empty) => {}
                    }
                }
                while receiver.try_recv().is_ok() {}
            }
        }));
    }
}
