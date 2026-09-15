use crate::{
    app::{PaneSide, WorkspaceWindow, native_input::NativeInput},
    document::ByteRange,
    search::CaseMode,
};
use gpui::EntityInputHandler;
use gpui::{Modifiers, prelude::*, px};
use std::sync::Arc;

#[gpui::test]
fn cancelling_a_prefix_restores_search_input_and_query(cx: &mut gpui::TestAppContext) {
    cx.update(crate::editor::init);
    cx.update(|cx| cx.set_reduce_motion(true));
    let (w, cx) = cx.add_window_view(|_, _| WorkspaceWindow::with_split_layout(false));
    w.update(cx, |w, cx| w.create_buffer("notes.org".into(), None, cx));
    cx.run_until_parked();
    cx.simulate_keystrokes("cmd-f");
    cx.simulate_keystrokes("a b");
    cx.run_until_parked();
    let search = cx.debug_bounds("floating-status-line").unwrap();
    cx.simulate_keystrokes("ctrl-x");
    cx.executor()
        .advance_clock(std::time::Duration::from_millis(401));
    cx.run_until_parked();
    assert!(cx.debug_bounds("prefix-hint-items").is_some());
    cx.simulate_keystrokes("ctrl-g");
    cx.run_until_parked();
    assert_eq!(cx.debug_bounds("floating-status-line").unwrap(), search);
    cx.simulate_keystrokes("c");
    w.update(cx, |w, cx| {
        assert!(!w.prefix_hint_visible());
        assert_eq!(
            w.search.session.as_ref().unwrap().input.read(cx).text,
            "abc"
        );
        assert!(!w.document_session().unwrap().read(cx).is_dirty());
    });
    cx.simulate_keystrokes("escape ctrl-x");
    cx.executor()
        .advance_clock(std::time::Duration::from_millis(401));
    cx.run_until_parked();
    assert!(cx.debug_bounds("prefix-hint-items").is_some());
    cx.simulate_keystrokes("cmd-f");
    cx.run_until_parked();
    w.update(cx, |w, _| {
        assert!(w.search_is_open());
        assert!(!w.prefix_hint_visible());
        assert!(w.keyboard.pending_keys().is_none());
    });
}

#[gpui::test]
fn escape_closes_transient_inputs_before_leaving_fullscreen(cx: &mut gpui::TestAppContext) {
    cx.update(crate::editor::init);
    cx.update(|cx| {
        cx.set_reduce_motion(true);
        cx.intercept_keystrokes(WorkspaceWindow::intercept_fullscreen_escape)
            .detach();
    });
    let (workspace, cx) = cx.add_window_view(|_, _| WorkspaceWindow::with_split_layout(false));
    workspace.update(cx, |w, cx| w.create_buffer("全屏搜索.org".into(), None, cx));
    cx.update(|window, _| window.toggle_fullscreen());
    cx.run_until_parked();

    for (incremental, replacing) in [(false, false), (false, true), (true, false)] {
        workspace.update(cx, |w, cx| {
            w.open_search(incremental, false, false, cx);
            if replacing {
                w.search_toggle_replacement(cx);
            }
        });
        cx.run_until_parked();
        cx.simulate_keystrokes("escape");
        cx.run_until_parked();
        workspace.update(cx, |w, _| assert!(!w.search_is_open()));
        cx.update(|window, _| assert!(window.is_fullscreen()));
    }

    workspace.update(cx, |w, cx| {
        w.open_buffer_picker(crate::app::buffers::PickerIntent::Switch, cx)
    });
    cx.run_until_parked();
    cx.simulate_keystrokes("escape");
    cx.run_until_parked();
    workspace.update(cx, |w, _| assert!(w.buffers.panel.is_none()));
    cx.update(|window, _| assert!(window.is_fullscreen()));

    cx.simulate_keystrokes("escape");
    cx.run_until_parked();
    cx.update(|window, _| assert!(!window.is_fullscreen()));
}

#[gpui::test]
fn search_window_shortcuts_keep_typing_out_of_document(cx: &mut gpui::TestAppContext) {
    cx.update(crate::editor::init);
    cx.update(|cx| cx.set_reduce_motion(true));
    let session = cx.new(|_| {
        crate::document::DocumentSession::from_utf8(
            "/tmp/search-window.org".into(),
            b"alpha alpha\n".to_vec(),
        )
        .unwrap()
    });
    let (workspace, cx) = cx.add_window_view(|_, _| WorkspaceWindow::with_split_layout(false));
    workspace.update(cx, |w, cx| {
        w.state = crate::app::WorkspaceLoadState::Ready {
            document: crate::app::ReadyDocument {
                session: session.clone(),
                editor_syntax: Arc::default(),
                editors: crate::app::PanePair {
                    left: None,
                    right: None,
                },
                readers: crate::app::PanePair {
                    left: None,
                    right: None,
                },
            },
        };
        w.ensure_editor_for(PaneSide::Left, cx);
        w.request_document_focus(cx);
        cx.notify();
    });
    cx.run_until_parked();
    let status = cx.debug_bounds("floating-status-line").unwrap();
    cx.simulate_keystrokes("cmd-f");
    cx.run_until_parked();
    let initial_language = workspace.update(cx, |w, _| w.language);
    for language in [
        crate::i18n::Language::English,
        crate::i18n::Language::Chinese,
    ] {
        workspace.update(cx, |w, cx| {
            w.language = language;
            cx.notify();
        });
        cx.run_until_parked();
        let count = cx.debug_bounds("search-result-count").unwrap();
        workspace.update(cx, |w, cx| {
            let s = w.search.session.as_ref().unwrap();
            let input = s.input.read(cx);
            assert!(
                input.content_width > 0.,
                "empty inputs must measure their placeholder"
            );
            assert!(
                f32::from(input.painted_bounds().unwrap().size.width) >= input.content_width,
                "the complete query placeholder must fit in {language:?}"
            );
        });
        cx.update(|window, _| {
            for label in [
                language.text("search.scope_full"),
                language.text("search.scope_selection"),
                language.text("search.scope_anchor"),
                language.text("search.scanning"),
                language.text("search.not_found"),
                "999 / 999",
                "100,000+",
            ] {
                let run = gpui::TextRun {
                    len: label.len(),
                    font: gpui::font(".SystemUIFont"),
                    color: gpui::rgb(0x60718c).into(),
                    background_color: None,
                    underline: None,
                    strikethrough: None,
                };
                let line = window
                    .text_system()
                    .shape_line(label.into(), px(12.), &[run], None);
                assert!(
                    count.size.width >= line.width,
                    "{label:?} must fit in {language:?}: text {:?}, slot {:?}",
                    line.width,
                    count.size.width
                );
            }
        });
        let empty_shell = cx.debug_bounds("floating-status-line").unwrap();
        let empty_input = cx.debug_bounds("document-search-input").unwrap();
        let next = cx.debug_bounds("search-next").unwrap();
        for query in ["a", "alpha", "missing", ""] {
            cx.simulate_keystrokes("cmd-a backspace");
            if !query.is_empty() {
                cx.simulate_input(query);
            }
            cx.run_until_parked();
            assert_eq!(
                cx.debug_bounds("floating-status-line").unwrap(),
                empty_shell,
                "short searches must not resize the shell in {language:?}"
            );
            assert_eq!(
                cx.debug_bounds("document-search-input").unwrap(),
                empty_input,
                "scope, results and no matches must leave the input fixed in {language:?}"
            );
            assert_eq!(cx.debug_bounds("search-result-count").unwrap(), count);
            assert_eq!(cx.debug_bounds("search-next").unwrap(), next);
        }
        let replace = cx.debug_bounds("search-mode-replace").unwrap();
        cx.simulate_click(replace.center(), Modifiers::default());
        cx.run_until_parked();
        workspace.update(cx, |w, cx| {
            let s = w.search.session.as_ref().unwrap();
            let input = s.replacement_input.read(cx);
            assert!(
                f32::from(input.painted_bounds().unwrap().size.width) >= input.content_width,
                "the complete replacement placeholder must fit in {language:?}"
            );
        });
        let find = cx.debug_bounds("search-mode-find").unwrap();
        cx.simulate_click(find.center(), Modifiers::default());
        cx.run_until_parked();
    }
    workspace.update(cx, |w, cx| {
        w.language = initial_language;
        cx.notify();
    });
    cx.run_until_parked();
    cx.simulate_keystrokes("a l p h a");
    cx.run_until_parked();
    workspace.update(cx, |w, _| {
        let s = w.search.session.as_ref().expect("Cmd-F opens search");
        assert_eq!(s.query.pattern, "alpha");
        assert_eq!(s.results.as_ref().unwrap().matches.len(), 2);
    });
    assert!(!cx.read(|cx| session.read(cx).is_dirty()));
    let collapsed = cx
        .debug_bounds("floating-status-line")
        .expect("one floating search shell");
    assert_eq!(f32::from(collapsed.size.height), 42.);
    assert_eq!(f32::from(collapsed.size.width), 480.);
    assert_eq!(collapsed.center().x, status.center().x);
    assert_eq!(collapsed.bottom(), status.bottom());
    let field = cx
        .debug_bounds("document-search-input")
        .expect("visible query input");
    assert!(f32::from(field.size.width) > 100.);
    assert_eq!(f32::from(field.size.height), 30.);
    cx.simulate_keystrokes("cmd-a");
    cx.simulate_input(&"搜索内容".repeat(20));
    cx.run_until_parked();
    let long_query = cx.debug_bounds("floating-status-line").unwrap();
    assert!(long_query.size.width > collapsed.size.width);
    assert!(f32::from(long_query.size.width) <= 960.);
    assert!(long_query.size.width <= status.size.width);
    assert_eq!(long_query.center(), collapsed.center());
    assert_eq!(long_query.size.height, collapsed.size.height);
    assert!(cx.debug_bounds("search-close").unwrap().right() <= long_query.right());
    cx.simulate_keystrokes("cmd-a");
    cx.simulate_input("不存在的内容");
    cx.run_until_parked();
    workspace.update(cx, |w, _| {
        assert!(w.search.session.as_ref().unwrap().failed())
    });
    assert_eq!(
        cx.debug_bounds("floating-status-line").unwrap(),
        collapsed,
        "no results must not add a row"
    );
    cx.simulate_keystrokes("cmd-a");
    cx.simulate_input("alpha");
    cx.run_until_parked();
    let more = cx.debug_bounds("search-more").unwrap();
    cx.simulate_click(more.center(), Modifiers::default());
    cx.run_until_parked();
    assert_eq!(
        cx.debug_bounds("floating-status-line").unwrap(),
        collapsed,
        "options must not resize search"
    );
    let menu = cx
        .debug_bounds("search-options-menu")
        .expect("anchored options menu");
    assert!(menu.bottom() < collapsed.top());
    let option = cx.debug_bounds("search-option-case-sensitive").unwrap();
    cx.simulate_click(option.center(), Modifiers::default());
    cx.run_until_parked();
    assert!(cx.debug_bounds("search-options-menu").is_none());
    workspace.update(cx, |w, _| {
        assert_eq!(
            w.search.session.as_ref().unwrap().query.case,
            CaseMode::Sensitive
        )
    });
    let more = cx.debug_bounds("search-more").unwrap();
    cx.simulate_click(more.center(), Modifiers::default());
    cx.run_until_parked();
    cx.simulate_keystrokes("escape");
    cx.run_until_parked();
    assert!(cx.debug_bounds("search-options-menu").is_none());
    workspace.update(cx, |w, _| assert!(w.search_is_open()));
    workspace.update(cx, |w, cx| {
        let s = w.search.session.as_ref().unwrap();
        let painted = s
            .input
            .read(cx)
            .painted_bounds()
            .expect("native text canvas painted");
        assert!(f32::from(painted.size.width) > 80.);
        assert_eq!(f32::from(painted.size.height), 22.);
    });
    let toggle = cx.debug_bounds("search-mode-replace").unwrap();
    cx.simulate_click(toggle.center(), Modifiers::default());
    cx.run_until_parked();
    let expanded = cx.debug_bounds("floating-status-line").unwrap();
    assert_eq!(expanded.bottom(), collapsed.bottom());
    assert_eq!(f32::from(expanded.size.height), 83.);
    let selected_mode = cx.debug_bounds("search-mode-replace").unwrap();
    cx.simulate_click(selected_mode.center(), Modifiers::default());
    cx.run_until_parked();
    assert_eq!(
        cx.debug_bounds("floating-status-line").unwrap(),
        expanded,
        "selecting the active mode must not toggle it off"
    );
    cx.simulate_input(&"替换内容".repeat(20));
    cx.run_until_parked();
    let long_replacement = cx.debug_bounds("floating-status-line").unwrap();
    assert!(long_replacement.size.width > expanded.size.width);
    assert_eq!(long_replacement.center(), expanded.center());
    assert_eq!(long_replacement.size.height, expanded.size.height);
    let query = cx.debug_bounds("document-search-input").unwrap();
    let replacement = cx.debug_bounds("document-replacement-input").unwrap();
    assert_eq!(query.left(), replacement.left());
    assert_eq!(query.right(), replacement.right());
    cx.simulate_keystrokes("cmd-a");
    cx.simulate_input("替换内容");
    cx.run_until_parked();
    let short_replacement = cx.debug_bounds("floating-status-line").unwrap();
    assert!(short_replacement.size.width <= expanded.size.width);
    assert_eq!(short_replacement.size.height, expanded.size.height);
    assert_eq!(short_replacement.bottom(), expanded.bottom());
    let expanded = short_replacement;
    workspace.update(cx, |w, _| {
        assert_eq!(
            w.search.session.as_ref().unwrap().replacement.as_str(),
            "替换内容"
        )
    });
    let toggle = cx.debug_bounds("search-mode-find").unwrap();
    cx.simulate_click(toggle.center(), Modifiers::default());
    cx.run_until_parked();
    assert_eq!(cx.debug_bounds("floating-status-line").unwrap(), collapsed);
    assert!(cx.debug_bounds("document-replacement-input").is_none());
    let toggle = cx.debug_bounds("search-mode-replace").unwrap();
    cx.simulate_click(toggle.center(), Modifiers::default());
    cx.run_until_parked();
    workspace.update(cx, |w, _| {
        assert_eq!(
            w.search.session.as_ref().unwrap().replacement.as_str(),
            "替换内容"
        )
    });
    cx.simulate_keystrokes("cmd-f");
    cx.run_until_parked();
    cx.simulate_keystrokes("cmd-a");
    cx.simulate_input("不存在的内容");
    cx.run_until_parked();
    assert_eq!(
        cx.debug_bounds("floating-status-line").unwrap(),
        expanded,
        "replacement mode also keeps its height when no match is found"
    );
    cx.simulate_keystrokes("cmd-a");
    cx.simulate_input("alpha");
    cx.run_until_parked();
    let viewport = cx.update(|window, _| window.viewport_size());
    let original_language = workspace.update(cx, |w, _| w.language);
    for language in [
        crate::i18n::Language::Chinese,
        crate::i18n::Language::English,
    ] {
        workspace.update(cx, |w, cx| {
            w.language = language;
            cx.notify();
        });
        for width in [360., 500., 900.] {
            cx.simulate_resize(gpui::size(px(width), px(600.)));
            cx.run_until_parked();
            let shell = cx.debug_bounds("floating-status-line").unwrap();
            let query = cx.debug_bounds("document-search-input").unwrap();
            let replacement = cx.debug_bounds("document-replacement-input").unwrap();
            assert!(
                f32::from(query.size.width) > 30.,
                "query must remain readable at {width}px"
            );
            assert!(replacement.right() <= shell.right());
            if width >= 500. {
                assert_eq!(query.left(), replacement.left());
                assert_eq!(
                    query.right(),
                    replacement.right(),
                    "input columns must align"
                );
            }
            assert!(cx.debug_bounds("search-replace-all").unwrap().right() <= shell.right());
            assert!(cx.debug_bounds("search-close").unwrap().right() <= shell.right());
        }
        workspace.update(cx, |w, cx| {
            let s = w.search.session.as_ref().unwrap();
            assert_eq!(
                s.input.read(cx).config.placeholder.as_ref(),
                language.text("search.query_placeholder")
            );
            assert_eq!(
                s.replacement_input.read(cx).config.placeholder.as_ref(),
                language.text("search.replacement_placeholder")
            );
            assert_eq!(s.query.pattern, "alpha");
        });
    }
    workspace.update(cx, |w, cx| {
        w.language = original_language;
        cx.notify();
    });
    cx.simulate_resize(viewport);
    cx.run_until_parked();
    cx.simulate_keystrokes("enter enter");
    cx.run_until_parked();
    workspace.update(cx, |w, _| {
        let s = w.search.session.as_ref().unwrap();
        assert_eq!(s.current, Some(ByteRange::new(0, 5)));
        assert_eq!(
            s.notice,
            super::notice::Notice::None,
            "wrapping should be silent"
        );
    });
    cx.simulate_keystrokes("enter escape");
    cx.run_until_parked();
    workspace.update(cx, |w, cx| {
        assert!(!w.search_is_open());
        assert_eq!(
            w.keyboard.status(),
            None,
            "closing search should restore a quiet status line"
        );
        assert_eq!(
            w.editor(PaneSide::Left)
                .unwrap()
                .read(cx)
                .selection()
                .range(),
            ByteRange::new(6, 11)
        );
    });
    cx.simulate_keystrokes("cmd-f");
    cx.run_until_parked();
    cx.update(|_, cx| cx.set_reduce_motion(false));
    workspace.update(cx, |w, cx| {
        w.close_search(false, cx);
        let p = w.search.presentation.as_mut().unwrap();
        let available = p.available_width;
        let current = w.status.sample_shell(available, std::time::Instant::now());
        let midway = super::geometry::ShellShape {
            width: (480. + available) / 2.,
            content_opacity: 0.5,
            ..current
        };
        w.status
            .set_shell_shape_for_test(midway, available, std::time::Instant::now(), false);
        w.status.set_shell_shape_for_test(
            super::geometry::ShellShape::status(available),
            available,
            std::time::Instant::now() + std::time::Duration::from_secs(60),
            true,
        );
    });
    cx.run_until_parked();
    assert!(cx.debug_bounds("search-returning-status-content").is_some());
    let returning = cx.debug_bounds("floating-status-line").unwrap();
    assert!(returning.size.width > collapsed.size.width);
    assert!(returning.size.width < status.size.width);
    assert_eq!(returning.bottom(), status.bottom());
    workspace.update(cx, |w, cx| {
        assert!(!w.search_is_open());
        let p = w.search.presentation.as_mut().unwrap();
        w.status.set_shell_shape_for_test(
            super::geometry::ShellShape::status(p.available_width),
            p.available_width,
            std::time::Instant::now(),
            false,
        );
        cx.notify();
    });
    cx.run_until_parked();
    assert!(cx.debug_bounds("search-returning-status-content").is_none());
    assert_eq!(cx.debug_bounds("floating-status-line").unwrap(), status);
}
#[gpui::test]
fn search_native_composition_is_not_a_query_until_commit(cx: &mut gpui::TestAppContext) {
    let (w, d) = super::tests::setup(cx, "中文 中文\n🙂");
    w.update(cx, |w, cx| w.open_search(true, false, false, cx));
    let input = w.update(cx, |w, _| w.search.session.as_ref().unwrap().input.clone());
    let (_, cx) = cx.add_window_view(|_, cx| NativeInput::new(Default::default(), cx));
    cx.update(|window, app| {
        input.update(app, |i, cx| {
            i.replace_and_mark_text_in_range(None, "中", Some(1..1), window, cx)
        });
    });
    cx.run_until_parked();
    w.update(cx, |w, _| {
        assert!(w.search.session.as_ref().unwrap().query.pattern.is_empty())
    });
    cx.update(|window, app| {
        input.update(app, |i, cx| {
            i.replace_text_in_range(None, "中文", window, cx)
        });
    });
    cx.run_until_parked();
    w.update(cx, |w, _| {
        let s = w.search.session.as_ref().unwrap();
        assert_eq!(s.query.pattern, "中文");
        assert_eq!(s.steps.len(), 1);
        assert_eq!(s.results.as_ref().unwrap().matches.len(), 2);
    });
    assert!(!cx.read(|cx| d.read(cx).is_dirty()));
}
