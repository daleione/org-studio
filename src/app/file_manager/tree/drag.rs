use super::WorkspaceWindow;
use gpui::{Context, Pixels, Point, Task};
use std::{
    collections::HashSet,
    path::{Path, PathBuf},
    time::Duration,
};

#[derive(Default)]
pub(super) struct TreeDrag {
    hover: Option<PathBuf>,
    pub(super) temporary: HashSet<PathBuf>,
    hover_task: Option<Task<()>>,
    end_task: Option<Task<()>>,
}

impl WorkspaceWindow {
    pub(super) fn update_file_tree_drag(
        &mut self,
        position: Point<Pixels>,
        sources: &[std::sync::Arc<Path>],
        cx: &mut Context<Self>,
    ) {
        let tree = &self.file_manager.tree;
        let target = tree
            .row_bounds
            .borrow()
            .iter()
            .filter(|(_, bounds)| bounds.contains(&position))
            .filter_map(|(id, _)| tree.model.entry(*id))
            .find(|entry| {
                entry.is_directory() && !sources.iter().any(|source| entry.path.starts_with(source))
            })
            .map(|entry| entry.path.to_path_buf());
        if tree.drag.hover == target {
            return;
        }
        let tree = &mut self.file_manager.tree;
        tree.drag.hover = target.clone();
        tree.drag.hover_task = None;
        let mut changed = false;
        tree.drag.temporary.retain(|path| {
            let keep = target
                .as_ref()
                .is_some_and(|target| target.starts_with(path.as_path()));
            if !keep {
                tree.model.set_expanded(path, false);
                changed = true;
            }
            keep
        });
        if let Some(target) = target.filter(|path| !tree.model.expanded(path)) {
            tree.drag.hover_task = Some(cx.spawn(async move |this, cx| {
                cx.background_executor()
                    .timer(Duration::from_millis(600))
                    .await;
                let _ = this.update(cx, |this, cx| {
                    if cx.has_active_drag()
                        && this.file_manager.tree.drag.hover.as_ref() == Some(&target)
                    {
                        this.file_manager.tree.model.set_expanded(&target, true);
                        this.file_manager.tree.drag.temporary.insert(target);
                        this.rebuild_file_tree(cx);
                        this.sync_file_directory_watches(cx);
                    }
                });
            }));
        }
        if self.file_manager.tree.drag.end_task.is_none() {
            self.file_manager.tree.drag.end_task = Some(cx.spawn(async move |this, cx| {
                loop {
                    cx.background_executor()
                        .timer(Duration::from_millis(100))
                        .await;
                    if !this
                        .update(cx, |this, cx| {
                            if cx.has_active_drag() {
                                true
                            } else {
                                this.finish_file_tree_drag(false, cx);
                                false
                            }
                        })
                        .unwrap_or(false)
                    {
                        break;
                    }
                }
            }));
        }
        if changed {
            self.rebuild_file_tree(cx);
            self.sync_file_directory_watches(cx);
        }
    }

    pub(super) fn finish_file_tree_drag(&mut self, placed: bool, cx: &mut Context<Self>) {
        let drag = std::mem::take(&mut self.file_manager.tree.drag);
        if !placed {
            for path in drag.temporary {
                self.file_manager.tree.model.set_expanded(&path, false);
            }
            self.rebuild_file_tree(cx);
            self.sync_file_directory_watches(cx);
        } else {
            self.save_file_tree_memory();
        }
    }
}
