use std::{
    collections::{HashMap, HashSet},
    path::{Path, PathBuf},
    sync::{Arc, Mutex, OnceLock},
    time::{Duration, Instant},
};

use gpui::{
    Context, FocusHandle, FontStyle, FontWeight, HighlightStyle, IntoElement, KeyDownEvent,
    ListAlignment, ListOffset, ListState, PathPromptOptions, Render, StyledText, Subscription,
    Task, Window, actions, div, img, list, prelude::*, px, rgb,
};
use tree_sitter_highlight::{HighlightConfiguration, HighlightEvent, Highlighter};

mod command_window;
mod file_manager_host;
mod folding;
mod markdown;
mod minimap;
#[cfg(test)]
mod org_line;
mod rows;
mod table;

use folding::{changed_range, visible_row_indices};
use rows::build_preview_rows;
use table::{TableRowStyle, build_markdown_table_styles, build_table_styles, render_table_row};

use crate::{
    command::{
        ArgumentSpec, Availability, BuiltinCommand, BuiltinCommandSpec, CapabilitySet,
        CommandDispatcher, CommandImplementation, CommandKey, CommandRegistry,
        CommandRegistryBuilder, CommandRole, InvocationOrigin, PrefixArgument, RedactionPolicy,
        RepeatPolicy, SideEffectClass, UndoPolicy,
    },
    document::{ByteRange, RopeSnapshot, SharedTextSnapshot},
    input::{
        BindingBehavior, BindingSpec, ContextRegistryBuilder, ContextSet, EmacsOutcome,
        KeyboardRouter, compile_input_profile,
    },
    keymap::KeyStroke,
    org_syntax::{
        BlockArena, BlockId, BlockKind, BlockNode,
        inline::{InlineKind, InlineSpan, InlineText, parse as parse_inline},
        parse,
    },
    theme::current_theme,
};

const OPEN_DOCUMENT_COMMAND: &str = "org-studio.workspace.open-file";
const RELOAD_DOCUMENT_COMMAND: &str = "org-studio.document.reload";
const QUIT_APPLICATION_COMMAND: &str = "org-studio.application.quit";
const SCROLL_FORWARD_COMMAND: &str = "org-studio.preview.scroll-forward";
const SCROLL_BACKWARD_COMMAND: &str = "org-studio.preview.scroll-backward";
const BEGINNING_COMMAND: &str = "org-studio.preview.beginning";
const END_COMMAND: &str = "org-studio.preview.end";
const OPEN_FILE_MANAGER_COMMAND: &str = "org-studio.file-manager.open";
const OPEN_DEFAULT_DIRED_COMMAND: &str = "org-studio.dired.open-default";
const RETURN_DOCUMENT_COMMAND: &str = "org-studio.file-manager.return-document";
const TOGGLE_SIDEBAR_COMMAND: &str = "org-studio.file-manager.toggle-sidebar";
const TOGGLE_MINIMAP_COMMAND: &str = "org-studio.preview.toggle-minimap";
const DIRED_NEXT_COMMAND: &str = "org-studio.dired.next-line";
const DIRED_PREVIOUS_COMMAND: &str = "org-studio.dired.previous-line";
const DIRED_OPEN_COMMAND: &str = "org-studio.dired.find-file";
const DIRED_UP_COMMAND: &str = "org-studio.dired.up-directory";
const DIRED_BACK_COMMAND: &str = "org-studio.dired.history-back";
const DIRED_FORWARD_COMMAND: &str = "org-studio.dired.history-forward";
const DIRED_MARK_COMMAND: &str = "org-studio.dired.mark";
const DIRED_UNMARK_COMMAND: &str = "org-studio.dired.unmark";
const DIRED_UNMARK_ALL_COMMAND: &str = "org-studio.dired.unmark-all";
const DIRED_INVERT_COMMAND: &str = "org-studio.dired.invert-marks";
const DIRED_DELETE_COMMAND: &str = "org-studio.dired.flag-delete";
const DIRED_EXECUTE_COMMAND: &str = "org-studio.dired.execute";
const DIRED_HELP_COMMAND: &str = "org-studio.dired.help";
const KEY_FEEDBACK_DURATION: Duration = Duration::from_secs(2);
const MAX_EXACT_SCROLL_LAYOUT_ROWS: usize = 4096;

const HIGHLIGHT_NAMES: &[&str] = &[
    "attribute",
    "boolean",
    "comment",
    "conditional",
    "constant",
    "constructor",
    "delimiter",
    "embedded",
    "escape",
    "field",
    "function",
    "function.call",
    "keyword",
    "keyword.operator",
    "label",
    "number",
    "operator",
    "parameter",
    "property",
    "punctuation.bracket",
    "punctuation.delimiter",
    "string",
    "storageclass",
    "type",
    "type.builtin",
    "type.qualifier",
    "variable",
];

actions!(
    org_preview,
    [
        OpenDocument,
        ReloadDocument,
        OpenFileManager,
        ReturnToDocument,
        ToggleSidebar,
        ToggleMinimap
    ]
);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ContentRoute {
    Document,
    FileManager,
}

pub struct PreviewDocument {
    pub path: PathBuf,
    pub text: SharedTextSnapshot,
    format: DocumentFormat,
    pub blocks: Arc<BlockArena>,
    markdown_blocks: Arc<Vec<markdown::MarkdownBlock>>,
    rows: Arc<Vec<PreviewRow>>,
    tables: Arc<HashMap<BlockId, TableRowStyle>>,
    image_sizes: Arc<HashMap<BlockId, (u32, u32)>>,
    display_map: Option<Arc<minimap::PreviewDisplayMap>>,
    pub metrics: LoadMetrics,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum DocumentFormat {
    Org,
    Markdown,
}

#[derive(Clone, Copy)]
pub(super) struct PreviewRow {
    pub(super) block_id: BlockId,
    pub(super) content: ByteRange,
    pub(super) continuation: bool,
    pub(super) source_line: u64,
    pub(super) show_line_number: bool,
    pub(super) blank: bool,
}

#[derive(Clone, Copy, Debug, Default)]
pub struct LoadMetrics {
    pub bytes: u64,
    pub read: Duration,
    pub rope: Duration,
    pub parse: Duration,
    pub display_map: Duration,
    pub total: Duration,
}

#[derive(Clone, Copy, Debug)]
struct CodeHighlightSpan {
    start: usize,
    end: usize,
    kind: CodeHighlightKind,
}

#[derive(Clone, Copy, Debug)]
enum CodeHighlightKind {
    Attribute,
    Boolean,
    Comment,
    Constant,
    Function,
    Keyword,
    Number,
    Operator,
    Property,
    Punctuation,
    String,
    Type,
    Variable,
}

fn highlight_code(language: &str, source: &str) -> Result<Vec<CodeHighlightSpan>, String> {
    let normalized = language.trim().to_ascii_lowercase();
    static CONFIGURATIONS: OnceLock<Mutex<HashMap<String, HighlightConfiguration>>> =
        OnceLock::new();
    let mut configurations = CONFIGURATIONS
        .get_or_init(|| Mutex::new(HashMap::new()))
        .lock()
        .expect("highlight configuration cache poisoned");
    if let Some(configuration) = configurations.get(&normalized) {
        return highlight_with_configuration(configuration, source);
    }
    let combined_query: String;
    let (language, name, highlights, injections, locals) = match normalized.as_str() {
        "sql" | "postgres" | "postgresql" => (
            tree_sitter_sequel::LANGUAGE.into(),
            "sql",
            tree_sitter_sequel::HIGHLIGHTS_QUERY,
            "",
            "",
        ),
        "rust" | "rs" => (
            tree_sitter_rust::LANGUAGE.into(),
            "rust",
            tree_sitter_rust::HIGHLIGHTS_QUERY,
            tree_sitter_rust::INJECTIONS_QUERY,
            "",
        ),
        "python" | "py" => (
            tree_sitter_python::LANGUAGE.into(),
            "python",
            tree_sitter_python::HIGHLIGHTS_QUERY,
            "",
            "",
        ),
        "sh" | "shell" | "bash" | "zsh" => (
            tree_sitter_bash::LANGUAGE.into(),
            "bash",
            tree_sitter_bash::HIGHLIGHT_QUERY,
            "",
            "",
        ),
        "javascript" | "js" => (
            tree_sitter_javascript::LANGUAGE.into(),
            "javascript",
            tree_sitter_javascript::HIGHLIGHT_QUERY,
            tree_sitter_javascript::INJECTIONS_QUERY,
            tree_sitter_javascript::LOCALS_QUERY,
        ),
        "jsx" => {
            combined_query = format!(
                "{}\n{}",
                tree_sitter_javascript::HIGHLIGHT_QUERY,
                tree_sitter_javascript::JSX_HIGHLIGHT_QUERY
            );
            (
                tree_sitter_javascript::LANGUAGE.into(),
                "jsx",
                combined_query.as_str(),
                tree_sitter_javascript::INJECTIONS_QUERY,
                tree_sitter_javascript::LOCALS_QUERY,
            )
        }
        "typescript" | "ts" => {
            combined_query = format!(
                "{}\n{}",
                tree_sitter_javascript::HIGHLIGHT_QUERY,
                tree_sitter_typescript::HIGHLIGHTS_QUERY
            );
            (
                tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into(),
                "typescript",
                combined_query.as_str(),
                "",
                tree_sitter_typescript::LOCALS_QUERY,
            )
        }
        "tsx" => {
            combined_query = format!(
                "{}\n{}\n{}",
                tree_sitter_javascript::HIGHLIGHT_QUERY,
                tree_sitter_javascript::JSX_HIGHLIGHT_QUERY,
                tree_sitter_typescript::HIGHLIGHTS_QUERY
            );
            (
                tree_sitter_typescript::LANGUAGE_TSX.into(),
                "tsx",
                combined_query.as_str(),
                "",
                tree_sitter_typescript::LOCALS_QUERY,
            )
        }
        "json" => (
            tree_sitter_json::LANGUAGE.into(),
            "json",
            tree_sitter_json::HIGHLIGHTS_QUERY,
            "",
            "",
        ),
        "go" | "golang" => (
            tree_sitter_go::LANGUAGE.into(),
            "go",
            tree_sitter_go::HIGHLIGHTS_QUERY,
            "",
            "",
        ),
        "c" | "h" => (
            tree_sitter_c::LANGUAGE.into(),
            "c",
            tree_sitter_c::HIGHLIGHT_QUERY,
            "",
            "",
        ),
        "cpp" | "c++" | "cc" | "cxx" | "hpp" => {
            combined_query = format!(
                "{}\n{}",
                tree_sitter_c::HIGHLIGHT_QUERY,
                tree_sitter_cpp::HIGHLIGHT_QUERY
            );
            (
                tree_sitter_cpp::LANGUAGE.into(),
                "cpp",
                combined_query.as_str(),
                "",
                "",
            )
        }
        _ => return Ok(Vec::new()),
    };
    let mut configuration =
        HighlightConfiguration::new(language, name, highlights, injections, locals)
            .map_err(|error| error.to_string())?;
    configuration.configure(HIGHLIGHT_NAMES);

    configurations.insert(normalized.clone(), configuration);
    highlight_with_configuration(
        configurations
            .get(&normalized)
            .expect("inserted highlight configuration"),
        source,
    )
}

fn highlight_with_configuration(
    configuration: &HighlightConfiguration,
    source: &str,
) -> Result<Vec<CodeHighlightSpan>, String> {
    let mut highlighter = Highlighter::new();
    let events = highlighter
        .highlight(configuration, source.as_bytes(), None, |_| None)
        .map_err(|error| error.to_string())?;
    let mut active = Vec::new();
    let mut spans = Vec::new();
    for event in events {
        match event.map_err(|error| error.to_string())? {
            HighlightEvent::HighlightStart(highlight) => active.push(highlight.0),
            HighlightEvent::HighlightEnd => {
                active.pop();
            }
            HighlightEvent::Source { start, end } => {
                if start < end
                    && let Some(index) = active.last()
                    && let Some(kind) = code_highlight_kind(HIGHLIGHT_NAMES[*index])
                {
                    spans.push(CodeHighlightSpan { start, end, kind });
                }
            }
        }
    }
    Ok(spans)
}

fn code_highlight_kind(name: &str) -> Option<CodeHighlightKind> {
    let root = name.split('.').next().unwrap_or(name);
    Some(match root {
        "attribute" => CodeHighlightKind::Attribute,
        "boolean" => CodeHighlightKind::Boolean,
        "comment" => CodeHighlightKind::Comment,
        "constant" => CodeHighlightKind::Constant,
        "constructor" => CodeHighlightKind::Type,
        "embedded" | "escape" => CodeHighlightKind::String,
        "delimiter" | "punctuation" => CodeHighlightKind::Punctuation,
        "field" | "property" => CodeHighlightKind::Property,
        "function" => CodeHighlightKind::Function,
        "keyword" | "conditional" | "storageclass" => CodeHighlightKind::Keyword,
        "number" => CodeHighlightKind::Number,
        "operator" => CodeHighlightKind::Operator,
        "label" => CodeHighlightKind::Attribute,
        "string" => CodeHighlightKind::String,
        "type" => CodeHighlightKind::Type,
        "parameter" | "variable" => CodeHighlightKind::Variable,
        _ => return None,
    })
}

enum PreviewLoadState {
    Empty,
    Loading {
        path: PathBuf,
    },
    Ready {
        generation: u64,
        document: Arc<PreviewDocument>,
    },
    Failed {
        path: PathBuf,
        message: String,
    },
}

pub struct PreviewApp {
    focus_handle: Option<FocusHandle>,
    focus_lost_subscription: Option<Subscription>,
    commands: Arc<CommandRegistry>,
    keyboard: KeyboardRouter,
    key_context: ContextSet,
    state: PreviewLoadState,
    generation: u64,
    load_task: Option<Task<()>>,
    file_watch_task: Option<Task<()>>,
    file_watch_request: u64,
    picker_task: Option<Task<()>>,
    list_state: ListState,
    folded: Arc<HashSet<BlockId>>,
    visible_rows: Arc<Vec<usize>>,
    last_ready: Option<(u64, Arc<PreviewDocument>)>,
    opened_at: Option<Instant>,
    first_frame_scheduled: Option<u64>,
    scroll_benchmark: Option<ScrollBenchmark>,
    which_key_task: Option<Task<()>>,
    which_key_request: u64,
    key_feedback_task: Option<Task<()>>,
    key_feedback_request: u64,
    which_key_items: Arc<Vec<(Arc<str>, Arc<str>)>>,
    dired_help_visible: bool,
    content_route: ContentRoute,
    sidebar_visible: bool,
    minimap_visible: bool,
    minimap_thumb_visibility: crate::settings::MinimapThumbVisibility,
    minimap_width: Option<u16>,
    minimap_resize_preview: Option<f32>,
    presentation_revision: u64,
    viewport_revision_key: Option<(u32, u32)>,
    minimap_pending_seek: Option<(u64, f32, bool)>,
    minimap_seek_scheduled: bool,
    dired: Option<crate::file_manager::DiredSession>,
    dired_error: Option<Arc<str>>,
    dired_task: Option<Task<()>>,
    dired_list_state: ListState,
    sidebar_list_state: ListState,
    dired_pending_presentation: Option<(
        crate::navigation::TransactionId,
        crate::navigation::ViewRevision,
        usize,
        f32,
    )>,
    sidebar_pending_presentation: Option<(
        crate::navigation::TransactionId,
        crate::navigation::ViewRevision,
        usize,
        f32,
    )>,
    dired_presentation_scheduled: bool,
    dired_viewport_memory: HashMap<PathBuf, (usize, f32)>,
    sidebar_viewport_memory: HashMap<PathBuf, (usize, f32)>,
}

struct ScrollBenchmark {
    target_frames: usize,
    warmup_remaining: usize,
    sampling_started: bool,
    scroll_pixels: f32,
    samples: Vec<Duration>,
    last_frame: Instant,
}

impl PreviewApp {
    pub fn new() -> Self {
        let list_overdraw = std::env::var("ORG_STUDIO_LIST_OVERDRAW")
            .ok()
            .and_then(|value| value.parse().ok())
            .unwrap_or(80.0);
        let (commands, keyboard, key_context) = preview_input();
        let preview_settings = crate::settings::PreviewSettings::load();
        let minimap_visible = std::env::var("ORG_STUDIO_MINIMAP")
            .ok()
            .and_then(|value| match value.as_str() {
                "1" | "true" | "on" => Some(true),
                "0" | "false" | "off" => Some(false),
                _ => None,
            })
            .unwrap_or(preview_settings.minimap_enabled);
        Self {
            focus_handle: None,
            focus_lost_subscription: None,
            commands,
            keyboard,
            key_context,
            state: PreviewLoadState::Empty,
            generation: 0,
            load_task: None,
            file_watch_task: None,
            file_watch_request: 0,
            picker_task: None,
            list_state: ListState::new(0, ListAlignment::Top, px(list_overdraw)),
            folded: Arc::new(HashSet::new()),
            visible_rows: Arc::new(Vec::new()),
            last_ready: None,
            opened_at: None,
            first_frame_scheduled: None,
            scroll_benchmark: std::env::var("ORG_STUDIO_SCROLL_BENCH_FRAMES")
                .ok()
                .and_then(|value| value.parse().ok())
                .filter(|frames| *frames > 0)
                .map(|target_frames| ScrollBenchmark {
                    target_frames,
                    warmup_remaining: std::env::var("ORG_STUDIO_SCROLL_BENCH_WARMUP_FRAMES")
                        .ok()
                        .and_then(|value| value.parse().ok())
                        .unwrap_or(0),
                    sampling_started: false,
                    scroll_pixels: std::env::var("ORG_STUDIO_SCROLL_BENCH_PIXELS")
                        .ok()
                        .and_then(|value| value.parse().ok())
                        .unwrap_or(640.0),
                    samples: Vec::with_capacity(target_frames),
                    last_frame: Instant::now(),
                }),
            which_key_task: None,
            which_key_request: 0,
            key_feedback_task: None,
            key_feedback_request: 0,
            which_key_items: Arc::new(Vec::new()),
            dired_help_visible: false,
            content_route: ContentRoute::Document,
            sidebar_visible: false,
            minimap_visible,
            minimap_thumb_visibility: crate::settings::initial_minimap_thumb_visibility(
                preview_settings.minimap_thumb_visibility,
            ),
            minimap_width: crate::settings::initial_minimap_width(preview_settings.minimap_width),
            minimap_resize_preview: None,
            presentation_revision: 0,
            viewport_revision_key: None,
            minimap_pending_seek: None,
            minimap_seek_scheduled: false,
            dired: None,
            dired_error: None,
            dired_task: None,
            dired_list_state: ListState::new(0, ListAlignment::Top, px(80.0)),
            sidebar_list_state: ListState::new(0, ListAlignment::Top, px(60.0)),
            dired_pending_presentation: None,
            sidebar_pending_presentation: None,
            dired_presentation_scheduled: false,
            dired_viewport_memory: HashMap::new(),
            sidebar_viewport_memory: HashMap::new(),
        }
    }

    fn dispatch_command(&mut self, name: &str, cx: &mut Context<Self>) {
        let prepared = CommandDispatcher::prepare(
            &self.commands,
            name,
            InvocationOrigin::PlatformAction,
            CapabilitySet::READ_FILE_SYSTEM.union(CapabilitySet::CONFIGURATION),
        );
        let Ok(prepared) = prepared else {
            return;
        };
        self.execute_command(prepared.implementation, prepared.invocation.prefix, cx);
    }

    fn dispatch_command_key(
        &mut self,
        command: CommandKey,
        prefix: PrefixArgument,
        cx: &mut Context<Self>,
    ) {
        let Ok(prepared) = CommandDispatcher::prepare_key(
            &self.commands,
            command,
            InvocationOrigin::Keyboard,
            CapabilitySet::READ_FILE_SYSTEM,
            prefix,
        ) else {
            return;
        };
        self.execute_command(prepared.implementation, prepared.invocation.prefix, cx);
    }

    fn execute_command(
        &mut self,
        implementation: CommandImplementation,
        prefix: PrefixArgument,
        cx: &mut Context<Self>,
    ) {
        match implementation {
            CommandImplementation::Builtin(BuiltinCommand::OpenDocument) => self.choose_file(cx),
            CommandImplementation::Builtin(BuiltinCommand::ReloadDocument) => {
                if self.content_route == ContentRoute::FileManager {
                    self.reload_file_manager(cx);
                } else {
                    self.reload(cx);
                }
            }
            CommandImplementation::Builtin(BuiltinCommand::QuitApplication) => cx.quit(),
            CommandImplementation::Builtin(BuiltinCommand::ScrollForward) => {
                self.list_state.scroll_by(px(640.0 * command_count(prefix)));
                cx.notify();
            }
            CommandImplementation::Builtin(BuiltinCommand::ScrollBackward) => {
                self.list_state
                    .scroll_by(px(-640.0 * command_count(prefix)));
                cx.notify();
            }
            CommandImplementation::Builtin(BuiltinCommand::BeginningOfDocument) => {
                self.list_state.scroll_to(ListOffset::default());
                cx.notify();
            }
            CommandImplementation::Builtin(BuiltinCommand::EndOfDocument) => {
                self.list_state.scroll_to(ListOffset {
                    item_ix: self.list_state.item_count(),
                    offset_in_item: px(0.0),
                });
                cx.notify();
            }
            CommandImplementation::Builtin(BuiltinCommand::OpenFileManager) => {
                self.choose_directory(cx);
            }
            CommandImplementation::Builtin(BuiltinCommand::OpenDefaultDired) => {
                self.open_default_dired(cx)
            }
            CommandImplementation::Builtin(BuiltinCommand::ReturnToDocument) => {
                self.return_to_document(cx)
            }
            CommandImplementation::Builtin(BuiltinCommand::ToggleSidebar) => {
                self.toggle_sidebar(cx)
            }
            CommandImplementation::Builtin(BuiltinCommand::ToggleMinimap) => {
                self.toggle_minimap(cx)
            }
            CommandImplementation::Builtin(BuiltinCommand::DiredNext) => {
                self.dired_move(command_count(prefix) as i64, cx)
            }
            CommandImplementation::Builtin(BuiltinCommand::DiredPrevious) => {
                self.dired_move(-(command_count(prefix) as i64), cx)
            }
            CommandImplementation::Builtin(BuiltinCommand::DiredOpen) => {
                self.dired_open_selected(cx)
            }
            CommandImplementation::Builtin(BuiltinCommand::DiredUp) => self.dired_up(cx),
            CommandImplementation::Builtin(BuiltinCommand::DiredBack) => {
                self.dired_history(false, cx)
            }
            CommandImplementation::Builtin(BuiltinCommand::DiredForward) => {
                self.dired_history(true, cx)
            }
            CommandImplementation::Builtin(BuiltinCommand::DiredMark) => {
                self.dired_mark(crate::file_manager::Mark::Selected, cx)
            }
            CommandImplementation::Builtin(BuiltinCommand::DiredUnmark) => self.dired_unmark(cx),
            CommandImplementation::Builtin(BuiltinCommand::DiredUnmarkAll) => {
                self.dired_unmark_all(cx)
            }
            CommandImplementation::Builtin(BuiltinCommand::DiredInvertMarks) => {
                self.dired_invert_marks(cx)
            }
            CommandImplementation::Builtin(BuiltinCommand::DiredFlagDelete) => {
                self.dired_mark(crate::file_manager::Mark::Delete, cx)
            }
            CommandImplementation::Builtin(BuiltinCommand::DiredExecute) => {
                self.dired_prepare_execute(cx)
            }
            CommandImplementation::Builtin(BuiltinCommand::DiredHelp) => {
                self.show_dired_shortcuts(cx)
            }
        }
    }

    fn key_down(&mut self, event: &KeyDownEvent, cx: &mut Context<Self>) {
        if event.keystroke.key == "escape" && self.cancel_minimap_interaction() {
            cx.stop_propagation();
            cx.notify();
            return;
        }
        let modifiers = event.keystroke.modifiers;
        let stroke = KeyStroke::new(
            event.keystroke.key.as_str(),
            modifiers.control,
            modifiers.alt,
            modifiers.shift,
            modifiers.platform,
        );
        let previous_status = self.keyboard.status().map(str::to_owned);
        let outcome = self.keyboard.route(stroke, self.key_context);
        let has_feedback = self.keyboard.status().is_some();
        if previous_status.as_deref() != self.keyboard.status() {
            cx.notify();
        }
        if matches!(outcome, EmacsOutcome::Pending) {
            self.cancel_key_feedback();
            self.schedule_which_key(cx);
        } else {
            self.cancel_which_key(cx);
            if has_feedback {
                self.schedule_key_feedback_clear(cx);
            } else {
                self.cancel_key_feedback();
            }
        }
        match outcome {
            EmacsOutcome::Command { command, prefix } => {
                cx.stop_propagation();
                self.dispatch_command_key(command, prefix, cx);
            }
            EmacsOutcome::Pending | EmacsOutcome::Disabled | EmacsOutcome::Cancelled => {
                cx.stop_propagation();
            }
            EmacsOutcome::Undefined if has_feedback => cx.stop_propagation(),
            EmacsOutcome::PassThrough | EmacsOutcome::Undefined => {}
        }
    }

    fn install_preview_keymap(&mut self) {
        self.install_route_keymap(false);
    }

    fn install_dired_keymap(&mut self) {
        self.install_route_keymap(true);
    }

    fn install_route_keymap(&mut self, dired: bool) {
        let contexts = built_in_contexts();
        let generation = self.keyboard.generation().wrapping_add(1);
        let bindings = if dired {
            dired_bindings()
        } else {
            preview_bindings()
        };
        let route_context = if dired { "dired" } else { "preview" };
        let Ok(configuration) = compile_input_profile(
            generation,
            &bindings,
            &self.commands,
            &contexts,
            &["workspace", route_context],
            &["prompt"],
        ) else {
            return;
        };
        self.key_context = contexts
            .set(["workspace", route_context])
            .expect("registered route contexts");
        self.keyboard.replace_configuration(configuration);
    }

    fn schedule_which_key(&mut self, cx: &mut Context<Self>) {
        self.which_key_request = self.which_key_request.wrapping_add(1);
        let request = self.which_key_request;
        self.dired_help_visible = false;
        self.which_key_items = Arc::new(Vec::new());
        let delay = cx.background_executor().timer(Duration::from_millis(400));
        self.which_key_task = Some(cx.spawn(async move |this, cx| {
            delay.await;
            let _ = this.update(cx, |this, cx| {
                if this.which_key_request != request || this.keyboard.status().is_none() {
                    return;
                }
                let items = this
                    .keyboard
                    .which_key_candidates()
                    .into_iter()
                    .take(24)
                    .map(|candidate| {
                        let title: Arc<str> = if candidate.disabled {
                            Arc::from("Disabled")
                        } else if let Some(command) = candidate.command {
                            this.commands
                                .descriptor(command)
                                .map(|descriptor| descriptor.title.clone())
                                .unwrap_or_else(|| Arc::from("Unknown command"))
                        } else if candidate.is_prefix {
                            Arc::from("Prefix")
                        } else {
                            Arc::from("Pass through")
                        };
                        (candidate.key, title)
                    })
                    .collect();
                this.which_key_items = Arc::new(items);
                cx.notify();
            });
        }));
    }

    fn cancel_which_key(&mut self, cx: &mut Context<Self>) {
        self.which_key_request = self.which_key_request.wrapping_add(1);
        self.which_key_task = None;
        if !self.which_key_items.is_empty() || self.dired_help_visible {
            self.which_key_items = Arc::new(Vec::new());
            self.dired_help_visible = false;
            cx.notify();
        }
    }

    fn schedule_key_feedback_clear(&mut self, cx: &mut Context<Self>) {
        self.key_feedback_request = self.key_feedback_request.wrapping_add(1);
        let request = self.key_feedback_request;
        let delay = cx.background_executor().timer(KEY_FEEDBACK_DURATION);
        self.key_feedback_task = Some(cx.spawn(async move |this, cx| {
            delay.await;
            let _ = this.update(cx, |this, cx| {
                if this.key_feedback_request != request {
                    return;
                }
                this.key_feedback_task = None;
                if this.keyboard.dismiss_status() {
                    cx.notify();
                }
            });
        }));
    }

    fn cancel_key_feedback(&mut self) {
        self.key_feedback_request = self.key_feedback_request.wrapping_add(1);
        self.key_feedback_task = None;
    }

    pub fn open(&mut self, path: PathBuf, cx: &mut Context<Self>) {
        self.cancel_minimap_interaction();
        self.presentation_revision = self.presentation_revision.wrapping_add(1);
        self.generation += 1;
        self.opened_at = Some(Instant::now());
        self.first_frame_scheduled = None;
        let generation = self.generation;
        self.state = PreviewLoadState::Loading { path: path.clone() };
        self.watch_document(path.clone(), cx);

        let background = cx.background_spawn(async move { load_document(path) });
        self.load_task = Some(cx.spawn(async move |this, cx| {
            let result = background.await;
            let _ = this.update(cx, |this, cx| {
                if this.apply_load_result(generation, result) {
                    cx.notify();
                }
            });
        }));

        cx.notify();
    }

    fn apply_load_result(
        &mut self,
        generation: u64,
        result: Result<PreviewDocument, (PathBuf, String)>,
    ) -> bool {
        if !accept_generation(self.generation, generation) {
            return false;
        }
        self.state = match result {
            Ok(document) => {
                let document = Arc::new(document);
                self.folded = Arc::new(HashSet::new());
                self.visible_rows = if document.format == DocumentFormat::Markdown {
                    Arc::new((0..document.rows.len()).collect())
                } else {
                    Arc::new(visible_row_indices(
                        &document.rows,
                        &document.blocks,
                        &self.folded,
                    ))
                };
                self.list_state.reset(self.visible_rows.len());
                if self.visible_rows.len() <= MAX_EXACT_SCROLL_LAYOUT_ROWS {
                    self.list_state.clone().measure_all();
                }
                self.last_ready = Some((generation, document.clone()));
                PreviewLoadState::Ready {
                    generation,
                    document,
                }
            }
            Err((path, message)) => PreviewLoadState::Failed { path, message },
        };
        true
    }

    fn choose_file(&mut self, cx: &mut Context<Self>) {
        let receiver = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: false,
            multiple: false,
            prompt: Some("Open Org document".into()),
        });

        self.picker_task = Some(cx.spawn(async move |this, cx| {
            let selected = receiver.await;
            if let Ok(Ok(Some(paths))) = selected
                && let Some(path) = paths.into_iter().next()
            {
                let _ = this.update(cx, |this, cx| this.open(path, cx));
            }
        }));
    }

    fn watch_document(&mut self, path: PathBuf, cx: &mut Context<Self>) {
        self.file_watch_request = self.file_watch_request.wrapping_add(1);
        let request = self.file_watch_request;
        if let Ok(watch) = crate::file_watcher::FileWatch::new(path.clone()) {
            self.file_watch_task = Some(cx.spawn(async move |this, cx| {
                if !watch.changed().await {
                    return;
                }
                cx.background_executor()
                    .timer(Duration::from_millis(100))
                    .await;
                watch.drain();
                let _ = this.update(cx, |this, cx| {
                    if this.file_watch_request == request {
                        this.open(path, cx);
                    }
                });
            }));
            return;
        }

        self.file_watch_task = None;
    }

    fn reload(&mut self, cx: &mut Context<Self>) {
        let path = match &self.state {
            PreviewLoadState::Loading { path } | PreviewLoadState::Failed { path, .. } => {
                Some(path.clone())
            }
            PreviewLoadState::Ready { document, .. } => Some(document.path.clone()),
            PreviewLoadState::Empty => None,
        };
        if let Some(path) = path {
            self.open(path, cx);
        }
    }

    pub fn toggle_minimap(&mut self, cx: &mut Context<Self>) {
        self.minimap_visible = !self.minimap_visible;
        self.save_preview_settings();
        cx.notify();
    }

    fn save_preview_settings(&self) {
        crate::settings::PreviewSettings {
            minimap_enabled: self.minimap_visible,
            minimap_thumb_visibility: self.minimap_thumb_visibility,
            minimap_width: self.minimap_width,
        }
        .save_async();
    }

    fn change_minimap_width(
        &mut self,
        change: minimap::MinimapWidthChange,
        cx: &mut Context<Self>,
    ) {
        match change {
            minimap::MinimapWidthChange::Preview(width) => {
                if self.minimap_resize_preview != Some(width) {
                    self.minimap_resize_preview = Some(width);
                    cx.notify();
                }
            }
            minimap::MinimapWidthChange::Commit(width) => {
                self.minimap_resize_preview = None;
                let width = width.round().clamp(48.0, minimap::MINIMAP_MANUAL_MAX_PX) as u16;
                if self.minimap_width != Some(width) {
                    self.minimap_width = Some(width);
                    self.presentation_revision = self.presentation_revision.wrapping_add(1);
                    self.cancel_minimap_interaction();
                    self.save_preview_settings();
                }
                cx.notify();
            }
            minimap::MinimapWidthChange::Reset => {
                self.minimap_resize_preview = None;
                if self.minimap_width.take().is_some() {
                    self.presentation_revision = self.presentation_revision.wrapping_add(1);
                    self.cancel_minimap_interaction();
                    self.save_preview_settings();
                }
                cx.notify();
            }
        }
    }

    pub fn minimap_visible(&self) -> bool {
        self.minimap_visible
    }

    fn body(&self, entity: gpui::Entity<Self>, editor_width: f32) -> gpui::Div {
        let theme = current_theme();
        let minimap_width = minimap::width_for_viewport(editor_width, self.minimap_width);
        match &self.state {
            PreviewLoadState::Empty => centered_message(
                "ORG STUDIO",
                "Open an Org document from the File menu or press Command-O.",
            ),
            PreviewLoadState::Loading { path } => {
                centered_message("OPENING DOCUMENT", &path.display().to_string())
            }
            PreviewLoadState::Failed { path, message } => {
                let error = format!("{}: {message}", path.display());
                if let Some((_, document)) = &self.last_ready {
                    div()
                        .size_full()
                        .flex()
                        .flex_col()
                        .bg(rgb(theme.background))
                        .child(
                            div()
                                .flex_none()
                                .px_6()
                                .py_3()
                                .bg(rgb(0xfff2f0))
                                .border_b_1()
                                .border_color(rgb(0xf2c8c2))
                                .text_size(px(13.0))
                                .text_color(rgb(0xa12b1f))
                                .child(format!(
                                    "Could not open document. Showing the previous file. {error}"
                                )),
                        )
                        .child(render_document(
                            document.clone(),
                            self.list_state.clone(),
                            self.visible_rows.clone(),
                            self.folded.clone(),
                            entity,
                            self.minimap_visible,
                            editor_width,
                            minimap_width,
                            self.minimap_resize_preview,
                            self.minimap_thumb_visibility,
                            self.presentation_revision,
                        ))
                } else {
                    centered_message("COULD NOT OPEN DOCUMENT", &error)
                }
            }
            PreviewLoadState::Ready { document, .. } => render_document(
                document.clone(),
                self.list_state.clone(),
                self.visible_rows.clone(),
                self.folded.clone(),
                entity,
                self.minimap_visible,
                editor_width,
                minimap_width,
                self.minimap_resize_preview,
                self.minimap_thumb_visibility,
                self.presentation_revision,
            ),
        }
    }

    fn toggle_fold(&mut self, block_id: BlockId, document: &Arc<PreviewDocument>) {
        if document.format == DocumentFormat::Markdown {
            return;
        }
        if !matches!(
            document.blocks.nodes()[block_id as usize].kind,
            BlockKind::Heading { .. }
        ) {
            return;
        }
        self.cancel_minimap_interaction();
        self.presentation_revision = self.presentation_revision.wrapping_add(1);
        let folded = Arc::make_mut(&mut self.folded);
        if !folded.remove(&block_id) {
            folded.insert(block_id);
        }
        let new_visible = Arc::new(visible_row_indices(
            &document.rows,
            &document.blocks,
            &self.folded,
        ));
        let (old_range, new_count) = changed_range(&self.visible_rows, &new_visible);
        self.list_state.splice(old_range, new_count);
        if new_visible.len() <= MAX_EXACT_SCROLL_LAYOUT_ROWS {
            self.list_state.clone().measure_all();
        }
        self.visible_rows = new_visible;
    }

    fn cancel_minimap_interaction(&mut self) -> bool {
        self.minimap_pending_seek = None;
        self.minimap_seek_scheduled = false;
        self.minimap_resize_preview = None;
        let was_dragging = match &self.state {
            PreviewLoadState::Ready { document, .. } => document
                .display_map
                .as_ref()
                .is_some_and(|map| map.cancel_minimap_interaction()),
            _ => self
                .last_ready
                .as_ref()
                .and_then(|(_, document)| document.display_map.as_ref())
                .is_some_and(|map| map.cancel_minimap_interaction()),
        };
        self.list_state.scrollbar_drag_ended();
        was_dragging
    }

    fn window_title(&self) -> String {
        if self.content_route == ContentRoute::FileManager
            && let Some(session) = self.dired.as_ref()
        {
            return format!(
                "{} - Files",
                session
                    .directory()
                    .file_name()
                    .unwrap_or_default()
                    .to_string_lossy()
            );
        }
        let path = match &self.state {
            PreviewLoadState::Loading { path } | PreviewLoadState::Failed { path, .. } => {
                Some(path)
            }
            PreviewLoadState::Ready { document, .. } => Some(&document.path),
            PreviewLoadState::Empty => None,
        };
        path.and_then(|path| path.file_name())
            .and_then(|name| name.to_str())
            .unwrap_or("Org Studio")
            .to_owned()
    }

    fn schedule_scroll_sample(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        cx.on_next_frame(window, |this, window, cx| {
            let now = Instant::now();
            let Some(benchmark) = this.scroll_benchmark.as_mut() else {
                return;
            };
            if benchmark.warmup_remaining > 0 {
                benchmark.warmup_remaining -= 1;
                if benchmark.warmup_remaining % 60 == 0 {
                    eprintln!(
                        "org_preview_scroll_warmup remaining={} display={:?}",
                        benchmark.warmup_remaining,
                        window.display(cx).map(|display| display.id())
                    );
                }
                benchmark.last_frame = now;
                this.schedule_scroll_sample(window, cx);
                return;
            }
            if !benchmark.sampling_started {
                benchmark.sampling_started = true;
                crate::perf_tracing::reset_samples();
                benchmark.last_frame = now;
                this.list_state.scroll_by(px(benchmark.scroll_pixels));
                this.schedule_scroll_sample(window, cx);
                return;
            }
            benchmark
                .samples
                .push(now.duration_since(benchmark.last_frame));
            benchmark.last_frame = now;
            if benchmark.samples.len() >= benchmark.target_frames {
                let mut samples = benchmark.samples.clone();
                samples.sort_unstable();
                let percentile = |p: f64| {
                    let index = ((samples.len() - 1) as f64 * p).ceil() as usize;
                    samples[index].as_secs_f64() * 1000.0
                };
                let cadence_ms = percentile(0.50);
                let late_frame_threshold_ms = cadence_ms * 1.5;
                let late_frames = samples
                    .iter()
                    .filter(|sample| {
                        sample.as_secs_f64() * 1000.0 > late_frame_threshold_ms
                    })
                    .count();
                let estimated_missed_vsyncs: u64 = samples
                    .iter()
                    .map(|sample| {
                        let elapsed_ms = sample.as_secs_f64() * 1000.0;
                        (elapsed_ms / cadence_ms).round().max(1.0) as u64 - 1
                    })
                    .sum();
                let over_12_5 = samples
                    .iter()
                    .filter(|sample| sample.as_secs_f64() * 1000.0 > 12.5)
                    .count();
                let over_16_67 = samples
                    .iter()
                    .filter(|sample| sample.as_secs_f64() * 1000.0 > 16.67)
                    .count();
                eprintln!(
                    "org_preview_scroll frames={} pixels_per_frame={:.1} cadence_ms={:.3} cadence_hz={:.2} p50_ms={:.3} p95_ms={:.3} p99_ms={:.3} max_ms={:.3} late_frames={} late_rate_pct={:.3} estimated_missed_vsyncs={} over_12_5={} over_16_67={}",
                    samples.len(),
                    benchmark.scroll_pixels,
                    cadence_ms,
                    1000.0 / cadence_ms,
                    percentile(0.50),
                    percentile(0.95),
                    percentile(0.99),
                    samples.last().unwrap().as_secs_f64() * 1000.0,
                    late_frames,
                    late_frames as f64 * 100.0 / samples.len() as f64,
                    estimated_missed_vsyncs,
                    over_12_5,
                    over_16_67,
                );
                crate::perf_tracing::report();
                this.scroll_benchmark = None;
                cx.quit();
            } else {
                this.list_state.scroll_by(px(benchmark.scroll_pixels));
                this.schedule_scroll_sample(window, cx);
            }
        });
    }
}

fn accept_generation(current: u64, completed: u64) -> bool {
    current == completed
}

impl Render for PreviewApp {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        profiling::scope!("PreviewApp::render");
        let viewport = window.viewport_size();
        let viewport_key = (
            f32::from(viewport.width).to_bits(),
            f32::from(viewport.height).to_bits(),
        );
        if self.viewport_revision_key != Some(viewport_key) {
            self.viewport_revision_key = Some(viewport_key);
            self.presentation_revision = self.presentation_revision.wrapping_add(1);
            self.cancel_minimap_interaction();
        }
        let focus_handle = self
            .focus_handle
            .get_or_insert_with(|| {
                let handle = cx.focus_handle();
                window.focus(&handle);
                handle
            })
            .clone();
        if self.focus_lost_subscription.is_none() {
            self.focus_lost_subscription = Some(cx.on_focus_lost(window, |this, _, cx| {
                if this.cancel_minimap_interaction() {
                    cx.notify();
                }
            }));
        }
        window.set_window_title(&self.window_title());
        if (self.dired_pending_presentation.is_some()
            || self.sidebar_pending_presentation.is_some())
            && !self.dired_presentation_scheduled
        {
            self.dired_presentation_scheduled = true;
            cx.on_next_frame(window, |this, _, cx| {
                this.dired_presentation_scheduled = false;
                if let Some((transaction, view_revision, rank, offset)) =
                    this.dired_pending_presentation.take()
                    && this.dired.as_ref().is_some_and(|session| {
                        session.presentation_is_current(transaction, view_revision)
                    })
                {
                    this.dired_list_state.scroll_to(ListOffset {
                        item_ix: rank,
                        offset_in_item: px(offset),
                    });
                }
                if let Some((transaction, view_revision, rank, offset)) =
                    this.sidebar_pending_presentation.take()
                    && this.dired.as_ref().is_some_and(|session| {
                        session.presentation_is_current(transaction, view_revision)
                    })
                {
                    this.sidebar_list_state.scroll_to(ListOffset {
                        item_ix: rank,
                        offset_in_item: px(offset),
                    });
                }
                cx.notify();
            });
        }
        if self.scroll_benchmark.is_some() {
            window.request_animation_frame();
        }
        if let PreviewLoadState::Ready { generation, .. } = &self.state
            && self.first_frame_scheduled != Some(*generation)
        {
            let generation = *generation;
            let opened_at = self.opened_at.unwrap_or_else(Instant::now);
            self.first_frame_scheduled = Some(generation);
            cx.on_next_frame(window, move |this, window, cx| {
                let elapsed = opened_at.elapsed();
                eprintln!(
                    "org_preview_first_readable_frame generation={} elapsed_ms={:.3}",
                    generation,
                    elapsed.as_secs_f64() * 1000.0
                );
                if generation == 1
                    && std::env::var_os("ORG_STUDIO_RELOAD_BENCH").is_some()
                    && let PreviewLoadState::Ready { document, .. } = &this.state
                {
                    this.open(document.path.clone(), cx);
                    return;
                }
                if this.scroll_benchmark.is_some() {
                    if let Some(benchmark) = this.scroll_benchmark.as_mut() {
                        benchmark.last_frame = Instant::now();
                    }
                    this.schedule_scroll_sample(window, cx);
                } else if std::env::var_os("ORG_STUDIO_EXIT_AFTER_FIRST_FRAME").is_some() {
                    cx.quit();
                }
            });
        }
        let entity = cx.entity();
        let key_status = self.keyboard.status().map(Arc::<str>::from);
        let which_key_items = self.which_key_items.clone();
        let dired_help_visible = self.dired_help_visible;
        let command_window_width = f32::from(window.viewport_size().width);
        div()
            .relative()
            .track_focus(&focus_handle)
            .size_full()
            .bg(rgb(current_theme().background))
            .text_color(rgb(current_theme().foreground))
            .font_family("Menlo")
            .text_size(px(14.0))
            .on_key_down(cx.listener(|this, event, _, cx| this.key_down(event, cx)))
            .on_action(cx.listener(|this, _: &OpenDocument, _, cx| {
                this.dispatch_command(OPEN_DOCUMENT_COMMAND, cx)
            }))
            .on_action(cx.listener(|this, _: &ReloadDocument, _, cx| {
                this.dispatch_command(RELOAD_DOCUMENT_COMMAND, cx)
            }))
            .on_action(cx.listener(|this, _: &OpenFileManager, _, cx| this.choose_directory(cx)))
            .on_action(cx.listener(|this, _: &ReturnToDocument, _, cx| this.return_to_document(cx)))
            .on_action(cx.listener(|this, _: &ToggleSidebar, _, cx| this.toggle_sidebar(cx)))
            .child(self.workspace_body(entity, command_window_width))
            .when(!which_key_items.is_empty(), |view| {
                view.child(if dired_help_visible {
                    dired_help_window(which_key_items.clone(), command_window_width)
                } else {
                    which_key_window(which_key_items.clone(), command_window_width)
                })
            })
            .when_some(
                if which_key_items.is_empty() {
                    key_status
                } else {
                    None
                },
                |view, status| {
                    view.child(
                        div()
                            .absolute()
                            .left(px(108.0))
                            .bottom(px(10.0))
                            .px_2()
                            .py_1()
                            .rounded_sm()
                            .bg(rgb(current_theme().background))
                            .text_color(rgb(current_theme().foreground))
                            .text_size(px(12.0))
                            .child(status.to_string()),
                    )
                },
            )
    }
}

fn dired_help_window(items: Arc<Vec<(Arc<str>, Arc<str>)>>, available_width: f32) -> gpui::Div {
    use command_window::{CommandGroup, CommandWindow};

    let take = |title: &str, keys: &[&str], columns| CommandGroup {
        title: Arc::from(title),
        max_columns: columns,
        items: keys
            .iter()
            .filter_map(|key| {
                items
                    .iter()
                    .find(|(candidate, _)| candidate.as_ref() == *key)
                    .cloned()
            })
            .collect(),
    };
    CommandWindow {
        title: Arc::from("Dired Commands"),
        close: Some((Arc::from("C-g"), Arc::from("Close"))),
        groups: vec![
            take(
                "NAVIGATION",
                &["n / j", "p / k", "^ / h", "H", "L", "g", "q"],
                3,
            ),
            take("MARKS", &["m", "u", "U", "t", "d"], 3),
            take("FILES", &["RET / l", "x"], 2),
            take("GLOBAL", &["C-x d", "C-x C-d"], 2),
        ],
    }
    .render(available_width)
}

fn which_key_window(items: Arc<Vec<(Arc<str>, Arc<str>)>>, available_width: f32) -> gpui::Div {
    use command_window::{CommandGroup, CommandWindow};

    let columns = items.len().clamp(1, 6);
    CommandWindow {
        title: Arc::from("Available Commands"),
        close: None,
        groups: vec![CommandGroup {
            title: Arc::from("COMMANDS"),
            max_columns: columns,
            items: items.as_ref().clone(),
        }],
    }
    .render(available_width)
}

fn preview_input() -> (Arc<CommandRegistry>, KeyboardRouter, ContextSet) {
    let mut builder = CommandRegistryBuilder::default();
    builder
        .register_builtin(BuiltinCommandSpec {
            name: OPEN_DOCUMENT_COMMAND.into(),
            aliases: &["find-file"],
            title: "Open File",
            description: "Open a local Org document",
            command: BuiltinCommand::OpenDocument,
            role: CommandRole::Action,
            argument_spec: ArgumentSpec::None,
            repeat: RepeatPolicy::Never,
            undo: UndoPolicy::None,
            availability: Availability::FocusedView,
            side_effect: SideEffectClass::ReadFileSystem,
            required_capabilities: CapabilitySet::READ_FILE_SYSTEM,
            redaction: RedactionPolicy::RedactArguments,
        })
        .expect("valid built-in open command");
    builder
        .register_builtin(BuiltinCommandSpec {
            name: RELOAD_DOCUMENT_COMMAND.into(),
            aliases: &["revert-buffer"],
            title: "Reload Document",
            description: "Reload the current document from disk",
            command: BuiltinCommand::ReloadDocument,
            role: CommandRole::Action,
            argument_spec: ArgumentSpec::None,
            repeat: RepeatPolicy::Never,
            undo: UndoPolicy::None,
            availability: Availability::FocusedView,
            side_effect: SideEffectClass::ReadFileSystem,
            required_capabilities: CapabilitySet::READ_FILE_SYSTEM,
            redaction: RedactionPolicy::None,
        })
        .expect("valid built-in reload command");
    for (name, title, command, argument_spec) in [
        (
            QUIT_APPLICATION_COMMAND,
            "Quit",
            BuiltinCommand::QuitApplication,
            ArgumentSpec::None,
        ),
        (
            SCROLL_FORWARD_COMMAND,
            "Scroll Forward",
            BuiltinCommand::ScrollForward,
            ArgumentSpec::Count,
        ),
        (
            SCROLL_BACKWARD_COMMAND,
            "Scroll Backward",
            BuiltinCommand::ScrollBackward,
            ArgumentSpec::Count,
        ),
        (
            BEGINNING_COMMAND,
            "Beginning of Document",
            BuiltinCommand::BeginningOfDocument,
            ArgumentSpec::None,
        ),
        (
            END_COMMAND,
            "End of Document",
            BuiltinCommand::EndOfDocument,
            ArgumentSpec::None,
        ),
    ] {
        builder
            .register_builtin(BuiltinCommandSpec {
                name: name.into(),
                aliases: &[],
                title,
                description: title,
                command,
                role: CommandRole::Action,
                argument_spec,
                repeat: RepeatPolicy::Repeatable,
                undo: UndoPolicy::None,
                availability: Availability::FocusedView,
                side_effect: SideEffectClass::None,
                required_capabilities: CapabilitySet::empty(),
                redaction: RedactionPolicy::None,
            })
            .expect("valid built-in preview command");
    }
    for (name, title, command, argument_spec) in [
        (
            OPEN_FILE_MANAGER_COMMAND,
            "Open File Manager",
            BuiltinCommand::OpenFileManager,
            ArgumentSpec::None,
        ),
        (
            OPEN_DEFAULT_DIRED_COMMAND,
            "Open Dired",
            BuiltinCommand::OpenDefaultDired,
            ArgumentSpec::None,
        ),
        (
            RETURN_DOCUMENT_COMMAND,
            "Return to Document",
            BuiltinCommand::ReturnToDocument,
            ArgumentSpec::None,
        ),
        (
            TOGGLE_SIDEBAR_COMMAND,
            "Toggle Sidebar",
            BuiltinCommand::ToggleSidebar,
            ArgumentSpec::None,
        ),
        (
            TOGGLE_MINIMAP_COMMAND,
            "Toggle Minimap",
            BuiltinCommand::ToggleMinimap,
            ArgumentSpec::None,
        ),
        (
            DIRED_NEXT_COMMAND,
            "Next Line",
            BuiltinCommand::DiredNext,
            ArgumentSpec::Count,
        ),
        (
            DIRED_PREVIOUS_COMMAND,
            "Previous Line",
            BuiltinCommand::DiredPrevious,
            ArgumentSpec::Count,
        ),
        (
            DIRED_OPEN_COMMAND,
            "Open",
            BuiltinCommand::DiredOpen,
            ArgumentSpec::None,
        ),
        (
            DIRED_UP_COMMAND,
            "Up Directory",
            BuiltinCommand::DiredUp,
            ArgumentSpec::None,
        ),
        (
            DIRED_BACK_COMMAND,
            "History Back",
            BuiltinCommand::DiredBack,
            ArgumentSpec::None,
        ),
        (
            DIRED_FORWARD_COMMAND,
            "History Forward",
            BuiltinCommand::DiredForward,
            ArgumentSpec::None,
        ),
        (
            DIRED_MARK_COMMAND,
            "Mark",
            BuiltinCommand::DiredMark,
            ArgumentSpec::None,
        ),
        (
            DIRED_UNMARK_COMMAND,
            "Unmark",
            BuiltinCommand::DiredUnmark,
            ArgumentSpec::None,
        ),
        (
            DIRED_UNMARK_ALL_COMMAND,
            "Unmark All",
            BuiltinCommand::DiredUnmarkAll,
            ArgumentSpec::None,
        ),
        (
            DIRED_INVERT_COMMAND,
            "Invert Marks",
            BuiltinCommand::DiredInvertMarks,
            ArgumentSpec::None,
        ),
        (
            DIRED_DELETE_COMMAND,
            "Flag Delete",
            BuiltinCommand::DiredFlagDelete,
            ArgumentSpec::None,
        ),
        (
            DIRED_EXECUTE_COMMAND,
            "Execute",
            BuiltinCommand::DiredExecute,
            ArgumentSpec::None,
        ),
        (
            DIRED_HELP_COMMAND,
            "Dired Help",
            BuiltinCommand::DiredHelp,
            ArgumentSpec::None,
        ),
    ] {
        let configuration = command == BuiltinCommand::ToggleMinimap;
        builder
            .register_builtin(BuiltinCommandSpec {
                name: name.into(),
                aliases: &[],
                title,
                description: title,
                command,
                role: CommandRole::Action,
                argument_spec,
                repeat: RepeatPolicy::Repeatable,
                undo: UndoPolicy::None,
                availability: Availability::FocusedView,
                side_effect: if configuration {
                    SideEffectClass::Configuration
                } else {
                    SideEffectClass::None
                },
                required_capabilities: if configuration {
                    CapabilitySet::CONFIGURATION
                } else {
                    CapabilitySet::empty()
                },
                redaction: RedactionPolicy::None,
            })
            .expect("valid built-in file manager command");
    }
    let commands = Arc::new(builder.build());
    let contexts = built_in_contexts();
    let active_context = contexts
        .set(["workspace", "preview"])
        .expect("registered built-in contexts");
    let configuration = compile_input_profile(
        1,
        &preview_bindings(),
        &commands,
        &contexts,
        &["workspace", "preview"],
        &["prompt"],
    )
    .expect("built-in input profile is valid");
    let keyboard = KeyboardRouter::new(
        configuration.generation,
        configuration.interner,
        configuration.grammar,
        configuration.enabled_when,
    );
    (commands, keyboard, active_context)
}

fn built_in_contexts() -> crate::input::ContextRegistry {
    let mut builder = ContextRegistryBuilder::default();
    for name in [
        "workspace",
        "preview",
        "prompt",
        "sidebar",
        "dired",
        "editor",
    ] {
        builder
            .register(name)
            .expect("valid unique built-in context");
    }
    builder.build()
}

fn preview_bindings() -> Vec<BindingSpec<'static>> {
    vec![
        BindingSpec {
            keys: "C-x C-f",
            behavior: BindingBehavior::Command(OPEN_DOCUMENT_COMMAND),
        },
        BindingSpec {
            keys: "C-x C-r",
            behavior: BindingBehavior::Command(RELOAD_DOCUMENT_COMMAND),
        },
        BindingSpec {
            keys: "g",
            behavior: BindingBehavior::Command(RELOAD_DOCUMENT_COMMAND),
        },
        BindingSpec {
            keys: "q",
            behavior: BindingBehavior::Command(QUIT_APPLICATION_COMMAND),
        },
        BindingSpec {
            keys: "C-x C-c",
            behavior: BindingBehavior::Command(QUIT_APPLICATION_COMMAND),
        },
        BindingSpec {
            keys: "SPC",
            behavior: BindingBehavior::Command(SCROLL_FORWARD_COMMAND),
        },
        BindingSpec {
            keys: "backspace",
            behavior: BindingBehavior::Command(SCROLL_BACKWARD_COMMAND),
        },
        BindingSpec {
            keys: "M-<",
            behavior: BindingBehavior::Command(BEGINNING_COMMAND),
        },
        BindingSpec {
            keys: "M->",
            behavior: BindingBehavior::Command(END_COMMAND),
        },
        BindingSpec {
            keys: "C-x d",
            behavior: BindingBehavior::Command(OPEN_DEFAULT_DIRED_COMMAND),
        },
        BindingSpec {
            keys: "C-x C-d",
            behavior: BindingBehavior::Command(TOGGLE_SIDEBAR_COMMAND),
        },
    ]
}

fn dired_bindings() -> Vec<BindingSpec<'static>> {
    vec![
        BindingSpec {
            keys: "n",
            behavior: BindingBehavior::Command(DIRED_NEXT_COMMAND),
        },
        BindingSpec {
            keys: "j",
            behavior: BindingBehavior::Command(DIRED_NEXT_COMMAND),
        },
        BindingSpec {
            keys: "p",
            behavior: BindingBehavior::Command(DIRED_PREVIOUS_COMMAND),
        },
        BindingSpec {
            keys: "k",
            behavior: BindingBehavior::Command(DIRED_PREVIOUS_COMMAND),
        },
        BindingSpec {
            keys: "RET",
            behavior: BindingBehavior::Command(DIRED_OPEN_COMMAND),
        },
        BindingSpec {
            keys: "l",
            behavior: BindingBehavior::Command(DIRED_OPEN_COMMAND),
        },
        BindingSpec {
            keys: "S-6",
            behavior: BindingBehavior::Command(DIRED_UP_COMMAND),
        },
        BindingSpec {
            keys: "h",
            behavior: BindingBehavior::Command(DIRED_UP_COMMAND),
        },
        BindingSpec {
            keys: "S-h",
            behavior: BindingBehavior::Command(DIRED_BACK_COMMAND),
        },
        BindingSpec {
            keys: "S-l",
            behavior: BindingBehavior::Command(DIRED_FORWARD_COMMAND),
        },
        BindingSpec {
            keys: "g",
            behavior: BindingBehavior::Command(RELOAD_DOCUMENT_COMMAND),
        },
        BindingSpec {
            keys: "m",
            behavior: BindingBehavior::Command(DIRED_MARK_COMMAND),
        },
        BindingSpec {
            keys: "u",
            behavior: BindingBehavior::Command(DIRED_UNMARK_COMMAND),
        },
        BindingSpec {
            keys: "S-u",
            behavior: BindingBehavior::Command(DIRED_UNMARK_ALL_COMMAND),
        },
        BindingSpec {
            keys: "t",
            behavior: BindingBehavior::Command(DIRED_INVERT_COMMAND),
        },
        BindingSpec {
            keys: "d",
            behavior: BindingBehavior::Command(DIRED_DELETE_COMMAND),
        },
        BindingSpec {
            keys: "x",
            behavior: BindingBehavior::Command(DIRED_EXECUTE_COMMAND),
        },
        BindingSpec {
            keys: "q",
            behavior: BindingBehavior::Command(RETURN_DOCUMENT_COMMAND),
        },
        BindingSpec {
            keys: "S-/",
            behavior: BindingBehavior::Command(DIRED_HELP_COMMAND),
        },
        BindingSpec {
            keys: "C-x d",
            behavior: BindingBehavior::Command(OPEN_DEFAULT_DIRED_COMMAND),
        },
        BindingSpec {
            keys: "C-x C-d",
            behavior: BindingBehavior::Command(TOGGLE_SIDEBAR_COMMAND),
        },
    ]
}

fn dired_command_items(commands: &CommandRegistry) -> Vec<(Arc<str>, Arc<str>)> {
    let mut items: Vec<(crate::command::CommandKey, Vec<&'static str>)> = Vec::new();
    for binding in dired_bindings() {
        let BindingBehavior::Command(name) = binding.behavior else {
            continue;
        };
        let Some(command) = commands.key(name) else {
            continue;
        };
        if let Some((_, keys)) = items.iter_mut().find(|(key, _)| *key == command) {
            keys.push(binding.keys);
        } else {
            items.push((command, vec![binding.keys]));
        }
    }

    let mut result = items
        .into_iter()
        .filter_map(|(key, keys)| {
            let descriptor = commands.descriptor(key)?;
            let keys = keys
                .into_iter()
                .map(display_dired_key)
                .collect::<Vec<_>>()
                .join(" / ");
            let title = if descriptor.name.as_str() == RELOAD_DOCUMENT_COMMAND {
                Arc::from("Refresh Directory")
            } else {
                descriptor.title.clone()
            };
            Some((Arc::from(keys), title))
        })
        .collect::<Vec<_>>();
    result.push((Arc::from("C-g"), Arc::from("Close command list")));
    result
}

fn display_dired_key(key: &str) -> &str {
    match key {
        "S-6" => "^",
        "S-/" => "?",
        "S-u" => "U",
        "S-h" => "H",
        "S-l" => "L",
        key => key,
    }
}

fn command_count(prefix: PrefixArgument) -> f32 {
    prefix.effective_count().unwrap_or(1).clamp(-1_000, 1_000) as f32
}

fn resolve_image_path(document_path: &Path, source: &str) -> PathBuf {
    let source = PathBuf::from(source);
    if source.is_absolute() {
        source
    } else {
        document_path
            .parent()
            .unwrap_or_else(|| Path::new("."))
            .join(source)
    }
}

fn build_image_sizes(document_path: &Path, blocks: &BlockArena) -> HashMap<BlockId, (u32, u32)> {
    blocks
        .nodes()
        .iter()
        .enumerate()
        .filter_map(|(block_id, block)| {
            let BlockKind::Image { path } = &block.kind else {
                return None;
            };
            image::image_dimensions(resolve_image_path(document_path, path))
                .ok()
                .filter(|&(width, height)| width > 0 && height > 0)
                .map(|size| (block_id as BlockId, size))
        })
        .collect()
}

fn build_markdown_image_sizes(
    document_path: &Path,
    blocks: &[markdown::MarkdownBlock],
) -> HashMap<BlockId, (u32, u32)> {
    blocks
        .iter()
        .enumerate()
        .filter_map(|(block_id, block)| {
            let markdown::MarkdownKind::Image { path } = &block.kind else {
                return None;
            };
            image::image_dimensions(resolve_image_path(document_path, path))
                .ok()
                .filter(|&(width, height)| width > 0 && height > 0)
                .map(|size| (block_id as BlockId, size))
        })
        .collect()
}

fn fitted_image_size(source_width: u32, source_height: u32, available_width: f32) -> (f32, f32) {
    let scale = (available_width.min(960.0) / source_width as f32)
        .min(480.0 / source_height as f32)
        .min(1.0);
    (source_width as f32 * scale, source_height as f32 * scale)
}

pub fn load_document(path: PathBuf) -> Result<PreviewDocument, (PathBuf, String)> {
    load_document_profiled(path)
}

pub fn load_document_profiled(path: PathBuf) -> Result<PreviewDocument, (PathBuf, String)> {
    load_document_profiled_impl(path, true)
}

pub fn load_document_profiled_without_display_map(
    path: PathBuf,
) -> Result<PreviewDocument, (PathBuf, String)> {
    load_document_profiled_impl(path, false)
}

fn load_document_profiled_impl(
    path: PathBuf,
    build_display_map: bool,
) -> Result<PreviewDocument, (PathBuf, String)> {
    let total_started = Instant::now();
    let read_started = Instant::now();
    let bytes = std::fs::read(&path).map_err(|error| (path.clone(), error.to_string()))?;
    let read = read_started.elapsed();
    let byte_count = bytes.len() as u64;
    let rope_started = Instant::now();
    let snapshot =
        RopeSnapshot::from_utf8(bytes).map_err(|error| (path.clone(), error.to_string()))?;
    let rope = rope_started.elapsed();
    let text: SharedTextSnapshot = Arc::new(snapshot);
    let parse_started = Instant::now();
    let format = match path
        .extension()
        .and_then(|extension| extension.to_str())
        .map(str::to_ascii_lowercase)
        .as_deref()
    {
        Some("md" | "markdown") => DocumentFormat::Markdown,
        _ => DocumentFormat::Org,
    };
    let (blocks, markdown_blocks, rows) = match format {
        DocumentFormat::Org => {
            let blocks = Arc::new(parse(text.as_ref()));
            let rows = Arc::new(build_preview_rows(text.as_ref(), &blocks));
            (blocks, Arc::new(Vec::new()), rows)
        }
        DocumentFormat::Markdown => {
            let (markdown_blocks, rows) = markdown::parse_markdown(text.as_ref());
            (
                Arc::new(BlockArena::default()),
                Arc::new(markdown_blocks),
                Arc::new(rows),
            )
        }
    };
    let parse = parse_started.elapsed();
    let tables = match format {
        DocumentFormat::Org => Arc::new(build_table_styles(text.as_ref(), &blocks)),
        DocumentFormat::Markdown => {
            Arc::new(build_markdown_table_styles(text.as_ref(), &markdown_blocks))
        }
    };
    let image_sizes = match format {
        DocumentFormat::Org => Arc::new(build_image_sizes(&path, &blocks)),
        DocumentFormat::Markdown => Arc::new(build_markdown_image_sizes(&path, &markdown_blocks)),
    };

    let mut document = PreviewDocument {
        path,
        text,
        format,
        blocks,
        markdown_blocks,
        rows,
        tables,
        image_sizes,
        display_map: None,
        metrics: LoadMetrics {
            bytes: byte_count,
            read,
            rope,
            parse,
            display_map: Duration::ZERO,
            total: Duration::ZERO,
        },
    };
    if build_display_map {
        let display_map_started = Instant::now();
        document.display_map = Some(Arc::new(minimap::build_display_map(&document)));
        document.metrics.display_map = display_map_started.elapsed();
    }
    document.metrics.total = total_started.elapsed();
    Ok(document)
}

fn centered_message(title: &str, detail: &str) -> gpui::Div {
    let theme = current_theme();
    div()
        .size_full()
        .flex()
        .flex_col()
        .bg(rgb(theme.background))
        .flex()
        .flex_col()
        .items_center()
        .justify_center()
        .gap_3()
        .child(
            div()
                .text_size(px(20.0))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(rgb(theme.heading[0]))
                .child(title.to_owned()),
        )
        .child(
            div()
                .max_w(px(520.0))
                .text_size(px(14.0))
                .line_height(px(21.0))
                .text_color(rgb(theme.foreground_dim))
                .child(detail.to_owned()),
        )
}

fn render_document(
    document: Arc<PreviewDocument>,
    list_state: ListState,
    visible_rows: Arc<Vec<usize>>,
    folded: Arc<HashSet<BlockId>>,
    entity: gpui::Entity<PreviewApp>,
    minimap_visible: bool,
    editor_width: f32,
    minimap_width: f32,
    minimap_resize_preview: Option<f32>,
    minimap_thumb_visibility: crate::settings::MinimapThumbVisibility,
    presentation_revision: u64,
) -> gpui::Div {
    let theme = current_theme();
    let preview_display_map = document.display_map.clone();
    let minimap_list_state = list_state.clone();
    let minimap_entity = entity.clone();
    let minimap_resize_entity = entity.clone();
    div()
        .size_full()
        .flex()
        .relative()
        .bg(rgb(theme.background))
        .child(
            div()
                .flex_1()
                .min_w_0()
                .h_full()
                .flex()
                .flex_col()
                .relative()
                .child(
                    div()
                        .absolute()
                        .top_0()
                        .bottom_0()
                        .left_0()
                        .w(px(50.0))
                        .bg(rgb(theme.background_alt))
                        .border_r_1()
                        .border_color(rgb(theme.border)),
                )
                .child({
                    let document = document.clone();
                    let visible_rows = visible_rows.clone();
                    let folded = folded.clone();
                    list(list_state, move |index, _window, _cx| {
                        let actual_index = visible_rows[index];
                        let row = document.rows[actual_index];
                        let display_map = document
                            .display_map
                            .as_ref()
                            .expect("preview display map must exist after loading");
                        let is_heading = display_map.is_heading(actual_index);
                        let is_table_row = display_map.is_table(actual_index);
                        let is_folded = is_heading && folded.contains(&row.block_id);
                        let document_for_click = document.clone();
                        let entity_for_click = entity.clone();
                        div()
                            .id(("preview-row", actual_index))
                            .w_full()
                            .min_h(px(24.0))
                            .when(index == 0, |element| element.pt_1())
                            .when(index + 1 == visible_rows.len(), |element| element.pb_2())
                            .when(is_heading, |element| {
                                element.cursor_pointer().on_click(move |_, _, cx| {
                                    entity_for_click.update(cx, |this, cx| {
                                        this.toggle_fold(row.block_id, &document_for_click);
                                        cx.notify();
                                    });
                                })
                            })
                            .flex()
                            .items_start()
                            .child(
                                div()
                                    .flex_none()
                                    .w(px(50.0))
                                    .pr_3()
                                    .h(px(24.0))
                                    .flex()
                                    .items_center()
                                    .justify_end()
                                    .text_right()
                                    .font_family("Menlo")
                                    .text_size(px(10.0))
                                    .text_color(rgb(theme.foreground_dim))
                                    .child(if row.show_line_number {
                                        row.source_line.to_string()
                                    } else {
                                        String::new()
                                    }),
                            )
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .min_h(px(24.0))
                                    .flex()
                                    .items_center()
                                    .pl_3()
                                    .pr_8()
                                    .when(is_table_row, |element| {
                                        element.bg(rgb(current_theme().background_alt))
                                    })
                                    .child(div().w_full().child(
                                        if document.format == DocumentFormat::Markdown {
                                            render_markdown_block(
                                                &document,
                                                actual_index,
                                                &document.markdown_blocks[row.block_id as usize],
                                                {
                                                    let minimap = minimap_visible
                                                        .then_some(minimap_width)
                                                        .unwrap_or(0.0);
                                                    (editor_width - 110.0 - minimap).max(120.0)
                                                },
                                            )
                                        } else {
                                            render_block(
                                                &document,
                                                actual_index,
                                                row,
                                                &document.blocks.nodes()[row.block_id as usize],
                                                is_folded,
                                                {
                                                    let minimap = minimap_visible
                                                        .then_some(minimap_width)
                                                        .unwrap_or(0.0);
                                                    (editor_width - 110.0 - minimap).max(120.0)
                                                },
                                            )
                                        },
                                    )),
                            )
                            .into_any()
                    })
                    .flex_1()
                    .w_full()
                }),
        )
        .when_some(
            minimap_visible.then_some(preview_display_map).flatten(),
            |layout, display_map| {
                layout.child(minimap::render(
                    display_map,
                    visible_rows.clone(),
                    folded.clone(),
                    minimap_list_state,
                    editor_width,
                    minimap_width,
                    minimap_thumb_visibility,
                    move |ratio, center, window, cx| {
                        minimap_entity.update(cx, |this, cx| {
                            this.minimap_pending_seek =
                                Some((presentation_revision, ratio, center));
                            if this.minimap_seek_scheduled {
                                return;
                            }
                            this.minimap_seek_scheduled = true;
                            cx.on_next_frame(window, |this, _, cx| {
                                this.minimap_seek_scheduled = false;
                                if let Some((revision, ratio, center)) =
                                    this.minimap_pending_seek.take()
                                    && accept_generation(this.presentation_revision, revision)
                                {
                                    minimap::seek_to_ratio(&this.list_state, ratio, center);
                                    cx.notify();
                                }
                            });
                        });
                    },
                    move |change, _, cx| {
                        minimap_resize_entity.update(cx, |this, cx| {
                            this.change_minimap_width(change, cx);
                        });
                    },
                ))
            },
        )
        .when_some(minimap_resize_preview, |layout, width| {
            layout.child(
                div()
                    .absolute()
                    .top_0()
                    .bottom_0()
                    .right(px(width))
                    .w(px(1.0))
                    .bg(rgb(theme.heading[0])),
            )
        })
}

fn render_block(
    document: &Arc<PreviewDocument>,
    display_row: usize,
    row: PreviewRow,
    block: &BlockNode,
    is_folded: bool,
    available_width: f32,
) -> gpui::Div {
    let theme = current_theme();
    let display_map = document
        .display_map
        .as_ref()
        .expect("preview display map must exist after loading");
    let display_runs = display_map.runs(display_row);
    let row_layout = display_map.layout(display_row);
    if row.blank {
        return div().h(px(row_layout.fixed_height.unwrap_or(row_layout.min_height)));
    }
    let text = display_runs.text.clone();
    let inline = || styled_inline_runs(text.clone(), display_runs.inline_spans.clone());

    let element = match &block.kind {
        BlockKind::BlankLine => {
            div().h(px(row_layout.fixed_height.unwrap_or(row_layout.min_height)))
        }
        BlockKind::Heading { level } => {
            let heading_index = (*level as usize).saturating_sub(1);
            let marker = format!(
                "{} ",
                theme.heading_bullets[heading_index % theme.heading_bullets.len()]
            );
            div()
                .flex()
                .items_center()
                .gap_1()
                .text_size(px(row_layout.font_size))
                .line_height(px(row_layout.line_height))
                .font_weight(if *level <= 2 {
                    FontWeight::SEMIBOLD
                } else {
                    FontWeight::MEDIUM
                })
                .text_color(rgb(theme.heading[heading_index.min(3)]))
                .child(
                    div()
                        .flex_none()
                        .font_family("Menlo")
                        .text_size(px(13.0))
                        .font_weight(FontWeight::MEDIUM)
                        .text_color(rgb(theme.heading[heading_index.min(3)]))
                        .child(marker),
                )
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .flex()
                        .items_center()
                        .child(inline())
                        .when(is_folded, |element| {
                            element.child(
                                div()
                                    .flex_none()
                                    .text_color(rgb(theme.keyword))
                                    .child("..."),
                            )
                        }),
                )
        }
        BlockKind::Paragraph => {
            let paragraph = div()
                .text_size(px(row_layout.font_size))
                .line_height(px(row_layout.line_height))
                .text_color(rgb(theme.foreground))
                .child(inline());
            if row.continuation {
                paragraph
            } else {
                paragraph
            }
        }
        BlockKind::Image { path } => {
            let source = resolve_image_path(&document.path, path);
            let (width, height) = document
                .display_map
                .as_ref()
                .and_then(|map| map.image_size(display_row, available_width))
                .unwrap_or_else(|| (available_width.min(640.0), 240.0));
            div()
                .w_full()
                .pt(px(row_layout.padding_top))
                .pb(px(row_layout.padding_bottom))
                .flex()
                .items_start()
                .justify_start()
                .child(img(source).w(px(width)).h(px(height)))
        }
        BlockKind::Planning => div()
            .font_family("Menlo")
            .text_size(px(row_layout.font_size))
            .line_height(px(row_layout.line_height))
            .text_color(rgb(theme.date))
            .child(text),
        BlockKind::ListItem => div()
            .pl(px(row_layout.padding_left))
            .text_size(px(row_layout.font_size))
            .line_height(px(row_layout.line_height))
            .text_color(rgb(theme.foreground))
            .child(inline()),
        BlockKind::FixedWidth => div()
            .font_family("Menlo")
            .text_size(px(row_layout.font_size))
            .line_height(px(row_layout.line_height))
            .text_color(rgb(theme.code_foreground))
            .child(text),
        BlockKind::FootnoteDefinition => div()
            .font_family("Menlo")
            .text_size(px(row_layout.font_size))
            .line_height(px(row_layout.line_height))
            .text_color(rgb(theme.link))
            .child(text),
        BlockKind::TableRow => render_table_row(
            &text,
            display_map
                .table_layout(display_row)
                .expect("table row layout must exist"),
        ),
        BlockKind::SourceBlock { .. } => {
            let marker = text.trim_start().to_ascii_lowercase();
            let is_boundary = marker.starts_with("#+begin_") || marker.starts_with("#+end_");
            let content = if is_boundary {
                StyledText::new(text.clone())
            } else {
                styled_code_runs(text.clone(), display_runs.code_spans.clone())
            };
            div()
                .min_h(px(row_layout.min_height))
                .pl(px(row_layout.padding_left))
                .pr(px(row_layout.padding_right))
                .pt(px(row_layout.padding_top))
                .pb(px(row_layout.padding_bottom))
                .bg(rgb(if is_boundary {
                    theme.code_boundary_background
                } else {
                    theme.code_background
                }))
                .text_color(if is_boundary {
                    rgb(theme.code_boundary)
                } else {
                    rgb(theme.code_foreground)
                })
                .font_family("Menlo")
                .text_size(px(row_layout.font_size))
                .line_height(px(row_layout.line_height))
                .child(content)
        }
        BlockKind::ExampleBlock | BlockKind::Raw | BlockKind::ExportBlock { .. } => div()
            .min_h(px(row_layout.min_height))
            .pl(px(row_layout.padding_left))
            .pr(px(row_layout.padding_right))
            .pt(px(row_layout.padding_top))
            .pb(px(row_layout.padding_bottom))
            .bg(rgb(theme.code_background))
            .font_family("Menlo")
            .text_size(px(row_layout.font_size))
            .line_height(px(row_layout.line_height))
            .text_color(rgb(theme.code_foreground))
            .child(text),
        BlockKind::QuoteBlock => div()
            .pl(px(row_layout.padding_left))
            .pr(px(row_layout.padding_right))
            .pt(px(row_layout.padding_top))
            .pb(px(row_layout.padding_bottom))
            .border_l_2()
            .border_color(rgb(theme.heading[1]))
            .text_color(rgb(theme.quote))
            .text_size(px(row_layout.font_size))
            .line_height(px(row_layout.line_height))
            .child(text),
        BlockKind::VerseBlock => div()
            .pl(px(row_layout.padding_left))
            .font_family("Menlo")
            .text_size(px(row_layout.font_size))
            .line_height(px(row_layout.line_height))
            .text_color(rgb(theme.quote))
            .child(text),
        BlockKind::CenterBlock => div()
            .w_full()
            .text_center()
            .text_size(px(row_layout.font_size))
            .line_height(px(row_layout.line_height))
            .text_color(rgb(theme.foreground))
            .child(text),
        BlockKind::SpecialBlock { name } => div()
            .min_h(px(row_layout.min_height))
            .pl(px(row_layout.padding_left))
            .pr(px(row_layout.padding_right))
            .pt(px(row_layout.padding_top))
            .pb(px(row_layout.padding_bottom))
            .bg(rgb(theme.code_background))
            .text_color(rgb(theme.attribute))
            .font_family("Menlo")
            .text_size(px(row_layout.font_size))
            .line_height(px(row_layout.line_height))
            .child(format!("{name}: {text}")),
        BlockKind::Drawer { .. } => div()
            .pl(px(row_layout.padding_left))
            .pr(px(row_layout.padding_right))
            .pt(px(row_layout.padding_top))
            .pb(px(row_layout.padding_bottom))
            .bg(rgb(theme.background_alt))
            .font_family("Menlo")
            .text_size(px(row_layout.font_size))
            .line_height(px(row_layout.line_height))
            .text_color(rgb(theme.meta))
            .child(text),
        BlockKind::Keyword => div()
            .pt(px(row_layout.padding_top))
            .pb(px(row_layout.padding_bottom))
            .font_family("Menlo")
            .text_size(px(row_layout.font_size))
            .line_height(px(row_layout.line_height))
            .text_color(rgb(theme.meta))
            .child(text),
        BlockKind::Comment | BlockKind::CommentBlock => div(),
        BlockKind::HorizontalRule => div()
            .mt(px(row_layout.margin_top))
            .mb(px(row_layout.margin_bottom))
            .h(px(row_layout.fixed_height.unwrap_or(1.0)))
            .w_full()
            .bg(rgb(theme.border)),
    };
    element
}

fn parse_document_inline(format: DocumentFormat, source: &str) -> InlineText {
    match format {
        DocumentFormat::Org => parse_inline(source),
        DocumentFormat::Markdown => markdown::parse_markdown_inline(source),
    }
}

fn render_markdown_block(
    document: &Arc<PreviewDocument>,
    display_row: usize,
    block: &markdown::MarkdownBlock,
    available_width: f32,
) -> gpui::Div {
    use markdown::MarkdownKind;
    let theme = current_theme();
    let display_map = document
        .display_map
        .as_ref()
        .expect("preview display map must exist after loading");
    let display_runs = display_map.runs(display_row);
    let row_layout = display_map.layout(display_row);
    let text = display_runs.text.clone();
    let inline = || styled_inline_runs(text.clone(), display_runs.inline_spans.clone());
    match &block.kind {
        MarkdownKind::Blank => {
            div().h(px(row_layout.fixed_height.unwrap_or(row_layout.min_height)))
        }
        MarkdownKind::Heading { level } => {
            let index = (*level as usize).saturating_sub(1).min(3);
            div()
                .flex()
                .items_center()
                .gap_1()
                .text_size(px(row_layout.font_size))
                .line_height(px(row_layout.line_height))
                .font_weight(if *level <= 2 {
                    FontWeight::SEMIBOLD
                } else {
                    FontWeight::MEDIUM
                })
                .text_color(rgb(theme.heading[index]))
                .child(
                    div()
                        .flex_none()
                        .text_size(px(13.0))
                        .child(format!("{} ", theme.heading_bullets[index])),
                )
                .child(inline())
        }
        MarkdownKind::Paragraph => div()
            .text_size(px(row_layout.font_size))
            .line_height(px(row_layout.line_height))
            .child(inline()),
        MarkdownKind::ListItem => div()
            .pl(px(row_layout.padding_left))
            .text_size(px(row_layout.font_size))
            .line_height(px(row_layout.line_height))
            .child(inline()),
        MarkdownKind::Quote => div()
            .pl(px(row_layout.padding_left))
            .pr(px(row_layout.padding_right))
            .pt(px(row_layout.padding_top))
            .pb(px(row_layout.padding_bottom))
            .border_l_2()
            .border_color(rgb(theme.heading[1]))
            .text_color(rgb(theme.quote))
            .text_size(px(row_layout.font_size))
            .line_height(px(row_layout.line_height))
            .child(inline()),
        MarkdownKind::Code { boundary, .. } => {
            let content = if *boundary {
                StyledText::new(text.clone())
            } else {
                styled_code_runs(text.clone(), display_runs.code_spans.clone())
            };
            div()
                .min_h(px(row_layout.min_height))
                .pl(px(row_layout.padding_left))
                .pr(px(row_layout.padding_right))
                .pt(px(row_layout.padding_top))
                .pb(px(row_layout.padding_bottom))
                .bg(rgb(if *boundary {
                    theme.code_boundary_background
                } else {
                    theme.code_background
                }))
                .text_color(rgb(if *boundary {
                    theme.code_boundary
                } else {
                    theme.code_foreground
                }))
                .font_family("Menlo")
                .text_size(px(row_layout.font_size))
                .line_height(px(row_layout.line_height))
                .child(content)
        }
        MarkdownKind::TableRow => render_table_row(
            &text,
            display_map
                .table_layout(display_row)
                .expect("markdown table row layout must exist"),
        ),
        MarkdownKind::HorizontalRule => div()
            .mt(px(row_layout.margin_top))
            .mb(px(row_layout.margin_bottom))
            .h(px(row_layout.fixed_height.unwrap_or(1.0)))
            .w_full()
            .bg(rgb(theme.border)),
        MarkdownKind::Image { path } => {
            let source = resolve_image_path(&document.path, path);
            let fitted = display_map.image_size(display_row, available_width);
            div()
                .w_full()
                .pt(px(row_layout.padding_top))
                .pb(px(row_layout.padding_bottom))
                .flex()
                .items_start()
                .child(if let Some((width, height)) = fitted {
                    img(source).w(px(width)).h(px(height))
                } else {
                    img(source).max_w(px(available_width.min(960.0)))
                })
        }
    }
}

fn styled_code_runs(text: gpui::SharedString, spans: Arc<[CodeHighlightSpan]>) -> StyledText {
    let highlights = spans
        .iter()
        .map(|span| (span.start..span.end, code_highlight_style(span.kind)));
    StyledText::new(text).with_highlights(highlights)
}

fn code_highlight_style(kind: CodeHighlightKind) -> HighlightStyle {
    let theme = current_theme();
    let color = match kind {
        CodeHighlightKind::Attribute => theme.attribute,
        CodeHighlightKind::Boolean | CodeHighlightKind::Constant => theme.constant,
        CodeHighlightKind::Comment => theme.comment,
        CodeHighlightKind::Function => theme.function,
        CodeHighlightKind::Keyword => theme.keyword,
        CodeHighlightKind::Number => theme.number,
        CodeHighlightKind::Operator | CodeHighlightKind::Punctuation => theme.operator,
        CodeHighlightKind::Property | CodeHighlightKind::Variable => theme.variable,
        CodeHighlightKind::String => theme.string,
        CodeHighlightKind::Type => theme.type_name,
    };
    HighlightStyle {
        color: Some(rgb(color).into()),
        font_style: matches!(kind, CodeHighlightKind::Comment).then_some(FontStyle::Italic),
        ..Default::default()
    }
}

fn styled_inline_runs(text: gpui::SharedString, spans: Arc<[InlineSpan]>) -> StyledText {
    let theme = current_theme();
    let highlights = spans.iter().map(|span| {
        let style = match span.kind {
            InlineKind::Bold => HighlightStyle {
                font_weight: Some(FontWeight::BOLD),
                ..Default::default()
            },
            InlineKind::Italic => HighlightStyle {
                font_style: Some(FontStyle::Italic),
                ..Default::default()
            },
            InlineKind::Underline => HighlightStyle {
                color: Some(rgb(theme.link).into()),
                ..Default::default()
            },
            InlineKind::Strike => HighlightStyle {
                fade_out: Some(0.55),
                ..Default::default()
            },
            InlineKind::Code | InlineKind::Verbatim => HighlightStyle {
                color: Some(rgb(theme.inline_code).into()),
                background_color: Some(rgb(theme.inline_code_background).into()),
                ..Default::default()
            },
            InlineKind::Link | InlineKind::FootnoteReference => HighlightStyle {
                color: Some(rgb(theme.link).into()),
                font_weight: Some(FontWeight::MEDIUM),
                ..Default::default()
            },
            InlineKind::Target | InlineKind::RadioTarget => HighlightStyle {
                color: Some(rgb(theme.attribute).into()),
                font_weight: Some(FontWeight::MEDIUM),
                ..Default::default()
            },
            InlineKind::Timestamp => HighlightStyle {
                color: Some(rgb(theme.date).into()),
                ..Default::default()
            },
            InlineKind::Entity | InlineKind::Latex => HighlightStyle {
                color: Some(rgb(theme.function).into()),
                ..Default::default()
            },
        };
        (span.range.clone(), style)
    });
    StyledText::new(text).with_highlights(highlights)
}

#[cfg(test)]
mod tests {
    use super::{accept_generation, dired_command_items, preview_input};

    #[test]
    fn stale_generations_are_rejected() {
        assert!(accept_generation(7, 7));
        assert!(!accept_generation(8, 7));
    }

    #[test]
    fn reload_failure_retains_previous_document_and_rejects_stale_completion() {
        let path = std::env::temp_dir().join(format!(
            "org-studio-reload-state-{}.org",
            std::process::id()
        ));
        std::fs::write(&path, "* retained\n").unwrap();
        let document = super::load_document(path.clone()).unwrap();
        let _ = std::fs::remove_file(&path);

        let mut app = super::PreviewApp::new();
        app.generation = 1;
        assert!(app.apply_load_result(1, Ok(document)));
        assert_eq!(
            app.last_ready.as_ref().map(|(generation, _)| *generation),
            Some(1)
        );

        app.generation = 2;
        assert!(app.apply_load_result(2, Err((path.clone(), "reload failed".to_owned()))));
        assert!(matches!(app.state, super::PreviewLoadState::Failed { .. }));
        assert_eq!(
            app.last_ready.as_ref().map(|(generation, _)| *generation),
            Some(1)
        );

        let stale_path = path.with_extension("md");
        std::fs::write(&stale_path, "# stale\n").unwrap();
        let stale = super::load_document(stale_path.clone()).unwrap();
        let _ = std::fs::remove_file(stale_path);
        assert!(!app.apply_load_result(1, Ok(stale)));
        assert!(matches!(app.state, super::PreviewLoadState::Failed { .. }));
        assert_eq!(
            app.last_ready.as_ref().map(|(generation, _)| *generation),
            Some(1)
        );
    }

    #[test]
    fn dired_help_lists_every_command_and_groups_alias_keys() {
        let (commands, _, _) = preview_input();
        let items = dired_command_items(&commands);
        let labels = items
            .iter()
            .map(|(keys, title)| (keys.as_ref(), title.as_ref()))
            .collect::<Vec<_>>();

        assert_eq!(items.len(), 18);
        assert!(labels.contains(&("n / j", "Next Line")));
        assert!(labels.contains(&("p / k", "Previous Line")));
        assert!(labels.contains(&("^ / h", "Up Directory")));
        assert!(labels.contains(&("RET / l", "Open")));
        assert!(labels.contains(&("H", "History Back")));
        assert!(labels.contains(&("L", "History Forward")));
        assert!(labels.contains(&("g", "Refresh Directory")));
        assert!(labels.contains(&("C-x d", "Open Dired")));
        assert!(labels.contains(&("C-x C-d", "Toggle Sidebar")));
        assert!(labels.contains(&("?", "Dired Help")));
        assert!(labels.contains(&("C-g", "Close command list")));
    }

    #[test]
    fn loads_markdown_without_sending_it_through_the_org_parser() {
        let path =
            std::env::temp_dir().join(format!("org-studio-markdown-{}.md", std::process::id()));
        std::fs::write(&path, "# Markdown\n\n- **native** preview\n").unwrap();
        let document = super::load_document(path.clone()).unwrap();
        let _ = std::fs::remove_file(path);

        assert_eq!(document.format, super::DocumentFormat::Markdown);
        assert!(document.blocks.nodes().is_empty());
        assert!(!document.markdown_blocks.is_empty());
        assert_eq!(document.rows.len(), 3);
    }

    #[test]
    fn markdown_parent_and_minimap_share_identical_display_runs() {
        let path = std::env::temp_dir().join(format!(
            "org-studio-shared-display-runs-{}.md",
            std::process::id()
        ));
        std::fs::write(
            &path,
            "# **标题** and *italic*\n\n```rust\nlet answer = 42;\n```\n",
        )
        .unwrap();
        let document = super::load_document(path.clone()).unwrap();
        let _ = std::fs::remove_file(path);
        let display_map = document.display_map.as_ref().unwrap();
        let heading_layout = display_map.layout(0);
        assert_eq!(heading_layout.font_size, 22.0);
        assert_eq!(heading_layout.line_height, 24.0);
        let blank_layout = display_map.layout(1);
        assert_eq!(blank_layout.fixed_height, Some(24.0));

        let heading =
            super::parse_document_inline(super::DocumentFormat::Markdown, "**标题** and *italic*");
        let heading_runs = display_map.runs(0);
        assert_eq!(heading_runs.text.as_ref(), heading.text);
        assert_eq!(heading_runs.inline_spans.as_ref(), heading.spans);

        let code_row = document
            .rows
            .iter()
            .position(|row| document.text.copy_range(row.content).contains("answer"))
            .expect("code row");
        assert!(!display_map.runs(code_row).code_spans.is_empty());
        let code_layout = display_map.layout(code_row);
        assert_eq!(code_layout.padding_left, 16.0);
        assert_eq!(code_layout.padding_right, 16.0);
        assert_eq!(code_layout.min_height, 24.0);
    }

    #[test]
    fn org_parent_and_minimap_share_identical_display_runs() {
        let path = std::env::temp_dir().join(format!(
            "org-studio-shared-display-runs-{}.org",
            std::process::id()
        ));
        std::fs::write(
            &path,
            "* *粗体* and /italic/\n\n#+begin_src rust\nlet n = 7;\n#+end_src\n",
        )
        .unwrap();
        let document = super::load_document(path.clone()).unwrap();
        let _ = std::fs::remove_file(path);
        let display_map = document.display_map.as_ref().unwrap();
        let heading_layout = display_map.layout(0);
        assert_eq!(heading_layout.font_size, 22.0);
        assert_eq!(heading_layout.line_height, 24.0);
        let blank_layout = display_map.layout(1);
        assert_eq!(blank_layout.fixed_height, Some(24.0));
        let source = document
            .text
            .copy_range(document.rows[0].content)
            .trim_end_matches(['\r', '\n'])
            .to_owned();
        let expected = super::parse_document_inline(super::DocumentFormat::Org, &source);
        let heading_runs = display_map.runs(0);
        assert_eq!(heading_runs.text.as_ref(), expected.text);
        assert_eq!(heading_runs.inline_spans.as_ref(), expected.spans);

        let code_row = document
            .rows
            .iter()
            .position(|row| document.text.copy_range(row.content).contains("let n"))
            .expect("code row");
        assert!(!display_map.runs(code_row).code_spans.is_empty());
        let code_layout = display_map.layout(code_row);
        assert_eq!(code_layout.padding_left, 16.0);
        assert_eq!(code_layout.padding_right, 16.0);
        assert_eq!(code_layout.line_height, 19.0);
        assert_eq!(code_layout.min_height, 24.0);
    }
}
