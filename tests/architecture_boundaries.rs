use std::path::{Path, PathBuf};

use syn::{Fields, ItemStruct, ItemUse, Path as SynPath, UseTree, Visibility, visit::Visit};

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
fn entity_and_load_boundaries_keep_their_fields_private() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    for (relative, name) in [
        ("src/preview/panel.rs", "PreviewPanel"),
        ("src/preview/document.rs", "LoadedDocument"),
        ("src/preview/file_manager_host.rs", "FileManagerHost"),
        ("src/preview/export_ui.rs", "ExportHost"),
        ("src/preview/status_line.rs", "StatusLineHost"),
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
fn removed_product_types_do_not_return_as_items() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let mut files = Vec::new();
    rust_files_under(&root.join("src"), &mut files);
    let forbidden = ["PreviewApp", "PreviewDocument", "RopeSnapshot"];
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
