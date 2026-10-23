use super::{
    WorkspaceWindow,
    echo_area::{EchoAction, EchoMessage},
};
use crate::{
    babel::CancelToken,
    document::{ByteOffset, DocumentId},
};
use gpui::{App, Context, Task};
use std::time::Duration;

mod confirmation;
#[cfg(test)]
mod tests;
pub(crate) use confirmation::CodeConfirmation;

pub(crate) struct BabelRun {
    pub(crate) document: DocumentId,
    pub(crate) documents: Vec<DocumentId>,
    pub(crate) source: ByteOffset,
    pub(crate) cancel: CancelToken,
    pub(crate) progress: crate::babel::RunProgress,
    started: std::time::Instant,
    _status_tick: Task<()>,
}

impl Drop for BabelRun {
    fn drop(&mut self) {
        self.cancel.cancel();
    }
}

impl WorkspaceWindow {
    pub(crate) fn start_babel_run(
        &mut self,
        document: DocumentId,
        source: ByteOffset,
        cancel: CancelToken,
        cx: &mut Context<Self>,
    ) {
        self.set_echo_message(None);
        self.babel_run = Some(BabelRun {
            document,
            documents: vec![document],
            source,
            cancel,
            progress: crate::babel::RunProgress::default(),
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

    pub(crate) fn babel_running_message(&self, cx: &App) -> Option<EchoMessage> {
        let run = self.babel_run.as_ref()?;
        let stopping = run.cancel.cancelled();
        let mut text = if stopping {
            self.buffer_text("正在停止执行…", "Stopping execution…")
                .to_owned()
        } else {
            format!(
                "{} · {:.1} s",
                self.buffer_text("正在执行", "Running code"),
                run.started.elapsed().as_secs_f64()
            )
        };
        if !stopping && let Some(label) = run.progress.label() {
            text = format!("{label} · {text}");
        }
        if !self
            .document_session()
            .is_some_and(|session| session.read(cx).id() == run.document)
            && let Some(session) = self.buffer_session(run.document, cx)
        {
            text = format!("{} · {text}", session.read(cx).display_name());
        }
        Some(
            EchoMessage::working(text).with_action(EchoAction::StopBabelExecution {
                request: self.babel_request,
                enabled: !stopping,
            }),
        )
    }

    pub(crate) fn stop_babel_execution(&mut self, request: u64, cx: &mut Context<Self>) {
        if self.babel_request == request
            && let Some(run) = &self.babel_run
        {
            run.cancel.cancel();
            cx.notify();
        }
    }
}
