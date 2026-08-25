use std::{env, time::Instant};

use org_studio::file_manager::{DiredSession, scan_directory};
use org_studio::navigation::{NavigationCause, SelectionIntent};

fn main() {
    let directory = env::args_os()
        .nth(1)
        .map(Into::into)
        .unwrap_or_else(|| env::current_dir().expect("current directory"));
    let scan_started = Instant::now();
    let result = scan_directory(directory.clone()).expect("directory scan failed");
    let scan_elapsed = scan_started.elapsed();
    let mut session = DiredSession::empty(directory.clone());
    let intent = DiredSession::intent_for(
        directory,
        NavigationCause::Enter,
        SelectionIntent::FirstSelectable,
    );
    let load = session.begin_navigation(intent);
    let reconcile_started = Instant::now();
    assert!(session.apply_scan(&load, result).is_some());
    let reconcile_elapsed = reconcile_started.elapsed();
    println!(
        "file_manager_benchmark entries={} scan_ms={:.3} sort_reconcile_ms={:.3}",
        session.entries().len(),
        scan_elapsed.as_secs_f64() * 1_000.0,
        reconcile_elapsed.as_secs_f64() * 1_000.0,
    );
}
