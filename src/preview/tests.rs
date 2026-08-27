use super::{
    GLOBAL_VISIBILITY_CYCLE_COMMAND, GlobalVisibility, accept_generation, dired_command_items,
    preview_input,
};

use crate::{input::EmacsOutcome, keymap::KeyStroke};

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

    let document = match &app.state {
        super::PreviewLoadState::Ready { document, .. } => document.clone(),
        _ => panic!("document should be ready"),
    };
    let parent = document
        .blocks
        .nodes()
        .iter()
        .position(|block| {
            matches!(
                block.kind,
                crate::org_syntax::BlockKind::Heading { level: 1 }
            )
        })
        .unwrap() as u32;
    let child = document
        .blocks
        .nodes()
        .iter()
        .position(|block| {
            matches!(
                block.kind,
                crate::org_syntax::BlockKind::Heading { level: 2 }
            )
        })
        .unwrap() as u32;

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

    let document = match &app.state {
        super::PreviewLoadState::Ready { document, .. } => document.clone(),
        _ => panic!("document should be ready"),
    };
    let parent = document
        .blocks
        .nodes()
        .iter()
        .position(|block| {
            matches!(
                block.kind,
                crate::org_syntax::BlockKind::Heading { level: 1 }
            )
        })
        .unwrap() as u32;
    let child = document
        .blocks
        .nodes()
        .iter()
        .position(|block| {
            matches!(
                block.kind,
                crate::org_syntax::BlockKind::Heading { level: 2 }
            )
        })
        .unwrap() as u32;

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
    let document = match &app.state {
        super::PreviewLoadState::Ready { document, .. } => document.clone(),
        _ => panic!("document should be ready"),
    };
    let parent = document
        .blocks
        .nodes()
        .iter()
        .position(|block| {
            matches!(
                block.kind,
                crate::org_syntax::BlockKind::Heading { level: 1 }
            )
        })
        .unwrap() as u32;

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

    assert_eq!(items.len(), 19);
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
