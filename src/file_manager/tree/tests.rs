use super::*;
use crate::file_manager::{EntryKind, scan_directory};

fn load(store: &mut DirectoryStore, directory: &Path) {
    let request = store.request(directory.to_path_buf(), true).unwrap();
    assert!(store.commit(&request, scan_directory(directory.to_path_buf())));
}

fn fixture() -> (tempfile::TempDir, DirectoryStore, FileTree) {
    let directory = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(directory.path().join("chapter/drafts")).unwrap();
    std::fs::write(
        directory.path().join("chapter/drafts/entry.org"),
        "* Heading\n",
    )
    .unwrap();
    std::fs::write(directory.path().join("notes.md"), "# Heading\n").unwrap();
    std::fs::write(directory.path().join(".hidden"), "value\n").unwrap();
    let mut store = DirectoryStore::default();
    load(&mut store, directory.path());
    let mut tree = FileTree::default();
    tree.set_root(directory.path().to_path_buf());
    tree.rebuild(&store);
    (directory, store, tree)
}

fn id(tree: &FileTree, name: &str) -> EntryId {
    tree.rows()
        .iter()
        .filter_map(TreeRow::entry)
        .find(|entry| entry.display_name.as_ref() == name)
        .unwrap()
        .id
}

#[test]
fn nested_expansion_keeps_the_root_and_siblings() {
    let (directory, mut store, mut tree) = fixture();
    let root = tree.root().unwrap().to_path_buf();
    tree.toggle(id(&tree, "chapter"));
    load(&mut store, &directory.path().join("chapter"));
    tree.rebuild(&store);
    tree.toggle(id(&tree, "drafts"));
    load(&mut store, &directory.path().join("chapter/drafts"));
    tree.rebuild(&store);
    assert_eq!(tree.root(), Some(root.as_path()));
    assert!(
        tree.rows()
            .iter()
            .filter_map(TreeRow::entry)
            .any(|entry| entry.display_name.as_ref() == "notes.md")
    );
    let row = tree
        .rows()
        .iter()
        .find(|row| {
            row.entry()
                .is_some_and(|entry| entry.display_name.as_ref() == "entry.org")
        })
        .unwrap();
    assert_eq!(row.depth, 3);
    assert!(
        !tree
            .rows()
            .iter()
            .filter_map(TreeRow::entry)
            .any(|entry| entry.kind == EntryKind::Parent)
    );
    assert!(
        !tree
            .rows()
            .iter()
            .filter_map(TreeRow::entry)
            .any(|entry| entry.metadata.hidden)
    );
}

#[test]
fn collapse_moves_focus_to_the_parent_and_removes_hidden_operation_targets() {
    let (directory, mut store, mut tree) = fixture();
    tree.toggle(id(&tree, "chapter"));
    load(&mut store, &directory.path().join("chapter"));
    tree.rebuild(&store);
    tree.select(id(&tree, "drafts"), TreeSelection::Single);
    let parent = id(&tree, "chapter");
    tree.toggle(parent);
    tree.rebuild(&store);
    assert_eq!(tree.cursor(), Some(parent));
    assert_eq!(
        tree.operation_targets().as_ref(),
        &[Arc::<Path>::from(directory.path().join("chapter"))]
    );
    assert!(
        !tree
            .active_directories()
            .contains(&directory.path().join("chapter/drafts"))
    );
}

#[test]
fn navigation_expands_stays_then_enters_and_never_requests_closed_branches() {
    let (directory, mut store, mut tree) = fixture();
    tree.select(id(&tree, "chapter"), TreeSelection::Single);
    assert!(tree.move_hierarchy(true));
    assert_eq!(tree.current().unwrap().display_name.as_ref(), "chapter");
    load(&mut store, &directory.path().join("chapter"));
    tree.rebuild(&store);
    assert!(!tree.move_hierarchy(true));
    assert_eq!(tree.current().unwrap().display_name.as_ref(), "drafts");
    assert!(
        !tree
            .active_directories()
            .contains(&directory.path().join("chapter/drafts"))
    );
    assert!(!tree.move_hierarchy(false));
    assert_eq!(tree.current().unwrap().display_name.as_ref(), "chapter");
}

#[test]
fn root_memories_restore_expansion_and_selection_without_moving_the_other_root() {
    let (directory, store, mut tree) = fixture();
    tree.toggle(id(&tree, "chapter"));
    tree.select(id(&tree, "notes.md"), TreeSelection::Single);
    let second = tempfile::tempdir().unwrap();
    tree.set_root(second.path().to_path_buf());
    tree.rebuild(&store);
    tree.set_root(directory.path().to_path_buf());
    tree.rebuild(&store);
    assert!(tree.expanded(&directory.path().join("chapter")));
    assert_eq!(tree.current().unwrap().display_name.as_ref(), "notes.md");
    let memory = serde_json::to_string(&tree.memories()).unwrap();
    let mut restored = FileTree::restore(serde_json::from_str(&memory).unwrap());
    restored.rebuild(&store);
    assert_eq!(
        restored.current().unwrap().display_name.as_ref(),
        "notes.md"
    );
}

#[test]
fn independent_directory_reads_coalesce_and_cancel_only_their_own_results() {
    let directory = tempfile::tempdir().unwrap();
    let second = tempfile::tempdir().unwrap();
    let mut store = DirectoryStore::default();
    let first = store
        .request(directory.path().to_path_buf(), false)
        .unwrap();
    let other = store.request(second.path().to_path_buf(), false).unwrap();
    assert!(
        store
            .request(directory.path().to_path_buf(), false)
            .is_none()
    );
    assert!(
        store
            .request(directory.path().to_path_buf(), true)
            .is_none()
    );
    assert!(store.commit(&first, scan_directory(directory.path().to_path_buf())));
    assert!(store.needs_refresh(directory.path()));
    store.retain_active(&HashSet::from([directory.path().to_path_buf()]));
    assert!(!store.commit(&other, scan_directory(second.path().to_path_buf())));
    let fresh = store.request(second.path().to_path_buf(), true).unwrap();
    assert!(store.commit(&fresh, scan_directory(second.path().to_path_buf())));
    assert!(!store.commit(&other, scan_directory(second.path().to_path_buf())));
}

#[test]
fn mismatched_directory_result_preserves_the_pending_request() {
    let first = tempfile::tempdir().unwrap();
    let second = tempfile::tempdir().unwrap();
    let mut store = DirectoryStore::default();
    let request = store.request(first.path().to_path_buf(), false).unwrap();
    assert!(!store.commit(&request, scan_directory(second.path().to_path_buf())));
    assert!(store.is_loading(first.path()));
    assert!(store.snapshot(first.path()).is_none());
    assert!(store.commit(&request, scan_directory(first.path().to_path_buf())));
    assert!(!store.is_loading(first.path()));
    assert!(store.is_current(first.path()));
}

#[test]
fn folder_groups_stay_first_in_both_sort_directions_and_names_sort_numerically() {
    let (directory, mut store, mut tree) = fixture();
    for name in ["item2.txt", "item10.txt"] {
        std::fs::write(directory.path().join(name), "value\n").unwrap();
    }
    load(&mut store, directory.path());
    tree.rebuild(&store);
    let names = |tree: &FileTree| {
        tree.rows()
            .iter()
            .filter_map(TreeRow::entry)
            .skip(1)
            .map(|entry| entry.display_name.to_string())
            .collect::<Vec<_>>()
    };
    assert_eq!(
        names(&tree),
        ["chapter", "item2.txt", "item10.txt", "notes.md"]
    );
    tree.sort.direction = super::super::SortDirection::Descending;
    tree.rebuild(&store);
    assert_eq!(
        names(&tree),
        ["chapter", "notes.md", "item10.txt", "item2.txt"]
    );
}

#[test]
fn deletion_selects_the_closest_sibling_instead_of_the_root() {
    let (directory, mut store, mut tree) = fixture();
    std::fs::write(directory.path().join("other.md"), "# Heading\n").unwrap();
    load(&mut store, directory.path());
    tree.rebuild(&store);
    tree.select(id(&tree, "notes.md"), TreeSelection::Single);
    std::fs::remove_file(directory.path().join("notes.md")).unwrap();
    load(&mut store, directory.path());
    tree.rebuild(&store);
    assert_eq!(tree.current().unwrap().display_name.as_ref(), "other.md");
}

#[test]
fn external_rename_preserves_expansion_selection_and_viewport() {
    let (directory, mut store, mut tree) = fixture();
    let old = directory.path().join("chapter");
    let new = directory.path().join("section");
    tree.toggle(id(&tree, "chapter"));
    tree.select(id(&tree, "chapter"), TreeSelection::Single);
    tree.capture_viewport(tree.rank(id(&tree, "chapter")).unwrap(), 3.);
    std::fs::rename(&old, &new).unwrap();
    load(&mut store, directory.path());
    tree.rebuild(&store);
    assert!(tree.expanded(&new));
    assert!(!tree.expanded(&old));
    assert_eq!(tree.current().unwrap().path.as_ref(), new);
    assert_eq!(tree.viewport().unwrap().1, 3.);
}

#[test]
fn hard_links_remain_distinct_and_a_deleted_link_never_selects_its_sibling_by_identity() {
    let directory = tempfile::tempdir().unwrap();
    let first = directory.path().join("first.txt");
    let second = directory.path().join("second.txt");
    let middle = directory.path().join("middle.txt");
    std::fs::write(&first, "value\n").unwrap();
    std::fs::hard_link(&first, &second).unwrap();
    std::fs::write(&middle, "other\n").unwrap();
    let mut store = DirectoryStore::default();
    load(&mut store, directory.path());
    let mut tree = FileTree::default();
    tree.set_root(directory.path().to_path_buf());
    tree.rebuild(&store);
    tree.select(id(&tree, "first.txt"), TreeSelection::Single);
    std::fs::remove_file(&first).unwrap();
    load(&mut store, directory.path());
    tree.rebuild(&store);
    assert_eq!(tree.current().unwrap().path.as_ref(), middle);
    assert_eq!(tree.rows().iter().filter_map(TreeRow::entry).count(), 3);
}

#[test]
fn an_error_is_cached_until_retry_and_does_not_erase_a_successful_snapshot() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("items");
    std::fs::create_dir(&path).unwrap();
    std::fs::write(path.join("entry.txt"), "value").unwrap();
    let mut store = DirectoryStore::default();
    load(&mut store, &path);
    std::fs::remove_dir_all(&path).unwrap();
    load(&mut store, directory.path());
    let request = store.request(path.clone(), true).unwrap();
    assert!(store.commit(&request, scan_directory(path.clone())));
    assert!(store.error(&path).is_some());
    assert!(store.snapshot(&path).is_some());
    assert!(store.request(path.clone(), false).is_none());
    std::fs::create_dir(&path).unwrap();
    load(&mut store, &path);
    assert!(store.error(&path).is_none());
    assert!(store.snapshot(&path).unwrap().entries.is_empty());
}
