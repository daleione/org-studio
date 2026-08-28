use super::{
    GLOBAL_VISIBILITY_CYCLE_COMMAND, GlobalVisibility, MAX_EAGER_LAYOUT_ROWS, accept_generation,
    dired_command_items, preview_input, should_eagerly_measure_rows,
};

use crate::{input::EmacsOutcome, keymap::KeyStroke};
use gpui::AppContext;

fn visible_source_lines(app: &super::PreviewApp, document: &super::PreviewDocument) -> Vec<u64> {
    app.visible_rows
        .iter()
        .map(|index| {
            document.text.line_of_byte(
                document
                    .projection
                    .rows
                    .get(*index)
                    .unwrap()
                    .source
                    .range
                    .start,
            ) + 1
        })
        .collect()
}

#[test]
fn eager_layout_is_limited_to_small_documents() {
    assert!(should_eagerly_measure_rows(MAX_EAGER_LAYOUT_ROWS));
    assert!(!should_eagerly_measure_rows(MAX_EAGER_LAYOUT_ROWS + 1));
}

fn simulate_next_frame<V: gpui::Render + 'static>(
    window: &gpui::WindowHandle<V>,
    cx: &mut gpui::TestAppContext,
) -> usize {
    let root = window.entity(cx).unwrap();
    let callback_count = cx.update(|cx| {
        cx.with_window(root.entity_id(), |window, cx| {
            window.simulate_next_frame(cx)
        })
        .unwrap()
    });
    cx.run_until_parked();
    callback_count
}

fn finish_fold_animation<V: gpui::Render + 'static>(
    window: &gpui::WindowHandle<V>,
    cx: &mut gpui::TestAppContext,
) {
    std::thread::sleep(super::LOCAL_FOLD_ANIMATION_DURATION + std::time::Duration::from_millis(20));
    assert!(simulate_next_frame(window, cx) > 0);
    assert!(simulate_next_frame(window, cx) > 0);
}

fn ready_document(app: &super::PreviewApp) -> std::sync::Arc<super::PreviewDocument> {
    match &app.state {
        super::PreviewLoadState::Ready { document, .. } => document.clone(),
        _ => panic!("document should be ready"),
    }
}

fn heading_id(
    document: &super::PreviewDocument,
    level: u16,
    ordinal: usize,
) -> crate::org_syntax::BlockId {
    document
        .blocks
        .nodes()
        .iter()
        .enumerate()
        .filter(|(_, block)| {
            matches!(block.kind, crate::org_syntax::BlockKind::Heading { level: actual } if actual == level)
        })
        .nth(ordinal)
        .map(|(index, _)| index as crate::org_syntax::BlockId)
        .expect("requested heading should exist")
}

#[test]
fn stale_generations_are_rejected() {
    assert!(accept_generation(7, 7));
    assert!(!accept_generation(8, 7));
}

#[test]
fn shift_tab_dispatches_the_global_visibility_cycle() {
    let (commands, mut keyboard, context) = preview_input();
    let expected = commands.key(GLOBAL_VISIBILITY_CYCLE_COMMAND).unwrap();
    assert_eq!(
        keyboard.route(KeyStroke::new("tab", false, false, true, false), context),
        EmacsOutcome::Command {
            command: expected,
            prefix: crate::command::PrefixArgument::None,
        }
    );
}

#[test]
fn sidebar_keymap_activates_both_sidebar_and_dired_contexts() {
    let mut app = super::PreviewApp::new();
    app.install_sidebar_keymap();
    let contexts = super::built_in_contexts();
    assert!(app.key_context.contains(contexts.key("workspace").unwrap()));
    assert!(app.key_context.contains(contexts.key("sidebar").unwrap()));
    assert!(app.key_context.contains(contexts.key("dired").unwrap()));
    assert!(!app.key_context.contains(contexts.key("preview").unwrap()));
}

#[test]
fn app_global_visibility_cycle_updates_list_fold_and_minimap_projection_together() {
    let path = std::env::temp_dir().join(format!(
        "org-studio-global-visibility-{}.org",
        std::process::id()
    ));
    std::fs::write(
        &path,
        "preamble\n* One\nbody\n** Child\nchild body\n* Two\nvisible\n",
    )
    .unwrap();
    let document = super::load_document(path.clone()).unwrap();
    let _ = std::fs::remove_file(path);
    let mut app = super::PreviewApp::new();
    app.generation = 1;
    assert!(app.apply_load_result(1, Ok(document)));

    app.cycle_global_visibility();
    assert_eq!(app.global_visibility, GlobalVisibility::Overview);
    assert_eq!(app.visible_rows.len(), 2);
    assert_eq!(app.fold_markers.len(), 2);
    assert_eq!(app.list_state.item_count(), app.visible_rows.len());

    app.cycle_global_visibility();
    assert_eq!(app.global_visibility, GlobalVisibility::Contents);
    assert_eq!(app.visible_rows.len(), 3);
    assert_eq!(app.fold_markers.len(), 2);
    assert_eq!(app.list_state.item_count(), app.visible_rows.len());

    app.cycle_global_visibility();
    assert_eq!(app.global_visibility, GlobalVisibility::All);
    assert!(app.fold_markers.is_empty());
    assert_eq!(app.visible_rows.len(), 7);
    assert_eq!(app.list_state.item_count(), app.visible_rows.len());
}

#[gpui::test]
fn shift_tab_animates_all_three_global_visibility_transitions(cx: &mut gpui::TestAppContext) {
    let path = std::env::temp_dir().join(format!(
        "org-studio-animated-global-visibility-{}.org",
        std::process::id()
    ));
    std::fs::write(
        &path,
        "preamble\n* One\nbody\n** Child\nchild body\n* Two\nvisible\n",
    )
    .unwrap();
    let document = super::load_document(path.clone()).unwrap();
    let _ = std::fs::remove_file(path);
    let window = cx.open_window(gpui::size(gpui::px(900.0), gpui::px(700.0)), |_, _| {
        super::PreviewApp::new()
    });
    window
        .update(cx, |app, _, cx| {
            app.generation = 1;
            assert!(app.apply_load_result(1, Ok(document)));
            cx.notify();
        })
        .unwrap();
    cx.run_until_parked();

    for (visibility, direction) in [
        (GlobalVisibility::Overview, super::FoldDirection::Collapse),
        (GlobalVisibility::Contents, super::FoldDirection::Expand),
        (GlobalVisibility::All, super::FoldDirection::Expand),
    ] {
        window
            .update(cx, |app, window, cx| {
                app.cycle_global_visibility_animated(window, cx);
                assert_eq!(app.global_visibility, visibility);
                let animation = app
                    .fold_animation
                    .as_ref()
                    .expect("every Shift-Tab state should animate");
                assert_eq!(animation.direction, direction);
                if visibility == GlobalVisibility::Overview {
                    assert!(animation.segments.len() > 1);
                }
                cx.notify();
            })
            .unwrap();
        cx.run_until_parked();
        assert!(simulate_next_frame(&window, cx) > 0);
        finish_fold_animation(&window, cx);
        window
            .read_with(cx, |app, _| {
                assert!(app.fold_animation.is_none());
                assert_eq!(app.list_state.item_count(), app.visible_rows.len());
            })
            .unwrap();
    }
}

#[test]
fn expanding_a_child_from_contents_does_not_collapse_its_parent() {
    let path = std::env::temp_dir().join(format!(
        "org-studio-contents-child-{}.org",
        std::process::id()
    ));
    std::fs::write(
        &path,
        "preamble\n* One\nparent body\n** Child\nchild body\n* Two\nsecond body\n",
    )
    .unwrap();
    let document = super::load_document(path.clone()).unwrap();
    let _ = std::fs::remove_file(path);
    let mut app = super::PreviewApp::new();
    app.generation = 1;
    assert!(app.apply_load_result(1, Ok(document)));
    app.cycle_global_visibility();
    app.cycle_global_visibility();

    let document = ready_document(&app);
    let parent = heading_id(&document, 1, 0);
    let child = heading_id(&document, 2, 0);

    assert!(!app.fold_markers.contains(&parent));
    assert!(app.fold_markers.contains(&child));

    app.toggle_fold(child, &document);
    assert_eq!(app.global_visibility, GlobalVisibility::Contents);
    assert!(!app.fold_markers.contains(&parent));
    assert!(!app.fold_markers.contains(&child));
    assert_eq!(visible_source_lines(&app, &document), vec![2, 4, 5, 6]);

    app.toggle_fold(child, &document);
    assert_eq!(app.global_visibility, GlobalVisibility::Contents);
    assert_eq!(app.visible_rows.len(), 3);
    assert!(!app.fold_markers.contains(&parent));
    assert!(app.fold_markers.contains(&child));

    app.cycle_global_visibility();
    assert_eq!(app.global_visibility, GlobalVisibility::Overview);
    assert_eq!(app.visible_rows.len(), 2);
}

#[test]
fn clicking_a_second_level_heading_in_contents_never_folds_its_parent() {
    let path = std::env::temp_dir().join(format!(
        "org-studio-contents-second-level-{}.org",
        std::process::id()
    ));
    std::fs::write(
        &path,
        "* Parent\nparent body\n** Child\nchild body\n*** Grandchild\ngrandchild body\n* Other\nother body\n",
    )
    .unwrap();
    let document = super::load_document(path.clone()).unwrap();
    let _ = std::fs::remove_file(path);
    let mut app = super::PreviewApp::new();
    app.generation = 1;
    assert!(app.apply_load_result(1, Ok(document)));
    app.cycle_global_visibility();
    app.cycle_global_visibility();

    let document = ready_document(&app);
    let parent = heading_id(&document, 1, 0);
    let child = heading_id(&document, 2, 0);

    assert_eq!(visible_source_lines(&app, &document), vec![1, 3, 5, 7]);
    assert!(!app.fold_markers.contains(&parent));

    app.toggle_fold(child, &document);
    assert_eq!(visible_source_lines(&app, &document), vec![1, 3, 7]);
    assert!(!app.fold_markers.contains(&parent));
    assert!(app.fold_markers.contains(&child));

    app.toggle_fold(child, &document);
    assert_eq!(visible_source_lines(&app, &document), vec![1, 3, 4, 5, 7]);
    assert!(!app.fold_markers.contains(&parent));
}

#[test]
fn heading_click_cycles_only_its_subtree_through_official_local_states() {
    let path =
        std::env::temp_dir().join(format!("org-studio-local-cycle-{}.org", std::process::id()));
    std::fs::write(
        &path,
        "* Parent\nparent body\n** Child\nchild body\n** Sibling\nsibling body\n* Other\nother body\n",
    )
    .unwrap();
    let document = super::load_document(path.clone()).unwrap();
    let _ = std::fs::remove_file(path);
    let mut app = super::PreviewApp::new();
    app.generation = 1;
    assert!(app.apply_load_result(1, Ok(document)));
    app.cycle_global_visibility();
    let document = ready_document(&app);
    let parent = heading_id(&document, 1, 0);

    app.toggle_fold(parent, &document);
    assert_eq!(
        app.local_cycle_continuation,
        Some((parent, super::LocalVisibility::Children))
    );
    assert_eq!(visible_source_lines(&app, &document), vec![1, 2, 3, 5, 7]);

    app.toggle_fold(parent, &document);
    assert_eq!(
        app.local_cycle_continuation,
        Some((parent, super::LocalVisibility::Subtree))
    );
    assert_eq!(
        visible_source_lines(&app, &document),
        vec![1, 2, 3, 4, 5, 6, 7]
    );

    app.toggle_fold(parent, &document);
    assert_eq!(
        app.local_cycle_continuation,
        Some((parent, super::LocalVisibility::Folded))
    );
    assert_eq!(visible_source_lines(&app, &document), vec![1, 7]);
}

#[gpui::test]
fn animated_collapse_inserts_one_flow_shell_and_fast_reclick_finishes_it(
    cx: &mut gpui::TestAppContext,
) {
    let path = std::env::temp_dir().join(format!(
        "org-studio-animated-local-cycle-{}.org",
        std::process::id()
    ));
    std::fs::write(
        &path,
        "* Parent\nparent body\n** Child\nchild body\n* Other\nother body\n",
    )
    .unwrap();
    let document = super::load_document(path.clone()).unwrap();
    let _ = std::fs::remove_file(path);
    let app = cx.new(|_| super::PreviewApp::new());

    app.update(cx, |app, cx| {
        app.generation = 1;
        assert!(app.apply_load_result(1, Ok(document)));
        let document = ready_document(app);
        let parent = heading_id(&document, 1, 0);

        app.toggle_fold_animated(parent, &document, 700.0, 790.0, None, cx);
        assert_eq!(visible_source_lines(app, &document), vec![1, 5, 6]);
        assert_eq!(
            app.fold_animation
                .as_ref()
                .map(|animation| animation.segments[0].transition_index),
            Some(1)
        );
        assert_eq!(app.list_state.item_count(), app.visible_rows.len() + 1);

        app.toggle_fold_animated(parent, &document, 700.0, 790.0, None, cx);
        assert_eq!(visible_source_lines(app, &document), vec![1, 2, 3, 5, 6]);
        let expansion = app.fold_animation.as_ref().unwrap();
        assert!(matches!(expansion.direction, super::FoldDirection::Expand));
        assert_eq!(expansion.segments[0].target_len, 2);
        assert_eq!(app.list_state.item_count(), app.visible_rows.len() - 1);
        app.discard_fold_animation();
    });

    app.read_with(cx, |app, _| {
        assert!(app.fold_animation.is_none());
        assert_eq!(app.list_state.item_count(), app.visible_rows.len());
    });
}

#[gpui::test]
fn reduced_motion_applies_local_fold_without_a_delayed_projection(cx: &mut gpui::TestAppContext) {
    cx.update(|cx| cx.set_reduce_motion(true));
    let path = std::env::temp_dir().join(format!(
        "org-studio-reduced-motion-fold-{}.org",
        std::process::id()
    ));
    std::fs::write(&path, "* Parent\nbody\n** Child\nchild body\n").unwrap();
    let document = super::load_document(path.clone()).unwrap();
    let _ = std::fs::remove_file(path);
    let app = cx.new(|_| super::PreviewApp::new());

    app.update(cx, |app, cx| {
        app.generation = 1;
        assert!(app.apply_load_result(1, Ok(document)));
        let document = ready_document(app);
        let parent = heading_id(&document, 1, 0);

        app.toggle_fold_animated(parent, &document, 700.0, 790.0, None, cx);
        assert_eq!(visible_source_lines(app, &document), vec![1]);
        assert!(app.fold_animation.is_none());
    });
}

#[gpui::test]
fn measured_fold_travel_renders_through_real_list_animation_frames(cx: &mut gpui::TestAppContext) {
    let path = std::env::temp_dir().join(format!(
        "org-studio-rendered-fold-travel-{}.org",
        std::process::id()
    ));
    std::fs::write(
        &path,
        "* Parent\nparent body\n** Child One\nchild one body\n** Child Two\nchild two body\n* Other\nother body\n",
    )
    .unwrap();
    let document = super::load_document(path.clone()).unwrap();
    let _ = std::fs::remove_file(path);
    let window = cx.open_window(gpui::size(gpui::px(900.0), gpui::px(700.0)), |_, _| {
        super::PreviewApp::new()
    });
    window
        .update(cx, |app, _window, cx| {
            app.generation = 1;
            assert!(app.apply_load_result(1, Ok(document)));
            cx.notify();
        })
        .unwrap();
    cx.run_until_parked();

    window
        .update(cx, |app, window, cx| {
            let document = ready_document(app);
            let parent = heading_id(&document, 1, 0);
            let first_hidden = app.list_state.bounds_for_item(1).unwrap();
            let following = app.list_state.bounds_for_item(6).unwrap();
            let expected_distance = f32::from(following.top() - first_hidden.top());

            app.toggle_fold_animated(parent, &document, 700.0, 790.0, Some(window), cx);
            let animation = app.fold_animation.as_ref().unwrap();
            let shell = &animation.segments[0];
            assert_eq!(shell.transition_index, 1);
            assert!((shell.distance - expected_distance).abs() < 0.01);
            assert_eq!(app.list_state.item_count(), app.visible_rows.len() + 1);
            cx.notify();
        })
        .unwrap();
    cx.run_until_parked();
    // Even if preparing the first Shell frame exceeds the whole nominal duration, animation
    // time starts only after that frame is presented.
    std::thread::sleep(super::LOCAL_FOLD_ANIMATION_DURATION + std::time::Duration::from_millis(20));
    assert!(simulate_next_frame(&window, cx) > 0);
    let first_gap = window
        .read_with(cx, |app, _| {
            let animation = app.fold_animation.as_ref().unwrap();
            assert_eq!(animation.progress, 0.0);
            assert!(animation.started_at.is_some());
            let heading = app.list_state.bounds_for_item(0).unwrap();
            let peer = app.list_state.bounds_for_item(2).unwrap();
            f32::from(peer.top() - heading.bottom())
        })
        .unwrap();
    std::thread::sleep(super::LOCAL_FOLD_ANIMATION_DURATION / 3);
    assert!(simulate_next_frame(&window, cx) > 0);
    let later_gap = window
        .read_with(cx, |app, _| {
            let animation = app.fold_animation.as_ref().unwrap();
            assert!(animation.progress > 0.0);
            let heading = app.list_state.bounds_for_item(0).unwrap();
            let shell = app.list_state.bounds_for_item(1).unwrap();
            let peer = app.list_state.bounds_for_item(2).unwrap();
            assert!((f32::from(shell.top() - heading.bottom())).abs() < 0.01);
            assert!((f32::from(peer.top() - shell.bottom())).abs() < 0.01);
            let gap = f32::from(peer.top() - heading.bottom());
            let expected_gap = animation.segments[0].distance * (1.0 - animation.progress);
            assert!(
                (gap - expected_gap).abs() < 1.0,
                "gap={gap} expected_gap={expected_gap} progress={}",
                animation.progress
            );
            gap
        })
        .unwrap();
    assert!(later_gap < first_gap);

    finish_fold_animation(&window, cx);
    window
        .read_with(cx, |app, _| {
            assert!(app.fold_animation.is_none());
            assert_eq!(app.list_state.item_count(), app.visible_rows.len());
        })
        .unwrap();

    window
        .update(cx, |app, window, cx| {
            let document = ready_document(app);
            let parent = heading_id(&document, 1, 0);
            app.toggle_fold_animated(parent, &document, 700.0, 790.0, Some(window), cx);
            let animation = app.fold_animation.as_ref().unwrap();
            assert!(matches!(animation.direction, super::FoldDirection::Expand));
            assert_eq!(animation.segments[0].transition_index, 1);
            assert_eq!(animation.segments[0].target_len, 3);
            assert_eq!(app.list_state.item_count(), app.visible_rows.len() - 2);
            cx.notify();
        })
        .unwrap();
    cx.run_until_parked();
    assert!(simulate_next_frame(&window, cx) > 0);
    let first_gap = window
        .read_with(cx, |app, _| {
            let animation = app.fold_animation.as_ref().unwrap();
            assert_eq!(animation.progress, 0.0);
            let heading = app.list_state.bounds_for_item(0).unwrap();
            let shell = app.list_state.bounds_for_item(1).unwrap();
            let peer = app.list_state.bounds_for_item(2).unwrap();
            assert!((f32::from(shell.top() - heading.bottom())).abs() < 0.01);
            assert!((f32::from(peer.top() - shell.bottom())).abs() < 0.01);
            f32::from(peer.top() - heading.bottom())
        })
        .unwrap();
    std::thread::sleep(super::LOCAL_FOLD_ANIMATION_DURATION / 3);
    assert!(simulate_next_frame(&window, cx) > 0);
    let later_gap = window
        .read_with(cx, |app, _| {
            let animation = app.fold_animation.as_ref().unwrap();
            assert!(animation.progress > 0.0);
            let heading = app.list_state.bounds_for_item(0).unwrap();
            let shell = app.list_state.bounds_for_item(1).unwrap();
            let peer = app.list_state.bounds_for_item(2).unwrap();
            assert!((f32::from(shell.top() - heading.bottom())).abs() < 0.01);
            assert!((f32::from(peer.top() - shell.bottom())).abs() < 0.01);
            let gap = f32::from(peer.top() - heading.bottom());
            let expected_gap = animation.segments[0].distance * animation.progress;
            assert!(
                (gap - expected_gap).abs() < 1.0,
                "gap={gap} expected_gap={expected_gap} progress={}",
                animation.progress
            );
            gap
        })
        .unwrap();
    assert!(later_gap > first_gap);

    finish_fold_animation(&window, cx);
    window
        .read_with(cx, |app, _| {
            assert!(app.fold_animation.is_none());
            assert_eq!(app.list_state.item_count(), app.visible_rows.len());
            let heading = app.list_state.bounds_for_item(0).unwrap();
            let peer = app.list_state.bounds_for_item(4).unwrap();
            assert!(peer.top() > heading.bottom());
        })
        .unwrap();

    window
        .update(cx, |app, window, cx| {
            let document = ready_document(app);
            let parent = heading_id(&document, 1, 0);
            app.toggle_fold_animated(parent, &document, 700.0, 790.0, Some(window), cx);
            let animation = app.fold_animation.as_ref().unwrap();
            assert!(matches!(animation.direction, super::FoldDirection::Expand));
            assert_eq!(animation.segments.len(), 2);
            assert_eq!(animation.segments[0].transition_index, 3);
            assert_eq!(animation.segments[1].transition_index, 5);
            assert!(animation.segments.iter().all(|shell| shell.target_len == 1));
            cx.notify();
        })
        .unwrap();
    cx.run_until_parked();
    assert!(simulate_next_frame(&window, cx) > 0);
    window
        .read_with(cx, |app, _| {
            let first_shell = app.list_state.bounds_for_item(3).unwrap();
            let second_shell = app.list_state.bounds_for_item(5).unwrap();
            assert!(f32::from(first_shell.size.height).abs() < 0.01);
            assert!(f32::from(second_shell.size.height).abs() < 0.01);
        })
        .unwrap();
    std::thread::sleep(super::LOCAL_FOLD_ANIMATION_DURATION / 3);
    assert!(simulate_next_frame(&window, cx) > 0);
    window
        .read_with(cx, |app, _| {
            let animation = app.fold_animation.as_ref().unwrap();
            assert!(animation.progress > 0.0);
            let first_child = app.list_state.bounds_for_item(2).unwrap();
            let first_shell = app.list_state.bounds_for_item(3).unwrap();
            let second_child = app.list_state.bounds_for_item(4).unwrap();
            let second_shell = app.list_state.bounds_for_item(5).unwrap();
            let peer = app.list_state.bounds_for_item(6).unwrap();
            assert!((f32::from(first_shell.top() - first_child.bottom())).abs() < 0.01);
            assert!((f32::from(second_child.top() - first_shell.bottom())).abs() < 0.01);
            assert!((f32::from(second_shell.top() - second_child.bottom())).abs() < 0.01);
            assert!((f32::from(peer.top() - second_shell.bottom())).abs() < 0.01);
            for (index, shell) in animation.segments.iter().enumerate() {
                let bounds = app
                    .list_state
                    .bounds_for_item(shell.transition_index)
                    .unwrap();
                let expected = shell.distance * animation.progress;
                assert!(
                    (f32::from(bounds.size.height) - expected).abs() < 1.0,
                    "shell {index} height did not share the expansion progress"
                );
            }
        })
        .unwrap();
    finish_fold_animation(&window, cx);
    window
        .read_with(cx, |app, _| {
            assert!(app.fold_animation.is_none());
            assert_eq!(app.list_state.item_count(), app.visible_rows.len());
            let first_child_body = app.list_state.bounds_for_item(3).unwrap();
            let second_child = app.list_state.bounds_for_item(4).unwrap();
            let second_child_body = app.list_state.bounds_for_item(5).unwrap();
            let peer = app.list_state.bounds_for_item(6).unwrap();
            assert!(first_child_body.size.height > gpui::px(0.0));
            assert!(second_child.top() >= first_child_body.bottom());
            assert!(second_child_body.size.height > gpui::px(0.0));
            assert!(peer.top() >= second_child_body.bottom());
        })
        .unwrap();
}

#[gpui::test]
fn collapse_keeps_an_offscreen_peer_behind_a_bounded_flow_shell(cx: &mut gpui::TestAppContext) {
    let path = std::env::temp_dir().join(format!(
        "org-studio-offscreen-fold-peer-{}.org",
        std::process::id()
    ));
    let mut source = String::from("* Parent\n");
    for index in 0..48 {
        source.push_str(&format!("** Child {index}\nbody {index}\n"));
    }
    source.push_str("* Other\nother body\n");
    std::fs::write(&path, source).unwrap();
    let document = super::load_document(path.clone()).unwrap();
    let _ = std::fs::remove_file(path);
    let window = cx.open_window(gpui::size(gpui::px(900.0), gpui::px(320.0)), |_, _| {
        super::PreviewApp::new()
    });
    window
        .update(cx, |app, _window, cx| {
            app.generation = 1;
            assert!(app.apply_load_result(1, Ok(document)));
            cx.notify();
        })
        .unwrap();
    cx.run_until_parked();

    window
        .update(cx, |app, window, cx| {
            let document = ready_document(app);
            let parent = heading_id(&document, 1, 0);
            let peer = heading_id(&document, 1, 1);
            let peer_row = document
                .projection
                .rows
                .iter()
                .position(|row| row.block_id == peer)
                .unwrap();
            let old_peer_position = app.visible_rows.binary_search(&peer_row).unwrap();
            assert!(
                app.list_state
                    .bounds_for_item(old_peer_position)
                    .unwrap()
                    .top()
                    > gpui::px(320.0)
            );

            app.toggle_fold_animated(parent, &document, 320.0, 790.0, Some(window), cx);
            let animation = app.fold_animation.as_ref().unwrap();
            let shell = &animation.segments[0];
            assert_eq!(shell.transition_index, 1);
            assert!(shell.distance <= 320.0);
            assert!(shell.rendered_rows.len() < 20);
            assert_eq!(app.visible_rows[1], peer_row);
            assert_eq!(app.list_state.item_count(), app.visible_rows.len() + 1);
            cx.notify();
        })
        .unwrap();
    cx.run_until_parked();
    assert!(simulate_next_frame(&window, cx) > 0);

    window
        .read_with(cx, |app, _| {
            let heading = app.list_state.bounds_for_item(0).unwrap();
            let shell = app.list_state.bounds_for_item(1).unwrap();
            let peer = app.list_state.bounds_for_item(2).unwrap();
            assert!((f32::from(shell.top() - heading.bottom())).abs() < 0.01);
            assert!((f32::from(peer.top() - shell.bottom())).abs() < 0.01);
            assert!(f32::from(peer.top() - heading.bottom()) > 24.0);
            assert!(app.fold_animation.is_some());
        })
        .unwrap();
    finish_fold_animation(&window, cx);
    window
        .read_with(cx, |app, _| {
            let heading = app.list_state.bounds_for_item(0).unwrap();
            let peer = app.list_state.bounds_for_item(1).unwrap();
            assert!((f32::from(peer.top() - heading.bottom())).abs() < 0.01);
            assert_eq!(app.list_state.item_count(), app.visible_rows.len());
            assert!(app.fold_animation.is_none());
        })
        .unwrap();
}

#[test]
fn reload_failure_retains_previous_document_and_rejects_stale_completion() {
    let path = std::env::temp_dir().join(format!(
        "org-studio-reload-state-{}.org",
        std::process::id()
    ));
    std::fs::write(&path, "* retained\n").unwrap();
    let document = super::load_document(path.clone()).unwrap();
    let _ = std::fs::remove_file(&path);

    let mut app = super::PreviewApp::new();
    app.generation = 1;
    assert!(app.apply_load_result(1, Ok(document)));
    assert_eq!(
        app.last_ready.as_ref().map(|(generation, _)| *generation),
        Some(1)
    );

    app.generation = 2;
    assert!(app.apply_load_result(2, Err((path.clone(), "reload failed".to_owned()))));
    assert!(matches!(app.state, super::PreviewLoadState::Failed { .. }));
    assert_eq!(
        app.last_ready.as_ref().map(|(generation, _)| *generation),
        Some(1)
    );

    let stale_path = path.with_extension("md");
    std::fs::write(&stale_path, "# stale\n").unwrap();
    let stale = super::load_document(stale_path.clone()).unwrap();
    let _ = std::fs::remove_file(stale_path);
    assert!(!app.apply_load_result(1, Ok(stale)));
    assert!(matches!(app.state, super::PreviewLoadState::Failed { .. }));
    assert_eq!(
        app.last_ready.as_ref().map(|(generation, _)| *generation),
        Some(1)
    );
}

#[test]
fn dired_help_lists_every_command_and_groups_alias_keys() {
    let (commands, _, _) = preview_input();
    let items = dired_command_items(&commands);
    let labels = items
        .iter()
        .map(|(keys, title)| (keys.as_ref(), title.as_ref()))
        .collect::<Vec<_>>();

    assert_eq!(items.len(), 25);
    assert!(labels.contains(&("n / j", "Next Line")));
    assert!(labels.contains(&("p / k", "Previous Line")));
    assert!(labels.contains(&("^ / h", "Up Directory")));
    assert!(labels.contains(&("RET / l", "Open")));
    assert!(labels.contains(&("H", "History Back")));
    assert!(labels.contains(&("L", "History Forward")));
    assert!(labels.contains(&("g", "Refresh Directory")));
    assert!(labels.contains(&("C-x d", "Open Dired")));
    assert!(labels.contains(&("C-x C-b", "Home")));
    assert!(labels.contains(&("C-x C-d", "Toggle Sidebar")));
    assert!(labels.contains(&("?", "Dired Help")));
    assert!(labels.contains(&("N", "Create File")));
    assert!(labels.contains(&("+", "Create Directory")));
    assert!(labels.contains(&("R", "Rename")));
    assert!(labels.contains(&("C", "Copy")));
    assert!(labels.contains(&("M", "Move")));
    assert!(labels.contains(&("D", "Move to Trash")));
    assert!(labels.contains(&("C-g", "Close command list")));
}

#[test]
fn loads_markdown_without_sending_it_through_the_org_parser() {
    let path = std::env::temp_dir().join(format!("org-studio-markdown-{}.md", std::process::id()));
    std::fs::write(&path, "# Markdown\n\n- **native** preview\n").unwrap();
    let document = super::load_document(path.clone()).unwrap();
    let _ = std::fs::remove_file(path);

    assert_eq!(document.format, super::DocumentFormat::Markdown);
    assert!(document.blocks.nodes().is_empty());
    assert!(!document.markdown_blocks.is_empty());
    assert_eq!(document.projection.rows.len(), 3);
}

#[test]
fn markdown_parent_and_minimap_share_identical_display_runs() {
    let path = std::env::temp_dir().join(format!(
        "org-studio-shared-display-runs-{}.md",
        std::process::id()
    ));
    std::fs::write(
        &path,
        "# **标题** and *italic*\n\n```rust\nlet answer = 42;\n```\n",
    )
    .unwrap();
    let document = super::load_document(path.clone()).unwrap();
    let _ = std::fs::remove_file(path);
    let display_map = document.display_map.as_ref().unwrap();
    let heading_layout = display_map.layout(0);
    assert_eq!(heading_layout.font_size, 22.0);
    assert_eq!(heading_layout.line_height, 24.0);
    let blank_layout = display_map.layout(1);
    assert_eq!(blank_layout.fixed_height, Some(24.0));

    let heading =
        super::parse_document_inline(super::DocumentFormat::Markdown, "**标题** and *italic*");
    let heading_runs = display_map.runs(0);
    assert_eq!(heading_runs.text.as_ref(), heading.text);
    assert_eq!(heading_runs.inline_spans.as_ref(), heading.spans);

    let code_row = document
        .projection
        .rows
        .iter()
        .position(|row| {
            document
                .text
                .copy_range(row.source.range)
                .contains("answer")
        })
        .expect("code row");
    assert!(!display_map.runs(code_row).code_spans.is_empty());
    let code_layout = display_map.layout(code_row);
    assert_eq!(code_layout.padding_left, 16.0);
    assert_eq!(code_layout.padding_right, 16.0);
    assert_eq!(code_layout.min_height, 24.0);
}

#[test]
fn org_parent_and_minimap_share_identical_display_runs() {
    let path = std::env::temp_dir().join(format!(
        "org-studio-shared-display-runs-{}.org",
        std::process::id()
    ));
    std::fs::write(
        &path,
        "* *粗体* and /italic/\n\n#+begin_src rust\nlet n = 7;\n#+end_src\n",
    )
    .unwrap();
    let document = super::load_document(path.clone()).unwrap();
    let _ = std::fs::remove_file(path);
    let display_map = document.display_map.as_ref().unwrap();
    let heading_layout = display_map.layout(0);
    assert_eq!(heading_layout.font_size, 22.0);
    assert_eq!(heading_layout.line_height, 24.0);
    let blank_layout = display_map.layout(1);
    assert_eq!(blank_layout.fixed_height, Some(24.0));
    let source = document
        .text
        .copy_range(document.projection.rows.get(0).unwrap().source.range)
        .trim_end_matches(['\r', '\n'])
        .to_owned();
    let expected = super::parse_document_inline(super::DocumentFormat::Org, &source);
    let heading_runs = display_map.runs(0);
    assert_eq!(heading_runs.text.as_ref(), expected.text);
    assert_eq!(heading_runs.inline_spans.as_ref(), expected.spans);

    let code_row = document
        .projection
        .rows
        .iter()
        .position(|row| document.text.copy_range(row.source.range).contains("let n"))
        .expect("code row");
    assert!(!display_map.runs(code_row).code_spans.is_empty());
    let code_layout = display_map.layout(code_row);
    assert_eq!(code_layout.padding_left, 16.0);
    assert_eq!(code_layout.padding_right, 16.0);
    assert_eq!(code_layout.line_height, 19.0);
    assert_eq!(code_layout.min_height, 24.0);
}
