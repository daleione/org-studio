use super::*;
use gpui::{Modifiers, VisualTestContext, point, size};
use std::time::Duration;

fn document_window<'a>(
    cx: &'a mut gpui::TestAppContext,
    extension: &str,
    language: Language,
) -> (Entity<WorkspaceWindow>, &'a mut VisualTestContext) {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join(format!("guide.{extension}"));
    let source = if extension == "org" {
        "* Guide\nA short paragraph.\n"
    } else {
        "# Guide\nA short paragraph.\n"
    };
    std::fs::write(&path, source).unwrap();
    let loaded = crate::preview::load_document(path).unwrap();
    let (workspace, cx) = cx.add_window_view(|_, _| WorkspaceWindow::with_split_layout(false));
    cx.simulate_resize(size(px(1100.0), px(700.0)));
    workspace.update(cx, |workspace, cx| {
        workspace.language = language;
        workspace.generation = 1;
        assert!(workspace.apply_load_result(1, Ok(loaded), cx));
        cx.notify();
    });
    cx.update(|window, cx| window.simulate_next_frame(cx));
    cx.run_until_parked();
    (workspace, cx)
}

fn advance(cx: &mut VisualTestContext, milliseconds: u64) {
    cx.run_until_parked();
    cx.executor()
        .advance_clock(Duration::from_millis(milliseconds));
    cx.run_until_parked();
}

#[gpui::test]
fn statistics_hover_waits_and_remains_open_while_entering_the_panel(cx: &mut gpui::TestAppContext) {
    let (_, cx) = document_window(cx, "org", Language::English);
    let trigger = cx.debug_bounds("status-1-segment-4").unwrap();
    assert_eq!(f32::from(trigger.size.width), STATISTICS_FULL_WIDTH);
    let outside = point(px(600.0), px(100.0));

    cx.simulate_mouse_move(trigger.center(), None, Modifiers::default());
    advance(cx, 150);
    assert!(cx.debug_bounds("status-popover").is_none());
    cx.simulate_mouse_move(outside, None, Modifiers::default());
    advance(cx, 350);
    assert!(cx.debug_bounds("status-popover").is_none());

    cx.simulate_mouse_move(trigger.center(), None, Modifiers::default());
    advance(cx, 301);
    let panel = cx.debug_bounds("status-popover").unwrap();
    assert!((f32::from(panel.right() - trigger.right())).abs() <= 2.0);
    assert!(panel.bottom() < trigger.origin.y);

    let gap = point(trigger.center().x, panel.bottom() + px(4.0));
    cx.simulate_mouse_move(gap, None, Modifiers::default());
    advance(cx, 100);
    cx.simulate_mouse_move(panel.center(), None, Modifiers::default());
    advance(cx, 350);
    assert!(cx.debug_bounds("status-popover").is_some());

    cx.simulate_mouse_move(outside, None, Modifiers::default());
    advance(cx, 199);
    assert!(cx.debug_bounds("status-popover").is_some());
    advance(cx, 2);
    assert!(cx.debug_bounds("status-popover").is_none());
}

#[gpui::test]
fn statistics_click_pins_and_dismisses_the_same_panel_in_both_surfaces(
    cx: &mut gpui::TestAppContext,
) {
    for (extension, language) in [("org", Language::English), ("md", Language::Chinese)] {
        let (workspace, cx) = document_window(cx, extension, language);
        let mut expected_statistics = None;
        for surface in [
            crate::app::PaneSurface::Editor,
            crate::app::PaneSurface::Reading,
        ] {
            workspace.update(cx, |workspace, cx| {
                workspace.set_active_surface(surface, cx);
            });
            cx.update(|window, cx| window.simulate_next_frame(cx));
            cx.run_until_parked();
            let statistics = cx.read(|cx| {
                workspace
                    .read(cx)
                    .status_snapshot(cx)
                    .unwrap()
                    .document_statistics
            });
            if let Some(expected) = expected_statistics {
                assert_eq!(statistics, Some(expected));
            } else {
                expected_statistics = statistics;
            }
            let trigger = cx.debug_bounds("status-1-segment-4").unwrap();
            let outside = point(px(600.0), px(100.0));
            cx.simulate_mouse_move(outside, None, Modifiers::default());
            cx.simulate_mouse_move(trigger.center(), None, Modifiers::default());
            advance(cx, 301);
            let before = cx.debug_bounds("status-popover").unwrap();
            cx.simulate_click(trigger.center(), Modifiers::default());
            assert_eq!(cx.debug_bounds("status-popover"), Some(before));
            cx.simulate_mouse_move(outside, None, Modifiers::default());
            advance(cx, 500);
            assert!(cx.debug_bounds("status-popover").is_some());

            cx.simulate_click(trigger.center(), Modifiers::default());
            assert!(cx.debug_bounds("status-popover").is_none());
            cx.simulate_click(trigger.center(), Modifiers::default());
            assert!(cx.debug_bounds("status-popover").is_some());
            cx.simulate_keystrokes("escape");
            assert!(cx.debug_bounds("status-popover").is_none());
            cx.read(|cx| {
                assert_eq!(
                    workspace.read(cx).document_workspace.active_surface(),
                    surface
                );
            });

            cx.simulate_click(trigger.center(), Modifiers::default());
            assert!(cx.debug_bounds("status-popover").is_some());
            cx.simulate_click(outside, Modifiers::default());
            advance(cx, 500);
            assert!(cx.debug_bounds("status-popover").is_none());
        }
    }
}

#[gpui::test]
fn pending_statistics_hover_cannot_replace_another_status_panel(cx: &mut gpui::TestAppContext) {
    let (workspace, cx) = document_window(cx, "org", Language::English);
    let trigger = cx.debug_bounds("status-1-segment-4").unwrap();
    cx.simulate_mouse_move(trigger.center(), None, Modifiers::default());
    advance(cx, 150);
    let outline = cx.debug_bounds("status-1-segment-1").unwrap();
    cx.simulate_click(outline.center(), Modifiers::default());
    advance(cx, 500);
    cx.read(|cx| {
        assert!(matches!(
            workspace
                .read(cx)
                .status
                .popover
                .as_ref()
                .map(|popover| &popover.content),
            Some(StatusPopoverContent::Outline { .. })
        ));
    });
    cx.simulate_keystrokes("escape");
    assert!(cx.debug_bounds("status-popover").is_none());
}
