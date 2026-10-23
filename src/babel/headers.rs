use std::collections::BTreeMap;

use crate::{
    document::{ByteRange, DocumentSnapshot, TextSnapshot},
    org_syntax::{BlockArena, BlockKind},
};

#[derive(Clone, Default)]
pub(super) struct Headers {
    arguments: BTreeMap<String, Vec<String>>,
    properties: BTreeMap<String, String>,
    pub(super) variables: Vec<(String, super::expression::Expression)>,
}

pub(super) fn tokenize(text: &str) -> Result<Vec<String>, String> {
    let mut tokens = Vec::new();
    let mut word = String::new();
    let mut quote = None;
    let mut escaped = false;
    let mut started = false;
    for c in text.chars() {
        if escaped {
            word.push(c);
            escaped = false;
        } else if c == '\\' && quote != Some('\'') {
            escaped = true;
            started = true;
        } else if let Some(delimiter) = quote {
            if c == delimiter {
                quote = None;
            } else {
                word.push(c);
            }
        } else if matches!(c, '\'' | '"') {
            started = true;
            quote = Some(c);
        } else if c.is_whitespace() {
            if started {
                tokens.push(std::mem::take(&mut word));
                started = false;
            }
        } else {
            word.push(c);
            started = true;
        }
    }
    if quote.is_some() {
        return Err("Unclosed quote in source block header".into());
    }
    if escaped {
        word.push('\\');
    }
    if started {
        tokens.push(word);
    }
    Ok(tokens)
}

impl Headers {
    pub(super) fn get(&self, name: &str) -> Option<&str> {
        self.arguments.get(name)?.first().map(String::as_str)
    }

    pub(super) fn values(&self, name: &str) -> &[String] {
        self.arguments
            .get(name)
            .map(Vec::as_slice)
            .unwrap_or_default()
    }

    pub(super) fn writes_result(&self) -> bool {
        !self
            .values(":results")
            .iter()
            .any(|mode| mode.eq_ignore_ascii_case("silent") || mode.eq_ignore_ascii_case("none"))
    }

    pub(super) fn keys(&self) -> impl Iterator<Item = &str> {
        self.arguments.keys().map(String::as_str)
    }

    pub(super) fn property(&self, name: &str) -> Option<&str> {
        self.properties
            .get(&name.to_lowercase())
            .map(String::as_str)
    }

    pub(super) fn extend(&mut self, other: Self) {
        for (name, expression) in other.variables {
            self.bind(name, expression);
        }
        for (key, values) in other.arguments {
            self.merge(key, values);
        }
        self.properties.extend(other.properties);
    }

    pub(super) fn bind(&mut self, name: String, expression: super::expression::Expression) {
        if let Some((_, old)) = self.variables.iter_mut().find(|(key, _)| *key == name) {
            *old = expression;
        } else {
            self.variables.push((name, expression));
        }
    }

    fn merge(&mut self, key: String, values: Vec<String>) {
        if key == ":results" {
            let current = self.arguments.entry(key).or_default();
            for value in values {
                const GROUPS: &[&[&str]] = &[
                    &["value", "output"],
                    &["table", "vector", "scalar", "list", "file"],
                    &["replace", "silent", "none", "append", "prepend"],
                    &["raw", "org", "html", "latex", "code", "drawer", "verbatim"],
                ];
                if let Some(group) = GROUPS.iter().find(|g| g.contains(&value.as_str())) {
                    current.retain(|v| !group.contains(&v.as_str()));
                }
                if !current.contains(&value) {
                    current.push(value);
                }
            }
        } else {
            self.arguments.insert(key, values);
        }
    }

    pub(super) fn parse(&mut self, text: &str) -> Result<(), String> {
        let parts = super::expression::split(text, char::is_whitespace)?;
        let mut key: Option<&str> = None;
        let mut group = Vec::new();
        let flush =
            |this: &mut Self, key: Option<&str>, group: &mut Vec<&str>| -> Result<(), String> {
                if let Some(key) = key {
                    let text = group.join(" ");
                    if key.eq_ignore_ascii_case(":var") {
                        for (name, expression) in super::expression::bindings(&text)? {
                            this.bind(name, expression);
                        }
                    } else {
                        this.merge(key.to_ascii_lowercase(), tokenize(&text)?);
                    }
                }
                group.clear();
                Ok(())
            };
        for part in parts {
            if part.starts_with(':')
                && part.len() > 1
                && part[1..]
                    .chars()
                    .all(|c| c.is_ascii_alphabetic() || c == '-')
            {
                flush(self, key, &mut group)?;
                key = Some(part);
            } else {
                group.push(part);
            }
        }
        flush(self, key, &mut group)
    }
}

pub(super) fn effective_headers(
    snapshot: &DocumentSnapshot,
    arena: &BlockArena,
    source_index: usize,
) -> Result<Headers, String> {
    effective_headers_for(snapshot, arena, source_index, None)
}

pub(super) fn effective_headers_for(
    snapshot: &DocumentSnapshot,
    arena: &BlockArena,
    source_index: usize,
    language: Option<&str>,
) -> Result<Headers, String> {
    let source = &arena.nodes()[source_index];
    let language = language.unwrap_or_else(|| match &source.kind {
        BlockKind::SourceBlock { language } => language.as_deref().unwrap_or(""),
        _ => "",
    });
    let mut general = Headers::default();
    let mut specific = Headers::default();
    // Only real keyword nodes participate: properties inside examples or source
    // blocks must never change a different block's execution policy.
    for node in arena.nodes() {
        if !matches!(node.kind, BlockKind::Keyword) {
            continue;
        }
        let text = snapshot.copy_range(node.content);
        let Some((key, value)) = keyword(&text) else {
            continue;
        };
        if key.eq_ignore_ascii_case("PROPERTY") {
            let (name, args) = value.split_once(char::is_whitespace).unwrap_or((value, ""));
            apply_property(&mut general, &mut specific, name, args, language)?;
        }
    }
    let mut frames: Vec<(u16, Headers, Headers)> = Vec::new();
    for node in &arena.nodes()[..source_index] {
        match &node.kind {
            BlockKind::Heading { level } => {
                while frames.last().is_some_and(|frame| frame.0 >= *level) {
                    frames.pop();
                }
                frames.push((*level, Headers::default(), Headers::default()));
            }
            BlockKind::Drawer { name } if name.eq_ignore_ascii_case("PROPERTIES") => {
                let (general, specific) = frames
                    .last_mut()
                    .map(|(_, general, specific)| (general, specific))
                    .unwrap_or((&mut general, &mut specific));
                let content = snapshot.copy_range(node.content);
                for line in content.lines() {
                    let line = line.trim();
                    // Property names themselves can contain colons.
                    if let Some(rest) = line.strip_prefix(':')
                        && let Some((end, _)) = rest.match_indices(':').find(|(index, _)| {
                            let value = &rest[index + 1..];
                            value.is_empty() || value.starts_with(char::is_whitespace)
                        })
                    {
                        apply_property(
                            general,
                            specific,
                            &rest[..end],
                            &rest[end + 1..],
                            language,
                        )?;
                    }
                }
            }
            _ => {}
        }
    }
    let mut headers = general;
    headers.extend(specific);
    for (_, general, specific) in frames {
        headers.extend(general);
        headers.extend(specific);
    }
    let opening = snapshot.copy_range(ByteRange::new(
        source.source.start.0,
        source.content.start.0,
    ));
    if matches!(source.kind, BlockKind::SourceBlock { .. }) {
        headers.parse(&opening)?;
    }
    let affiliated = affiliated_keywords(snapshot, arena, source_index).collect::<Vec<_>>();
    for text in affiliated.into_iter().rev() {
        if let Some((key, value)) = keyword(&text)
            && (key.eq_ignore_ascii_case("HEADER") || key.eq_ignore_ascii_case("HEADERS"))
        {
            headers.parse(value)?;
        }
    }
    Ok(headers)
}

fn apply_property(
    general: &mut Headers,
    specific: &mut Headers,
    name: &str,
    args: &str,
    language: &str,
) -> Result<(), String> {
    let append = name.ends_with('+');
    let name = name.trim_end_matches('+');
    let target = if name.eq_ignore_ascii_case("header-args") {
        general
    } else if name
        .to_ascii_lowercase()
        .strip_prefix("header-args:")
        .is_some_and(|l| l.eq_ignore_ascii_case(language))
    {
        specific
    } else {
        let value = general.properties.entry(name.to_lowercase()).or_default();
        if append && !value.is_empty() {
            value.push(' ');
        } else {
            value.clear();
        }
        value.push_str(args.trim());
        return Ok(());
    };
    if !append {
        target.arguments.clear();
        target.variables.clear();
    }
    target.parse(args)
}

pub(super) fn keyword(text: &str) -> Option<(&str, &str)> {
    let (key, value) = text.trim().strip_prefix("#+")?.split_once(':')?;
    Some((key, value.trim()))
}

pub(super) fn source_name(
    snapshot: &DocumentSnapshot,
    arena: &BlockArena,
    index: usize,
) -> Option<String> {
    affiliated_keywords(snapshot, arena, index).find_map(|text| {
        let (key, value) = keyword(&text)?;
        key.eq_ignore_ascii_case("NAME").then(|| value.to_owned())
    })
}

fn affiliated_keywords<'a>(
    snapshot: &'a DocumentSnapshot,
    arena: &'a BlockArena,
    index: usize,
) -> impl Iterator<Item = String> + 'a {
    let parent = arena.nodes()[index].parent;
    arena.nodes()[..index]
        .iter()
        .rev()
        .take_while(move |node| node.parent == parent && matches!(node.kind, BlockKind::Keyword))
        .map(|node| snapshot.copy_range(node.content))
        .take_while(|text| {
            crate::org_syntax::attributes::is_affiliated_keyword(text)
                || keyword(text).is_some_and(|(key, _)| key.eq_ignore_ascii_case("HEADERS"))
        })
}
