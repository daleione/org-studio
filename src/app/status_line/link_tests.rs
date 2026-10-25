use super::*;
use gpui::{AppContext, Context, Modifiers, Render, point};

struct StatusHarness {
    app: Entity<WorkspaceWindow>,
    snapshot: StatusLineSnapshot,
    width: f32,
    echo: bool,
}

impl Render for StatusHarness {
    fn render(&mut self, window: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        let layout =
            self.snapshot
                .layout_in_window(self.width, StatusLineSettings::default(), window);
        div().w(px(self.width)).child(render_status_line_content(
            &self.snapshot,
            layout,
            self.app.clone(),
            window,
            self.echo.then(|| {
                div()
                    .flex_1()
                    .child_text("Command input")
                    .into_any_element()
            }),
            false,
            None,
        ))
    }
}

#[gpui::test]
fn link_hover_preserves_status_slots_and_yields_to_messages(cx: &mut gpui::TestAppContext) {
    let app = cx.new(|_| WorkspaceWindow::with_split_layout(false));
    let (harness, view) = cx.add_window_view(|_, _| StatusHarness {
        app,
        snapshot: super::tests::snapshot(),
        width: 1400.0,
        echo: false,
    });
    let selectors = [
        "status-7-segment-0",
        "status-7-segment-2",
        "status-7-segment-6",
    ];
    for width in [1400.0, 430.0, 210.0] {
        harness.update(view, |harness, cx| {
            harness.width = width;
            harness.snapshot.link_target = None;
            cx.notify();
        });
        let before = selectors.map(|selector| view.debug_bounds(selector));
        harness.update(view, |harness, cx| {
            harness.snapshot.link_target = Some(
                format!(
                    "https://example.org/{}/guide.org#overview",
                    "long-path/".repeat(80)
                )
                .into(),
            );
            cx.notify();
        });
        let target = view.debug_bounds("status-7-link-target").unwrap();
        assert!(
            target.right() <= px(width),
            "long targets stay inside the bar"
        );
        assert!(view.debug_bounds("status-7-segment-1").is_none());
        let after = selectors.map(|selector| view.debug_bounds(selector));
        assert_eq!(before, after, "hover must not move status controls");

        harness.update(view, |harness, cx| {
            harness.snapshot.transient = Some(StatusMessage {
                text: "Could not save file".into(),
                tone: StatusTone::Error,
            });
            cx.notify();
        });
        assert!(view.debug_bounds("status-7-link-target").is_none());
        harness.update(view, |harness, cx| {
            harness.snapshot.transient = None;
            harness.echo = true;
            cx.notify();
        });
        assert!(view.debug_bounds("status-7-link-target").is_none());
        harness.update(view, |harness, cx| {
            harness.echo = false;
            harness.snapshot.link_target = None;
            cx.notify();
        });
        let restored = selectors.map(|selector| view.debug_bounds(selector));
        assert_eq!(before, restored);
    }
}

#[gpui::test]
fn reading_link_hover_reaches_its_pane_status_and_restores_on_leave(cx: &mut gpui::TestAppContext) {
    let directory = tempfile::tempdir().unwrap();
    for (name, source, second_row, table) in [
        (
            "guide.org",
            "[[https://example.org/reference][Reference]]\n\n[[#details][Details]]\n\nPlain text\n",
            2,
            false,
        ),
        (
            "guide.md",
            "[Reference](https://example.org/reference \"Reference title\")\n\n[Details](#details)\n\nPlain text\n",
            2,
            false,
        ),
        (
            "table.org",
            "| [[https://example.org/reference][Reference]] |\n| [[#details][Details]] |\n| Plain text |\n",
            1,
            true,
        ),
        (
            "table.md",
            "| [Reference](https://example.org/reference \"Reference title\") |\n|---|\n| [Details](#details) |\n| Plain text |\n",
            2,
            true,
        ),
    ] {
        let path = directory.path().join(name);
        std::fs::write(&path, source).unwrap();
        let loaded = crate::preview::load_document(path).unwrap();
        let (workspace, view) = cx.add_window_view(|_, _| WorkspaceWindow::with_split_layout(true));
        view.simulate_resize(gpui::size(px(1100.0), px(700.0)));
        workspace.update(view, |workspace, cx| {
            workspace.minimap_visible = false;
            workspace.generation = 1;
            assert!(workspace.apply_load_result(1, Ok(loaded), cx));
            workspace.toggle_pane_surface(crate::app::PaneSide::Left, cx);
            cx.notify();
        });
        view.update(|window, cx| window.simulate_next_frame(cx));
        view.run_until_parked();
        let panel = view.read(|cx| {
            workspace
                .read(cx)
                .reading_panel_for(crate::app::PaneSide::Left)
                .unwrap()
        });
        let position_for = |index, cx: &gpui::App| {
            let row = panel.read(cx).list_state().bounds_for_item(index).unwrap();
            let style = preview_style(workspace.read(cx).reading_style);
            let frame_width =
                crate::preview::layout::reading_frame_width(f32::from(row.size.width), 0.0, *style);
            point(
                row.center().x - px(frame_width * 0.5)
                    + px(style.spacing.horizontal_padding * 0.5
                        + 6.0
                        + if table {
                            style.spacing.table_cell_x + 1.0
                        } else {
                            0.0
                        }),
                row.center().y,
            )
        };
        let position = view.read(|cx| position_for(0, cx));
        view.simulate_mouse_move(position, None, Modifiers::default());
        view.run_until_parked();
        view.read(|cx| {
            let workspace = workspace.read(cx);
            assert_eq!(
                workspace
                    .document_status_snapshot(crate::app::PaneSide::Left, cx)
                    .unwrap()
                    .link_target
                    .as_deref(),
                Some("https://example.org/reference"),
                "{name}"
            );
            assert!(
                workspace
                    .document_status_snapshot(crate::app::PaneSide::Right, cx)
                    .unwrap()
                    .link_target
                    .is_none()
            );
        });
        assert!(
            view.debug_bounds("status-1-link-target").is_some(),
            "hovering must immediately invalidate the rendered status"
        );
        let second_position = view.read(|cx| position_for(second_row, cx));
        for (position, expected) in [
            (second_position, "#details"),
            (position, "https://example.org/reference"),
        ] {
            view.simulate_mouse_move(position, None, Modifiers::default());
            view.run_until_parked();
            view.read(|cx| {
                assert_eq!(
                    panel.read(cx).hovered_link_target().as_deref(),
                    Some(expected),
                    "switching between rows must update the target immediately"
                )
            });
        }
        view.simulate_mouse_move(
            point(position.x + px(250.0), position.y),
            None,
            Modifiers::default(),
        );
        view.run_until_parked();
        assert!(view.debug_bounds("status-1-link-target").is_none());
        view.simulate_mouse_move(position, None, Modifiers::default());
        view.run_until_parked();
        assert!(view.debug_bounds("status-1-link-target").is_some());
        view.simulate_event(gpui::MouseDownEvent {
            position,
            button: gpui::MouseButton::Left,
            click_count: 2,
            ..Default::default()
        });
        view.simulate_event(gpui::MouseUpEvent {
            position,
            button: gpui::MouseButton::Left,
            click_count: 2,
            ..Default::default()
        });
        assert_eq!(
            view.opened_url().as_deref(),
            Some("https://example.org/reference")
        );
        view.simulate_resize(gpui::size(px(1000.0), px(700.0)));
        assert!(view.debug_bounds("status-1-link-target").is_none());
        let position = view.read(|cx| position_for(0, cx));
        view.simulate_mouse_move(position, None, Modifiers::default());
        view.run_until_parked();
        assert!(view.debug_bounds("status-1-link-target").is_some());
        view.simulate_mouse_move(point(px(2.0), px(2.0)), None, Modifiers::default());
        view.run_until_parked();
        assert!(view.debug_bounds("status-1-link-target").is_none());
        view.read(|cx| assert!(panel.read(cx).hovered_link_target().is_none()));
    }
}
