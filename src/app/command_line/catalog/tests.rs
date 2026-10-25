use super::*;

fn entry(input: &str, labels: [&str; 2], aliases: &[&str]) -> Entry {
    Entry {
        key: CommandKey::from_index(0),
        input: input.into(),
        description: String::new(),
        scope: None,
        names: labels
            .into_iter()
            .chain(aliases.iter().copied())
            .map(str::to_owned)
            .collect(),
        qualified_input: "example.actions.open".into(),
    }
}

fn inputs(candidates: &[Candidate]) -> Vec<&str> {
    candidates.iter().map(|c| c.entry.input.as_str()).collect()
}

#[test]
fn visible_names_precede_aliases_and_internal_namespaces_do_not_match_fragments() {
    let mut stored = entry("store", ["", ""], &[]);
    stored.qualified_input = "note-app.actions.store".into();
    let all = [
        stored,
        entry("open", ["", ""], &["note-visit"]),
        entry("note-switch-outline", ["", ""], &[]),
        entry("note-switch-grid", ["", ""], &[]),
    ];
    let found = candidates(&all, "note", &["store".into(), "open".into()]);
    assert_eq!(
        inputs(&found),
        ["note-switch-grid", "note-switch-outline", "open"]
    );
    assert!(candidates(&all, "actions", &[]).is_empty());
    let found = candidates(&all, "note-app.actions.store", &[]);
    assert_eq!(inputs(&found), ["store"]);
    assert!(matches!(
        &found[0].alternative_match,
        Some(TextMatch { text, range })
            if text == "note-app.actions.store" && *range == (0..text.len())
    ));
}

#[test]
fn contiguous_matches_rank_exact_prefix_then_earlier_and_shorter_substrings() {
    let all = [
        "xx-open",
        "reopen",
        "open-notebook",
        "open",
        "open-note",
        "xopen",
        "open-file",
    ]
    .map(|input| entry(input, ["", ""], &[]));
    let history = ["open-notebook".into(), "open-file".into()];
    let found = candidates(&all, ":OpEn", &history);
    assert_eq!(
        inputs(&found),
        [
            "open",
            "open-file",
            "open-note",
            "open-notebook",
            "xopen",
            "reopen",
            "xx-open"
        ]
    );
    assert!(candidates(&all, "opn", &[]).is_empty());
    assert!(candidates(&all, "open note", &[]).is_empty());
    let recent = candidates(&all, "", &history);
    assert_eq!(
        &inputs(&recent)[..3],
        ["open-notebook", "open-file", "xx-open"]
    );
    assert!(
        recent
            .iter()
            .all(|candidate| candidate.input_match.is_none()
                && candidate.alternative_match.is_none())
    );
}

#[test]
fn matching_keeps_display_ranges_for_command_names_chinese_and_aliases() {
    let all = [entry(
        "open-note",
        ["打开笔记", "Open note"],
        &["visit-note"],
    )];
    let found = candidates(&all, "OPEN", &[]);
    assert_eq!(found[0].input_match, Some(0..4));
    assert!(found[0].alternative_match.is_none());
    let found = candidates(&all, "笔记", &[]);
    assert!(matches!(
        &found[0].alternative_match,
        Some(TextMatch { text, range })
            if text == "打开笔记" && &text[range.clone()] == "笔记"
    ));
    let found = candidates(&all, "VISIT", &[]);
    assert!(matches!(
        &found[0].alternative_match,
        Some(TextMatch { text, range })
            if text == "visit-note" && *range == (0..5)
    ));
    assert!(candidates(&all, "笔记 Open", &[]).is_empty());
    let all = [entry("open-note", ["", ""], &["note-open"])];
    let found = candidates(&all, "note", &[]);
    assert_eq!(found[0].input_match, Some(5..9));
    assert!(matches!(
        &found[0].alternative_match,
        Some(TextMatch { text, range }) if text == "note-open" && *range == (0..4)
    ));
}
