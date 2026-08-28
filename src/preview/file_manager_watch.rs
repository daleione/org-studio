use std::{path::PathBuf, time::Duration};

use gpui::{AppContext, Context};

use super::{DiredStatus, PreviewApp};
use crate::{
    file_manager::DiredSession, file_watcher::DirectoryWatch, navigation::NavigationCause,
};

impl PreviewApp {
    pub(super) fn ensure_dired_directory_watch(
        &mut self,
        directory: PathBuf,
        cx: &mut Context<Self>,
    ) {
        if self.dired_watch_task.is_some()
            && self.dired_watch_directory.as_ref() == Some(&directory)
        {
            return;
        }
        self.stop_dired_directory_watch();
        self.dired_watch_request = self.dired_watch_request.wrapping_add(1);
        let request = self.dired_watch_request;
        self.dired_watch_directory = Some(directory.clone());
        let watch_directory = directory.clone();
        let setup = cx.background_spawn(async move { DirectoryWatch::new(watch_directory) });
        self.dired_watch_task = Some(cx.spawn(async move |this, cx| {
            let watch = match setup.await {
                Ok(watch) => watch,
                Err(error) => {
                    let _ = this.update(cx, |this, cx| {
                        if this.dired_watch_request == request {
                            this.dired_watch_task = None;
                            this.dired_watch_directory = None;
                            this.dired_status = Some(DiredStatus::Error(
                                format!("Directory watching unavailable: {error}").into(),
                            ));
                            cx.notify();
                        }
                    });
                    return;
                }
            };
            loop {
                let changed = match watch.changed().await {
                    Ok(changed) => changed,
                    Err(error) => {
                        let _ = this.update(cx, |this, cx| {
                            if this.dired_watch_request == request {
                                this.dired_watch_task = None;
                                this.dired_watch_directory = None;
                                this.dired_status = Some(DiredStatus::Error(
                                    format!("Directory watch failed: {error}").into(),
                                ));
                                cx.notify();
                            }
                        });
                        return;
                    }
                };
                if !changed {
                    let _ = this.update(cx, |this, cx| {
                        if this.dired_watch_request == request {
                            this.dired_watch_task = None;
                            this.dired_watch_directory = None;
                            this.dired_status = Some(DiredStatus::Error(
                                "Directory watch stopped unexpectedly".into(),
                            ));
                            cx.notify();
                        }
                    });
                    return;
                }
                cx.background_executor()
                    .timer(Duration::from_millis(100))
                    .await;
                watch.drain();
                let keep_watching = this
                    .update(cx, |this, cx| {
                        if this.dired_watch_request != request
                            || this.dired_watch_directory.as_ref() != Some(&directory)
                            || this.dired.as_ref().map(DiredSession::directory)
                                != Some(directory.as_path())
                        {
                            return false;
                        }
                        this.refresh_file_manager(NavigationCause::FileSystemDelta, cx);
                        true
                    })
                    .unwrap_or(false);
                if !keep_watching {
                    return;
                }
            }
        }));
    }

    pub(super) fn stop_dired_directory_watch(&mut self) {
        self.dired_watch_request = self.dired_watch_request.wrapping_add(1);
        self.dired_watch_task = None;
        self.dired_watch_directory = None;
    }
}
