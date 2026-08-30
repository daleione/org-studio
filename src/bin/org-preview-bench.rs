use std::{path::PathBuf, time::Instant};

use org_studio::{
    org_syntax::BlockNode,
    preview::{load_document_profiled, load_document_profiled_without_display_map},
};

fn main() {
    let paths: Vec<PathBuf> = std::env::args_os().skip(1).map(PathBuf::from).collect();
    if paths.is_empty() {
        eprintln!("usage: cargo run --release --bin org-preview-bench -- FILE.org [...]");
        std::process::exit(2);
    }

    for path in paths {
        let bytes = std::fs::metadata(&path)
            .map(|metadata| metadata.len())
            .unwrap_or(0);
        let started = Instant::now();
        let load = if std::env::var_os("ORG_STUDIO_BENCH_SKIP_DISPLAY_MAP").is_some() {
            load_document_profiled_without_display_map
        } else {
            load_document_profiled
        };
        match load(path.clone()) {
            Ok(loaded) => {
                let document = loaded.into_preview();
                let elapsed = started.elapsed();
                let mib = bytes as f64 / (1024.0 * 1024.0);
                let throughput = if elapsed.as_secs_f64() > 0.0 {
                    mib / elapsed.as_secs_f64()
                } else {
                    0.0
                };
                println!(
                    "path={} bytes={} blocks={} read_ms={:.3} rope_ms={:.3} parse_ms={:.3} display_map_ms={:.3} total_ms={:.3} throughput_mib_s={:.1} arena_node_bytes={}",
                    path.display(),
                    bytes,
                    document.blocks.nodes().len(),
                    document.metrics.read.as_secs_f64() * 1000.0,
                    document.metrics.rope.as_secs_f64() * 1000.0,
                    document.metrics.parse.as_secs_f64() * 1000.0,
                    document.metrics.display_map.as_secs_f64() * 1000.0,
                    elapsed.as_secs_f64() * 1000.0,
                    throughput,
                    std::mem::size_of::<BlockNode>(),
                );
            }
            Err((_, error)) => {
                eprintln!("path={} error={error}", path.display());
                std::process::exit(1);
            }
        }
    }
}
