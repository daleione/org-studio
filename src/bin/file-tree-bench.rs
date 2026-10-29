//! Real-directory scan and cached-outline latency, independent of GPU/frame timing.
use org_studio::file_manager::{DirectoryStore, FileTree, TreeRow, scan_directory};
use std::{path::Path, time::Instant};

fn read(store: &mut DirectoryStore, path: &Path) -> f64 {
    let started = Instant::now();
    let request = store.request(path.to_path_buf(), false).unwrap();
    assert!(store.commit(&request, scan_directory(path.to_path_buf())));
    started.elapsed().as_secs_f64() * 1000.
}

fn distribution(mut operation: impl FnMut()) -> serde_json::Value {
    let mut samples = (0..40)
        .map(|_| {
            let started = Instant::now();
            operation();
            started.elapsed().as_secs_f64() * 1000.
        })
        .collect::<Vec<_>>();
    samples.sort_by(f64::total_cmp);
    serde_json::json!({ "p50_ms": samples[20], "p95_ms": samples[38] })
}

fn main() {
    let fixture = tempfile::tempdir().unwrap();
    let flat = fixture.path().join("flat");
    std::fs::create_dir(&flat).unwrap();
    for index in 0..10_000 {
        std::fs::write(flat.join(format!("entry-{index}.txt")), "text\n").unwrap();
    }
    let mut store = DirectoryStore::default();
    let flat_scan = read(&mut store, &flat);
    let mut tree = FileTree::default();
    tree.set_root(flat.clone());
    let started = Instant::now();
    tree.rebuild(&store);
    let flat_first_projection = started.elapsed().as_secs_f64() * 1000.;
    let flat_projection = distribution(|| tree.rebuild(&store));
    let navigation = distribution(|| {
        for _ in 0..1000 {
            tree.move_cursor(1, false);
            tree.move_cursor(-1, false);
        }
    });
    let nested = fixture.path().join("nested");
    std::fs::create_dir(&nested).unwrap();
    for branch in 0..100 {
        let path = nested.join(format!("branch-{branch:02}"));
        std::fs::create_dir(&path).unwrap();
        for entry in 0..1000 {
            std::fs::write(path.join(format!("entry-{entry}.txt")), "text\n").unwrap();
        }
    }
    let nested_scan = read(&mut store, &nested);
    tree.set_root(nested.clone());
    tree.rebuild(&store);
    let branch = tree
        .rows()
        .iter()
        .filter_map(TreeRow::entry)
        .find(|entry| entry.path.parent() == Some(nested.as_path()))
        .unwrap()
        .clone();
    let branch_scan = read(&mut store, &branch.path);
    tree.toggle(branch.id);
    let expanded_projection = distribution(|| tree.rebuild(&store));
    assert_eq!(
        tree.active_directories().len(),
        2,
        "closed branches must remain unread"
    );
    println!(
        "{}",
        serde_json::to_string_pretty(&serde_json::json!({
            "fixture": "newly created files; OS filesystem cache is warm",
            "flat_files": 10000, "flat_first_scan_ms": flat_scan,
        "flat_first_projection_ms": flat_first_projection,
            "flat_cached_projection": flat_projection,
            "navigation_2000_moves": navigation,
            "nested_files": 100000, "nested_root_scan_ms": nested_scan,
            "one_branch_scan_ms": branch_scan,
            "cached_branch_projection": expanded_projection,
            "active_directories": tree.active_directories().len(),
            "projected_rows": tree.rows().len(),
        }))
        .unwrap()
    );
}
