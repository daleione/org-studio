use std::{
    hint::black_box,
    path::PathBuf,
    time::{Duration, Instant},
};

use org_studio::{
    org_syntax::{BlockKind, inline},
    preview::load_document,
};

const VISIBLE_BLOCKS: usize = 32;
const SAMPLES: usize = 2_000;

fn main() {
    let path = std::env::args_os()
        .nth(1)
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("/private/tmp/org-preview-50m.org"));
    let loaded = load_document(path.clone())
        .unwrap_or_else(|(_, error)| panic!("{}: {error}", path.display()));
    let document = loaded.into_preview();
    let nodes = document.blocks.nodes();
    assert!(!nodes.is_empty(), "benchmark document must contain blocks");

    let mut warm = Vec::with_capacity(nodes.len().min(4096));
    for block in nodes.iter().take(4096) {
        let source = document.text.copy_range(block.content);
        warm.push(inline::parse(&source));
    }

    let cold = sample(|| {
        let start = next_start(nodes.len());
        for offset in 0..VISIBLE_BLOCKS {
            let block = &nodes[(start + offset) % nodes.len()];
            let source = document.text.copy_range(block.content);
            if matches!(
                block.kind,
                BlockKind::Heading { .. } | BlockKind::Paragraph | BlockKind::ListItem
            ) {
                black_box(inline::parse(black_box(&source)));
            } else {
                black_box(source);
            }
        }
    });

    let warm_path = sample(|| {
        let start = next_start(warm.len());
        for offset in 0..VISIBLE_BLOCKS {
            black_box(warm[(start + offset) % warm.len()].clone());
        }
    });

    print_stats("cold_visible_pipeline", &cold);
    print_stats("warm_inline_cache", &warm_path);
}

fn next_start(len: usize) -> usize {
    static CURSOR: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
    CURSOR.fetch_add(VISIBLE_BLOCKS, std::sync::atomic::Ordering::Relaxed) % len
}

fn sample(mut work: impl FnMut()) -> Vec<Duration> {
    for _ in 0..100 {
        work();
    }
    (0..SAMPLES)
        .map(|_| {
            let start = Instant::now();
            work();
            start.elapsed()
        })
        .collect()
}

fn print_stats(name: &str, samples: &[Duration]) {
    let mut sorted = samples.to_vec();
    sorted.sort_unstable();
    let percentile = |p: f64| {
        let index = ((sorted.len() - 1) as f64 * p).ceil() as usize;
        sorted[index].as_secs_f64() * 1000.0
    };
    println!(
        "org_preview_cpu path={} samples={} visible_blocks={} p50_ms={:.3} p95_ms={:.3} p99_ms={:.3}",
        name,
        samples.len(),
        VISIBLE_BLOCKS,
        percentile(0.50),
        percentile(0.95),
        percentile(0.99),
    );
}
