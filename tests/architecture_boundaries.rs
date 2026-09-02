use std::path::{Path, PathBuf};

use syn::{
    Fields, Item, ItemStruct, ItemUse, Path as SynPath, Type, UseTree, Visibility, visit::Visit,
};

fn rust_files_under(root: &Path, files: &mut Vec<PathBuf>) {
    for entry in std::fs::read_dir(root).expect("architecture root must exist") {
        let path = entry.expect("architecture entry must be readable").path();
        if path.is_dir() {
            rust_files_under(&path, files);
        } else if path.extension().is_some_and(|extension| extension == "rs") {
            files.push(path);
        }
    }
}

fn parse(path: &Path) -> syn::File {
    let source = std::fs::read_to_string(path).expect("Rust source must be UTF-8");
    syn::parse_file(&source).unwrap_or_else(|error| panic!("{}: {error}", path.display()))
}

#[derive(Default)]
struct DependencyCollector {
    paths: Vec<String>,
}

impl DependencyCollector {
    fn collect_use_tree(&mut self, prefix: &str, tree: &UseTree) {
        match tree {
            UseTree::Path(path) => {
                let next = if prefix.is_empty() {
                    path.ident.to_string()
                } else {
                    format!("{prefix}::{}", path.ident)
                };
                self.collect_use_tree(&next, &path.tree);
            }
            UseTree::Name(name) => self.paths.push(if prefix.is_empty() {
                name.ident.to_string()
            } else {
                format!("{prefix}::{}", name.ident)
            }),
            UseTree::Rename(rename) => self.paths.push(if prefix.is_empty() {
                rename.ident.to_string()
            } else {
                format!("{prefix}::{}", rename.ident)
            }),
            UseTree::Glob(_) => self.paths.push(format!("{prefix}::*")),
            UseTree::Group(group) => {
                for item in &group.items {
                    self.collect_use_tree(prefix, item);
                }
            }
        }
    }
}

impl<'ast> Visit<'ast> for DependencyCollector {
    fn visit_item_use(&mut self, node: &'ast ItemUse) {
        self.collect_use_tree("", &node.tree);
        syn::visit::visit_item_use(self, node);
    }

    fn visit_path(&mut self, node: &'ast SynPath) {
        self.paths.push(
            node.segments
                .iter()
                .map(|segment| segment.ident.to_string())
                .collect::<Vec<_>>()
                .join("::"),
        );
        syn::visit::visit_path(self, node);
    }
}

fn dependencies(path: &Path) -> Vec<String> {
    let syntax = parse(path);
    let mut collector = DependencyCollector::default();
    collector.visit_file(&syntax);
    collector.paths
}

fn fields_of(path: &Path, name: &str) -> Fields {
    let syntax = parse(path);
    syntax
        .items
        .into_iter()
        .find_map(|item| match item {
            syn::Item::Struct(item) if item.ident == name => Some(item.fields),
            _ => None,
        })
        .unwrap_or_else(|| panic!("{} must define {name}", path.display()))
}

#[test]
fn document_foundation_has_no_preview_or_editor_dependency() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let mut files = Vec::new();
    for relative in ["src/document", "src/org_syntax"] {
        rust_files_under(&root.join(relative), &mut files);
    }

    let mut violations = Vec::new();
    for path in files {
        for dependency in dependencies(&path) {
            if dependency.starts_with("crate::preview")
                || dependency.starts_with("crate::editor")
                || dependency.starts_with("super::preview")
                || dependency.starts_with("super::editor")
            {
                violations.push(format!("{} depends on {dependency}", path.display()));
            }
            if dependency.starts_with("gpui") {
                let is_session = path.file_name().is_some_and(|name| name == "session.rs");
                let allowed_session_model_dependency = matches!(
                    dependency.as_str(),
                    "gpui::Context"
                        | "gpui::EventEmitter"
                        | "gpui::AppContext"
                        | "gpui::test"
                        | "gpui::TestAppContext"
                );
                if !is_session || !allowed_session_model_dependency {
                    violations.push(format!("{} depends on {dependency}", path.display()));
                }
            }
        }
    }
    assert!(
        violations.is_empty(),
        "document/syntax dependency violations:\n{}",
        violations.join("\n")
    );
}

#[test]
fn product_shell_has_one_definition_in_app() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let mut files = Vec::new();
    rust_files_under(&root.join("src"), &mut files);
    let definitions = files
        .iter()
        .flat_map(|path| {
            parse(path).items.into_iter().filter_map(|item| match item {
                syn::Item::Struct(ItemStruct { ident, vis, .. }) if ident == "WorkspaceWindow" => {
                    Some((path.clone(), vis))
                }
                _ => None,
            })
        })
        .collect::<Vec<_>>();
    assert_eq!(
        definitions.len(),
        1,
        "WorkspaceWindow must have one definition"
    );
    assert_eq!(
        definitions[0].0,
        root.join("src/app/mod.rs"),
        "the product shell belongs in src/app"
    );
    assert!(matches!(definitions[0].1, Visibility::Public(_)));
}

#[test]
fn workspace_state_types_live_in_app() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let mut files = Vec::new();
    rust_files_under(&root.join("src"), &mut files);
    let workspace_types = [
        "WorkspaceWindow",
        "WorkspaceLoadState",
        "ReadyDocument",
        "DocumentWorkspaceState",
        "DocumentViewPreferences",
        "ContentRoute",
    ];
    let mut violations = Vec::new();
    for path in files {
        for item in parse(&path).items {
            let name = match item {
                Item::Struct(item) => Some(item.ident),
                Item::Enum(item) => Some(item.ident),
                Item::Type(item) => Some(item.ident),
                _ => None,
            };
            if let Some(name) = name
                && workspace_types.iter().any(|workspace| name == *workspace)
                && path != root.join("src/app/mod.rs")
            {
                violations.push(format!("{} defines {name}", path.display()));
            }
        }
    }
    assert!(
        violations.is_empty(),
        "workspace state ownership violations:\n{}",
        violations.join("\n")
    );
}

#[test]
fn entity_and_load_boundaries_keep_their_fields_private() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    for (relative, name) in [
        ("src/preview/reading_panel.rs", "ReadingPreviewPanel"),
        ("src/preview/document.rs", "LoadedDocument"),
        ("src/app/file_manager.rs", "FileManagerHost"),
        ("src/app/export_ui.rs", "ExportHost"),
        ("src/app/status_line.rs", "StatusLineHost"),
        ("src/editor/mod.rs", "SemanticEditor"),
    ] {
        for field in fields_of(&root.join(relative), name) {
            assert!(
                matches!(field.vis, Visibility::Inherited),
                "{relative}::{name}.{} must be private",
                field
                    .ident
                    .map_or_else(|| "<unnamed>".into(), |ident| ident.to_string())
            );
        }
    }
}

#[test]
fn workspace_window_implementations_live_in_app() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let mut files = Vec::new();
    rust_files_under(&root.join("src"), &mut files);
    let app_root = root.join("src/app");
    let mut violations = Vec::new();

    for path in files {
        for item in parse(&path).items {
            let Item::Impl(item) = item else {
                continue;
            };
            let Type::Path(self_type) = item.self_ty.as_ref() else {
                continue;
            };
            if self_type
                .path
                .segments
                .last()
                .is_some_and(|segment| segment.ident == "WorkspaceWindow")
                && !path.starts_with(&app_root)
            {
                violations.push(path.display().to_string());
            }
        }
    }

    assert!(
        violations.is_empty(),
        "WorkspaceWindow implementations must live in src/app:\n{}",
        violations.join("\n")
    );
}

#[test]
fn preview_production_code_has_no_product_shell_dependency() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let mut files = Vec::new();
    rust_files_under(&root.join("src/preview"), &mut files);
    files.retain(|path| path.file_name().is_none_or(|name| name != "tests.rs"));
    let mut violations = Vec::new();

    for path in files {
        for dependency in dependencies(&path) {
            if dependency.starts_with("crate::app") || dependency.contains("WorkspaceWindow") {
                violations.push(format!("{} depends on {dependency}", path.display()));
            }
        }
    }

    assert!(
        violations.is_empty(),
        "preview product-shell dependency violations:\n{}",
        violations.join("\n")
    );
}

#[test]
fn removed_product_types_do_not_return_as_items() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let mut files = Vec::new();
    rust_files_under(&root.join("src"), &mut files);
    let forbidden = [
        "PreviewApp",
        "PreviewDocument",
        "RopeSnapshot",
        "DocumentMode",
        "SplitScrollHost",
        "DocumentLayout",
        "DocumentSurfaceFocus",
        "RightPreviewState",
        "PreviewPanel",
        "PreviewProjectionSnapshot",
    ];
    let mut violations = Vec::new();
    for path in files {
        for item in parse(&path).items {
            let name = match item {
                syn::Item::Struct(item) => Some(item.ident),
                syn::Item::Enum(item) => Some(item.ident),
                syn::Item::Type(item) => Some(item.ident),
                _ => None,
            };
            if let Some(name) = name
                && forbidden.iter().any(|forbidden| name == *forbidden)
            {
                violations.push(format!("{} defines {name}", path.display()));
            }
        }
    }
    assert!(violations.is_empty(), "{}", violations.join("\n"));
}

#[test]
fn reading_core_does_not_own_the_workspace_or_export_pipeline() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let mut files = [
        "src/preview/action.rs",
        "src/preview/projection.rs",
        "src/preview/reading_panel.rs",
        "src/preview/style.rs",
    ]
    .into_iter()
    .map(|relative| root.join(relative))
    .collect::<Vec<_>>();
    for relative in [
        "src/preview/display_map",
        "src/preview/minimap",
        "src/preview/view",
    ] {
        rust_files_under(&root.join(relative), &mut files);
    }

    let mut violations = Vec::new();
    for path in files {
        for dependency in dependencies(&path) {
            if dependency.starts_with("crate::app")
                || dependency.starts_with("crate::export")
                || dependency.starts_with("typst")
                || dependency.contains("WorkspaceWindow")
            {
                violations.push(format!("{} depends on {dependency}", path.display()));
            }
        }
    }
    assert!(
        violations.is_empty(),
        "Reading core dependency violations:\n{}",
        violations.join("\n")
    );
}

#[test]
fn reading_panel_has_no_mutable_text_or_command_capability() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let fields = fields_of(
        &root.join("src/preview/reading_panel.rs"),
        "ReadingPreviewPanel",
    );
    let forbidden = [
        "DocumentBuffer",
        "DocumentSession",
        "CommandRegistry",
        "CapabilitySet",
        "CommandImplementation",
    ];
    let mut collector = DependencyCollector::default();
    for field in fields {
        collector.visit_type(&field.ty);
    }
    for name in forbidden {
        assert!(
            !collector
                .paths
                .iter()
                .any(|path| path.split("::").any(|segment| segment == name)),
            "ReadingPreviewPanel must not own {name}"
        );
    }
}

#[test]
fn minimap_runtime_state_is_host_owned() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let mut files = Vec::new();
    rust_files_under(&root.join("src/preview/minimap"), &mut files);

    let mut violations = Vec::new();
    for path in files {
        for item in parse(&path).items {
            if let Item::Static(item) = item {
                let name = item.ident.to_string();
                if ["CACHE", "RASTER", "GENERATION", "TILE", "THUMB"]
                    .iter()
                    .any(|token| name.contains(token))
                    && name != "TEXT_RASTERIZER_PREWARMED"
                {
                    violations.push(format!("{} defines static {name}", path.display()));
                }
            }
        }
    }
    assert!(
        violations.is_empty(),
        "Minimap host-isolation violations:\n{}",
        violations.join("\n")
    );
}
