use std::{fmt, sync::Arc};

use crate::{
    command::CommandRegistry,
    input::{
        ContextBuildError, ContextPredicate, ContextRegistry, EmacsGrammar, RouterConfiguration,
    },
    keymap::{
        ActiveKeymapError, ActiveKeymaps, KeyParseError, KeySequence, KeymapBuildError,
        KeymapBuilder, StrokeInterner,
    },
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BindingBehavior<'a> {
    Command(&'a str),
    Disabled,
    PassThrough,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct BindingSpec<'a> {
    pub keys: &'a str,
    pub behavior: BindingBehavior<'a>,
}

pub fn compile_input_profile(
    generation: u64,
    bindings: &[BindingSpec<'_>],
    commands: &CommandRegistry,
    contexts: &ContextRegistry,
    required_contexts: &[&str],
    forbidden_contexts: &[&str],
) -> Result<RouterConfiguration, InputCompileError> {
    let mut interner = StrokeInterner::default();
    let mut keymap = KeymapBuilder::new();
    for binding in bindings {
        let sequence = KeySequence::parse(binding.keys).map_err(|source| {
            InputCompileError::InvalidSequence {
                keys: binding.keys.into(),
                source,
            }
        })?;
        let sequence = interner.intern_sequence(&sequence);
        let result = match binding.behavior {
            BindingBehavior::Command(name) => {
                let command = commands
                    .key(name)
                    .ok_or_else(|| InputCompileError::UnknownCommand(name.into()))?;
                keymap.bind(&sequence, command)
            }
            BindingBehavior::Disabled => keymap.disable(&sequence),
            BindingBehavior::PassThrough => keymap.pass_through(&sequence),
        };
        result.map_err(|source| InputCompileError::BindingConflict {
            keys: binding.keys.into(),
            source,
        })?;
    }
    let required = contexts.set(required_contexts.iter().copied())?;
    let forbidden = contexts.set(forbidden_contexts.iter().copied())?;
    let base = Arc::new(keymap.freeze(generation));
    let active = Arc::new(ActiveKeymaps::new(generation, None, vec![], base)?);
    Ok(RouterConfiguration {
        generation,
        interner,
        grammar: EmacsGrammar::new(active),
        enabled_when: ContextPredicate::new(required, forbidden),
    })
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum InputCompileError {
    InvalidSequence {
        keys: Arc<str>,
        source: KeyParseError,
    },
    UnknownCommand(Arc<str>),
    BindingConflict {
        keys: Arc<str>,
        source: KeymapBuildError,
    },
    Context(ContextBuildError),
    ActiveKeymaps(ActiveKeymapError),
}

impl From<ContextBuildError> for InputCompileError {
    fn from(value: ContextBuildError) -> Self {
        Self::Context(value)
    }
}

impl From<ActiveKeymapError> for InputCompileError {
    fn from(value: ActiveKeymapError) -> Self {
        Self::ActiveKeymaps(value)
    }
}

impl fmt::Display for InputCompileError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{self:?}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        command::{
            ArgumentSpec, Availability, BuiltinCommand, BuiltinCommandSpec, CapabilitySet,
            CommandRegistryBuilder, CommandRole, RedactionPolicy, RepeatPolicy, SideEffectClass,
            UndoPolicy,
        },
        input::ContextRegistryBuilder,
    };

    fn commands() -> CommandRegistry {
        let mut builder = CommandRegistryBuilder::default();
        builder
            .register_builtin(BuiltinCommandSpec {
                name: "org-studio.test.open".into(),
                aliases: &["find-file"],
                title: "Open",
                description: "Open",
                command: BuiltinCommand::OpenDocument,
                role: CommandRole::Action,
                argument_spec: ArgumentSpec::None,
                repeat: RepeatPolicy::Never,
                undo: UndoPolicy::None,
                availability: Availability::Always,
                side_effect: SideEffectClass::None,
                required_capabilities: CapabilitySet::empty(),
                redaction: RedactionPolicy::None,
            })
            .unwrap();
        builder.build()
    }

    #[test]
    fn compiles_aliases_and_contexts_into_a_frozen_profile() {
        let commands = commands();
        let mut contexts = ContextRegistryBuilder::default();
        contexts.register("workspace").unwrap();
        contexts.register("prompt").unwrap();
        let contexts = contexts.build();
        let profile = compile_input_profile(
            9,
            &[BindingSpec {
                keys: "C-x C-f",
                behavior: BindingBehavior::Command("find-file"),
            }],
            &commands,
            &contexts,
            &["workspace"],
            &["prompt"],
        )
        .unwrap();
        assert_eq!(profile.generation, 9);
    }

    #[test]
    fn reports_unknown_commands_and_prefix_conflicts_at_compile_time() {
        let commands = commands();
        let mut contexts = ContextRegistryBuilder::default();
        contexts.register("workspace").unwrap();
        let contexts = contexts.build();
        assert!(matches!(
            compile_input_profile(
                1,
                &[BindingSpec {
                    keys: "C-x",
                    behavior: BindingBehavior::Command("missing")
                }],
                &commands,
                &contexts,
                &["workspace"],
                &[],
            ),
            Err(InputCompileError::UnknownCommand(_))
        ));
        assert!(matches!(
            compile_input_profile(
                1,
                &[
                    BindingSpec {
                        keys: "C-x",
                        behavior: BindingBehavior::Command("find-file")
                    },
                    BindingSpec {
                        keys: "C-x C-f",
                        behavior: BindingBehavior::Command("find-file")
                    },
                ],
                &commands,
                &contexts,
                &["workspace"],
                &[],
            ),
            Err(InputCompileError::BindingConflict { .. })
        ));
    }
}
