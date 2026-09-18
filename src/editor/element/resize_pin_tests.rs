use crate::document::DocumentSession;
use crate::editor::SemanticEditor;
use gpui::{AppContext, px};

/// Reproduction for the "drag to the very bottom, then enlarge the window"
/// jitter: a minimap drag jumps to the end without measuring the middle of
/// the document, so those rows still carry one-line height estimates.
/// Enlarging the window pulls rows into the viewport and the total height
/// wobbles between estimate and measurement. The pinned-at-end scroll must
/// stay glued to the document bottom instead of oscillating between the
/// pin path and the anchor path.
#[gpui::test]
fn enlarging_while_pinned_at_bottom_stays_pinned(cx: &mut gpui::TestAppContext) {
    cx.update(crate::editor::init);
    let line = "lorem ipsum dolor sit amet consectetur adipiscing elit ".repeat(12);
    let source = format!("{line}\n").repeat(600);
    let session =
        cx.new(|_| DocumentSession::from_utf8("pin.md".into(), source.into_bytes()).unwrap());
    let (editor, view) = cx.add_window_view(|_, cx| SemanticEditor::new(session, cx));

    view.simulate_resize(gpui::size(px(520.), px(600.)));
    view.run_until_parked();

    // Jump to the very bottom the way a minimap thumb drag does: the middle
    // of the document is never measured.
    editor.update(view, |editor, cx| {
        let bounds = editor.minimap.bounds.expect("minimap painted");
        let bottom = bounds.bottom() - px(1.0);
        editor.seek_from_minimap(bottom, cx);
    });
    view.run_until_parked();
    let stable_height = editor.update(view, |editor, _| editor.animated_document_height());

    // Enlarge the window continuously the way a live window drag delivers
    // resizes: many size steps without letting the layout settle in
    // between, then sample the scroll every frame.
    let mut samples = Vec::new();
    for step in 0..40 {
        let width = 520.0 + step as f32 * 6.0;
        view.simulate_resize(gpui::size(px(width), px(600.)));
        editor.update(view, |editor, _| {
            let viewport_height = f32::from(editor.viewport.unwrap().size.height);
            let max_scroll = (editor.animated_document_height() - viewport_height).max(0.0);
            samples.push((
                editor.scroll_y,
                max_scroll,
                editor.animated_document_height(),
                editor.layout_reflow_pending,
            ));
        });
    }
    view.run_until_parked();
    editor.update(view, |editor, _| {
        let viewport_height = f32::from(editor.viewport.unwrap().size.height);
        let max_scroll = (editor.animated_document_height() - viewport_height).max(0.0);
        samples.push((
            editor.scroll_y,
            max_scroll,
            editor.animated_document_height(),
            editor.layout_reflow_pending,
        ));
    });
    for (index, (scroll_y, max_scroll, document_height, reflow_pending)) in
        samples.iter().enumerate()
    {
        assert!(
            *scroll_y <= max_scroll + 1.0,
            "frame {index}: scroll left the pinned end (scroll {scroll_y:.1} > max {max_scroll:.1}); samples: {samples:?}",
        );
        if *reflow_pending {
            assert!(
                (*document_height - stable_height).abs() <= 0.5,
                "frame {index}: pending reflow exposed an estimated layout height ({document_height:.1} != stable {stable_height:.1}); samples: {samples:?}",
            );
        }
    }
}
