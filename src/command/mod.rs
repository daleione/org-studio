use std::{borrow::Borrow, collections::HashMap, fmt, sync::Arc};

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct CommandName(Arc<str>);

impl CommandName {
    pub fn parse(name: impl Into<Arc<str>>) -> Result<Self, CommandRegistrationError> {
        let name = name.into();
        let valid = !name.is_empty()
            && !name.starts_with('.')
            && !name.ends_with('.')
            && name.bytes().all(|byte| {
                byte.is_ascii_lowercase() || byte.is_ascii_digit() || matches!(byte, b'.' | b'-')
            });
        if valid {
            Ok(Self(name))
        } else {
            Err(CommandRegistrationError::InvalidName(name))
        }
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl Borrow<str> for CommandName {
    fn borrow(&self) -> &str {
        self.as_str()
    }
}

impl fmt::Display for CommandName {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct CommandKey(u32);

impl CommandKey {
    pub fn index(self) -> usize {
        self.0 as usize
    }

    #[cfg(test)]
    pub(crate) const fn from_index(index: u32) -> Self {
        Self(index)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BuiltinCommand {
    OpenDocument,
    ShowHome,
    OpenAgenda,
    OpenAgendaText,
    ReloadDocument,
    SaveDocument,
    SaveDocumentAs,
    OrgContextCommand,
    ExecuteSourceBlock,
    ToggleInlineImagePreviews,
    UndoDocument,
    RedoDocument,
    ExportDocument,
    QuitApplication,
    ScrollForward,
    ScrollBackward,
    BeginningOfDocument,
    EndOfDocument,
    OpenFileManager,
    OpenDefaultDired,
    ReturnToDocument,
    ToggleSidebar,
    ToggleMinimap,
    ShowEditor,
    ShowReading,
    ShowSplit,
    ToggleSoftWrap,
    IncreaseContentFontSize,
    DecreaseContentFontSize,
    ResetContentFontSize,
    GlobalVisibilityCycle,
    DiredNext,
    DiredPrevious,
    DiredOpen,
    DiredUp,
    DiredBack,
    DiredForward,
    DiredMark,
    DiredUnmark,
    DiredUnmarkAll,
    DiredInvertMarks,
    DiredFlagDelete,
    DiredExecute,
    DiredCreateFile,
    DiredCreateDirectory,
    DiredRename,
    DiredCopy,
    DiredMove,
    DiredTrash,
    DiredHelp,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CommandRole {
    Action,
    Motion,
    Operator,
    TextObject,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ArgumentSpec {
    None,
    Count,
    RawPrefix,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RepeatPolicy {
    Never,
    Repeatable,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum UndoPolicy {
    None,
    StateOnly,
    Transaction,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Availability {
    Always,
    FocusedView,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SideEffectClass {
    None,
    ReadFileSystem,
    WriteFileSystem,
    ExternalProcess,
    Network,
    Configuration,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RedactionPolicy {
    None,
    RedactArguments,
    DoNotRecord,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct CapabilitySet(u32);

impl CapabilitySet {
    pub const READ_FILE_SYSTEM: Self = Self(1 << 0);
    pub const WRITE_FILE_SYSTEM: Self = Self(1 << 1);
    pub const EXTERNAL_PROCESS: Self = Self(1 << 2);
    pub const NETWORK: Self = Self(1 << 3);
    pub const CONFIGURATION: Self = Self(1 << 4);

    pub const fn empty() -> Self {
        Self(0)
    }

    pub const fn contains(self, required: Self) -> bool {
        self.0 & required.0 == required.0
    }

    pub const fn union(self, other: Self) -> Self {
        Self(self.0 | other.0)
    }
}

#[derive(Clone, Debug)]
pub struct CommandDescriptor {
    pub key: CommandKey,
    pub name: CommandName,
    pub aliases: Arc<[CommandName]>,
    pub title: Arc<str>,
    pub description: Arc<str>,
    pub role: CommandRole,
    pub argument_spec: ArgumentSpec,
    pub repeat: RepeatPolicy,
    pub undo: UndoPolicy,
    pub availability: Availability,
    pub side_effect: SideEffectClass,
    pub required_capabilities: CapabilitySet,
    pub redaction: RedactionPolicy,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PrefixArgument {
    None,
    Universal { repeats: u8 },
    Numeric(i64),
}

impl PrefixArgument {
    pub fn effective_count(self) -> Option<i64> {
        match self {
            Self::None => None,
            Self::Numeric(value) => Some(value),
            Self::Universal { repeats } => {
                Some((0..repeats).fold(1_i64, |value, _| value.saturating_mul(4)))
            }
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum InvocationOrigin {
    Keyboard,
    Menu,
    Mouse,
    CommandPalette,
    Macro,
    Automation,
    PlatformAction,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub enum CommandArguments {
    #[default]
    None,
    Prompt(Arc<str>),
    Dired(Arc<[Arc<str>]>),
    Edit(Arc<[u8]>),
    Extension(Arc<[u8]>),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CommandInvocation {
    pub command: CommandKey,
    pub origin: InvocationOrigin,
    pub prefix: PrefixArgument,
    pub arguments: CommandArguments,
    pub capabilities: CapabilitySet,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CommandImplementation {
    Builtin(BuiltinCommand),
}

#[derive(Clone, Debug)]
struct RegisteredCommand {
    descriptor: Arc<CommandDescriptor>,
    implementation: CommandImplementation,
}

#[derive(Clone, Debug)]
pub struct CommandRegistry {
    names: HashMap<CommandName, CommandKey>,
    commands: Box<[RegisteredCommand]>,
}

impl CommandRegistry {
    pub fn key(&self, name: &str) -> Option<CommandKey> {
        self.names.get(name).copied()
    }

    pub fn descriptor(&self, key: CommandKey) -> Option<&CommandDescriptor> {
        self.commands
            .get(key.index())
            .map(|command| command.descriptor.as_ref())
    }

    fn registered(&self, key: CommandKey) -> Option<&RegisteredCommand> {
        self.commands.get(key.index())
    }
}

#[derive(Default)]
pub struct CommandRegistryBuilder {
    names: HashMap<CommandName, CommandKey>,
    commands: Vec<RegisteredCommand>,
}

impl CommandRegistryBuilder {
    pub fn register_builtin(
        &mut self,
        spec: BuiltinCommandSpec<'_>,
    ) -> Result<CommandKey, CommandRegistrationError> {
        let name = CommandName::parse(spec.name)?;
        if !name.as_str().starts_with("org-studio.") {
            return Err(CommandRegistrationError::InvalidBuiltinNamespace(name));
        }
        if self.names.contains_key(&name) {
            return Err(CommandRegistrationError::DuplicateName(name));
        }

        let key = CommandKey(self.commands.len() as u32);
        let aliases = spec
            .aliases
            .iter()
            .map(|alias| CommandName::parse(Arc::<str>::from(*alias)))
            .collect::<Result<Vec<_>, _>>()?;
        for alias in &aliases {
            if self.names.contains_key(alias) || alias == &name {
                return Err(CommandRegistrationError::DuplicateName(alias.clone()));
            }
        }

        self.names.insert(name.clone(), key);
        for alias in &aliases {
            self.names.insert(alias.clone(), key);
        }
        self.commands.push(RegisteredCommand {
            descriptor: Arc::new(CommandDescriptor {
                key,
                name,
                aliases: aliases.into(),
                title: spec.title.into(),
                description: spec.description.into(),
                role: spec.role,
                argument_spec: spec.argument_spec,
                repeat: spec.repeat,
                undo: spec.undo,
                availability: spec.availability,
                side_effect: spec.side_effect,
                required_capabilities: spec.required_capabilities,
                redaction: spec.redaction,
            }),
            implementation: CommandImplementation::Builtin(spec.command),
        });
        Ok(key)
    }

    pub fn build(self) -> CommandRegistry {
        CommandRegistry {
            names: self.names,
            commands: self.commands.into_boxed_slice(),
        }
    }
}

pub struct BuiltinCommandSpec<'a> {
    pub name: Arc<str>,
    pub aliases: &'a [&'a str],
    pub title: &'a str,
    pub description: &'a str,
    pub command: BuiltinCommand,
    pub role: CommandRole,
    pub argument_spec: ArgumentSpec,
    pub repeat: RepeatPolicy,
    pub undo: UndoPolicy,
    pub availability: Availability,
    pub side_effect: SideEffectClass,
    pub required_capabilities: CapabilitySet,
    pub redaction: RedactionPolicy,
}

pub struct PreparedCommand<'a> {
    pub invocation: CommandInvocation,
    pub descriptor: &'a CommandDescriptor,
    pub implementation: CommandImplementation,
}

pub struct CommandDispatcher;

impl CommandDispatcher {
    pub fn prepare<'a>(
        registry: &'a CommandRegistry,
        name: &str,
        origin: InvocationOrigin,
        capabilities: CapabilitySet,
    ) -> Result<PreparedCommand<'a>, CommandDispatchError> {
        let key = registry
            .key(name)
            .ok_or_else(|| CommandDispatchError::UnknownCommand(name.into()))?;
        Self::prepare_key(registry, key, origin, capabilities, PrefixArgument::None)
    }

    pub fn prepare_key<'a>(
        registry: &'a CommandRegistry,
        key: CommandKey,
        origin: InvocationOrigin,
        capabilities: CapabilitySet,
        prefix: PrefixArgument,
    ) -> Result<PreparedCommand<'a>, CommandDispatchError> {
        let registered = registry
            .registered(key)
            .ok_or(CommandDispatchError::InvalidCommandKey(key))?;
        let required = registered.descriptor.required_capabilities;
        if !capabilities.contains(required) {
            return Err(CommandDispatchError::MissingCapabilities {
                command: registered.descriptor.name.clone(),
                required,
            });
        }
        Ok(PreparedCommand {
            invocation: CommandInvocation {
                command: key,
                origin,
                prefix,
                arguments: CommandArguments::None,
                capabilities,
            },
            descriptor: &registered.descriptor,
            implementation: registered.implementation,
        })
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum CommandRegistrationError {
    InvalidName(Arc<str>),
    InvalidBuiltinNamespace(CommandName),
    DuplicateName(CommandName),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum CommandDispatchError {
    UnknownCommand(Arc<str>),
    InvalidCommandKey(CommandKey),
    MissingCapabilities {
        command: CommandName,
        required: CapabilitySet,
    },
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spec(name: &str, aliases: &'static [&'static str]) -> BuiltinCommandSpec<'static> {
        BuiltinCommandSpec {
            name: Arc::from(name),
            aliases,
            title: "Open Document",
            description: "Open a local document",
            command: BuiltinCommand::OpenDocument,
            role: CommandRole::Action,
            argument_spec: ArgumentSpec::None,
            repeat: RepeatPolicy::Never,
            undo: UndoPolicy::None,
            availability: Availability::FocusedView,
            side_effect: SideEffectClass::ReadFileSystem,
            required_capabilities: CapabilitySet::READ_FILE_SYSTEM,
            redaction: RedactionPolicy::RedactArguments,
        }
    }

    #[test]
    fn interns_names_and_aliases_to_the_same_key() {
        let mut builder = CommandRegistryBuilder::default();
        let key = builder
            .register_builtin(spec("org-studio.workspace.open-file", &["find-file"]))
            .unwrap();
        let registry = builder.build();
        assert_eq!(registry.key("org-studio.workspace.open-file"), Some(key));
        assert_eq!(registry.key("find-file"), Some(key));
    }

    #[test]
    fn rejects_invalid_namespaces_and_duplicate_aliases() {
        let mut builder = CommandRegistryBuilder::default();
        assert!(matches!(
            builder.register_builtin(spec("plugin.open-file", &[])),
            Err(CommandRegistrationError::InvalidBuiltinNamespace(_))
        ));
        builder
            .register_builtin(spec("org-studio.workspace.open-file", &["find-file"]))
            .unwrap();
        assert!(matches!(
            builder.register_builtin(spec("org-studio.workspace.open-other", &["find-file"])),
            Err(CommandRegistrationError::DuplicateName(_))
        ));
    }

    #[test]
    fn dispatcher_checks_capabilities_before_preparing_command() {
        let mut builder = CommandRegistryBuilder::default();
        builder
            .register_builtin(spec("org-studio.workspace.open-file", &[]))
            .unwrap();
        let registry = builder.build();
        assert!(matches!(
            CommandDispatcher::prepare(
                &registry,
                "org-studio.workspace.open-file",
                InvocationOrigin::Automation,
                CapabilitySet::empty(),
            ),
            Err(CommandDispatchError::MissingCapabilities { .. })
        ));
        let prepared = CommandDispatcher::prepare(
            &registry,
            "org-studio.workspace.open-file",
            InvocationOrigin::Keyboard,
            CapabilitySet::READ_FILE_SYSTEM,
        )
        .unwrap();
        assert_eq!(
            prepared.implementation,
            CommandImplementation::Builtin(BuiltinCommand::OpenDocument)
        );
    }
}
