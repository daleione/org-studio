use super::{execute_plan, prepare_execution, value::Value};
use crate::document::{ByteOffset, DocumentSnapshot, Selection};
use std::path::Path;

fn snapshot(text: &str) -> DocumentSnapshot {
    DocumentSnapshot::from_utf8(text.as_bytes().to_vec()).unwrap()
}
fn run(text: &str, needle: &str) -> super::PreparedBabelOutput {
    let plan = prepare_execution(
        &snapshot(text),
        Path::new("/tmp/babel-analysis.org"),
        Selection::caret(ByteOffset(text.rfind(needle).unwrap() as u64)),
        Vec::new(),
    )
    .unwrap();
    execute_plan(plan).unwrap()
}

#[test]
fn python_parameters_follow_org_header_precedence_and_keep_typed_values_separate_from_stdout() {
    let text = "#+PROPERTY: header-args:python :var a=1 b=2 :results value\n* Analysis\n:PROPERTIES:\n:header-args:python: :var a=4\n:END:\n#+HEADER: :var c=5 b=8\n#+begin_src python :var b=7 :var text=\"a,b: c\\n\\\"quoted\\\"\" :var big=18446744073709551615 :var empty=\"\" :var missing=nil\nprint('debug only')\nassert empty == '' and missing is None\nreturn [a, b, c, text, big]\n#+end_src\n";
    let output = run(text, "print(");
    assert_eq!(
        output.data.value,
        Value::List(vec![
            Value::Integer("4".into()),
            Value::Integer("8".into()),
            Value::Integer("5".into()),
            Value::Text("a,b: c\n\"quoted\"".into()),
            Value::Integer("18446744073709551615".into())
        ])
    );
    assert!(
        !output
            .result_edit
            .unwrap()
            .replacement
            .contains("debug only")
    );
    assert_eq!(
        run("#+begin_src python\n42\n#+end_src\n", "42").data.value,
        Value::Nil,
        "Python value uses explicit return"
    );
    assert_eq!(run("#+begin_src python :var n=12345678901234567890123456789012345678901234567890\nreturn n\n#+end_src\n", "return n").data.value, Value::Integer("12345678901234567890123456789012345678901234567890".into()));
    assert_eq!(
        run(
            "#+begin_src python :var n=2\nn += 1\ndef read_n():\n    return n\nreturn read_n()\n#+end_src\n",
            "n += 1",
        )
        .data
        .value,
        Value::Integer("3".into()),
        "parameters must support local assignment and nested functions"
    );
}

#[test]
fn static_tables_keep_org_source_and_apply_indices_before_header_removal() {
    let text = "#+name: samples\n| label | amount |\n|-------+--------|\n| a\\vert{}b | 0012 |\n| \\nbsp{}pad\\nbsp{} | 3.5 |\n\n#+begin_src python :var rows=samples n=samples[2,-1]\nassert rows == [['a\\\\vert{}b', 12], ['\\\\nbsp{}pad\\\\nbsp{}', 3.5]]\nreturn n\n#+end_src\n";
    assert_eq!(
        run(text, "return n").data.value,
        Value::Integer("12".into())
    );
    use super::resolve::select;
    let value = super::value::Data {
        value: Value::List(vec![
            Value::List(vec![
                Value::Integer("10".into()),
                Value::Integer("20".into()),
            ]),
            Value::List(vec![
                Value::Integer("30".into()),
                Value::Integer("40".into()),
            ]),
        ]),
        ..Default::default()
    };
    assert_eq!(
        select(value.clone(), "0,-1").unwrap().value,
        Value::Integer("20".into())
    );
    assert_eq!(
        select(value.clone(), "0:1,1").unwrap().value,
        Value::List(vec![
            Value::Integer("20".into()),
            Value::Integer("40".into())
        ])
    );
    assert!(select(value, "8,*").is_err());
}

#[test]
fn each_source_reference_executes_and_only_the_root_result_is_replaced() {
    let directory = tempfile::tempdir().unwrap();
    let text = "#+name: next-number\n#+begin_src python\nfrom pathlib import Path\np = Path('counter.txt')\nn = int(p.read_text()) + 1 if p.exists() else 1\np.write_text(str(n))\nreturn n\n#+end_src\n\n#+RESULTS: next-number\n: 999\n\n#+name: pair\n#+begin_src python :var a=next-number b=next-number\nreturn [a,b]\n#+end_src\n\n#+RESULTS: pair\n#+PLOT: with:histograms\n| old |\n\nFollowing text\n";
    let plan = prepare_execution(
        &snapshot(text),
        &directory.path().join("notes.org"),
        Selection::caret(ByteOffset(text.find("return [a,b]").unwrap() as u64)),
        Vec::new(),
    )
    .unwrap();
    let output = execute_plan(plan).unwrap();
    assert_eq!(
        output.data.value,
        Value::List(vec![Value::Integer("1".into()), Value::Integer("2".into())])
    );
    let edit = output.result_edit.unwrap();
    assert!(edit.replacement.contains("#+RESULTS: pair\n#+PLOT:"));
    let after = format!(
        "{}{}{}",
        &text[..edit.range.start.0 as usize],
        edit.replacement,
        &text[edit.range.end.0 as usize..]
    );
    assert!(after.contains("#+RESULTS: next-number\n: 999"));
    assert!(after.ends_with("\nFollowing text\n"));
}

#[test]
fn call_argument_layers_positional_defaults_and_named_call_results() {
    let text = "#+name: multiply\n#+begin_src python :var a=1 b=2\nreturn a*b\n#+end_src\n\n#+name: calculated\n#+CALL: multiply[:var a=3 :results output](a=4,b=5) :var b=6 :results value scalar\n\n#+CALL: multiply(7)\n\n#+begin_src python :var n=calculated\nreturn n+1\n#+end_src\n";
    let output = run(text, "#+CALL: multiply[");
    assert_eq!(output.data.value, Value::Integer("24".into()));
    assert!(
        output
            .result_edit
            .unwrap()
            .replacement
            .contains("#+RESULTS: calculated\n: 24\n")
    );
    let plan = prepare_execution(
        &snapshot(text),
        Path::new("/tmp/babel-analysis.org"),
        Selection::caret(ByteOffset(text.find("#+CALL: multiply(7)").unwrap() as u64)),
        Vec::new(),
    )
    .unwrap();
    assert_eq!(
        plan.block_name(),
        Some("multiply"),
        "an unnamed CALL still identifies the code being executed"
    );
    let output = execute_plan(plan).unwrap();
    assert_eq!(output.data.value, Value::Integer("14".into()));
    assert!(
        output
            .result_edit
            .unwrap()
            .replacement
            .contains("#+RESULTS:\n: 14\n")
    );
    assert_eq!(
        run(text, "return n+1").data.value,
        Value::Integer("25".into())
    );
}

#[test]
fn cross_file_calls_use_caller_context_but_explicit_value_references_use_definition_context() {
    let directory = tempfile::tempdir().unwrap();
    let library = "#+name: items\n| 100 |\n\n#+name: take-items\n#+begin_src python :var rows=items\nreturn rows[0][0]\n#+end_src\n";
    let library_path = directory.path().join("library.org");
    std::fs::write(&library_path, format!("\u{feff}{library}")).unwrap();
    let text = "#+name: items\n| 10 |\n\n#+CALL: library.org:take-items()\n\n#+begin_src python :var n=library.org:take-items()\nreturn n\n#+end_src\n";
    let path = directory.path().join("caller.org");
    let execute = |needle: &str, open| {
        let plan = prepare_execution(
            &snapshot(text),
            &path,
            Selection::caret(ByteOffset(text.find(needle).unwrap() as u64)),
            open,
        )
        .unwrap();
        execute_plan(plan).unwrap()
    };
    assert_eq!(
        execute("#+CALL", Vec::new()).data.value,
        Value::Integer("10".into())
    );
    assert_eq!(
        execute("return n", Vec::new()).data.value,
        Value::Integer("100".into())
    );
    let disk_plan = prepare_execution(
        &snapshot(text),
        &path,
        Selection::caret(ByteOffset(text.find("return n").unwrap() as u64)),
        Vec::new(),
    )
    .unwrap();
    assert!(
        disk_plan.versions[1].matches(None),
        "UTF-8 BOM must not invalidate an unchanged library"
    );
    let changed = snapshot(&library.replace("return rows[0][0]", "return rows[0][0]+1"));
    assert_eq!(
        execute("return n", vec![(library_path.clone(), changed.clone())])
            .data
            .value,
        Value::Integer("101".into())
    );
    let alias = directory.path().join("alias.org");
    std::fs::hard_link(&library_path, &alias).unwrap();
    assert_eq!(
        execute("return n", vec![(alias, changed.clone())])
            .data
            .value,
        Value::Integer("101".into()),
        "an open hard-link alias also supplies unsaved content"
    );
    let plan = prepare_execution(
        &snapshot(text),
        &path,
        Selection::caret(ByteOffset(text.find("return n").unwrap() as u64)),
        vec![(library_path, changed.clone())],
    )
    .unwrap();
    let version = &plan.versions[1];
    assert!(version.matches(Some(&changed)));
    assert!(
        !version.matches(None),
        "unsaved library must not silently use disk data"
    );
    assert!(!version.matches(Some(&snapshot(library))));
}

#[test]
fn invalid_dependencies_and_unsupported_capabilities_fail_during_preflight() {
    for text in [
        "#+name: x\n#+begin_src python :var n=x\nreturn n\n#+end_src\n",
        "#+begin_src python :var n=unknown\nreturn n\n#+end_src\n",
        "#+name: x\n| 1 |\n\n#+name: x\n| 2 |\n\n#+begin_src python\nreturn 1\n#+end_src\n",
        "#+begin_src python :session persistent\nreturn 1\n#+end_src\n",
        "#+begin_src python :cache yes\nreturn 1\n#+end_src\n",
        "#+begin_src python :eval never\nreturn 1\n#+end_src\n",
        "#+begin_src sql :eval NEVER\nSELECT 1;\n#+end_src\n",
        "#+begin_src python :var n=(dangerous-lisp)\nreturn 1\n#+end_src\n",
        "#+begin_src typst :file sample.svg :var n=1\nHello\n#+end_src\n",
        "#+begin_src sh :var n='(1 2) :results output\nprintf x\n#+end_src\n",
    ] {
        let result = prepare_execution(
            &snapshot(text),
            Path::new("/tmp/babel-invalid.org"),
            Selection::caret(ByteOffset(text.find("#+begin_src").unwrap() as u64)),
            Vec::new(),
        );
        assert!(result.is_err(), "{text}");
    }
}

#[test]
fn cancellation_stops_the_dependency_process_and_does_not_publish_partial_results() {
    let directory = tempfile::tempdir().unwrap();
    let text = "#+name: slow\n#+begin_src sh :results output\nsleep 3\nprintf finished > marker.txt\n#+end_src\n\n#+begin_src python :var n=slow\nreturn n\n#+end_src\n\n#+RESULTS:\n: previous\n";
    let plan = prepare_execution(
        &snapshot(text),
        &directory.path().join("notes.org"),
        Selection::caret(ByteOffset(text.find("return n").unwrap() as u64)),
        Vec::new(),
    )
    .unwrap();
    let cancel = plan.cancel.clone();
    let cancelling = std::thread::spawn(move || {
        std::thread::sleep(std::time::Duration::from_millis(150));
        cancel.cancel();
    });
    let started = std::time::Instant::now();
    assert!(execute_plan(plan).err().unwrap().contains("cancelled"));
    cancelling.join().unwrap();
    assert!(started.elapsed() < std::time::Duration::from_secs(2));
    assert!(!directory.path().join("marker.txt").exists());
}

#[test]
fn row_names_empty_tables_and_shell_scalars_use_standard_results() {
    let text = "#+name: samples\n| label | amount |\n|-------+--------|\n| alpha | 2 |\n| beta | 4 |\n\n#+begin_src python :var rows=samples :rownames yes\nreturn rows\n#+end_src\n";
    let output = run(text, "return rows");
    let result = output.result_edit.unwrap().replacement;
    assert!(
        result.contains("alpha") && result.contains("beta") && result.contains("amount"),
        "{result}"
    );
    let empty = run(
        "#+begin_src python :colnames yes\nreturn [['label','amount']]\n#+end_src\n",
        "return",
    );
    let result = empty.result_edit.unwrap().replacement;
    assert_eq!(result.lines().filter(|l| l.starts_with('|')).count(), 2);
    assert_eq!(empty.data.value, Value::List(Vec::new()));
    let output = run(
        "#+begin_src sh :var value=\"some ' quoted; $(echo unsafe)\" :results output\nprintf '%s' \"$value\"\n#+end_src\n",
        "printf",
    );
    assert_eq!(
        output.data.value,
        Value::Text("some ' quoted; $(echo unsafe)".into())
    );
}
