use crate::{
    agenda::{AgendaCommand, TaskKey},
    app::WorkspaceWindow,
    document::TextSnapshot,
};
use gpui::Context;
use std::sync::Arc;

impl WorkspaceWindow {
    pub(super) fn toggle_agenda_clock(&mut self, key: TaskKey, cx: &mut Context<Self>) {
        let Some(task) = self.agenda.task(key) else {
            return;
        };
        let Some(path) = self.agenda.clock_store_path.clone() else {
            self.agenda.state.workflow_message = Some(Arc::from("计时存储不可用"));
            return;
        };
        let mut next = self.agenda.clock_store.clone();
        let now = std::time::SystemTime::now();
        if self.agenda.clock_is_active_for(&task) {
            next.stop(now);
        } else {
            next.start(&task, now);
        }
        if let Err(error) = next.save(&path) {
            self.agenda.state.workflow_message =
                Some(Arc::from(format!("计时状态保存失败：{error}")));
            return;
        }
        self.agenda.clock_store = next;
        self.flush_agenda_clock(cx);
    }

    pub(crate) fn flush_agenda_clock(&mut self, cx: &mut Context<Self>) {
        let Some(path) = self.agenda.clock_store_path.clone() else {
            return;
        };
        let pending = self.agenda.clock_store.pending.clone();
        for log in pending {
            let line = log.org_line();
            let disk_has_log = std::fs::read_to_string(&log.file)
                .is_ok_and(|text| contains_log(&text, &log.heading_title, &line));
            if disk_has_log {
                let mut next = self.agenda.clock_store.clone();
                next.pending.retain(|entry| entry != &log);
                match next.save(&path) {
                    Ok(()) => self.agenda.clock_store = next,
                    Err(error) => {
                        self.agenda.state.workflow_message =
                            Some(Arc::from(format!("计时确认保存失败：{error}")))
                    }
                }
                continue;
            }
            if let Some(session) = self.buffer_for_path(&log.file, cx) {
                let snapshot = session.read(cx).snapshot();
                let text =
                    snapshot.copy_range(crate::document::ByteRange::new(0, snapshot.len_bytes()));
                if contains_log(&text, &log.heading_title, &line) {
                    self.agenda.state.workflow_message = Some(Arc::from(
                        "计时日志已写入文档，保存后完成；恢复记录已保留。",
                    ));
                    continue;
                }
            }
            let candidates = self
                .agenda
                .runtime
                .index
                .snapshot()
                .files
                .iter()
                .flat_map(|file| file.tasks.iter())
                .filter(|task| {
                    task.source.path.as_path() == log.file
                        && task.title.as_ref() == log.heading_title
                })
                .cloned()
                .collect::<Vec<_>>();
            let [task] = candidates.as_slice() else {
                self.agenda.state.workflow_message =
                    Some(Arc::from("待写计时记录已保留，任务缺失或标题不唯一。"));
                continue;
            };
            if let Err(error) =
                self.apply_agenda_command(task, &AgendaCommand::AppendLogbook(Arc::from(line)), cx)
            {
                self.agenda.state.workflow_message =
                    Some(Arc::from(format!("计时写入失败，恢复记录已保留：{error}")));
            }
        }
    }
}

fn contains_log(text: &str, title: &str, line: &str) -> bool {
    let Ok(snapshot) = crate::document::DocumentSnapshot::from_utf8(text.as_bytes().to_vec())
    else {
        return false;
    };
    let blocks = Arc::new(crate::org_syntax::parse(&snapshot));
    let analysis = crate::org_semantic::analyze(&snapshot, blocks.clone());
    let headings = analysis
        .headings
        .iter()
        .filter(|heading| heading.title.as_ref() == title)
        .collect::<Vec<_>>();
    let [heading] = headings.as_slice() else {
        return false;
    };
    let Some(index) = blocks
        .nodes()
        .iter()
        .position(|node| node.syntax_id == heading.syntax_id)
    else {
        return false;
    };
    blocks.nodes().iter().filter(|node| node.parent == Some(index as u32) && matches!(&node.kind, crate::org_syntax::BlockKind::Drawer { name } if name.eq_ignore_ascii_case("LOGBOOK")))
        .any(|node| snapshot.copy_range(node.source).lines().any(|value| value.trim() == line))
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::AppContext;
    #[gpui::test]
    fn failed_clock_write_keeps_recovery_and_retry_is_idempotent(cx: &mut gpui::TestAppContext) {
        let root = std::env::temp_dir().join(format!("agenda-clock-retry-{}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        let path = root.join("task.org");
        let receipt = root.join("clock.json");
        std::fs::write(&path, "* TODO Task\n").unwrap();
        let shard =
            crate::agenda::shard_from_disk(crate::agenda::FileId(1), 1, path.clone()).unwrap();
        let task = shard.tasks[0].clone();
        let workspace = cx.new(|_| WorkspaceWindow::with_split_layout(false));
        workspace.update(cx, |workspace, cx| {
            workspace.agenda.runtime.index.replace(shard);
            workspace.agenda.clock_store_path = Some(receipt.clone());
            workspace.agenda.clock_store = crate::agenda::ClockStore::default();
            workspace
                .agenda
                .clock_store
                .start(&task, std::time::SystemTime::UNIX_EPOCH);
            std::fs::write(&path, "* TODO Task\nExternal change\n").unwrap();
            workspace.toggle_agenda_clock(task.key, cx);
            assert_eq!(
                crate::agenda::ClockStore::load(&receipt)
                    .unwrap()
                    .pending
                    .len(),
                1
            );
            assert!(!std::fs::read_to_string(&path).unwrap().contains("CLOCK:"));
            workspace.agenda.runtime.index.replace(
                crate::agenda::shard_from_disk(crate::agenda::FileId(1), 2, path.clone()).unwrap(),
            );
            workspace.flush_agenda_clock(cx);
            workspace.flush_agenda_clock(cx);
            assert!(
                crate::agenda::ClockStore::load(&receipt)
                    .unwrap()
                    .pending
                    .is_empty()
            );
            assert_eq!(
                std::fs::read_to_string(&path)
                    .unwrap()
                    .matches("CLOCK:")
                    .count(),
                1
            );
        });
        std::fs::remove_file(path).unwrap();
        std::fs::remove_file(receipt).unwrap();
        std::fs::remove_dir(root).unwrap();
    }
    #[test]
    fn clock_retry_only_recognizes_log_in_its_own_heading() {
        let line = "CLOCK: [2026-09-07 Mon 10:00:00]--[2026-09-07 Mon 11:00:00] => 01:00";
        let source = format!("* TODO Parent\n** TODO Child\n:LOGBOOK:\n{line}\n:END:\n");
        assert!(!contains_log(&source, "Parent", line));
        assert!(contains_log(&source, "Child", line));
        assert!(!contains_log(
            &format!("{source}* TODO Child\n"),
            "Child",
            line
        ));
    }
}
