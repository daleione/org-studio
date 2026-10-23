use super::{
    document::{Document, path_key},
    execution::{self, BabelExecutionRequest},
    expression::{self, Argument, Expression},
    headers::{self, Headers},
    value::{Data, Value},
};
use crate::{
    document::{DocumentSnapshot, Selection, TextSnapshot},
    org_syntax::BlockKind,
};
use std::{
    collections::HashMap,
    path::{Path, PathBuf},
};

pub(super) enum Input {
    Data(Data),
    Execution(Box<Invocation>),
    Select(Box<Input>, String),
}

pub(super) struct Invocation {
    pub request: BabelExecutionRequest,
    pub inputs: Vec<(String, Input)>,
    pub path: PathBuf,
    pub location: String,
    pub definition_name: Option<String>,
}

pub(super) struct Resolver {
    pub documents: Vec<Document>,
    open: HashMap<PathBuf, DocumentSnapshot>,
    stack: Vec<(usize, usize)>,
    invocations: usize,
}

impl Resolver {
    pub fn new(
        path: &Path,
        snapshot: &DocumentSnapshot,
        open: Vec<(PathBuf, DocumentSnapshot)>,
    ) -> Result<Self, String> {
        Ok(Self {
            documents: vec![Document::new(path_key(path), snapshot.clone())?],
            open: open.into_iter().map(|(p, s)| (path_key(&p), s)).collect(),
            stack: Vec::new(),
            invocations: 0,
        })
    }

    fn target(&mut self, context: usize, name: &str) -> Result<(usize, usize), String> {
        let full_name = name;
        let (document, name) = if let Some((file, name)) = name.rsplit_once(':') {
            if !file.to_ascii_lowercase().ends_with(".org") {
                return self.documents[context]
                    .named(full_name)
                    .map(|i| (context, i));
            }
            let path = path_key(
                &self.documents[context]
                    .path
                    .parent()
                    .unwrap_or(Path::new("."))
                    .join(file),
            );
            if !path.exists()
                && !self
                    .open
                    .keys()
                    .any(|p| super::document::same_path(p, &path))
                && let Ok(index) = self.documents[context].named(full_name)
            {
                return Ok((context, index));
            }
            let index = match self
                .documents
                .iter()
                .position(|d| super::document::same_path(&d.path, &path))
            {
                Some(index) => index,
                None => {
                    let snapshot = if let Some(snapshot) = self.open.get(&path).or_else(|| {
                        self.open
                            .iter()
                            .find(|(p, _)| super::document::same_path(p, &path))
                            .map(|(_, s)| s)
                    }) {
                        snapshot.clone()
                    } else {
                        let file = std::fs::File::open(&path).map_err(|e| {
                            format!("Could not read Babel library {}: {e}", path.display())
                        })?;
                        use std::io::Read;
                        let mut bytes = Vec::new();
                        file.take(16 * 1024 * 1024 + 1)
                            .read_to_end(&mut bytes)
                            .map_err(|e| e.to_string())?;
                        if bytes.len() > 16 * 1024 * 1024 {
                            return Err("Babel library exceeds 16 MiB".into());
                        }
                        DocumentSnapshot::from_utf8(bytes)
                            .map_err(|_| "Babel libraries must be UTF-8 Org documents")?
                    };
                    if self.documents.len() >= super::run::MAX_INVOCATIONS
                        || self
                            .documents
                            .iter()
                            .map(|d| d.snapshot.len_bytes())
                            .sum::<u64>()
                            + snapshot.len_bytes()
                            > 32 * 1024 * 1024
                    {
                        return Err("Babel snapshots exceed 64 documents or 32 MiB".into());
                    }
                    let index = self.documents.len();
                    self.documents.push(Document::new(path, snapshot)?);
                    index
                }
            };
            (index, name)
        } else {
            (context, name)
        };
        Ok((document, self.documents[document].named(name)?))
    }

    pub fn entry(
        &mut self,
        document: usize,
        index: usize,
        context: usize,
        arguments: &[Argument],
        call_headers: Option<Headers>,
    ) -> Result<Invocation, String> {
        if self.stack.contains(&(document, index)) {
            let mut path = self
                .stack
                .iter()
                .map(|(d, i)| {
                    headers::source_name(
                        &self.documents[*d].snapshot,
                        &self.documents[*d].blocks,
                        *i,
                    )
                    .unwrap_or_else(|| {
                        format!(
                            "line at {}",
                            self.documents[*d].blocks.nodes()[*i].source.start.0
                        )
                    })
                })
                .collect::<Vec<_>>();
            path.push(
                headers::source_name(
                    &self.documents[document].snapshot,
                    &self.documents[document].blocks,
                    index,
                )
                .unwrap_or_else(|| "CALL".into()),
            );
            return Err(format!("Babel dependency cycle: {}", path.join(" → ")));
        }
        if self.stack.len() >= super::run::MAX_DEPTH
            || self.invocations >= super::run::MAX_INVOCATIONS
        {
            return Err("Babel dependency limit exceeded (32 levels / 64 invocations)".into());
        }
        self.invocations += 1;
        self.stack.push((document, index));
        let result = self.build_entry(document, index, context, arguments, call_headers);
        self.stack.pop();
        result
    }

    fn build_entry(
        &mut self,
        document: usize,
        index: usize,
        context: usize,
        arguments: &[Argument],
        override_headers: Option<Headers>,
    ) -> Result<Invocation, String> {
        let doc = &self.documents[document];
        let node = &doc.blocks.nodes()[index];
        if super::document::is_call(&doc.snapshot, &node.kind, node.content) {
            let text = doc.snapshot.copy_range(node.content);
            let call = expression::call(headers::keyword(&text).unwrap().1)?;
            let (target_document, target_index) = self.target(document, &call.target)?;
            let target = &self.documents[target_document];
            let language = match &target.blocks.nodes()[target_index].kind {
                BlockKind::SourceBlock { language } => language.as_deref(),
                _ => return Err("#+CALL must target a named source block".into()),
            };
            let mut headers =
                headers::effective_headers(&target.snapshot, &target.blocks, target_index)?;
            let doc = &self.documents[document];
            headers.extend(headers::effective_headers_for(
                &doc.snapshot,
                &doc.blocks,
                index,
                language,
            )?);
            headers.parse(&call.inside)?;
            bind_arguments(&mut headers, &call.arguments)?;
            headers.parse(&call.end)?;
            bind_arguments(&mut headers, arguments)?;
            if let Some(overrides) = override_headers {
                headers.extend(overrides);
            }
            let mut invocation =
                self.entry(target_document, target_index, document, &[], Some(headers))?;
            let doc = &self.documents[document];
            invocation.request.set_result_site(
                &doc.snapshot,
                &doc.blocks,
                index,
                Selection::caret(node_start(doc, index)),
            )?;
            invocation.location = format!("{} → {}", location(doc, index), invocation.location);
            return Ok(invocation);
        }
        if !matches!(node.kind, BlockKind::SourceBlock { .. }) {
            return Err("Babel execution requires a source block or #+CALL".into());
        }
        let mut headers = match override_headers {
            Some(headers) => headers,
            None => headers::effective_headers(&doc.snapshot, &doc.blocks, index)?,
        };
        bind_arguments(&mut headers, arguments)?;
        let request = execution::prepare_at(
            &doc.snapshot,
            &self.documents[context].path,
            Selection::caret(node.source.start),
            &doc.blocks,
            index,
            Some(headers.clone()),
        )?;
        if request.external_program().is_none()
            && request.mysql.is_none()
            && !headers.variables.is_empty()
        {
            return Err(format!(
                "{} does not support :var yet",
                request.language_name()
            ));
        }
        let mut inputs = Vec::new();
        for (name, expression) in &headers.variables {
            let input = self.input(context, expression)?;
            if let Input::Data(data) = &input
                && matches!(
                    request.executor,
                    execution::BabelExecutor::Shell
                        | execution::BabelExecutor::Bash
                        | execution::BabelExecutor::MySql
                )
                && matches!(data.value, Value::List(_) | Value::Hline)
            {
                return Err(format!(
                    "{} variable {name} must be a scalar",
                    request.language_name()
                ));
            }
            inputs.push((name.clone(), input));
        }
        Ok(Invocation {
            request,
            inputs,
            path: self.documents[document].path.clone(),
            location: location(&self.documents[document], index),
            definition_name: headers::source_name(
                &self.documents[document].snapshot,
                &self.documents[document].blocks,
                index,
            ),
        })
    }

    fn input(&mut self, context: usize, expression: &Expression) -> Result<Input, String> {
        let input = match expression {
            Expression::Literal(value) => {
                return Ok(Input::Data(Data {
                    value: value.clone(),
                    ..Data::default()
                }));
            }
            Expression::Reference {
                target,
                arguments,
                selector,
            } => {
                let (document, index) = self.target(context, target)?;
                let doc = &self.documents[document];
                let input = if matches!(doc.blocks.nodes()[index].kind, BlockKind::TableRow) {
                    if !arguments.is_empty() {
                        return Err("Static tables do not accept call arguments".into());
                    }
                    Input::Data(doc.table(index)?)
                } else {
                    let invocation = self.entry(document, index, document, arguments, None)?;
                    if invocation.request.file_result() {
                        return Err(
                            "File results cannot be used as Babel variable dependencies yet".into(),
                        );
                    }
                    Input::Execution(Box::new(invocation))
                };
                if let Some(selector) = selector {
                    match input {
                        Input::Data(data) => Input::Data(select(data, selector)?),
                        input => Input::Select(Box::new(input), selector.clone()),
                    }
                } else {
                    input
                }
            }
        };
        Ok(input)
    }
}

fn location(doc: &Document, index: usize) -> String {
    let start = node_start(doc, index);
    let line = doc.snapshot.line_of_byte(start) + 1;
    let name = headers::source_name(&doc.snapshot, &doc.blocks, index)
        .map(|n| format!(" · {n}"))
        .unwrap_or_default();
    format!(
        "{}:{line}{name}",
        doc.path.file_name().unwrap_or_default().to_string_lossy()
    )
}

fn node_start(doc: &Document, index: usize) -> crate::document::ByteOffset {
    doc.blocks.nodes()[index].source.start
}

fn bind_arguments(headers: &mut Headers, arguments: &[Argument]) -> Result<(), String> {
    let names = headers
        .variables
        .iter()
        .map(|(n, _)| n.clone())
        .collect::<Vec<_>>();
    let mut position = 0;
    let mut named = false;
    for argument in arguments {
        let name = if let Some(name) = &argument.name {
            named = true;
            name.clone()
        } else {
            if named {
                return Err("Positional Babel arguments must precede named arguments".into());
            }
            let name = names
                .get(position)
                .ok_or("Too many positional Babel arguments")?
                .clone();
            position += 1;
            name
        };
        headers.bind(name, argument.expression.clone());
    }
    Ok(())
}

pub(super) fn select(data: Data, selector: &str) -> Result<Data, String> {
    let selectors = selector.split(',').map(str::trim).collect::<Vec<_>>();
    if selectors.is_empty() || selectors.len() > 2 {
        return Err("Use table[row,column] or table[row-range,column-range]".into());
    }
    fn choose(value: Value, selector: &str) -> Result<Value, String> {
        let Value::List(values) = value else {
            return Err("Babel selectors require a list or table".into());
        };
        let len = values.len();
        let index = |text: &str| -> Result<usize, String> {
            let n = text
                .trim()
                .parse::<isize>()
                .map_err(|_| "Invalid Babel table index")?;
            let n = if n < 0 { len as isize + n } else { n };
            if n < 0 || n >= len as isize {
                return Err("Babel table index is out of range".into());
            }
            Ok(n as usize)
        };
        if selector == "*" || selector.is_empty() {
            return Ok(Value::List(values));
        }
        if let Some((start, end)) = selector.split_once(':') {
            let start = if start.is_empty() { 0 } else { index(start)? };
            let end = if end.is_empty() {
                len.saturating_sub(1)
            } else {
                index(end)?
            };
            if start > end {
                return Err("Babel table range starts after its end".into());
            }
            return Ok(Value::List(
                values.get(start..=end).unwrap_or_default().to_vec(),
            ));
        }
        Ok(values[index(selector)?].clone())
    }
    let mut value = choose(data.value, selectors[0])?;
    if let Some(column) = selectors.get(1) {
        let row_scalar =
            !selectors[0].is_empty() && !selectors[0].contains(':') && selectors[0] != "*";
        value = if row_scalar {
            choose(value, column)?
        } else {
            let Value::List(rows) = value else {
                unreachable!()
            };
            Value::List(
                rows.into_iter()
                    .map(|row| choose(row, column))
                    .collect::<Result<_, _>>()?,
            )
        };
    }
    Ok(Data {
        value,
        ..Data::default()
    })
}
