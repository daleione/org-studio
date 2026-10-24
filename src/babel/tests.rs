use super::*;
use std::{fs, sync::atomic::Ordering};

fn snapshot(source: &str) -> DocumentSnapshot {
    DocumentSnapshot::from_utf8(source.as_bytes().to_vec()).unwrap()
}

fn mysql_request(source: &str) -> Result<BabelExecutionRequest, String> {
    prepare_source_block_execution(
        &snapshot(source),
        Path::new("/tmp/mysql-notes.org"),
        Selection::caret(ByteOffset(source.rfind("SELECT").unwrap_or(0) as u64)),
    )
}

#[test]
fn header_tokenization_preserves_quoted_values_and_trailing_backslashes() {
    for (input, expected) in [
        (":dbpassword \"\"", ""),
        (":dbpassword \":secret\"", ":secret"),
        (":dbpassword 'secret\\path'", "secret\\path"),
        (":dbpassword secret\\", "secret\\"),
        (":dbpassword secret\\ value", "secret value"),
    ] {
        let mut headers = crate::babel::headers::Headers::default();
        headers.parse(input).unwrap();
        assert_eq!(headers.get(":dbpassword"), Some(expected));
        assert_eq!(
            crate::babel::syntax::tokenize_header(input).unwrap(),
            [":dbpassword", expected]
        );
    }
    assert!(crate::babel::syntax::tokenize_header(":file \"unfinished").is_err());
}

#[test]
fn mysql_inherits_file_subtree_and_affiliated_arguments() {
    let source = "#+PROPERTY: header-args:sql :engine mysql :dbhost file-host :results table replace\n\
* Reports\n:PROPERTIES:\n:header-args:sql:\t:database reports :dbuser reader :dbhost subtree-host\n:END:\n\
#+name: report\n#+HEADER: :dbport 3307\n#+CAPTION: Query\n#+ATTR_ORG: :width 42\n#+begin_src sql :dbhost block-host :dbpassword \":quoted secret\"\nSELECT 1;\n#+end_src\n";
    let request = mysql_request(source).unwrap();
    assert_eq!(request.block_name(), Some("report"));
    let mysql = request.mysql.unwrap();
    let target = mysql.target().unwrap();
    assert_eq!(target.user, "reader");
    assert_eq!(target.host, "block-host");
    assert_eq!(target.port, 3307);
    assert_eq!(target.database, "reports");
    assert_eq!(mysql.headers.get(":dbpassword"), Some(":quoted secret"));

    let separated = source.replace("#+begin_src", "\n#+begin_src");
    let request = mysql_request(&separated).unwrap();
    assert!(request.block_name().is_none());
    let mysql = request.mysql.unwrap();
    assert!(mysql.headers.get(":dbport").is_none());
}

#[test]
fn mysql_respects_effective_eval_and_rejects_unsupported_headers() {
    let denied = "#+PROPERTY: header-args:sql :eval never\n#+begin_src sql\nSELECT 1;\n#+end_src\n";
    assert!(mysql_request(denied).err().unwrap().contains(":eval"));
    let allowed = denied.replace("#+begin_src sql", "#+begin_src sql :eval yes");
    mysql_request(&allowed).unwrap();
    for args in [
        ":engine postgresql",
        ":results append",
        ":results raw",
        ":session shared",
        ":cache yes",
    ] {
        let source = format!("#+begin_src sql {args}\nSELECT 1;\n#+end_src\n");
        assert!(mysql_request(&source).is_err(), "{args}");
    }
    let source = "#+begin_src sql :results none\nSELECT 1;\n#+end_src\n";
    let request = mysql_request(source).unwrap();
    assert!(!request.writes_result());
    assert!(matches!(
        request.result,
        RequestedResult::Data { silent: true, .. }
    ));
}

#[test]
fn mysql_ignores_example_properties_and_sibling_subtrees() {
    let source = "#+begin_example\n#+PROPERTY: header-args:sql :eval never\n#+end_example\n\
* Previous\n:PROPERTIES:\n:header-args:sql: :eval never\n:END:\n\
* Current\n#+begin_src sql :engine mysql\nSELECT 1;\n#+end_src\n";
    mysql_request(source).unwrap();
}

#[test]
fn mysql_result_replacement_consumes_entire_table_and_preserves_prose() {
    let source = "#+name: report\n#+begin_src sql\nSELECT 1;\n#+end_src\n\n\
Explanation.\n\n#+RESULTS[oldhash]: report\n#+CAPTION: My report\n#+PLOT: ind:1 deps:(2) with:lines\n| a | b |\n|---+---|\n| 1 | 2 |\n| 3 | 4 |\n\nFollowing paragraph.\n* Next\n";
    let request = mysql_request(source).unwrap();
    let RequestedResult::Data {
        range, attributes, ..
    } = request.result
    else {
        panic!("expected table")
    };
    assert_eq!(
        attributes,
        "#+CAPTION: My report\n#+PLOT: ind:1 deps:(2) with:lines\n"
    );
    let text = snapshot(source);
    let old = text.copy_range(range);
    assert!(old.starts_with("#+RESULTS[oldhash]: report"));
    assert!(old.contains("| 3 | 4 |"));
    assert!(!old.contains("Explanation"));
    assert!(!old.contains("Following"));
    assert!(
        text.copy_range(ByteRange::new(range.end.0, text.len_bytes()))
            .starts_with("\nFollowing")
    );
}

#[test]
fn mysql_rejects_ambiguous_names_and_results_before_connecting() {
    for source in [
        "#+name: report\n#+begin_src sql\nSELECT 1;\n#+end_src\n#+name: report\n#+begin_src sql\nSELECT 2;\n#+end_src\n",
        "#+name: report\n#+begin_src sql\nSELECT 1;\n#+end_src\n#+RESULTS: report\n| 1 |\n\n#+RESULTS: report\n| 2 |\n",
        "#+name: report\n#+begin_src sql\nSELECT 1;\n#+end_src\n#+RESULTS: other\n| 1 |\n",
        "#+begin_src sql\nSELECT 1;\n#+end_src\n#+RESULTS:\n[[file:old.csv]]\n",
    ] {
        assert!(mysql_request(source).is_err());
    }
}

#[gpui::test]
fn sql_result_refresh_keeps_the_visible_caret_in_place(cx: &mut gpui::TestAppContext) {
    use crate::document::{DocumentCommand, DocumentSession, EditOrigin, EditTransaction};
    use gpui::AppContext;

    cx.update(crate::editor::init);
    let payload = "| Date       | Value |\n|------------+-------|\n| 2025-01-01 |     2 |\n| 2025-01-02 |     4 |\n| 2025-01-03 |     6 |\n";
    for at_end in [false, true] {
        let prefix = "Introductory text\n".repeat(80);
        let suffix = if at_end {
            String::new()
        } else {
            format!("\nAfter the chart\n{}", "Following text\n".repeat(80))
        };
        let source = format!(
            "{prefix}#+NAME: measurements_query\n#+begin_src sql :results table replace\nSELECT measured_on, value FROM measurements;\n#+end_src\n\n#+RESULTS: measurements_query\n#+NAME: measurements_data\n#+PLOT: title:\"Measurements\" ind:1 deps:(2) timeind:1 timefmt:\"%Y-%m-%d\" with:linespoints\n{payload}{suffix}"
        );
        let sql = ByteOffset(source.find("SELECT ").unwrap() as u64);
        let caret = if at_end {
            sql
        } else {
            ByteOffset(source.find("After the chart").unwrap() as u64)
        };
        let session = cx.new(|_| {
            DocumentSession::from_utf8("measurements.org".into(), source.into_bytes()).unwrap()
        });
        let (editor, view) =
            cx.add_window_view(|_, cx| crate::editor::SemanticEditor::new(session.clone(), cx));
        view.simulate_resize(gpui::size(gpui::px(1100.0), gpui::px(650.0)));
        view.run_until_parked();
        view.executor()
            .advance_clock(std::time::Duration::from_millis(81));
        view.run_until_parked();
        editor.update(view, |editor, cx| {
            editor.set_selection(Selection::caret(caret), cx);
            let target = if at_end {
                ByteOffset(session.read(cx).snapshot().len_bytes())
            } else {
                sql
            };
            assert!(editor.scroll_to_source_offset(target, cx));
        });
        view.run_until_parked();
        let before = editor.read_with(view, |editor, cx| {
            (
                editor.painted_caret().unwrap().bounds,
                editor.top_source_anchor(&session.read(cx).snapshot()),
            )
        });
        for value in [8, 9, 2] {
            let snapshot = session.read_with(view, |session, _| session.snapshot());
            let request = prepare_source_block_execution(
                &snapshot,
                Path::new("/tmp/measurements.org"),
                Selection::caret(sql),
            )
            .unwrap();
            let edit = request
                .data_result_edit(&payload.replace("     2", &format!("     {value}")))
                .unwrap();
            session.update(view, |session, cx| {
                session
                    .edit(
                        DocumentCommand::new(
                            EditTransaction::new(snapshot.revision(), vec![edit]),
                            Selection::caret(caret),
                            Selection::caret(caret),
                            EditOrigin::Other,
                        ),
                        cx,
                    )
                    .unwrap();
            });
            for indexed in [false, true] {
                if indexed {
                    view.executor()
                        .advance_clock(std::time::Duration::from_millis(81));
                }
                view.run_until_parked();
                editor.read_with(view, |editor, cx| {
                    assert_eq!(editor.selection(), Selection::caret(caret));
                    assert_eq!(
                        editor.painted_caret().unwrap().bounds,
                        before.0,
                        "the visible caret must not move, at_end={at_end}, indexed={indexed}"
                    );
                    assert_eq!(
                        editor.top_source_anchor(&session.read(cx).snapshot()),
                        before.1,
                        "the viewport must not move while SQL results refresh"
                    );
                });
            }
        }
    }
}

#[gpui::test]
fn sql_result_refresh_preserves_the_plot_and_independent_summary(cx: &mut gpui::TestAppContext) {
    use crate::document::{DocumentCommand, DocumentSession, EditOrigin, EditTransaction};
    use gpui::AppContext;

    let original = include_str!("../../tests/fixtures/sql-report-summary.org");
    let path = Path::new("/tmp/measurements.org");
    let session = cx.new(|_| {
        DocumentSession::from_utf8(path.to_path_buf(), original.as_bytes().to_vec()).unwrap()
    });
    let (editor, cx) =
        cx.add_window_view(|_, cx| crate::editor::SemanticEditor::new(session.clone(), cx));
    let rows = [
        ["2025-02-01", "8", "1.50", "12"],
        ["2025-02-02", "10", "2.25", "16"],
    ];
    for (count, expected) in [(1, ["8", "1.50", "12"]), (2, ["18", "3.75", "28"])] {
        let snapshot = session.read_with(cx, |session, _| session.snapshot());
        let before = snapshot.copy_range(ByteRange::new(0, snapshot.len_bytes()));
        let old_summary = before.split_once("* Summary\n").unwrap().1.to_owned();
        let request = prepare_source_block_execution(
            &snapshot,
            path,
            Selection::caret(ByteOffset(before.find("SELECT ").unwrap() as u64)),
        )
        .unwrap();
        assert_eq!(request.block_name(), Some("measurements_query"));
        let RequestedResult::Data {
            range,
            attributes,
            newline,
            ..
        } = &request.result
        else {
            panic!("SQL produces a table result");
        };
        assert!(attributes.contains("#+NAME: measurements_data\n"));
        assert!(attributes.contains("#+PLOT:"));
        assert!(!snapshot.copy_range(*range).contains("* Summary"));
        let columns = ["Date", "Value", "Weight", "Samples"].map(str::to_owned);
        let values = rows[..count]
            .iter()
            .map(|row| row.iter().map(|value| Some((*value).to_owned())).collect())
            .collect::<Vec<_>>();
        // Exercise the SQL result boundary and table serializer without a database.
        let payload = crate::babel::result::serialize_table(
            &columns,
            &values,
            &[false, true, true, true],
            true,
            newline,
            10_000,
        )
        .unwrap();
        let edit = request.data_result_edit(&payload).unwrap();
        session.update(cx, |session, cx| {
            session
                .edit(
                    DocumentCommand::new(
                        EditTransaction::new(snapshot.revision(), vec![edit]),
                        Selection::default(),
                        Selection::default(),
                        EditOrigin::Other,
                    ),
                    cx,
                )
                .unwrap();
        });
        cx.run_until_parked();
        let snapshot = session.read_with(cx, |session, _| session.snapshot());
        let refreshed = snapshot.copy_range(ByteRange::new(0, snapshot.len_bytes()));
        assert_eq!(refreshed.split_once("* Summary\n").unwrap().1, old_summary);
        assert_eq!(refreshed.matches("#+NAME: measurements_data").count(), 1);
        assert_eq!(refreshed.matches("#+PLOT:").count(), 1);
        let repeated = prepare_source_block_execution(
            &snapshot,
            path,
            Selection::caret(ByteOffset(refreshed.find("SELECT ").unwrap() as u64)),
        )
        .unwrap();
        assert!(repeated.data_result_edit(&payload).is_none());
        let plots = crate::plot::collect_plots(&snapshot, &crate::org_syntax::parse(&snapshot));
        assert_eq!(plots.len(), 1);
        assert!(matches!(crate::plot::prepare_plot(&snapshot, &plots[0]),
            crate::plot::PlotPreparation::Ready(plot) if plot.plan.rows == count));

        editor.update(cx, |editor, cx| {
            editor.set_selection(
                Selection::caret(ByteOffset(refreshed.find("#+TBLFM:").unwrap() as u64)),
                cx,
            );
            assert!(editor.recalculate_table_at_selection(true, cx).unwrap());
        });
        cx.run_until_parked();
        let calculated = session.read_with(cx, |session, _| {
            let snapshot = session.snapshot();
            snapshot.copy_range(ByteRange::new(0, snapshot.len_bytes()))
        });
        let summary = calculated
            .lines()
            .find(|line| line.contains("Total"))
            .unwrap();
        let parsed =
            crate::document::table::parse_line(summary, crate::document::DocumentFormat::Org);
        for (cell, expected) in parsed.cells.iter().skip(1).zip(expected) {
            assert_eq!(&summary[cell.text_range.clone()], expected);
        }
        session.update(cx, |session, cx| {
            session.undo(cx).unwrap();
        });
        cx.run_until_parked();
        session.read_with(cx, |session, _| {
            let snapshot = session.snapshot();
            assert_eq!(
                snapshot.copy_range(ByteRange::new(0, snapshot.len_bytes())),
                refreshed
            );
        });
        session.update(cx, |session, cx| {
            session.redo(cx).unwrap();
        });
        cx.run_until_parked();
    }
}

#[test]
fn mysql_named_connections_follow_heading_scope() {
    let source = "\
:PROPERTIES:
:DB_DRAWER: mysql://reader@drawer.example/drawer_db
:END:
#+PROPERTY: DB_FILE mysql://reader@file.example/file_db
#+PROPERTY: header-args:sql :dbconnection DB_A
* Queries
:PROPERTIES:
:DB_A: mysql://reader@a.example/db_a
:DB_B: mysql://reader@b.example/db_b
:END:
#+begin_src sql
SELECT 'first';
#+end_src
#+begin_src sql :dbconnection db_b
SELECT 'second';
#+end_src
** Local override
:PROPERTIES:
:db_a: mysql://reader@child.example/child_db
:END:
#+begin_src sql
SELECT 'child';
#+end_src
** Sibling
#+begin_src sql
SELECT 'sibling';
#+end_src
* File defaults
#+begin_src sql :dbconnection DB_FILE
SELECT 'file';
#+end_src
#+begin_src sql :dbconnection DB_DRAWER
SELECT 'drawer';
#+end_src
";
    for (query, host, database) in [
        ("SELECT 'first'", "a.example", "db_a"),
        ("SELECT 'second'", "b.example", "db_b"),
        ("SELECT 'child'", "child.example", "child_db"),
        ("SELECT 'sibling'", "a.example", "db_a"),
        ("SELECT 'file'", "file.example", "file_db"),
        ("SELECT 'drawer'", "drawer.example", "drawer_db"),
    ] {
        let request = prepare_source_block_execution(
            &snapshot(source),
            Path::new("/tmp/mysql-notes.org"),
            Selection::caret(ByteOffset(source.find(query).unwrap() as u64)),
        )
        .unwrap();
        let target = request.mysql.unwrap().target().unwrap();
        assert_eq!(
            (target.host.as_str(), target.database.as_str()),
            (host, database)
        );
    }
}

#[test]
fn mysql_named_connections_reject_missing_or_invalid_properties() {
    for properties in [
        "",
        "* Other\n:PROPERTIES:\n:DB_A: mysql://reader@other.example/db\n:END:\n* Current\n",
        "#+begin_example\n#+PROPERTY: DB_A mysql://reader@example.test/db\n:PROPERTIES:\n:DB_A: mysql://reader@example.test/db\n:END:\n#+end_example\n",
        "#+PROPERTY: DB_A mysql://reader@file.example/db\n* Current\n:PROPERTIES:\n:DB_A:\n:END:\n",
        "#+PROPERTY: DB_A mysql://reader@file.example/db\n#+PROPERTY: DB_A\n",
        "#+PROPERTY: DB_A mysql://reader:secret-password@db.example:bad/db\n",
    ] {
        let source =
            format!("{properties}#+begin_src sql :dbconnection DB_A\nSELECT 1;\n#+end_src\n");
        let error = mysql_request(&source)
            .err()
            .expect("connection must be rejected");
        assert!(error.contains("connection"), "{error}");
        assert!(!error.contains("secret-password"), "{error}");
    }
}

#[test]
fn mysql_connections_inherit_and_explicit_fields_override_them() {
    let source = "#+PROPERTY: header-args:sql :dbconnection mysql://reader:p%40ss@db.example:3307/reports :eval query\n#+begin_src sql -n :results table replace :dbhost block.example :database metrics :exports both\nSELECT 1;\n#+end_src\n";
    let named = format!(
        "#+PROPERTY: REPORTS mysql://reader:p%40ss@db.example:3307/reports\n{}",
        source.replace("mysql://reader:p%40ss@db.example:3307/reports", "REPORTS")
    );
    let mut identities = Vec::new();
    for source in [source, named.as_str()] {
        let request = mysql_request(source).unwrap();
        let mysql = request.mysql.unwrap();
        assert!(mysql.ready());
        let target = mysql.target().unwrap();
        assert_eq!(target.user, "reader");
        assert_eq!(target.host, "block.example");
        assert_eq!(target.port, 3307);
        assert_eq!(target.database, "metrics");
        assert!(mysql.ask_each_time);
        assert!(!mysql.identity().contains("p%40ss"));
        assert!(!mysql.identity().contains("p@ss"));
        identities.push(mysql.identity());
    }
    assert_eq!(identities[0], identities[1]);
}

#[test]
fn plantuml_requires_an_explicit_output_file() {
    let text = snapshot("#+begin_src plantuml\nA -> B\n#+end_src\n");
    let error = prepare_source_block_execution(
        &text,
        Path::new("notes.org"),
        Selection::caret(ByteOffset(24)),
    )
    .err()
    .unwrap();
    assert!(error.contains(":file"));
}

#[test]
fn mermaid_requires_an_explicit_output_file() {
    let source = "#+begin_src mermaid\nflowchart TB; A --> B\n#+end_src\n";
    let text = snapshot(source);
    let error = prepare_source_block_execution(
        &text,
        Path::new("notes.org"),
        Selection::caret(ByteOffset(source.find("flowchart").unwrap() as u64)),
    )
    .err()
    .unwrap();
    assert!(error.contains(":file"), "{error}");
    assert!(error.contains("Mermaid"), "{error}");
}

#[test]
fn mermaid_execution_publishes_svg_and_reports_the_language() {
    use crate::document::{DocumentBuffer, EditTransaction};

    let unique = NEXT_TEMP_OUTPUT.fetch_add(1, Ordering::Relaxed);
    let root = std::env::temp_dir().join(format!(
        "org-studio-mermaid-babel-test-{}-{unique}",
        std::process::id()
    ));
    fs::create_dir(&root).unwrap();
    let document_path = root.join("notes.org");
    let source =
        "#+begin_src mermaid :file images/flow.svg\nflowchart TB; A[Start] --> B[End]\n#+end_src\n";
    let mut buffer = DocumentBuffer::from_utf8(source.as_bytes().to_vec()).unwrap();
    let before = buffer.snapshot();
    let request = prepare_source_block_execution(
        &before,
        &document_path,
        Selection::caret(ByteOffset(source.find("flowchart").unwrap() as u64)),
    )
    .unwrap();
    assert_eq!(request.language_name(), "Mermaid");

    let mut output = execute_source_block(request).unwrap().publish().unwrap();
    assert_eq!(output.language_name, "Mermaid");
    let edit = output.result_edit.take().unwrap();
    buffer
        .commit(EditTransaction::new(before.revision(), vec![edit]))
        .unwrap();

    let svg = fs::read(root.join("images/flow.svg")).unwrap();
    assert!(String::from_utf8_lossy(&svg).contains("<svg"));
    assert!(
        buffer
            .snapshot()
            .copy_range(ByteRange::new(0, buffer.snapshot().len_bytes()))
            .contains("#+RESULTS:\n[[file:images/flow.svg]]")
    );

    fs::remove_file(root.join("images/flow.svg")).unwrap();
    fs::remove_dir(root.join("images")).unwrap();
    fs::remove_dir(root).unwrap();
}

#[test]
fn typst_requires_an_explicit_supported_output_file() {
    let source = "#+begin_src typst\nHello\n#+end_src\n";
    let text = snapshot(source);
    let error = prepare_source_block_execution(
        &text,
        Path::new("notes.org"),
        Selection::caret(ByteOffset(source.find("Hello").unwrap() as u64)),
    )
    .err()
    .unwrap();
    assert!(error.contains("Typst"));
    assert!(error.contains(":file"));

    let source = "#+begin_src typst :file result.txt\nHello\n#+end_src\n";
    let text = snapshot(source);
    let error = prepare_source_block_execution(
        &text,
        Path::new("notes.org"),
        Selection::caret(ByteOffset(source.find("Hello").unwrap() as u64)),
    )
    .err()
    .unwrap();
    assert!(error.contains(".svg, .png, or .pdf"));
}

#[test]
fn quoted_file_header_builds_a_real_results_transaction() {
    let source = "#+begin_src plantuml :file \"images/hello world.svg\"\nA -> B\n#+end_src\n";
    let text = snapshot(source);
    let request = prepare_source_block_execution(
        &text,
        Path::new("notes.org"),
        Selection::caret(ByteOffset(source.find("A -> B").unwrap() as u64)),
    )
    .unwrap();
    assert_eq!(
        request.file_target(),
        Some(Path::new("images/hello world.svg"))
    );
    let edit = request.file_edit().unwrap();
    assert!(edit.range.is_empty());
    assert_eq!(
        edit.replacement,
        "\n#+RESULTS:\n[[file:images/hello world.svg]]\n"
    );
}

#[test]
fn output_file_cannot_escape_the_document_directory() {
    for file in ["../outside.svg", "/tmp/outside.svg"] {
        let source = format!("#+begin_src plantuml :file {file}\nA -> B\n#+end_src\n");
        let text = snapshot(&source);
        let error = prepare_source_block_execution(
            &text,
            Path::new("notes/notes.org"),
            Selection::caret(ByteOffset(source.find("A -> B").unwrap() as u64)),
        )
        .err()
        .unwrap();
        assert!(error.contains("document directory"), "{error}");
    }
}

#[cfg(unix)]
#[test]
fn output_file_cannot_follow_a_parent_symlink_outside_the_document_directory() {
    use std::os::unix::fs::symlink;

    let unique = NEXT_TEMP_OUTPUT.fetch_add(1, Ordering::Relaxed);
    let base = std::env::temp_dir().join(format!(
        "org-studio-babel-sandbox-test-{}-{unique}",
        std::process::id()
    ));
    let root = base.join("document");
    let outside = base.join("outside");
    fs::create_dir_all(&root).unwrap();
    fs::create_dir_all(&outside).unwrap();
    symlink(&outside, root.join("images")).unwrap();

    let source = "#+begin_src plantuml :file images/diagram.svg\nA -> B\n#+end_src\n";
    let document = snapshot(source);
    let request = prepare_source_block_execution(
        &document,
        &root.join("notes.org"),
        Selection::caret(ByteOffset(source.find("A -> B").unwrap() as u64)),
    )
    .unwrap();
    let error = execute_source_block(request).err().unwrap();
    assert!(error.contains("outside"), "{error}");

    fs::remove_file(root.join("images")).unwrap();
    fs::remove_dir(root).unwrap();
    fs::remove_dir(outside).unwrap();
    fs::remove_dir(base).unwrap();
}

#[test]
fn repeated_execution_replaces_the_existing_file_result() {
    let source = "#+begin_src plantuml :file diagram.svg\nA -> B\n#+end_src\n\n#+RESULTS:\n[[file:old.svg]]\n\nafter\n";
    let text = snapshot(source);
    let request = prepare_source_block_execution(
        &text,
        Path::new("notes.org"),
        Selection::caret(ByteOffset(source.find("A -> B").unwrap() as u64)),
    )
    .unwrap();
    let edit = request.file_edit().unwrap();
    assert_eq!(
        &source[edit.range.as_usize()],
        "\n#+RESULTS:\n[[file:old.svg]]\n"
    );
    assert_eq!(edit.replacement, "\n#+RESULTS:\n[[file:diagram.svg]]\n");
}

#[test]
fn repeated_execution_preserves_the_results_image_attributes() {
    let source = "#+begin_src plantuml :file diagram.svg\nA -> B\n#+end_src\n\n#+RESULTS:\n#+ATTR_ORG: :width 320\n[[file:old.svg]]\n\nafter\n";
    let text = snapshot(source);
    let request = prepare_source_block_execution(
        &text,
        Path::new("notes.org"),
        Selection::caret(ByteOffset(source.find("A -> B").unwrap() as u64)),
    )
    .unwrap();
    let edit = request.file_edit().unwrap();
    assert_eq!(
        &source[edit.range.as_usize()],
        "\n#+RESULTS:\n#+ATTR_ORG: :width 320\n[[file:old.svg]]\n"
    );
    assert_eq!(
        edit.replacement,
        "\n#+RESULTS:\n#+ATTR_ORG: :width 320\n[[file:diagram.svg]]\n"
    );
}

#[test]
fn repeated_execution_with_the_same_result_link_does_not_edit_the_document() {
    let source = "#+begin_src plantuml :file diagram.svg\nA -> B\n#+end_src\n\n#+RESULTS:\n[[file:diagram.svg]]\n";
    let text = snapshot(source);
    let request = prepare_source_block_execution(
        &text,
        Path::new("notes.org"),
        Selection::caret(ByteOffset(source.find("A -> B").unwrap() as u64)),
    )
    .unwrap();

    assert!(request.file_edit().is_none());
}

#[test]
fn silent_results_write_the_file_without_editing_the_document() {
    let source = "#+begin_src plantuml :file diagram.svg :results silent\nA -> B\n#+end_src\n";
    let text = snapshot(source);
    let request = prepare_source_block_execution(
        &text,
        Path::new("notes.org"),
        Selection::caret(ByteOffset(source.find("A -> B").unwrap() as u64)),
    )
    .unwrap();
    assert!(request.file_edit().is_none());
}

#[test]
fn execution_publishes_svg_and_a_committable_results_edit() {
    use crate::document::{DocumentBuffer, EditTransaction};

    let unique = NEXT_TEMP_OUTPUT.fetch_add(1, Ordering::Relaxed);
    let root = std::env::temp_dir().join(format!(
        "org-studio-babel-test-{}-{unique}",
        std::process::id()
    ));
    fs::create_dir(&root).unwrap();
    let document_path = root.join("notes.org");
    let source = "#+begin_src plantuml :file images/diagram.svg\n@startuml\nAlice -> Bob: hello\n@enduml\n#+end_src\n";
    let mut buffer = DocumentBuffer::from_utf8(source.as_bytes().to_vec()).unwrap();
    let before = buffer.snapshot();
    let request = prepare_source_block_execution(
        &before,
        &document_path,
        Selection::caret(ByteOffset(source.find("Alice").unwrap() as u64)),
    )
    .unwrap();

    let mut output = execute_source_block(request).unwrap().publish().unwrap();
    let edit = output.result_edit.take().unwrap();
    buffer
        .commit(EditTransaction::new(before.revision(), vec![edit]))
        .unwrap();

    let svg = fs::read(root.join("images/diagram.svg")).unwrap();
    assert!(String::from_utf8_lossy(&svg).contains("<svg"));
    assert!(
        buffer
            .snapshot()
            .copy_range(ByteRange::new(0, buffer.snapshot().len_bytes()))
            .contains("#+RESULTS:\n[[file:images/diagram.svg]]")
    );

    fs::remove_file(root.join("images/diagram.svg")).unwrap();
    fs::remove_dir(root.join("images")).unwrap();
    fs::remove_dir(root).unwrap();
}

#[test]
fn typst_execution_publishes_svg_and_reports_source_lines() {
    let unique = NEXT_TEMP_OUTPUT.fetch_add(1, Ordering::Relaxed);
    let root = std::env::temp_dir().join(format!(
        "org-studio-typst-babel-test-{}-{unique}",
        std::process::id()
    ));
    fs::create_dir(&root).unwrap();
    let document_path = root.join("notes.org");
    let source = "#+begin_src typst :file images/card.svg\n#set page(width: 120pt, height: 80pt, margin: 8pt)\n#rect(fill: red)[Hello]\n#+end_src\n";
    let document = snapshot(source);
    let request = prepare_source_block_execution(
        &document,
        &document_path,
        Selection::caret(ByteOffset(source.find("#rect").unwrap() as u64)),
    )
    .unwrap();
    assert_eq!(request.language_name(), "Typst");

    let output = execute_source_block(request).unwrap().publish().unwrap();
    assert_eq!(output.language_name, "Typst");
    let svg = fs::read(root.join("images/card.svg")).unwrap();
    assert!(String::from_utf8_lossy(&svg).contains("<svg"));

    let updated = "#+begin_src typst :file images/card.svg\n#set page(width: 120pt, height: 80pt, margin: 8pt)\n#rect(fill: blue)[Updated]\n#+end_src\n";
    let document = snapshot(updated);
    let request = prepare_source_block_execution(
        &document,
        &document_path,
        Selection::caret(ByteOffset(updated.find("#rect").unwrap() as u64)),
    )
    .unwrap();
    execute_source_block(request).unwrap().publish().unwrap();
    let updated_svg = fs::read(root.join("images/card.svg")).unwrap();
    assert_ne!(svg, updated_svg, "the second run must replace the output");

    let invalid = "#+begin_src typst :file invalid.svg\n#set page(width:)\n#+end_src\n";
    let document = snapshot(invalid);
    let request = prepare_source_block_execution(
        &document,
        &document_path,
        Selection::caret(ByteOffset(invalid.find("#set").unwrap() as u64)),
    )
    .unwrap();
    let error = execute_source_block(request).err().unwrap();
    assert!(error.contains("line 1"), "{error}");

    fs::remove_file(root.join("images/card.svg")).unwrap();
    fs::remove_dir(root.join("images")).unwrap();
    fs::remove_dir(root).unwrap();
}

#[test]
fn typst_font_size_change_replaces_the_auto_sized_svg() {
    let unique = NEXT_TEMP_OUTPUT.fetch_add(1, Ordering::Relaxed);
    let root = std::env::temp_dir().join(format!(
        "org-studio-typst-font-babel-test-{}-{unique}",
        std::process::id()
    ));
    fs::create_dir(&root).unwrap();
    let document_path = root.join("notes.org");
    let render = |font_size: u8| {
        let source = format!(
            "#+begin_src typst :file result.svg\n#set page(width: auto, height: auto, margin: 18pt)\n#set text(size: {font_size}pt)\nA deliberately long line whose auto-sized page follows the text width.\n#+end_src\n"
        );
        let document = snapshot(&source);
        let request = prepare_source_block_execution(
            &document,
            &document_path,
            Selection::caret(ByteOffset(source.find("deliberately").unwrap() as u64)),
        )
        .unwrap();
        execute_source_block(request).unwrap().publish().unwrap();
        fs::read(root.join("result.svg")).unwrap()
    };

    let small = render(12);
    let large = render(30);

    assert_ne!(
        small, large,
        "the second run must publish the new font size"
    );
    fs::remove_file(root.join("result.svg")).unwrap();
    fs::remove_dir(root).unwrap();
}

#[test]
fn shell_stdout_replaces_previous_fixed_width_result() {
    let source = "#+begin_src sh :results output\nprintf 'new\\n'\n#+end_src\n\n#+RESULTS:\n: old\n\nafter\n";
    let document = snapshot(source);
    let request = prepare_source_block_execution(
        &document,
        Path::new("notes.org"),
        Selection::caret(ByteOffset(source.find("printf").unwrap() as u64)),
    )
    .unwrap();
    assert_eq!(request.external_program(), Some("/bin/sh"));
    let output = execute_source_block(request).unwrap().publish().unwrap();
    let edit = output.result_edit.as_ref().unwrap();
    let mut updated = source.to_owned();
    updated.replace_range(edit.range.as_usize(), &edit.replacement);
    assert_eq!(updated, source.replace(": old\n", ": new\n"));
    assert!(output.file().is_none());
}

#[test]
fn repeated_shell_runs_replace_every_result_line() {
    use crate::document::{DocumentBuffer, EditTransaction};

    let source = "#+begin_src sh :results output\nprintf 'One\\nTwo\\n'\n#+end_src\n\n#+RESULTS:\n: One\n: Two\n: Two\n\n* Next\n";
    let expected = source.replacen(": Two\n: Two\n", ": Two\n", 1);
    let mut buffer = DocumentBuffer::from_utf8(source.as_bytes().to_vec()).unwrap();

    for _ in 0..3 {
        let before = buffer.snapshot();
        let request = prepare_source_block_execution(
            &before,
            Path::new("notes.org"),
            Selection::caret(ByteOffset(source.find("printf").unwrap() as u64)),
        )
        .unwrap();
        let output = execute_source_block(request).unwrap();
        if let Some(edit) = output.result_edit {
            buffer
                .commit(EditTransaction::new(before.revision(), vec![edit]))
                .unwrap();
        }
        let current = buffer.snapshot();
        assert_eq!(
            current.copy_range(ByteRange::new(0, current.len_bytes())),
            expected
        );
    }
}

#[test]
fn shell_silent_does_not_edit_and_eval_never_does_not_run() {
    let source = "#+begin_src sh :results silent\nprintf 'ok\\n'\n#+end_src\n";
    let document = snapshot(source);
    let request = prepare_source_block_execution(
        &document,
        Path::new("notes.org"),
        Selection::caret(ByteOffset(source.find("printf").unwrap() as u64)),
    )
    .unwrap();
    assert!(execute_source_block(request).unwrap().result_edit.is_none());

    let source = source.replace(":results silent", ":eval never");
    let document = snapshot(&source);
    let error = prepare_source_block_execution(
        &document,
        Path::new("notes.org"),
        Selection::caret(ByteOffset(source.find("printf").unwrap() as u64)),
    )
    .err()
    .unwrap();
    assert!(error.contains("disables execution"));
}

#[test]
fn python_stdout_is_written_as_org_fixed_width_text() {
    let source = "#+begin_src python :results output\nprint(1 + 2)\n#+end_src\n";
    let document = snapshot(source);
    let request = prepare_source_block_execution(
        &document,
        Path::new("notes.org"),
        Selection::caret(ByteOffset(source.find("print").unwrap() as u64)),
    )
    .unwrap();
    let output = execute_source_block(request).unwrap();
    assert_eq!(
        output.result_edit.unwrap().replacement,
        "\n#+RESULTS:\n: 3\n"
    );
}
