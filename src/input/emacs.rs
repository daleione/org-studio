use std::sync::Arc;

use crate::{
    command::{CommandKey, PrefixArgument},
    keymap::{
        ActiveKeymaps, KeyContinuation, KeyStroke, PendingSequence, ResolveOutcome, StrokeInterner,
    },
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum EmacsOutcome {
    Pending,
    Command {
        command: CommandKey,
        prefix: PrefixArgument,
    },
    Disabled,
    PassThrough,
    Undefined,
    Cancelled,
}

pub struct EmacsGrammar {
    pending: PendingSequence,
    prefix: PrefixArgument,
    escape_meta: bool,
    numeric_negative: bool,
}

impl EmacsGrammar {
    pub fn new(keymaps: Arc<ActiveKeymaps>) -> Self {
        Self {
            pending: PendingSequence::new(keymaps),
            prefix: PrefixArgument::None,
            escape_meta: false,
            numeric_negative: false,
        }
    }

    pub fn feed(&mut self, mut stroke: KeyStroke, interner: &StrokeInterner) -> EmacsOutcome {
        if stroke.control() && stroke.key() == "g" {
            self.cancel();
            return EmacsOutcome::Cancelled;
        }
        if !self.pending.is_pending() && stroke.key() == "escape" && !stroke.control() {
            self.escape_meta = true;
            return EmacsOutcome::Pending;
        }
        if self.escape_meta {
            stroke = stroke.with_meta();
            self.escape_meta = false;
        }
        if !self.pending.is_pending() && self.consume_prefix_argument(&stroke) {
            return EmacsOutcome::Pending;
        }
        let Some(stroke) = interner.lookup(&stroke) else {
            self.reset_argument();
            return EmacsOutcome::Undefined;
        };
        match self.pending.feed(stroke) {
            ResolveOutcome::Pending => EmacsOutcome::Pending,
            ResolveOutcome::Command(command) => {
                let prefix = self.take_prefix();
                EmacsOutcome::Command { command, prefix }
            }
            ResolveOutcome::Disabled => {
                self.reset_argument();
                EmacsOutcome::Disabled
            }
            ResolveOutcome::PassThrough => {
                self.reset_argument();
                EmacsOutcome::PassThrough
            }
            ResolveOutcome::Undefined => {
                self.reset_argument();
                EmacsOutcome::Undefined
            }
        }
    }

    pub fn cancel(&mut self) {
        self.pending.cancel();
        self.reset_argument();
    }

    pub fn is_capturing(&self) -> bool {
        self.pending.is_pending()
            || self.escape_meta
            || !matches!(self.prefix, PrefixArgument::None)
    }

    pub fn prefix_argument(&self) -> PrefixArgument {
        self.prefix
    }

    pub fn keymaps(&self) -> Arc<ActiveKeymaps> {
        self.pending.keymaps()
    }

    pub fn continuations(&self) -> Vec<KeyContinuation> {
        self.pending.keymaps().continuations(self.pending.strokes())
    }

    fn consume_prefix_argument(&mut self, stroke: &KeyStroke) -> bool {
        if stroke.control() && stroke.key() == "u" {
            self.prefix = match self.prefix {
                PrefixArgument::Universal { repeats } => PrefixArgument::Universal {
                    repeats: repeats.saturating_add(1),
                },
                _ => PrefixArgument::Universal { repeats: 1 },
            };
            return true;
        }
        let meta = stroke.meta();
        let accepting_digits = !matches!(self.prefix, PrefixArgument::None) || meta;
        if accepting_digits && stroke.key() == "-" {
            self.prefix = PrefixArgument::Numeric(0);
            self.numeric_negative = true;
            return true;
        }
        let Some(digit) = stroke.key().parse::<i64>().ok().filter(|digit| *digit < 10) else {
            return false;
        };
        if !accepting_digits {
            return false;
        }
        self.prefix = PrefixArgument::Numeric(match self.prefix {
            PrefixArgument::Numeric(value) if value < 0 => value.saturating_mul(10) - digit,
            PrefixArgument::Numeric(value) if self.numeric_negative => {
                value.saturating_mul(10) - digit
            }
            PrefixArgument::Numeric(value) => value.saturating_mul(10).saturating_add(digit),
            _ if self.numeric_negative => -digit,
            _ => digit,
        });
        true
    }

    fn take_prefix(&mut self) -> PrefixArgument {
        let prefix = self.prefix;
        self.prefix = PrefixArgument::None;
        self.numeric_negative = false;
        prefix
    }

    fn reset_argument(&mut self) {
        self.prefix = PrefixArgument::None;
        self.escape_meta = false;
        self.numeric_negative = false;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        command::CommandRegistryBuilder,
        keymap::{KeySequence, KeymapBuilder, StrokeInterner},
    };

    fn grammar() -> (EmacsGrammar, StrokeInterner, CommandKey) {
        let registry = CommandRegistryBuilder::default().build();
        assert!(registry.key("missing").is_none());
        let command = CommandKey::from_index(1);
        let mut interner = StrokeInterner::default();
        let sequence = interner.intern_sequence(&KeySequence::parse("C-x C-f").unwrap());
        let meta = interner.intern_sequence(&KeySequence::parse("M-f").unwrap());
        let mut builder = KeymapBuilder::new();
        builder.bind(&sequence, command).unwrap();
        builder.bind(&meta, command).unwrap();
        let maps =
            Arc::new(ActiveKeymaps::new(1, None, vec![], Arc::new(builder.freeze(1))).unwrap());
        (EmacsGrammar::new(maps), interner, command)
    }

    #[test]
    fn resolves_prefix_sequence_and_universal_argument() {
        let (mut grammar, interner, command) = grammar();
        assert_eq!(
            grammar.feed(KeyStroke::parse("C-u").unwrap(), &interner),
            EmacsOutcome::Pending
        );
        assert_eq!(
            grammar.feed(KeyStroke::parse("C-x").unwrap(), &interner),
            EmacsOutcome::Pending
        );
        assert_eq!(
            grammar.feed(KeyStroke::parse("C-f").unwrap(), &interner),
            EmacsOutcome::Command {
                command,
                prefix: PrefixArgument::Universal { repeats: 1 }
            }
        );
    }

    #[test]
    fn escape_applies_meta_to_the_next_stroke() {
        let (mut grammar, interner, command) = grammar();
        assert_eq!(
            grammar.feed(KeyStroke::parse("escape").unwrap(), &interner),
            EmacsOutcome::Pending
        );
        assert_eq!(
            grammar.feed(KeyStroke::parse("f").unwrap(), &interner),
            EmacsOutcome::Command {
                command,
                prefix: PrefixArgument::None
            }
        );
    }

    #[test]
    fn control_g_cancels_pending_state() {
        let (mut grammar, interner, _) = grammar();
        grammar.feed(KeyStroke::parse("C-x").unwrap(), &interner);
        assert_eq!(
            grammar.feed(KeyStroke::parse("C-g").unwrap(), &interner),
            EmacsOutcome::Cancelled
        );
    }

    #[test]
    fn parses_repeated_universal_and_negative_numeric_arguments() {
        let (mut grammar, interner, command) = grammar();
        grammar.feed(KeyStroke::parse("C-u").unwrap(), &interner);
        grammar.feed(KeyStroke::parse("C-u").unwrap(), &interner);
        assert_eq!(grammar.prefix_argument().effective_count(), Some(16));
        grammar.cancel();
        grammar.feed(KeyStroke::parse("M--").unwrap(), &interner);
        grammar.feed(KeyStroke::parse("3").unwrap(), &interner);
        grammar.feed(KeyStroke::parse("C-x").unwrap(), &interner);
        assert_eq!(
            grammar.feed(KeyStroke::parse("C-f").unwrap(), &interner),
            EmacsOutcome::Command {
                command,
                prefix: PrefixArgument::Numeric(-3)
            }
        );
    }
}
