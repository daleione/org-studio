use std::path::PathBuf;

use org_studio::{org_syntax::BlockKind, preview::load_document};

#[test]
fn loads_representative_preview_fixture() {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/preview-basics.org");
    let document = load_document(path).expect("fixture should load");
    let nodes = document.blocks.nodes();

    assert!(nodes.len() > 10);
    assert!(
        nodes
            .iter()
            .any(|node| matches!(node.kind, BlockKind::Heading { level: 1 }))
    );
    assert!(
        nodes
            .iter()
            .any(|node| matches!(node.kind, BlockKind::ListItem))
    );
    assert!(
        nodes
            .iter()
            .any(|node| matches!(node.kind, BlockKind::TableRow))
    );
    assert!(
        nodes
            .iter()
            .any(|node| matches!(node.kind, BlockKind::SourceBlock { .. }))
    );
}
