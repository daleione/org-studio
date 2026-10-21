use std::collections::BTreeMap;

use crate::{
    document::{ByteRange, DocumentSnapshot, TextSnapshot},
    org_syntax::{BlockArena, BlockKind},
};

#[derive(Default)]
pub(super) struct Headers {
    arguments: BTreeMap<String, Vec<String>>,
    properties: BTreeMap<String, String>,
}

pub(super) struct HeaderToken {
    pub(super) text: String,
    pub(super) quoted: bool,
}

pub(super) fn tokenize(text: &str) -> Result<Vec<HeaderToken>, String> {
    let mut tokens = Vec::new();
    let mut word = String::new();
    let mut quoted = false;
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
            quoted = true;
            started = true;
            quote = Some(c);
        } else if c.is_whitespace() {
            if started {
                tokens.push(HeaderToken {
                    text: std::mem::take(&mut word),
                    quoted,
                });
                started = false;
                quoted = false;
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
        tokens.push(HeaderToken { text: word, quoted });
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

    pub(super) fn keys(&self) -> impl Iterator<Item = &str> {
        self.arguments.keys().map(String::as_str)
    }

    pub(super) fn property(&self, name: &str) -> Option<&str> {
        self.properties
            .get(&name.to_lowercase())
            .map(String::as_str)
    }

    fn extend(&mut self, other: Self) {
        self.arguments.extend(other.arguments);
        self.properties.extend(other.properties);
    }

    pub(super) fn parse(&mut self, text: &str) -> Result<(), String> {
        let mut key = None;
        for HeaderToken { text: word, quoted } in tokenize(text)? {
            if !quoted
                && word.starts_with(':')
                && word.len() > 1
                && word[1..]
                    .chars()
                    .all(|c| c.is_ascii_alphabetic() || c == '-')
            {
                let name = word.to_ascii_lowercase();
                self.arguments.insert(name.clone(), Vec::new());
                key = Some(name);
            } else if let Some(key) = &key {
                self.arguments.get_mut(key).unwrap().push(word);
            }
        }
        Ok(())
    }
}

pub(super) fn effective_headers(
    snapshot: &DocumentSnapshot,
    arena: &BlockArena,
    source_index: usize,
) -> Result<Headers, String> {
    let source = &arena.nodes()[source_index];
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
            apply_property(&mut general, &mut specific, name, args)?;
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
                        apply_property(general, specific, &rest[..end], &rest[end + 1..])?;
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
    let affiliated = affiliated_keywords(snapshot, arena, source_index).collect::<Vec<_>>();
    for text in affiliated.into_iter().rev() {
        if let Some((key, value)) = keyword(&text)
            && (key.eq_ignore_ascii_case("HEADER") || key.eq_ignore_ascii_case("HEADERS"))
        {
            headers.parse(value)?;
        }
    }
    let opening = snapshot.copy_range(ByteRange::new(
        source.source.start.0,
        source.content.start.0,
    ));
    headers.parse(&opening)?;
    Ok(headers)
}

fn apply_property(
    general: &mut Headers,
    specific: &mut Headers,
    name: &str,
    args: &str,
) -> Result<(), String> {
    let append = name.ends_with('+');
    let name = name.trim_end_matches('+');
    let target = if name.eq_ignore_ascii_case("header-args") {
        general
    } else if name.eq_ignore_ascii_case("header-args:sql") {
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
