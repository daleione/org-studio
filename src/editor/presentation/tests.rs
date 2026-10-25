use crate::command::OrgDisplayFeature;
use crate::document::{ByteOffset, ByteRange, DocumentSession, Selection, TextSnapshot};
use crate::editor::SemanticEditor;
use gpui::{AppContext, Entity, Modifiers, TestAppContext, VisualTestContext, px};

fn setup<'a>(
    cx: &'a mut TestAppContext,
    source: &str,
) -> (
    Entity<DocumentSession>,
    Entity<SemanticEditor>,
    &'a mut VisualTestContext,
) {
    cx.update(crate::editor::init);
    let session = cx.new(|_| {
        DocumentSession::from_utf8("presentation.org".into(), source.as_bytes().to_vec()).unwrap()
    });
    let (editor, view) = cx.add_window_view(|_, cx| SemanticEditor::new(session.clone(), cx));
    view.simulate_resize(gpui::size(px(1100.), px(800.)));
    view.run_until_parked();
    (session, editor, view)
}

fn source(session: &Entity<DocumentSession>, view: &VisualTestContext) -> String {
    view.read(|cx| {
        let snapshot = session.read(cx).snapshot();
        snapshot.copy_range(ByteRange::new(0, snapshot.len_bytes()))
    })
}

fn row_text(editor: &Entity<SemanticEditor>, line: u64, view: &VisualTestContext) -> String {
    view.read(|cx| {
        editor
            .read(cx)
            .hit_rows
            .iter()
            .find(|row| row.line.0 == line)
            .unwrap()
            .layout
            .text
            .to_string()
    })
}

#[gpui::test]
fn typing_heading_prefix_keeps_every_star_until_the_separator(cx: &mut TestAppContext) {
    use gpui::EntityInputHandler;

    let (session, editor, view) = setup(cx, "");
    for count in 1..=6 {
        view.simulate_input("*");
        view.run_until_parked();
        let expected = "*".repeat(count);
        assert_eq!(source(&session, view), expected);
        assert_eq!(row_text(&editor, 0, view), expected);
        view.update(|window, cx| {
            editor.update(cx, |editor, cx| {
                assert_eq!(editor.selection.head(), ByteOffset(count as u64));
                assert_eq!(
                    editor.selected_text_range(false, window, cx).unwrap().range,
                    count..count
                );
            })
        });
    }
    view.simulate_input(" ");
    view.run_until_parked();
    assert_eq!(source(&session, view), "****** ");
    assert_eq!(row_text(&editor, 0, view), "․․․․․○ ");
    view.simulate_keystrokes("backspace");
    view.run_until_parked();
    assert_eq!(source(&session, view), "******");
    assert_eq!(row_text(&editor, 0, view), "******");
    view.simulate_input(" ");
    view.simulate_input("Section");
    view.run_until_parked();
    assert_eq!(source(&session, view), "****** Section");
    assert_eq!(row_text(&editor, 0, view), "․․․․․○ Section");
    view.simulate_keystrokes("cmd-z");
    view.run_until_parked();
    assert_eq!(source(&session, view), "******");
    assert_eq!(row_text(&editor, 0, view), "******");
    view.simulate_keystrokes("cmd-shift-z");
    view.run_until_parked();
    assert_eq!(source(&session, view), "****** Section");
    assert_eq!(row_text(&editor, 0, view), "․․․․․○ Section");
}

#[gpui::test]
fn heading_bullets_follow_levels_without_editing_source(cx: &mut TestAppContext) {
    let original = (1..=6)
        .map(|level| format!("{} Section {level}\n", "*".repeat(level)))
        .collect::<String>();
    let (session, editor, view) = setup(cx, &original);
    for (line, bullet) in ["◉", "○", "✸", "✿", "◉", "○"].into_iter().enumerate() {
        assert_eq!(
            row_text(&editor, line as u64, view),
            format!("{}{bullet} Section {}", "․".repeat(line), line + 1)
        );
    }
    view.read(|cx| {
        let rows = editor
            .read(cx)
            .hit_rows
            .iter()
            .filter(|row| row.line.0 < 6)
            .collect::<Vec<_>>();
        assert_eq!(rows.len(), 6);
        for (line, row) in rows.iter().enumerate() {
            assert_eq!(row.text_origin_x, rows[0].text_origin_x);
            for source_index in 0..=line + 1 {
                let display_index = row.display.source_to_display(source_index);
                assert_eq!(row.display.display_to_source(display_index), source_index);
                let position = row.position_for_display_index(display_index).unwrap();
                assert_eq!(row.closest_display_index(position), display_index);
            }
        }
        for pair in rows.windows(2) {
            let bullet_x = |row: &crate::editor::HitRow| {
                let index = row.layout.text.find(['◉', '○', '✸', '✿']).unwrap();
                row.position_for_display_index(index).unwrap().x
            };
            assert!(bullet_x(pair[1]) > bullet_x(pair[0]));
        }
    });
    assert_eq!(source(&session, view), original);
}

#[gpui::test]
fn heading_bullet_edits_and_undo_follow_source_levels(cx: &mut TestAppContext) {
    let (session, editor, view) = setup(cx, "*** Section");
    assert_eq!(row_text(&editor, 0, view), "․․✸ Section");
    editor.update(view, |editor, cx| {
        editor.set_selection(Selection::caret(ByteOffset(0)), cx)
    });
    view.simulate_keystrokes("right");
    view.read(|cx| assert_eq!(editor.read(cx).selection.head(), ByteOffset(1)));
    view.simulate_keystrokes("backspace");
    view.run_until_parked();
    assert_eq!(source(&session, view), "** Section");
    assert_eq!(row_text(&editor, 0, view), "․○ Section");
    view.simulate_keystrokes("cmd-z");
    view.run_until_parked();
    assert_eq!(source(&session, view), "*** Section");
    assert_eq!(row_text(&editor, 0, view), "․․✸ Section");
    editor.update(view, |editor, cx| {
        editor.set_selection(Selection::caret(ByteOffset(3)), cx)
    });
    view.simulate_input("*");
    view.run_until_parked();
    assert_eq!(source(&session, view), "**** Section");
    assert_eq!(row_text(&editor, 0, view), "․․․✿ Section");
}

#[gpui::test]
fn links_are_projected_without_changing_literal_blocks_or_source(cx: &mut TestAppContext) {
    let original = "* Chapter [[https://example.org/guide][Guide]] [2/5]\n[[file:notes.org]]\n#+begin_src text\n[[file:literal.org][Literal]] *literal*\n#+end_src\n#+begin_example\n[[file:literal.org][Literal]] *literal*\n#+end_example\n=literal [[file:notes.org]]=\n[[][Empty]]\n# *strong* [[file:notes.org][Notes]]\n";
    let (session, editor, view) = setup(cx, original);
    assert_eq!(row_text(&editor, 0, view), "◉ Chapter Guide [2/5]");
    assert_eq!(row_text(&editor, 1, view), "file:notes.org");
    for line in [3, 6] {
        assert_eq!(
            row_text(&editor, line, view),
            "[[file:literal.org][Literal]] *literal*"
        );
    }
    assert_eq!(row_text(&editor, 8, view), "literal [[file:notes.org]]");
    assert_eq!(row_text(&editor, 9, view), "[[][Empty]]");
    assert_eq!(row_text(&editor, 10, view), "# *strong* Notes");
    assert_eq!(source(&session, view), original);
}

#[gpui::test]
fn boundary_backspace_reveals_real_source_and_bracket_repair_hides_it(cx: &mut TestAppContext) {
    let original = "[[https://example.org/guide][Guide]]";
    let (session, editor, view) = setup(cx, original);
    editor.update(view, |editor, cx| {
        editor.set_selection(Selection::caret(ByteOffset(original.len() as u64)), cx)
    });
    view.simulate_keystrokes("backspace");
    view.run_until_parked();
    assert_eq!(source(&session, view), &original[..original.len() - 1]);
    assert_eq!(row_text(&editor, 0, view), &original[..original.len() - 1]);
    view.simulate_input("]");
    view.run_until_parked();
    assert_eq!(source(&session, view), original);
    assert_eq!(row_text(&editor, 0, view), "Guide");
    view.simulate_keystrokes("cmd-z");
    view.run_until_parked();
    assert_eq!(row_text(&editor, 0, view), &original[..original.len() - 1]);
    view.simulate_keystrokes("cmd-shift-z");
    view.run_until_parked();
    assert_eq!(row_text(&editor, 0, view), "Guide");
}

#[gpui::test]
fn emphasis_defaults_hide_markers_and_follow_source_deletion_repair_and_undo(
    cx: &mut TestAppContext,
) {
    let cases = [
        ("*", "Bold"),
        ("/", "Italic"),
        ("_", "Underline"),
        ("+", "Strike"),
        ("~", "Code"),
        ("=", "Raw"),
    ];
    let lines = cases
        .iter()
        .map(|(marker, content)| format!("Before {marker}{content}{marker}"))
        .collect::<Vec<_>>();
    let original = lines.join("\n");
    let (session, editor, view) = setup(cx, &original);
    for (index, (_, content)) in cases.iter().enumerate() {
        assert_eq!(
            row_text(&editor, index as u64, view),
            format!("Before {content}")
        );
    }
    assert_eq!(source(&session, view), original);

    let mut line_start = 0;
    for (index, (marker, content)) in cases.iter().enumerate() {
        let end = line_start + lines[index].len();
        let incomplete = &lines[index][..lines[index].len() - 1];
        let mut deleted = original.clone();
        deleted.remove(end - 1);
        editor.update(view, |editor, cx| {
            editor.set_selection(Selection::caret(ByteOffset(end as u64)), cx)
        });
        view.simulate_keystrokes("backspace");
        view.run_until_parked();
        assert_eq!(source(&session, view), deleted);
        assert_eq!(row_text(&editor, index as u64, view), incomplete);

        view.simulate_input(marker);
        view.run_until_parked();
        assert_eq!(source(&session, view), original);
        assert_eq!(
            row_text(&editor, index as u64, view),
            format!("Before {content}")
        );
        view.simulate_keystrokes("cmd-z");
        view.run_until_parked();
        assert_eq!(source(&session, view), deleted);
        assert_eq!(row_text(&editor, index as u64, view), incomplete);
        view.simulate_keystrokes("cmd-shift-z");
        view.run_until_parked();
        assert_eq!(source(&session, view), original);
        assert_eq!(
            row_text(&editor, index as u64, view),
            format!("Before {content}")
        );
        line_start = end + 1;
    }
}

#[gpui::test]
fn movement_skips_hidden_addresses_and_respects_unicode_graphemes(cx: &mut TestAppContext) {
    let original = "[[https://example.org/guide][A😀B]]";
    let (session, editor, view) = setup(cx, original);
    editor.update(view, |editor, cx| {
        editor.set_selection(Selection::caret(ByteOffset(0)), cx)
    });
    view.simulate_keystrokes("right");
    let start = original.find("A😀").unwrap();
    view.read(|cx| assert_eq!(editor.read(cx).selection.head().0, (start + 1) as u64));
    view.simulate_keystrokes("right");
    view.read(|cx| {
        assert_eq!(
            editor.read(cx).selection.head().0,
            (start + "A😀".len()) as u64
        )
    });
    view.simulate_keystrokes("right");
    view.read(|cx| assert_eq!(editor.read(cx).selection.head().0, original.len() as u64));
    view.simulate_keystrokes("left");
    view.read(|cx| {
        assert_eq!(
            editor.read(cx).selection.head().0,
            (start + "A😀".len()) as u64
        )
    });
    assert_eq!(source(&session, view), original);
}

#[gpui::test]
fn horizontal_movement_leaves_hidden_line_edges_in_one_step(cx: &mut TestAppContext) {
    let original = "[[file:guide.org][A😀B]]\nnext";
    let (session, editor, view) = setup(cx, original);
    let start = original.find("A😀B").unwrap() as u64;
    let end = start + "A😀B".len() as u64;
    let line_end = original.find('\n').unwrap() as u64;
    for (key, from, target) in [
        ("left", start, 0),
        ("alt-left", start, 0),
        ("shift-left", start, 0),
        ("right", end, line_end),
        ("alt-right", end, line_end),
        ("shift-right", end, line_end),
    ] {
        editor.update(view, |editor, cx| {
            editor.set_selection(Selection::caret(ByteOffset(from)), cx)
        });
        view.simulate_keystrokes(key);
        view.read(|cx| {
            assert_eq!(
                editor.read(cx).selection.head(),
                ByteOffset(target),
                "{key}"
            )
        });
    }
    assert_eq!(source(&session, view), original);
}

#[gpui::test]
fn ime_in_visible_label_uses_source_utf16_ranges_and_one_undo_step(cx: &mut TestAppContext) {
    use gpui::EntityInputHandler;
    let original = "e\u{301}😀 [[https://example.org/guide][Guide]]";
    let (session, editor, view) = setup(cx, original);
    let label = original.find("Guide").unwrap() as u64;
    editor.update(view, |editor, cx| {
        editor.set_selection(Selection::new(ByteOffset(label), ByteOffset(label + 5)), cx)
    });
    view.update(|window, cx| {
        editor.update(cx, |editor, cx| {
            editor.replace_and_mark_text_in_range(None, "zhi", Some(3..3), window, cx)
        })
    });
    view.run_until_parked();
    assert_eq!(row_text(&editor, 0, view), "e\u{301}😀 zhi");
    view.update(|window, cx| {
        editor.update(cx, |editor, cx| {
            let snapshot = editor.snapshot(cx);
            let start = snapshot.byte_to_utf16(ByteOffset(label)).unwrap().0 as usize;
            let bounds = editor
                .bounds_for_range(start..start + 3, gpui::Bounds::default(), window, cx)
                .unwrap();
            assert!(bounds.size.width > px(0.));
            let returned = editor
                .character_index_for_point(bounds.origin, window, cx)
                .unwrap();
            assert_eq!(returned, start);
            editor.replace_text_in_range(None, "指南😀", window, cx);
        })
    });
    view.run_until_parked();
    assert_eq!(row_text(&editor, 0, view), "e\u{301}😀 指南😀");
    assert_eq!(
        source(&session, view),
        "e\u{301}😀 [[https://example.org/guide][指南😀]]"
    );
    view.simulate_keystrokes("cmd-z");
    view.run_until_parked();
    assert_eq!(source(&session, view), original);
}

#[gpui::test]
fn display_toggles_do_not_edit_source_or_share_pane_overrides(cx: &mut TestAppContext) {
    let original = "*** Chapter [[file:notes.org][Notes]] *strong* \\alpha{} [2/5]\n";
    let (session, editor, view) = setup(cx, original);
    assert_eq!(
        row_text(&editor, 0, view),
        "․․✸ Chapter Notes strong \\alpha{} [2/5]"
    );
    let initial_revision = view.read(|cx| session.read(cx).revision());
    let second = view.new(|cx| SemanticEditor::new(session.clone(), cx));
    editor.update(view, |editor, cx| {
        editor.toggle_org_display(OrgDisplayFeature::DescriptiveLinks, cx);
        editor.toggle_org_display(OrgDisplayFeature::LeadingStars, cx);
        editor.toggle_org_display(OrgDisplayFeature::EmphasisMarkers, cx);
        editor.toggle_org_display(OrgDisplayFeature::Entities, cx);
    });
    view.run_until_parked();
    assert_eq!(
        row_text(&editor, 0, view),
        "*** Chapter [[file:notes.org][Notes]] *strong* α [2/5]"
    );
    view.read(|cx| {
        assert_eq!(session.read(cx).revision(), initial_revision);
        let options = second
            .read(cx)
            .display_options(session.read(cx).syntax_path(), &session.read(cx).snapshot());
        assert!(options.descriptive_links);
        assert!(options.hide_emphasis_markers);
        assert_eq!(
            options.heading_style,
            crate::org_semantic::display::OrgHeadingStyle::Bullets
        );
    });
    editor.update(view, |editor, cx| {
        editor.toggle_org_display(OrgDisplayFeature::EmphasisMarkers, cx)
    });
    view.run_until_parked();
    assert_eq!(
        row_text(&editor, 0, view),
        "*** Chapter [[file:notes.org][Notes]] strong α [2/5]"
    );
    view.read(|cx| assert_eq!(session.read(cx).revision(), initial_revision));
    assert_eq!(source(&session, view), original);
    editor.update(view, |editor, cx| {
        editor.toggle_org_display(OrgDisplayFeature::HeadingBullets, cx)
    });
    view.run_until_parked();
    assert_eq!(
        row_text(&editor, 0, view),
        "․․✸ Chapter [[file:notes.org][Notes]] strong α [2/5]"
    );
    view.read(|cx| assert_eq!(session.read(cx).revision(), initial_revision));
}

#[gpui::test]
fn heading_bullets_respect_literal_blocks_lists_and_source_search(cx: &mut TestAppContext) {
    let original = "*** Chapter\n  * list item\n#+begin_src text\n** literal\n#+end_src\n#+begin_example\n** example\n#+end_example\n";
    let (session, editor, view) = setup(cx, original);
    assert_eq!(row_text(&editor, 0, view), "․․✸ Chapter");
    assert_eq!(row_text(&editor, 1, view), "  * list item");
    assert_eq!(row_text(&editor, 3, view), "** literal");
    assert_eq!(row_text(&editor, 6, view), "** example");
    let range = ByteRange::new(0, 2);
    editor.update(view, |editor, cx| {
        editor.search_preview(std::sync::Arc::from([range]), Some(range), cx)
    });
    view.run_until_parked();
    assert_eq!(row_text(&editor, 0, view), "*** Chapter");
    editor.update(view, |editor, cx| editor.search_finish(false, cx));
    view.run_until_parked();
    assert_eq!(row_text(&editor, 0, view), "․․✸ Chapter");
    assert_eq!(source(&session, view), original);
}

#[gpui::test]
fn heading_markers_keep_wrapping_and_hit_testing_aligned(cx: &mut TestAppContext) {
    let original = format!("**** {}\n", "sample words ".repeat(30));
    let (session, editor, view) = setup(cx, &original);
    view.simulate_resize(gpui::size(px(520.), px(800.)));
    view.run_until_parked();
    let (pretty_origin, pretty_width) = view.read(|cx| {
        let editor = editor.read(cx);
        let row = editor.hit_rows.iter().find(|row| row.line.0 == 0).unwrap();
        assert!(!row.layout.wrap_boundaries().is_empty());
        let start = row.layout.text.find("sample").unwrap();
        let first = row.position_for_display_index(start).unwrap();
        assert_eq!(row.display.display_to_source(start), 5);
        assert_eq!(row.closest_display_index(first), start);
        for boundary in row.layout.wrap_boundaries() {
            let run = &row.layout.runs()[boundary.run_ix];
            let boundary_index = run.glyphs[boundary.glyph_ix].index;
            let index = row
                .layout
                .runs()
                .iter()
                .flat_map(|run| &run.glyphs)
                .find(|glyph| glyph.index > boundary_index)
                .unwrap()
                .index;
            let position = row.position_for_display_index(index).unwrap();
            assert!(position.y > first.y);
            assert!(position.x >= px(0.0));
            assert_eq!(row.closest_display_index(position), index);
        }
        assert!(editor.minimap.active_frame_has_complete_layout());
        let minimap_layout = editor.minimap.prepared_layout().unwrap();
        assert_eq!(
            minimap_layout.line_wrap_starts(0),
            editor.display_map.line_wrap_starts(0)
        );
        assert_eq!(
            minimap_layout.line_wrap_starts(0).len(),
            row.layout.wrap_boundaries().len()
        );
        (row.text_origin_x, row.layout.wrap_width.unwrap())
    });
    editor.update(view, |editor, cx| {
        editor.toggle_org_display(OrgDisplayFeature::HeadingBullets, cx)
    });
    view.run_until_parked();
    view.read(|cx| {
        let row = editor
            .read(cx)
            .hit_rows
            .iter()
            .find(|row| row.line.0 == 0)
            .unwrap();
        assert_eq!(row.text_origin_x, pretty_origin);
        assert_eq!(row.layout.wrap_width.unwrap(), pretty_width);
    });
    assert_eq!(source(&session, view), original);
}

#[gpui::test]
fn file_display_settings_ignore_literal_directives_and_follow_undo(cx: &mut TestAppContext) {
    let original = "#+STARTUP: literallinks\n#+begin_example\n#+STARTUP: descriptivelinks\n#+end_example\n[[file:notes.org][Notes]]\n";
    let (session, editor, view) = setup(cx, original);
    assert_eq!(row_text(&editor, 4, view), "[[file:notes.org][Notes]]");
    editor.update(view, |editor, cx| {
        editor.set_selection(Selection::new(ByteOffset(11), ByteOffset(23)), cx)
    });
    view.simulate_input("descriptivelinks");
    view.run_until_parked();
    assert_eq!(row_text(&editor, 4, view), "Notes");
    view.simulate_keystrokes("cmd-z");
    view.run_until_parked();
    assert_eq!(source(&session, view), original);
    assert_eq!(row_text(&editor, 4, view), "[[file:notes.org][Notes]]");
}

#[gpui::test]
fn search_discloses_hidden_link_targets_in_prose_and_tables(cx: &mut TestAppContext) {
    let original =
        "[[https://example.org/prose][Prose]]\n| [[https://example.org/table][Table]] | value |\n";
    let (session, editor, view) = setup(cx, original);
    for (line, (offset, _)) in original.match_indices("example.org").enumerate() {
        let range = ByteRange::new(offset as u64, (offset + "example.org".len()) as u64);
        editor.update(view, |editor, cx| {
            editor.search_preview(std::sync::Arc::from([range]), Some(range), cx)
        });
        view.run_until_parked();
        view.read(|cx| {
            let editor = editor.read(cx);
            let row = editor
                .hit_rows
                .iter()
                .find(|row| row.line.0 == line as u64)
                .unwrap();
            if let Some(table) = &row.table_layout {
                assert!(
                    table
                        .fragments
                        .iter()
                        .any(|fragment| fragment.layout.text.contains("example.org/table"))
                );
            } else {
                assert!(row.layout.text.contains("example.org/prose"));
            }
        });
    }
    editor.update(view, |editor, cx| editor.search_finish(true, cx));
    view.run_until_parked();
    assert_eq!(row_text(&editor, 0, view), "Prose");
    assert_eq!(source(&session, view), original);
}

#[gpui::test]
fn timestamp_component_still_uses_source_geometry_after_a_hidden_link(cx: &mut TestAppContext) {
    let original = "[[file:plan.org][Plan]] <2026-11-03 Tue>\n";
    let (session, editor, view) = setup(cx, original);
    let position = view.read(|cx| {
        let row = &editor.read(cx).hit_rows[0];
        let local = row.layout.text.find("2026").unwrap();
        let point = row.position_for_display_index(local).unwrap();
        gpui::point(
            row.text_origin_x + point.x + px(2.),
            row.origin_y + point.y + px(7.),
        )
    });
    view.simulate_mouse_move(position, None, Modifiers::default());
    view.executor()
        .advance_clock(std::time::Duration::from_millis(400));
    view.run_until_parked();
    assert!(view.debug_bounds("timestamp-picker").is_some());
    assert_eq!(source(&session, view), original);
}
