use std::{collections::HashMap, fmt, ops::Range, sync::Arc};

use crate::command::CommandKey;

const CONTROL: u8 = 1 << 0;
const META: u8 = 1 << 1;
const SHIFT: u8 = 1 << 2;
const COMMAND: u8 = 1 << 3;
const MAX_MINOR_OVERLAYS: usize = 3;

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct KeyStroke {
    key: Arc<str>,
    modifiers: u8,
}

impl KeyStroke {
    pub fn new(
        key: impl Into<Arc<str>>,
        control: bool,
        meta: bool,
        mut shift: bool,
        command: bool,
    ) -> Self {
        let key = key.into();
        let key = match key.as_ref() {
            "?" => {
                shift = true;
                "/"
            }
            "^" => {
                shift = true;
                "6"
            }
            "<" => {
                shift = true;
                ","
            }
            ">" => {
                shift = true;
                "."
            }
            _ => key.as_ref(),
        };
        let mut modifiers = 0;
        if control {
            modifiers |= CONTROL;
        }
        if meta {
            modifiers |= META;
        }
        if shift {
            modifiers |= SHIFT;
        }
        if command {
            modifiers |= COMMAND;
        }
        Self {
            key: normalize_key(key),
            modifiers,
        }
    }

    pub fn with_meta(mut self) -> Self {
        self.modifiers |= META;
        self
    }

    pub fn parse(source: &str) -> Result<Self, KeyParseError> {
        let source = source.trim();
        if source.is_empty() {
            return Err(KeyParseError::EmptyStroke);
        }
        let mut rest = source;
        let mut modifiers = 0;
        loop {
            let Some((modifier, tail)) = modifier_prefix(rest) else {
                break;
            };
            if modifiers & modifier != 0 {
                return Err(KeyParseError::DuplicateModifier(source.into()));
            }
            modifiers |= modifier;
            rest = tail;
        }
        if rest.is_empty() {
            return Err(KeyParseError::MissingKey(source.into()));
        }
        if rest == "<" || rest == ">" {
            modifiers |= SHIFT;
        }
        let key = match rest {
            "<" => Arc::from(","),
            ">" => Arc::from("."),
            _ => normalize_key(rest),
        };
        Ok(Self { key, modifiers })
    }

    pub fn key(&self) -> &str {
        &self.key
    }

    pub fn control(&self) -> bool {
        self.modifiers & CONTROL != 0
    }

    pub fn meta(&self) -> bool {
        self.modifiers & META != 0
    }

    pub fn shift(&self) -> bool {
        self.modifiers & SHIFT != 0
    }

    pub fn command(&self) -> bool {
        self.modifiers & COMMAND != 0
    }

    pub fn notation(&self) -> String {
        let mut notation = String::with_capacity(self.key.len() + 8);
        if self.control() {
            notation.push_str("C-");
        }
        if self.meta() {
            notation.push_str("M-");
        }
        if self.shift() {
            notation.push_str("S-");
        }
        if self.command() {
            notation.push_str("s-");
        }
        notation.push_str(match self.key() {
            "enter" => "RET",
            "escape" => "ESC",
            "space" => "SPC",
            key => key,
        });
        notation
    }
}

fn modifier_prefix(source: &str) -> Option<(u8, &str)> {
    const PREFIXES: [(&str, u8); 12] = [
        ("Control-", CONTROL),
        ("control-", CONTROL),
        ("Command-", COMMAND),
        ("command-", COMMAND),
        ("Shift-", SHIFT),
        ("shift-", SHIFT),
        ("ctrl-", CONTROL),
        ("cmd-", COMMAND),
        ("alt-", META),
        ("C-", CONTROL),
        ("M-", META),
        ("S-", SHIFT),
    ];
    PREFIXES
        .iter()
        .find_map(|(prefix, modifier)| source.strip_prefix(prefix).map(|tail| (*modifier, tail)))
}

fn normalize_key(source: &str) -> Arc<str> {
    match source.to_ascii_lowercase().as_str() {
        "ret" | "return" | "enter" => Arc::from("enter"),
        "esc" | "escape" => Arc::from("escape"),
        "spc" | "space" => Arc::from("space"),
        "tab" => Arc::from("tab"),
        "backspace" | "bs" => Arc::from("backspace"),
        "delete" | "del" => Arc::from("delete"),
        "up" | "down" | "left" | "right" | "home" | "end" | "pageup" | "pagedown" => {
            Arc::from(source.to_ascii_lowercase())
        }
        _ => Arc::from(source.to_ascii_lowercase()),
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct KeySequence(Vec<KeyStroke>);

impl KeySequence {
    pub fn parse(source: &str) -> Result<Self, KeyParseError> {
        let strokes = source
            .split_ascii_whitespace()
            .map(KeyStroke::parse)
            .collect::<Result<Vec<_>, _>>()?;
        if strokes.is_empty() {
            Err(KeyParseError::EmptySequence)
        } else {
            Ok(Self(strokes))
        }
    }

    pub fn strokes(&self) -> &[KeyStroke] {
        &self.0
    }
}

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct StrokeKey(u32);

impl StrokeKey {
    pub fn index(self) -> usize {
        self.0 as usize
    }
}

#[derive(Clone, Default)]
pub struct StrokeInterner {
    keys: HashMap<KeyStroke, StrokeKey>,
    strokes: Vec<KeyStroke>,
}

impl StrokeInterner {
    pub fn intern(&mut self, stroke: KeyStroke) -> StrokeKey {
        if let Some(key) = self.keys.get(&stroke) {
            return *key;
        }
        let key = StrokeKey(self.strokes.len() as u32);
        self.strokes.push(stroke.clone());
        self.keys.insert(stroke, key);
        key
    }

    pub fn intern_sequence(&mut self, sequence: &KeySequence) -> Vec<StrokeKey> {
        sequence
            .strokes()
            .iter()
            .cloned()
            .map(|stroke| self.intern(stroke))
            .collect()
    }

    pub fn lookup(&self, stroke: &KeyStroke) -> Option<StrokeKey> {
        self.keys.get(stroke).copied()
    }

    pub fn stroke(&self, key: StrokeKey) -> Option<&KeyStroke> {
        self.strokes.get(key.index())
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
enum TerminalBinding {
    #[default]
    None,
    Command(CommandKey),
    Disabled,
    PassThrough,
}

#[derive(Default)]
struct BuilderNode {
    terminal: TerminalBinding,
    children: HashMap<StrokeKey, usize>,
}

pub struct KeymapBuilder {
    nodes: Vec<BuilderNode>,
}

impl Default for KeymapBuilder {
    fn default() -> Self {
        Self::new()
    }
}

impl KeymapBuilder {
    pub fn new() -> Self {
        Self {
            nodes: vec![BuilderNode::default()],
        }
    }

    pub fn bind(
        &mut self,
        sequence: &[StrokeKey],
        command: CommandKey,
    ) -> Result<(), KeymapBuildError> {
        self.insert(sequence, TerminalBinding::Command(command))
    }

    pub fn disable(&mut self, sequence: &[StrokeKey]) -> Result<(), KeymapBuildError> {
        self.insert(sequence, TerminalBinding::Disabled)
    }

    pub fn pass_through(&mut self, sequence: &[StrokeKey]) -> Result<(), KeymapBuildError> {
        self.insert(sequence, TerminalBinding::PassThrough)
    }

    fn insert(
        &mut self,
        sequence: &[StrokeKey],
        terminal: TerminalBinding,
    ) -> Result<(), KeymapBuildError> {
        if sequence.is_empty() {
            return Err(KeymapBuildError::EmptySequence);
        }
        let mut node = 0;
        for stroke in sequence {
            if self.nodes[node].terminal != TerminalBinding::None {
                return Err(KeymapBuildError::TerminalConflictsWithPrefix);
            }
            let next = if let Some(next) = self.nodes[node].children.get(stroke) {
                *next
            } else {
                let next = self.nodes.len();
                self.nodes.push(BuilderNode::default());
                self.nodes[node].children.insert(*stroke, next);
                next
            };
            node = next;
        }
        if self.nodes[node].terminal != TerminalBinding::None {
            return Err(KeymapBuildError::DuplicateSequence);
        }
        if !self.nodes[node].children.is_empty() && terminal != TerminalBinding::None {
            return Err(KeymapBuildError::TerminalConflictsWithPrefix);
        }
        self.nodes[node].terminal = terminal;
        Ok(())
    }

    pub fn freeze(self, generation: u64) -> CompiledKeymap {
        let mut nodes = Vec::with_capacity(self.nodes.len());
        let edge_capacity = self.nodes.iter().map(|node| node.children.len()).sum();
        let mut edges = Vec::with_capacity(edge_capacity);
        for node in self.nodes {
            let start = edges.len() as u32;
            let mut children = node.children.into_iter().collect::<Vec<_>>();
            children.sort_unstable_by_key(|(stroke, _)| *stroke);
            edges.extend(children.into_iter().map(|(stroke, target)| FrozenEdge {
                stroke,
                target: target as u32,
            }));
            nodes.push(FrozenNode {
                terminal: node.terminal,
                edges: start..edges.len() as u32,
            });
        }
        CompiledKeymap {
            generation,
            nodes: nodes.into_boxed_slice(),
            edges: edges.into_boxed_slice(),
        }
    }
}

#[derive(Clone, Debug)]
struct FrozenNode {
    terminal: TerminalBinding,
    edges: Range<u32>,
}

#[derive(Clone, Copy, Debug)]
struct FrozenEdge {
    stroke: StrokeKey,
    target: u32,
}

#[derive(Clone, Debug)]
pub struct CompiledKeymap {
    generation: u64,
    nodes: Box<[FrozenNode]>,
    edges: Box<[FrozenEdge]>,
}

impl CompiledKeymap {
    pub fn generation(&self) -> u64 {
        self.generation
    }

    pub fn lookup(&self, sequence: &[StrokeKey]) -> KeyLookup {
        if sequence.is_empty() {
            return KeyLookup::None;
        }
        let mut node_index = 0_u32;
        for stroke in sequence {
            let Some(node) = self.nodes.get(node_index as usize) else {
                return KeyLookup::None;
            };
            let edges = &self.edges[node.edges.start as usize..node.edges.end as usize];
            let Ok(index) = edges.binary_search_by_key(stroke, |edge| edge.stroke) else {
                return KeyLookup::None;
            };
            node_index = edges[index].target;
        }
        let Some(node) = self.nodes.get(node_index as usize) else {
            return KeyLookup::None;
        };
        if !node.edges.is_empty() {
            KeyLookup::Prefix
        } else {
            match node.terminal {
                TerminalBinding::None => KeyLookup::None,
                TerminalBinding::Command(command) => KeyLookup::Command(command),
                TerminalBinding::Disabled => KeyLookup::Disabled,
                TerminalBinding::PassThrough => KeyLookup::PassThrough,
            }
        }
    }

    pub fn continuations(&self, sequence: &[StrokeKey]) -> Vec<KeyContinuation> {
        let mut node_index = 0_u32;
        for stroke in sequence {
            let node = match self.nodes.get(node_index as usize) {
                Some(node) => node,
                None => return Vec::new(),
            };
            let edges = &self.edges[node.edges.start as usize..node.edges.end as usize];
            let Ok(index) = edges.binary_search_by_key(stroke, |edge| edge.stroke) else {
                return Vec::new();
            };
            node_index = edges[index].target;
        }
        let Some(node) = self.nodes.get(node_index as usize) else {
            return Vec::new();
        };
        self.edges[node.edges.start as usize..node.edges.end as usize]
            .iter()
            .filter_map(|edge| {
                let child = self.nodes.get(edge.target as usize)?;
                let lookup = if !child.edges.is_empty() {
                    KeyLookup::Prefix
                } else {
                    match child.terminal {
                        TerminalBinding::None => KeyLookup::None,
                        TerminalBinding::Command(command) => KeyLookup::Command(command),
                        TerminalBinding::Disabled => KeyLookup::Disabled,
                        TerminalBinding::PassThrough => KeyLookup::PassThrough,
                    }
                };
                Some(KeyContinuation {
                    stroke: edge.stroke,
                    lookup,
                })
            })
            .collect()
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct KeyContinuation {
    pub stroke: StrokeKey,
    pub lookup: KeyLookup,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum KeyLookup {
    None,
    Prefix,
    Command(CommandKey),
    Disabled,
    PassThrough,
}

#[derive(Clone, Debug)]
pub struct ActiveKeymaps {
    generation: u64,
    transient: Option<Arc<CompiledKeymap>>,
    minor: Vec<Arc<CompiledKeymap>>,
    base: Arc<CompiledKeymap>,
}

impl ActiveKeymaps {
    pub fn new(
        generation: u64,
        transient: Option<Arc<CompiledKeymap>>,
        minor: Vec<Arc<CompiledKeymap>>,
        base: Arc<CompiledKeymap>,
    ) -> Result<Self, ActiveKeymapError> {
        if minor.len() > MAX_MINOR_OVERLAYS {
            return Err(ActiveKeymapError::TooManyMinorOverlays {
                supplied: minor.len(),
                maximum: MAX_MINOR_OVERLAYS,
            });
        }
        Ok(Self {
            generation,
            transient,
            minor,
            base,
        })
    }

    pub fn generation(&self) -> u64 {
        self.generation
    }

    pub fn resolve(&self, sequence: &[StrokeKey]) -> KeyLookup {
        let mut pass_through = false;
        let layers = self
            .transient
            .iter()
            .chain(self.minor.iter())
            .chain(std::iter::once(&self.base));
        for layer in layers {
            match layer.lookup(sequence) {
                KeyLookup::None => continue,
                KeyLookup::PassThrough => pass_through = true,
                result => return result,
            }
        }
        if pass_through {
            KeyLookup::PassThrough
        } else {
            KeyLookup::None
        }
    }

    pub fn with_transient(&self, generation: u64, transient: Arc<CompiledKeymap>) -> Arc<Self> {
        Arc::new(Self {
            generation,
            transient: Some(transient),
            minor: self.minor.clone(),
            base: self.base.clone(),
        })
    }

    pub fn continuations(&self, sequence: &[StrokeKey]) -> Vec<KeyContinuation> {
        let mut merged = HashMap::<StrokeKey, KeyLookup>::new();
        let layers = self
            .transient
            .iter()
            .chain(self.minor.iter())
            .chain(std::iter::once(&self.base));
        for layer in layers {
            for candidate in layer.continuations(sequence) {
                match candidate.lookup {
                    KeyLookup::PassThrough => {}
                    lookup => {
                        merged.entry(candidate.stroke).or_insert(lookup);
                    }
                }
            }
        }
        let mut candidates = merged
            .into_iter()
            .map(|(stroke, lookup)| KeyContinuation { stroke, lookup })
            .collect::<Vec<_>>();
        candidates.sort_unstable_by_key(|candidate| candidate.stroke);
        candidates
    }
}

#[derive(Clone, Debug)]
pub struct PendingSequence {
    keymaps: Arc<ActiveKeymaps>,
    strokes: Vec<StrokeKey>,
}

impl PendingSequence {
    pub fn new(keymaps: Arc<ActiveKeymaps>) -> Self {
        Self {
            keymaps,
            strokes: Vec::with_capacity(4),
        }
    }

    pub fn generation(&self) -> u64 {
        self.keymaps.generation()
    }

    pub fn feed(&mut self, stroke: StrokeKey) -> ResolveOutcome {
        self.strokes.push(stroke);
        match self.keymaps.resolve(&self.strokes) {
            KeyLookup::Prefix => ResolveOutcome::Pending,
            KeyLookup::Command(command) => {
                self.strokes.clear();
                ResolveOutcome::Command(command)
            }
            KeyLookup::Disabled => {
                self.strokes.clear();
                ResolveOutcome::Disabled
            }
            KeyLookup::PassThrough => {
                self.strokes.clear();
                ResolveOutcome::PassThrough
            }
            KeyLookup::None => {
                self.strokes.clear();
                ResolveOutcome::Undefined
            }
        }
    }

    pub fn cancel(&mut self) {
        self.strokes.clear();
    }

    pub fn is_pending(&self) -> bool {
        !self.strokes.is_empty()
    }

    pub fn strokes(&self) -> &[StrokeKey] {
        &self.strokes
    }

    pub fn keymaps(&self) -> Arc<ActiveKeymaps> {
        self.keymaps.clone()
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ResolveOutcome {
    Pending,
    Command(CommandKey),
    Disabled,
    PassThrough,
    Undefined,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum KeyParseError {
    EmptyStroke,
    EmptySequence,
    MissingKey(Arc<str>),
    DuplicateModifier(Arc<str>),
}

impl fmt::Display for KeyParseError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{self:?}")
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum KeymapBuildError {
    EmptySequence,
    DuplicateSequence,
    TerminalConflictsWithPrefix,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ActiveKeymapError {
    TooManyMinorOverlays { supplied: usize, maximum: usize },
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(index: u32) -> CommandKey {
        CommandKey::from_index(index)
    }

    fn sequence(interner: &mut StrokeInterner, source: &str) -> Vec<StrokeKey> {
        interner.intern_sequence(&KeySequence::parse(source).unwrap())
    }

    #[test]
    fn parses_emacs_and_platform_spellings() {
        let emacs = KeyStroke::parse("C-x").unwrap();
        let platform = KeyStroke::parse("ctrl-x").unwrap();
        assert_eq!(emacs, platform);
        assert!(emacs.control());
        assert_eq!(KeyStroke::parse("M-S-RET").unwrap().key(), "enter");
        assert!(KeyStroke::parse("cmd-o").unwrap().command());
        assert_eq!(
            KeyStroke::new("?", false, false, false, false),
            KeyStroke::parse("S-/").unwrap()
        );
        assert_eq!(
            KeyStroke::new("^", false, false, false, false),
            KeyStroke::parse("S-6").unwrap()
        );
    }

    #[test]
    fn frozen_trie_resolves_prefix_and_command() {
        let mut interner = StrokeInterner::default();
        let open = sequence(&mut interner, "C-x C-f");
        let reload = sequence(&mut interner, "C-x C-r");
        let mut builder = KeymapBuilder::new();
        builder.bind(&open, key(1)).unwrap();
        builder.bind(&reload, key(2)).unwrap();
        let map = builder.freeze(7);
        assert_eq!(map.lookup(&open[..1]), KeyLookup::Prefix);
        assert_eq!(map.lookup(&open), KeyLookup::Command(key(1)));
        assert_eq!(map.generation(), 7);
    }

    #[test]
    fn overlay_can_disable_or_pass_through_to_base() {
        let mut interner = StrokeInterner::default();
        let mark = sequence(&mut interner, "m");
        let open = sequence(&mut interner, "enter");
        let mut base = KeymapBuilder::new();
        base.bind(&mark, key(1)).unwrap();
        base.bind(&open, key(2)).unwrap();
        let mut overlay = KeymapBuilder::new();
        overlay.disable(&mark).unwrap();
        overlay.pass_through(&open).unwrap();
        let active = ActiveKeymaps::new(
            1,
            None,
            vec![Arc::new(overlay.freeze(1))],
            Arc::new(base.freeze(1)),
        )
        .unwrap();
        assert_eq!(active.resolve(&mark), KeyLookup::Disabled);
        assert_eq!(active.resolve(&open), KeyLookup::Command(key(2)));
    }

    #[test]
    fn pending_sequence_keeps_its_compiled_generation() {
        let mut interner = StrokeInterner::default();
        let open = sequence(&mut interner, "C-x C-f");
        let mut builder = KeymapBuilder::new();
        builder.bind(&open, key(9)).unwrap();
        let active =
            Arc::new(ActiveKeymaps::new(11, None, vec![], Arc::new(builder.freeze(11))).unwrap());
        let mut pending = PendingSequence::new(active);
        assert_eq!(pending.feed(open[0]), ResolveOutcome::Pending);
        assert_eq!(pending.generation(), 11);
        assert_eq!(pending.feed(open[1]), ResolveOutcome::Command(key(9)));
        assert!(!pending.is_pending());
    }

    #[test]
    fn minor_overlay_count_has_a_hard_limit() {
        let base = Arc::new(KeymapBuilder::new().freeze(1));
        let overlays = (0..=MAX_MINOR_OVERLAYS)
            .map(|_| Arc::new(KeymapBuilder::new().freeze(1)))
            .collect();
        assert!(matches!(
            ActiveKeymaps::new(1, None, overlays, base),
            Err(ActiveKeymapError::TooManyMinorOverlays { .. })
        ));
    }
}
