use std::{fs, path::PathBuf};

use org_studio::{org_syntax::BlockKind, preview::load_document};

fn temp_fixture(name: &str, bytes: &[u8]) -> PathBuf {
    let path = std::env::temp_dir().join(format!("org-studio-{}-{name}", std::process::id()));
    fs::write(&path, bytes).unwrap();
    path
}

#[test]
fn supports_empty_bom_crlf_and_large_structures() {
    let empty = load_document(temp_fixture("empty.org", b"")).unwrap();
    assert_eq!(empty.session().id(), empty.preview().document_id);
    assert_eq!(empty.session().revision(), empty.preview().revision);
    let empty = empty.into_preview();
    assert!(empty.blocks.nodes().is_empty());

    let encoded = load_document(temp_fixture(
        "encoded.org",
        b"\xef\xbb\xbf* Title\r\nBody\r\n",
    ))
    .unwrap()
    .into_preview();
    assert_eq!(encoded.text.len_bytes(), 15);

    let paragraph = "x".repeat(128 * 1024);
    let long = load_document(temp_fixture("long.org", paragraph.as_bytes()))
        .unwrap()
        .into_preview();
    assert_eq!(long.blocks.nodes().len(), 1);

    let mut structures = String::new();
    for index in 0..10_000 {
        structures.push_str(&format!("* Heading {index}\n- [ ] Item {index}\n"));
    }
    structures.push_str("| wide | table |\n#+begin_src rust\nfn main() {}\n#+end_src\n");
    let large = load_document(temp_fixture("structures.org", structures.as_bytes()))
        .unwrap()
        .into_preview();
    assert!(large.blocks.nodes().len() >= 20_002);
    assert!(
        large
            .blocks
            .nodes()
            .iter()
            .any(|node| matches!(node.kind, BlockKind::TableRow))
    );
    assert!(
        large
            .blocks
            .nodes()
            .iter()
            .any(|node| matches!(node.kind, BlockKind::SourceBlock { .. }))
    );
}

#[test]
fn invalid_utf8_is_an_explicit_error() {
    let error = load_document(temp_fixture("invalid.org", &[0xff, 0xfe]))
        .err()
        .unwrap();
    assert!(error.1.contains("not valid UTF-8"));
}
