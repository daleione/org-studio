use std::{fmt, sync::Arc, time::{Duration, Instant}};

use crate::{
    input::{ContextPredicate, ContextSet, EmacsGrammar, EmacsOutcome},
    command::CommandKey,
    keymap::{KeyLookup, KeyParseError, KeySequence, KeyStroke, KeymapBuildError, KeymapBuilder, StrokeInterner},
};

#[derive(Clone, Copy, Debug)]
pub struct TransientPolicy {
    pub one_key: bool,
    pub exit_after_command: bool,
    pub exit_after_undefined: bool,
    pub timeout: Option<Duration>,
}

impl Default for TransientPolicy {
    fn default() -> Self {
        Self { one_key: false, exit_after_command: true, exit_after_undefined: true, timeout: None }
    }
}

struct InstalledTransient {
    base: Arc<crate::keymap::ActiveKeymaps>,
    policy: TransientPolicy,
    expires_at: Option<Instant>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct WhichKeyCandidate {
    pub key: Arc<str>,
    pub command: Option<CommandKey>,
    pub is_prefix: bool,
    pub disabled: bool,
}

pub struct RouterConfiguration {
    pub generation: u64,
    pub interner: StrokeInterner,
    pub grammar: EmacsGrammar,
    pub enabled_when: ContextPredicate,
}

pub struct KeyboardRouter {
    generation: u64,
    interner: StrokeInterner,
    grammar: EmacsGrammar,
    enabled_when: ContextPredicate,
    pending_configuration: Option<Box<RouterConfiguration>>,
    status: Option<Arc<str>>,
    sequence: String,
    transient: Option<InstalledTransient>,
}

impl KeyboardRouter {
    pub fn new(
        generation: u64,
        interner: StrokeInterner,
        grammar: EmacsGrammar,
        enabled_when: ContextPredicate,
    ) -> Self {
        Self {
            generation,
            interner,
            grammar,
            enabled_when,
            pending_configuration: None,
            status: None,
            sequence: String::with_capacity(32),
            transient: None,
        }
    }

    pub fn generation(&self) -> u64 { self.generation }
    pub fn status(&self) -> Option<&str> { self.status.as_deref() }

    pub fn which_key_candidates(&self) -> Vec<WhichKeyCandidate> {
        self.grammar.continuations().into_iter().filter_map(|candidate| {
            let stroke = self.interner.stroke(candidate.stroke)?;
            Some(WhichKeyCandidate {
                key: Arc::from(stroke.notation()),
                command: match candidate.lookup { KeyLookup::Command(command) => Some(command), _ => None },
                is_prefix: candidate.lookup == KeyLookup::Prefix,
                disabled: candidate.lookup == KeyLookup::Disabled,
            })
        }).collect()
    }

    pub fn install_transient(
        &mut self,
        generation: u64,
        bindings: &[(&str, CommandKey)],
        policy: TransientPolicy,
        now: Instant,
    ) -> Result<(), TransientInstallError> {
        self.clear_transient();
        let base = self.grammar.keymaps();
        let mut builder = KeymapBuilder::new();
        for (keys, command) in bindings {
            let sequence = KeySequence::parse(keys).map_err(|source| TransientInstallError::InvalidSequence { keys: (*keys).into(), source })?;
            let sequence = self.interner.intern_sequence(&sequence);
            builder.bind(&sequence, *command).map_err(|source| TransientInstallError::BindingConflict { keys: (*keys).into(), source })?;
        }
        let transient_map = Arc::new(builder.freeze(generation));
        let active = base.with_transient(generation, transient_map);
        self.grammar = EmacsGrammar::new(active);
        self.transient = Some(InstalledTransient {
            base,
            policy,
            expires_at: policy.timeout.map(|timeout| now + timeout),
        });
        Ok(())
    }

    pub fn clear_transient(&mut self) {
        if let Some(transient) = self.transient.take() {
            self.grammar = EmacsGrammar::new(transient.base);
        }
    }

    pub fn expire_transient(&mut self, now: Instant) -> bool {
        let expired = self.transient.as_ref().and_then(|value| value.expires_at).is_some_and(|deadline| now >= deadline);
        if expired { self.clear_transient(); }
        expired
    }

    pub fn replace_configuration(&mut self, configuration: RouterConfiguration) {
        if self.grammar.is_capturing() {
            self.pending_configuration = Some(Box::new(configuration));
        } else {
            self.install(configuration);
        }
    }

    pub fn route(&mut self, stroke: KeyStroke, context: ContextSet) -> EmacsOutcome {
        self.expire_transient(Instant::now());
        if !self.enabled_when.matches(context) {
            self.clear_transient();
            self.cancel();
            return EmacsOutcome::PassThrough;
        }
        let was_capturing = self.grammar.is_capturing();
        let modified = stroke.control() || stroke.meta() || stroke.command();
        let notation = stroke.notation();
        let outcome = self.grammar.feed(stroke, &self.interner);
        let terminal_key = !matches!(outcome, EmacsOutcome::Pending);
        match outcome {
            EmacsOutcome::Pending => {
                if !self.sequence.is_empty() { self.sequence.push(' '); }
                self.sequence.push_str(&notation);
                self.status = Some(self.pending_status());
            }
            EmacsOutcome::Command { .. } => {
                self.clear_status();
                self.install_pending();
            }
            EmacsOutcome::Cancelled => {
                self.sequence.clear();
                self.status = Some(Arc::from("Quit"));
                self.install_pending();
            }
            EmacsOutcome::Disabled => {
                self.sequence.clear();
                self.status = Some(Arc::from("Key is disabled"));
                self.install_pending();
            }
            EmacsOutcome::Undefined if was_capturing || modified => {
                if !self.sequence.is_empty() { self.sequence.push(' '); }
                self.sequence.push_str(&notation);
                self.status = Some(Arc::from(format!("{} is undefined", self.sequence)));
                self.sequence.clear();
                self.install_pending();
            }
            EmacsOutcome::Undefined | EmacsOutcome::PassThrough => {
                self.clear_status();
                self.install_pending();
            }
        }
        let should_clear_transient = self.transient.as_ref().is_some_and(|transient| {
            (transient.policy.one_key && terminal_key)
                || (transient.policy.exit_after_command && matches!(outcome, EmacsOutcome::Command { .. }))
                || (transient.policy.exit_after_undefined && matches!(outcome, EmacsOutcome::Undefined | EmacsOutcome::Disabled))
                || matches!(outcome, EmacsOutcome::Cancelled)
        });
        if should_clear_transient { self.clear_transient(); }
        outcome
    }

    pub fn cancel(&mut self) {
        self.grammar.cancel();
        self.clear_transient();
        self.sequence.clear();
        self.status = Some(Arc::from("Quit"));
        self.install_pending();
    }

    fn pending_status(&self) -> Arc<str> {
        Arc::from(match self.grammar.prefix_argument().effective_count() {
            Some(count) => format!("{} [{count}]", self.sequence),
            None => self.sequence.clone(),
        })
    }

    fn clear_status(&mut self) {
        self.sequence.clear();
        self.status = None;
    }

    fn install_pending(&mut self) {
        if let Some(configuration) = self.pending_configuration.take() {
            self.install(*configuration);
        }
    }

    fn install(&mut self, configuration: RouterConfiguration) {
        self.generation = configuration.generation;
        self.interner = configuration.interner;
        self.grammar = configuration.grammar;
        self.enabled_when = configuration.enabled_when;
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum TransientInstallError {
    InvalidSequence { keys: Arc<str>, source: KeyParseError },
    BindingConflict { keys: Arc<str>, source: KeymapBuildError },
}

impl fmt::Display for TransientInstallError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result { write!(formatter, "{self:?}") }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use super::*;
    use crate::{
        command::CommandKey,
        input::ContextRegistryBuilder,
        keymap::{ActiveKeymaps, KeySequence, KeymapBuilder},
    };

    fn configuration(generation: u64) -> (RouterConfiguration, ContextSet) {
        let mut contexts = ContextRegistryBuilder::default();
        contexts.register("preview").unwrap();
        let contexts = contexts.build();
        let preview = contexts.set(["preview"]).unwrap();
        let predicate = ContextPredicate::new(preview, ContextSet::empty());
        let mut interner = StrokeInterner::default();
        let sequence = interner.intern_sequence(&KeySequence::parse("C-x C-f").unwrap());
        let mut map = KeymapBuilder::new();
        map.bind(&sequence, CommandKey::from_index(1)).unwrap();
        let maps = Arc::new(
            ActiveKeymaps::new(generation, None, vec![], Arc::new(map.freeze(generation))).unwrap(),
        );
        (RouterConfiguration {
            generation,
            interner,
            grammar: EmacsGrammar::new(maps),
            enabled_when: predicate,
        }, preview)
    }

    fn router(configuration: RouterConfiguration) -> KeyboardRouter {
        KeyboardRouter::new(
            configuration.generation,
            configuration.interner,
            configuration.grammar,
            configuration.enabled_when,
        )
    }

    #[test]
    fn route_is_disabled_outside_its_compiled_context() {
        let (initial, preview) = configuration(1);
        let mut router = router(initial);
        assert_eq!(router.route(KeyStroke::parse("C-x").unwrap(), ContextSet::empty()), EmacsOutcome::PassThrough);
        assert_eq!(router.route(KeyStroke::parse("C-x").unwrap(), preview), EmacsOutcome::Pending);
    }

    #[test]
    fn configuration_swap_waits_for_the_pending_sequence() {
        let (initial, preview) = configuration(1);
        let mut router = router(initial);
        router.route(KeyStroke::parse("C-x").unwrap(), preview);
        router.replace_configuration(configuration(2).0);
        assert_eq!(router.generation(), 1);
        router.route(KeyStroke::parse("C-f").unwrap(), preview);
        assert_eq!(router.generation(), 2);
    }

    #[test]
    fn undefined_prefix_has_feedback_but_plain_text_passes_through() {
        let (configuration, preview) = configuration(1);
        let mut router = router(configuration);
        router.route(KeyStroke::parse("x").unwrap(), preview);
        assert_eq!(router.status(), None);
        router.route(KeyStroke::parse("C-x").unwrap(), preview);
        router.route(KeyStroke::parse("C-z").unwrap(), preview);
        assert_eq!(router.status(), Some("C-x C-z is undefined"));
    }

    #[test]
    fn transient_map_exits_after_command_and_restores_base_candidates() {
        let (configuration, preview) = configuration(1);
        let mut router = router(configuration);
        let command = CommandKey::from_index(1);
        router.install_transient(
            2,
            &[("y", command)],
            TransientPolicy { one_key: true, ..TransientPolicy::default() },
            Instant::now(),
        ).unwrap();
        assert!(router.which_key_candidates().iter().any(|candidate| candidate.key.as_ref() == "y"));
        assert!(matches!(
            router.route(KeyStroke::parse("y").unwrap(), preview),
            EmacsOutcome::Command { .. }
        ));
        assert!(!router.which_key_candidates().iter().any(|candidate| candidate.key.as_ref() == "y"));
    }

    #[test]
    fn transient_map_expires_at_its_deadline() {
        let (configuration, _) = configuration(1);
        let mut router = router(configuration);
        let now = Instant::now();
        router.install_transient(
            2,
            &[("y", CommandKey::from_index(1))],
            TransientPolicy { timeout: Some(Duration::from_millis(10)), ..TransientPolicy::default() },
            now,
        ).unwrap();
        assert!(!router.expire_transient(now + Duration::from_millis(9)));
        assert!(router.expire_transient(now + Duration::from_millis(10)));
    }

    #[test]
    fn which_key_reads_only_the_current_prefix_children() {
        let (configuration, preview) = configuration(1);
        let mut router = router(configuration);
        router.route(KeyStroke::parse("C-x").unwrap(), preview);
        let candidates = router.which_key_candidates();
        assert_eq!(candidates.len(), 1);
        assert_eq!(candidates[0].key.as_ref(), "C-f");
        assert_eq!(candidates[0].command, Some(CommandKey::from_index(1)));
    }
}
