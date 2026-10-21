use super::{
    WorkspaceWindow,
    echo_area::{EchoAction, EchoMessage},
};
use crate::{
    babel::mysql::CancelToken,
    document::{ByteOffset, DocumentId},
};
use gpui::{App, Context, Task};
use std::{collections::HashSet, time::Duration};

mod confirmation;
#[cfg(test)]
mod tests;

#[derive(Default)]
pub(crate) struct MySqlHost {
    pub(crate) confirmation: Option<confirmation::QueryConfirmation>,
    pub(crate) run: Option<MySqlRun>,
    pub(crate) trusted: HashSet<(std::path::PathBuf, String)>,
}

pub(crate) struct MySqlRun {
    pub(crate) document: DocumentId,
    pub(crate) source: ByteOffset,
    pub(crate) cancel: CancelToken,
    started: std::time::Instant,
    _status_tick: Task<()>,
}

impl Drop for MySqlRun {
    fn drop(&mut self) {
        self.cancel.cancel();
    }
}

impl WorkspaceWindow {
    pub(crate) fn start_mysql_run(
        &mut self,
        document: DocumentId,
        source: ByteOffset,
        cancel: CancelToken,
        cx: &mut Context<Self>,
    ) {
        self.set_echo_message(None);
        self.mysql.run = Some(MySqlRun {
            document,
            source,
            cancel,
            started: std::time::Instant::now(),
            _status_tick: cx.spawn(async move |this, cx| {
                loop {
                    cx.background_executor()
                        .timer(Duration::from_millis(250))
                        .await;
                    if this.update(cx, |_, cx| cx.notify()).is_err() {
                        break;
                    }
                }
            }),
        });
        cx.notify();
    }

    pub(crate) fn mysql_running_message(&self, cx: &App) -> Option<EchoMessage> {
        let run = self.mysql.run.as_ref()?;
        let stopping = run.cancel.cancelled();
        let mut text = if stopping {
            self.buffer_text("正在停止查询…", "Stopping query…")
                .to_owned()
        } else {
            format!(
                "{} · {:.1} s",
                self.buffer_text("正在查询", "Running query"),
                run.started.elapsed().as_secs_f64()
            )
        };
        if !self
            .document_session()
            .is_some_and(|session| session.read(cx).id() == run.document)
            && let Some(session) = self.buffer_session(run.document, cx)
        {
            text = format!("{} · {text}", session.read(cx).display_name());
        }
        Some(
            EchoMessage::working(text).with_action(EchoAction::StopMySqlQuery {
                request: self.babel_request,
                enabled: !stopping,
            }),
        )
    }

    pub(crate) fn stop_mysql_query(&mut self, request: u64, cx: &mut Context<Self>) {
        if self.babel_request == request
            && let Some(run) = &self.mysql.run
        {
            run.cancel.cancel();
            cx.notify();
        }
    }
}
