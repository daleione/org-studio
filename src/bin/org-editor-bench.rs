use std::{
    hint::black_box,
    path::PathBuf,
    time::{Duration, Instant},
};

use org_studio::document::{
    ByteOffset, ByteRange, DocumentBuffer, EditTransaction, TextEdit, TextSnapshot,
};

const WARMUP_EDITS: usize = 200;
const SAMPLED_EDITS: usize = 2_000;

fn main() {
    let path = std::env::args_os()
        .nth(1)
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("target/perf-fixtures/org-preview-50m.org"));
    let bytes = std::fs::read(&path)
        .unwrap_or_else(|error| panic!("failed to read {}: {error}", path.display()));
    let mut buffer = DocumentBuffer::from_utf8(bytes)
        .unwrap_or_else(|error| panic!("failed to load {}: {error}", path.display()));
    let original = buffer.snapshot();
    let original_bytes = original.len_bytes();
    let mut caret = original_bytes / 2;
    while caret > 0 && !original.is_char_boundary(org_studio::document::ByteOffset(caret)) {
        caret -= 1;
    }
    let initial_utf16 = original
        .byte_to_utf16(ByteOffset(caret))
        .expect("benchmark caret must map to UTF-16");
    assert_eq!(
        original.utf16_to_byte(initial_utf16).unwrap(),
        ByteOffset(caret)
    );

    for _ in 0..WARMUP_EDITS {
        insert(&mut buffer, &mut caret, "a");
    }
    let samples = (0..SAMPLED_EDITS)
        .map(|index| {
            let text = if index % 97 == 0 { "🙂" } else { "a" };
            let started = Instant::now();
            insert(&mut buffer, &mut caret, text);
            started.elapsed()
        })
        .collect::<Vec<_>>();

    black_box(buffer.snapshot());
    assert_eq!(
        original.len_bytes(),
        original_bytes,
        "immutable snapshot changed after edits"
    );
    print_stats(&path, original_bytes, buffer.revision().0, &samples);
}

fn insert(buffer: &mut DocumentBuffer, caret: &mut u64, text: &str) {
    buffer
        .commit(EditTransaction::new(
            buffer.revision(),
            vec![TextEdit::new(ByteRange::new(*caret, *caret), text)],
        ))
        .expect("benchmark edit must commit");
    *caret += text.len() as u64;
    let snapshot = buffer.snapshot();
    let utf16 = snapshot
        .byte_to_utf16(ByteOffset(*caret))
        .expect("edited caret must map to UTF-16");
    assert_eq!(snapshot.utf16_to_byte(utf16).unwrap(), ByteOffset(*caret));
    black_box((snapshot, utf16));
}

fn print_stats(path: &std::path::Path, bytes: u64, revision: u64, samples: &[Duration]) {
    let mut sorted = samples.to_vec();
    sorted.sort_unstable();
    let percentile = |p: f64| {
        let index = ((sorted.len() - 1) as f64 * p).ceil() as usize;
        sorted[index].as_secs_f64() * 1_000_000.0
    };
    println!(
        "org_editor_typing_core fixture={} bytes={} samples={} revision={} p50_us={:.3} p95_us={:.3} p99_us={:.3}",
        path.display(),
        bytes,
        samples.len(),
        revision,
        percentile(0.50),
        percentile(0.95),
        percentile(0.99),
    );
}
