use super::{
    CancelToken,
    document::{self, DocumentVersion},
    execution::{self, BabelExecutionRequest, PreparedBabelOutput},
    resolve::{Input, Invocation, Resolver},
    value::Data,
};
use crate::{
    document::{DocumentSnapshot, Selection, TextSnapshot},
    org_syntax::BlockKind,
};
use std::{
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

pub(super) const MAX_DEPTH: usize = 32;
pub(super) const MAX_INVOCATIONS: usize = 64;
const MAX_VALUE_BYTES: usize = 8 * 1024 * 1024;
const DEADLINE: Duration = Duration::from_secs(90);

#[derive(Clone, Default)]
pub(crate) struct RunProgress(std::sync::Arc<std::sync::Mutex<Option<String>>>);
impl RunProgress {
    pub(crate) fn label(&self) -> Option<String> {
        self.0.lock().ok()?.clone()
    }
    fn set(&self, label: String) {
        if let Ok(mut current) = self.0.lock() {
            *current = Some(label);
        }
    }
}

pub(crate) struct ExecutionPlan {
    root: Invocation,
    pub(crate) versions: Vec<DocumentVersion>,
    pub(crate) cancel: CancelToken,
    pub(crate) progress: RunProgress,
}

impl ExecutionPlan {
    pub(crate) fn block_name(&self) -> Option<&str> {
        self.root
            .request
            .block_name()
            .or(self.root.definition_name.as_deref())
    }

    pub(crate) fn requests(&self) -> Vec<(&Path, &BabelExecutionRequest)> {
        fn visit<'a>(
            invocation: &'a Invocation,
            output: &mut Vec<(&'a Path, &'a BabelExecutionRequest)>,
        ) {
            for (_, input) in &invocation.inputs {
                fn input_visit<'a>(
                    input: &'a Input,
                    output: &mut Vec<(&'a Path, &'a BabelExecutionRequest)>,
                ) {
                    match input {
                        Input::Execution(invocation) => visit(invocation, output),
                        Input::Select(input, _) => input_visit(input, output),
                        Input::Data(_) => {}
                    }
                }
                input_visit(input, output);
            }
            output.push((&invocation.path, &invocation.request));
        }
        let mut output = Vec::new();
        visit(&self.root, &mut output);
        output
    }
}

pub(crate) fn prepare_execution(
    snapshot: &DocumentSnapshot,
    path: &Path,
    selection: Selection,
    open: Vec<(PathBuf, DocumentSnapshot)>,
) -> Result<ExecutionPlan, String> {
    let mut resolver = Resolver::new(path, snapshot, open)?;
    let doc = &resolver.documents[0];
    let caret = selection.head();
    let index = doc
        .blocks
        .nodes()
        .iter()
        .position(|node| {
            (matches!(node.kind, BlockKind::SourceBlock { .. })
                || document::is_call(&doc.snapshot, &node.kind, node.content))
                && caret >= node.source.start
                && (caret < node.source.end
                    || caret == node.source.end && caret.0 == snapshot.len_bytes())
        })
        .ok_or("Point is not inside an Org source block or #+CALL line")?;
    let mut root = resolver.entry(0, index, 0, &[], None)?;
    let doc = &resolver.documents[0];
    root.request
        .set_result_site(&doc.snapshot, &doc.blocks, index, selection)?;
    let cancel = CancelToken::default();
    fn share(invocation: &mut Invocation, cancel: &CancelToken) {
        invocation.request.cancel = cancel.clone();
        if let Some(mysql) = &mut invocation.request.mysql {
            mysql.cancel = cancel.clone();
        }
        fn share_input(input: &mut Input, cancel: &CancelToken) {
            match input {
                Input::Execution(i) => share(i, cancel),
                Input::Select(i, _) => share_input(i, cancel),
                Input::Data(_) => {}
            }
        }
        for (_, input) in &mut invocation.inputs {
            share_input(input, cancel);
        }
    }
    share(&mut root, &cancel);
    Ok(ExecutionPlan {
        root,
        versions: resolver.documents.iter().map(|d| d.version()).collect(),
        cancel,
        progress: RunProgress::default(),
    })
}

pub(crate) fn execute_plan(plan: ExecutionPlan) -> Result<PreparedBabelOutput, String> {
    struct Watchdog(
        Option<std::sync::mpsc::Sender<()>>,
        Option<std::thread::JoinHandle<()>>,
    );
    impl Drop for Watchdog {
        fn drop(&mut self) {
            if let Some(sender) = self.0.take() {
                let _ = sender.send(());
            }
            if let Some(thread) = self.1.take() {
                let _ = thread.join();
            }
        }
    }
    let (sender, receiver) = std::sync::mpsc::channel();
    let cancel = plan.cancel.clone();
    let _watchdog = Watchdog(
        Some(sender),
        Some(std::thread::spawn(move || {
            if receiver.recv_timeout(DEADLINE) == Err(std::sync::mpsc::RecvTimeoutError::Timeout) {
                cancel.cancel();
            }
        })),
    );
    fn evaluate(
        mut invocation: Invocation,
        deadline: Instant,
        bytes: &mut usize,
        progress: &RunProgress,
    ) -> Result<PreparedBabelOutput, String> {
        if invocation.request.cancel.cancelled() {
            return Err("Babel execution cancelled".into());
        }
        if Instant::now() >= deadline {
            return Err("Babel dependency execution exceeded 90 seconds".into());
        }
        fn evaluate_input(
            input: Input,
            deadline: Instant,
            bytes: &mut usize,
            progress: &RunProgress,
        ) -> Result<Data, String> {
            match input {
                Input::Data(data) => Ok(data),
                Input::Execution(invocation) => evaluate(*invocation, deadline, bytes, progress)
                    .map(|output| output.data.into_raw()),
                Input::Select(input, selector) => super::resolve::select(
                    evaluate_input(*input, deadline, bytes, progress)?,
                    &selector,
                ),
            }
        }
        let mut variables = Vec::new();
        for (name, input) in invocation.inputs {
            let data = evaluate_input(input, deadline, bytes, progress)
                .map_err(|error| format!("{} → {error}", invocation.location))?
                .preprocess(&invocation.request.headers)?;
            *bytes += data.value.byte_len();
            if *bytes > MAX_VALUE_BYTES {
                return Err("Babel values exceed the 8 MiB job limit".into());
            }
            if invocation.request.result_names.is_none()
                && (data.columns.is_some() || !data.row_names.is_empty())
            {
                invocation.request.result_names = Some(data.clone());
            }
            variables.push((name, data.value));
        }
        invocation.request.variables = variables;
        progress.set(format!(
            "{} · {}",
            invocation.request.language_name(),
            invocation.location
        ));
        let cancel = invocation.request.cancel.clone();
        let result = execution::execute_source_block(invocation.request)
            .map_err(|error| format!("{}: {error}", invocation.location))?;
        if cancel.cancelled() {
            return Err("Babel execution cancelled; previous results were preserved".into());
        }
        if Instant::now() >= deadline {
            return Err("Babel dependency execution exceeded 90 seconds".into());
        }
        Ok(result)
    }
    evaluate(plan.root, Instant::now() + DEADLINE, &mut 0, &plan.progress)
}

impl std::ops::Deref for ExecutionPlan {
    type Target = BabelExecutionRequest;
    fn deref(&self) -> &Self::Target {
        &self.root.request
    }
}
