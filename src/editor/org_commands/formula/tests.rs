use super::*;
use crate::document::DocumentSession;
use gpui::AppContext;

fn calculated(source: &str) -> Result<String, String> {
    let snapshot = DocumentSnapshot::from_utf8(source.as_bytes().to_vec()).unwrap();
    let caret = ByteOffset(source.find("#+TBLFM:").unwrap() as u64);
    let change = recalculate(&snapshot, caret, "\n", true)?.unwrap();
    let mut result = source.to_owned();
    result.replace_range(
        change.range.start.0 as usize..change.range.end.0 as usize,
        &change.replacement,
    );
    Ok(result)
}

fn cell(source: &str, line: usize, column: usize) -> String {
    let text = source.lines().nth(line).unwrap();
    let parsed = source_table::parse_line(text, DocumentFormat::Org);
    text[parsed.cells[column].text_range.clone()].to_owned()
}

#[test]
fn column_formulas_skip_headers_and_field_formulas_override_them() {
    let source = "| Item | Qty | Unit | Total |\n|------+-----+------+-------|\n| A | 2 | 3 | 0 |\n| B | 4 | 5 | 0 |\n#+TBLFM: $4=$2*$3::@3$4=vsum(@2$2..@3$2)\n";
    let result = calculated(source).unwrap();
    assert_eq!(cell(&result, 0, 3), "Total");
    assert_eq!(cell(&result, 2, 3), "6");
    assert_eq!(cell(&result, 3, 3), "6");
    assert!(result.contains("#+TBLFM: $4=$2*$3::@3$4=vsum(@2$2..@3$2)"));
}

#[test]
fn range_aggregation_and_calc_division_order() {
    let source = "| n | x | total |\n|---+---+-------|\n| A | 2 | 0 |\n| B | 4 | 0 |\n| Mean | 0 | 0 |\n#+TBLFM: $3=$2/2*2::@4$3=vmean(@2$2..@3$2);%.1f\n";
    let result = calculated(source).unwrap();
    assert_eq!(cell(&result, 4, 2), "3.0");
    assert_eq!(cell(&result, 2, 2), "0.5");
    assert_eq!(cell(&result, 3, 2), "1");
}

#[test]
fn current_row_recalculation_uses_relative_columns() {
    let source = "| 2 | 3 | 0 |\n| 4 | 5 | 0 |\n#+TBLFM: $3=$-2+$-1\n";
    let snapshot = DocumentSnapshot::from_utf8(source.as_bytes().to_vec()).unwrap();
    let change = recalculate(&snapshot, ByteOffset(2), "\n", false)
        .unwrap()
        .unwrap();
    let mut result = source.to_owned();
    result.replace_range(
        change.range.start.0 as usize..change.range.end.0 as usize,
        &change.replacement,
    );
    assert_eq!(cell(&result, 0, 2), "5");
    assert_eq!(cell(&result, 1, 2), "0");
}

#[test]
fn full_recalculation_preserves_headers_and_coordinates_are_numbers() {
    let source = "| 0 | Heading | 0 |\n|---+---------+---|\n| 2 | A | 0 |\n| 4 | B | 0 |\n#+TBLFM: $3=$1*2::@1$1=10::@3$3=@#+$#\n";
    let result = calculated(source).unwrap();
    assert_eq!(cell(&result, 0, 0), "0");
    assert_eq!(cell(&result, 0, 2), "0");
    assert_eq!(cell(&result, 2, 2), "4");
    assert_eq!(cell(&result, 3, 2), "6");
}

#[test]
fn current_header_row_can_apply_its_field_formula() {
    let source = "| 0 | Label |\n|---+-------|\n| 1 | Body |\n#+TBLFM: @1$1=10\n";
    let snapshot = DocumentSnapshot::from_utf8(source.as_bytes().to_vec()).unwrap();
    let change = recalculate(&snapshot, ByteOffset(2), "\n", false)
        .unwrap()
        .unwrap();
    let mut result = source.to_owned();
    result.replace_range(
        change.range.start.0 as usize..change.range.end.0 as usize,
        &change.replacement,
    );
    assert_eq!(cell(&result, 0, 0), "10");
}

#[test]
fn formula_text_inside_source_block_is_not_a_table() {
    let source = "#+begin_src python\ntext = '''\n| 1 | 0 |\n#+TBLFM: $2=$1*2\n'''\n#+end_src\n";
    let snapshot = DocumentSnapshot::from_utf8(source.as_bytes().to_vec()).unwrap();
    for marker in ["| 1 | 0 |", "#+TBLFM:"] {
        let caret = ByteOffset(source.find(marker).unwrap() as u64);
        assert!(recalculate(&snapshot, caret, "\n", true).unwrap().is_none());
    }
}

#[test]
fn unsafe_integer_inputs_and_results_fail_without_writing() {
    for source in [
        "| 9007199254740992 | 0 |\n#+TBLFM: $2=$1+1\n",
        "| 1 | 0 |\n#+TBLFM: $2=9007199254740992+1\n",
        "| 9007199254740991 | 0 |\n#+TBLFM: $2=$1+1\n",
    ] {
        assert!(calculated(source).unwrap_err().contains("precision"));
    }
}

#[test]
fn last_row_target_and_numeric_empty_mode() {
    let source =
        "| 2 |   | 0 |\n| 4 | 3 | 0 |\n| 0 | 0 | 0 |\n#+TBLFM: $3=$1+$2;N::@>$3=vsum(@1$1..@2$1)\n";
    let result = calculated(source).unwrap();
    assert_eq!(cell(&result, 0, 2), "2");
    assert_eq!(cell(&result, 1, 2), "7");
    assert_eq!(cell(&result, 2, 2), "6");
}

#[test]
fn range_target_overrides_column_formula_in_its_rectangle() {
    let source =
        "| 1 | 2 | 0 |\n| 3 | 4 | 0 |\n| 5 | 6 | 0 |\n#+TBLFM: $3=$1+$2::@2$2..@3$3=$1*10\n";
    let result = calculated(source).unwrap();
    assert_eq!(cell(&result, 0, 2), "3");
    assert_eq!(cell(&result, 1, 1), "30");
    assert_eq!(cell(&result, 1, 2), "30");
    assert_eq!(cell(&result, 2, 1), "50");
    assert_eq!(cell(&result, 2, 2), "50");
}

#[test]
fn a1_coordinates_and_repeated_edge_references() {
    let source = "| 1 | 2 | 0 |\n| 3 | 4 | 0 |\n#+TBLFM: $3=$1+$2::C2=A1*10::$>>=$<*2\n";
    let result = calculated(source).unwrap();
    assert_eq!(cell(&result, 0, 1), "2");
    assert_eq!(cell(&result, 1, 1), "6");
    assert_eq!(cell(&result, 1, 2), "10");
}

#[test]
fn hline_references_select_rows_between_separators() {
    let source = "| Name | Value | Result |\n|------+-------+--------|\n| A | 2 | 0 |\n| B | 3 | 0 |\n|------+-------+--------|\n| Sum | 0 | 0 |\n#+TBLFM: @>$3=vsum(@I$2..@II$2)+@I+2$2\n";
    let result = calculated(source).unwrap();
    assert_eq!(cell(&result, 5, 2), "8");

    let source = "|---+---|\n| 4 | 0 |\n| 6 | 0 |\n#+TBLFM: @2$2=@-I$1\n";
    assert_eq!(cell(&calculated(source).unwrap(), 2, 1), "4");

    let source =
        "|---+---|\n| 4 | 0 |\n| 6 | 0 |\n|---+---|\n| 0 | 0 |\n#+TBLFM: @3$2=vsum(@I$1..II$1)\n";
    assert_eq!(cell(&calculated(source).unwrap(), 4, 1), "10");
}

#[test]
fn named_columns_constants_and_fields_resolve() {
    let source = "#+CONSTANTS: tax=0.1\n| ! | base | total |\n|---+------+-------|\n| # | 100 | 0 |\n#+TBLFM: $3=$base*(1+$tax)\n";
    let result = calculated(source).unwrap();
    assert_eq!(cell(&result, 3, 2), "110");

    let source = "| ! | left | right | sum |\n|---+------+-------+-----|\n| # | 2 | 3 | 0 |\n#+TBLFM: $4=vsum($left..$right)\n";
    assert_eq!(cell(&calculated(source).unwrap(), 2, 3), "5");

    let source = "| 0 | 5 | 0 |\n| ^ | n |   |\n| 1 | 2 | 0 |\n#+TBLFM: @3$3=$n+$2\n";
    let result = calculated(source).unwrap();
    assert_eq!(cell(&result, 2, 2), "7");

    let source = "| _ |   | out |\n| A | 2 | 0 |\n#+TBLFM: $out=$2*2\n";
    let result = calculated(source).unwrap();
    assert_eq!(cell(&result, 1, 2), "4");

    let source =
        "* Rates\n:PROPERTIES:\n:Rate: 0.25\n:END:\n| 4 | 0 |\n#+TBLFM: $2=$1*(1+$PROP_Rate)\n";
    let result = calculated(source).unwrap();
    assert_eq!(cell(&result, 4, 1), "5");
}

#[test]
fn special_rows_limit_column_recalculation_but_keep_field_formulas() {
    let source = "| ! | Item | Value | Total |\n|---+------+-------+-------|\n| # | A | 2 | 0 |\n| * | B | 3 | 0 |\n|   | C | 4 | 0 |\n| $ | factor=2 | | |\n#+TBLFM: $4=$3*$factor::@4$4=9\n";
    let result = calculated(source).unwrap();
    assert_eq!(cell(&result, 2, 3), "4");
    assert_eq!(cell(&result, 3, 3), "6");
    assert_eq!(cell(&result, 4, 3), "9");
    assert_eq!(cell(&result, 5, 1), "factor=2");
}

#[test]
fn named_remote_table_supports_field_and_range_references() {
    let source = "#+NAME: prices\n| 2 | 3 |\n| 4 | 5 |\n\n| 0 | 0 |\n#+TBLFM: @1$1=remote(prices,@2$2)::@1$2=vsum(remote(prices,@1$1..@2$1))\n";
    let result = calculated(source).unwrap();
    assert_eq!(cell(&result, 4, 0), "5");
    assert_eq!(cell(&result, 4, 1), "6");

    let source = "#+CONSTANTS: rate=0.5\n#+NAME: prices\n| 7 |\n\n| prices | 0 | 0 |\n#+TBLFM: $2=remote($1,@1$1)::@1$3=remote(prices,$rate)\n";
    let result = calculated(source).unwrap();
    assert_eq!(cell(&result, 4, 1), "7");
    assert_eq!(cell(&result, 4, 2), "0.5");

    let source = "#+NAME: copy\n| 3 | 8 |\n| 5 | 9 |\n\n| 0 | 0 |\n| 0 | 0 |\n#+TBLFM: $1=remote(copy,@@#$1)::$2=remote(copy,@1$$#)\n";
    let result = calculated(source).unwrap();
    assert_eq!(cell(&result, 4, 0), "3");
    assert_eq!(cell(&result, 5, 0), "5");
    assert_eq!(cell(&result, 4, 1), "8");

    let source = "* Source\n:PROPERTIES:\n:ID: table-123\n:END:\n| 11 |\n* Result\n| 0 |\n#+TBLFM: $1=remote(table-123,@1$1)\n";
    assert_eq!(cell(&calculated(source).unwrap(), 6, 0), "11");

    let source = "#+NAME: captioned\n#+CAPTION: Source data\n| 9 |\n\n| 0 |\n#+TBLFM: $1=remote(captioned,@1$1)\n";
    assert_eq!(cell(&calculated(source).unwrap(), 4, 0), "9");

    let source = "#+NAME: named\n| ! | a | b |\n|---+---+---|\n| # | 2 | 3 |\n\n| Label | Sum |\n|-------+-----|\n| Row   | 0   |\n#+TBLFM: @2$2=vsum(remote(named,$a..$b))\n";
    assert_eq!(cell(&calculated(source).unwrap(), 7, 1), "5");

    let source = "#+NAME: short\n| ! | a |\n| # | 2 |\n\n| 0 | 0 |\n| 0 | 0 |\n| 0 | 0 |\n#+TBLFM: @3$2=remote(short,$a)\n";
    assert!(
        calculated(source)
            .unwrap_err()
            .contains("outside the table")
    );
}

#[test]
fn multiple_formula_lines_use_only_the_line_at_the_caret() {
    let source = "| 2 | 0 |\n#+TBLFM: $2=$1*2\n#+TBLFM: $2=$1*3\n";
    let snapshot = DocumentSnapshot::from_utf8(source.as_bytes().to_vec()).unwrap();
    let caret = ByteOffset(source.rfind("#+TBLFM:").unwrap() as u64);
    let change = recalculate(&snapshot, caret, "\n", true).unwrap().unwrap();
    assert_eq!(cell(&change.replacement, 0, 1), "6");
    assert_eq!(cell(&calculated(source).unwrap(), 0, 1), "4");
}

#[test]
fn iterative_recalculation_converges_and_rejects_cycles() {
    let source = "| 0 |\n| 0 |\n#+TBLFM: @1$1=@2$1+1::@2$1=2\n";
    let snapshot = DocumentSnapshot::from_utf8(source.as_bytes().to_vec()).unwrap();
    let caret = ByteOffset(source.find("#+TBLFM:").unwrap() as u64);
    let single = recalculate(&snapshot, caret, "\n", true).unwrap().unwrap();
    assert_eq!(cell(&single.replacement, 0, 0), "1");
    let iterated = recalculate_iteratively(&snapshot, caret, "\n")
        .unwrap()
        .unwrap();
    assert_eq!(cell(&iterated.replacement, 0, 0), "3");
    assert_eq!(cell(&iterated.replacement, 1, 0), "2");

    let source = "| 0 |\n#+TBLFM: @1$1=@1$1+1\n";
    let snapshot = DocumentSnapshot::from_utf8(source.as_bytes().to_vec()).unwrap();
    assert!(
        recalculate_iteratively(&snapshot, ByteOffset(0), "\n")
            .unwrap_err()
            .contains("did not converge")
    );
}

#[test]
fn buffer_recalculation_updates_every_table_and_remote_dependencies() {
    let source =
        "#+NAME: base\n| 1 | 0 |\n#+TBLFM: $2=$1+1\n\n| 0 |\n#+TBLFM: $1=remote(base,@1$2)*2\n";
    let snapshot = DocumentSnapshot::from_utf8(source.as_bytes().to_vec()).unwrap();
    let apply = |iterate| {
        let change = recalculate_buffer_tables(&snapshot, ByteOffset(0), "\n", iterate)
            .unwrap()
            .unwrap();
        let mut text = source.to_owned();
        text.replace_range(
            change.range.start.0 as usize..change.range.end.0 as usize,
            &change.replacement,
        );
        text
    };
    let single = apply(false);
    assert_eq!(cell(&single, 1, 1), "2");
    assert_eq!(cell(&single, 4, 0), "0");
    let iterated = apply(true);
    assert_eq!(cell(&iterated, 1, 1), "2");
    assert_eq!(cell(&iterated, 4, 0), "4");

    let invalid = "| 1 |\n#+TBLFM: $1=bad($1)\n";
    let snapshot = DocumentSnapshot::from_utf8(invalid.as_bytes().to_vec()).unwrap();
    assert!(recalculate_buffer_tables(&snapshot, ByteOffset(0), "\n", false).is_err());

    let unicode = "| 1 | 中文 |\n#+TBLFM: $1=1000\n";
    let snapshot = DocumentSnapshot::from_utf8(unicode.as_bytes().to_vec()).unwrap();
    let caret = ByteOffset(unicode.find('文').unwrap() as u64);
    let change = recalculate_buffer_tables(&snapshot, caret, "\n", false)
        .unwrap()
        .unwrap();
    let mut result = unicode.to_owned();
    result.replace_range(
        change.range.start.0 as usize..change.range.end.0 as usize,
        &change.replacement,
    );
    assert!(result.is_char_boundary(change.caret.0 as usize));
}

#[test]
fn duration_modes_accept_time_inputs_and_format_results() {
    let source = "| 2:12 | 1:47 | 0 |\n| 2:12 | 1:47 | 0 |\n| 3:02:20 | -2:07:00 | 0 |\n#+TBLFM: @1$3=$1+$2;T::@2$3=$1+$2;U::@3$3=$1+$2;t\n";
    let result = calculated(source).unwrap();
    assert_eq!(cell(&result, 0, 2), "03:59:00");
    assert_eq!(cell(&result, 1, 2), "03:59");
    assert_eq!(cell(&result, 2, 2), "0.92");
}

#[test]
fn calc_display_modes_format_without_changing_input_cells() {
    let source =
        "| 12.3456 | 0 | 0 | 0 | 0 |\n#+TBLFM: @1$2=$1;f2::@1$3=$1;s3::@1$4=$1;e3::@1$5=$1;%.1e\n";
    let result = calculated(source).unwrap();
    assert_eq!(cell(&result, 0, 0), "12.3456");
    assert_eq!(cell(&result, 0, 1), "12.35");
    assert_eq!(cell(&result, 0, 2), "1.23e1");
    assert_eq!(cell(&result, 0, 3), "12.35e0");
    assert_eq!(cell(&result, 0, 4), "1.2e1");

    let source = "| 0 |\n#+TBLFM: $1=1/3;f-1\n";
    assert_eq!(cell(&calculated(source).unwrap(), 0, 0), "0.3");

    let source = "| 0 |\n#+TBLFM: $1=100.01;n3\n";
    assert_eq!(cell(&calculated(source).unwrap(), 0, 0), "100");

    let source = "| 0 |\n#+TBLFM: $1=1e-300;e3\n";
    assert_eq!(cell(&calculated(source).unwrap(), 0, 0), "1.00e-300");
}

#[test]
fn calc_functions_conditions_and_text_results() {
    let source = "| 30 | 0 | 0 | 0 |\n| 10 | 0 | 0 | 0 |\n#+TBLFM: $2=sin($1)*2::@1$3=if($1 < 20, 0, string(\"adult\"))::@2$3=if($1 < 20, string(\"teen\"), 1/0)::@1$4=vmedian(@1$1..@2$1)\n";
    let result = calculated(source).unwrap();
    assert_eq!(cell(&result, 0, 1), "1");
    assert_eq!(cell(&result, 1, 1), "0.34729636");
    assert_eq!(cell(&result, 0, 2), "adult");
    assert_eq!(cell(&result, 1, 2), "teen");
    assert_eq!(cell(&result, 0, 3), "20");

    let source = "| text | 0 |\n#+TBLFM: $2=if(\"$1\" == \"text\", 3, 4)\n";
    assert_eq!(cell(&calculated(source).unwrap(), 0, 1), "3");

    let source = "| 15 | 0 |\n#+TBLFM: $2=if($1 < 20, teen, string(\"\"))\n";
    assert_eq!(cell(&calculated(source).unwrap(), 0, 1), "teen");

    let source = "| 0 |\n#+TBLFM: $1=if(1, string(\"a::b;cool\"), 0)\n";
    assert_eq!(cell(&calculated(source).unwrap(), 0, 0), "a::b;cool");

    let source = "|  | 2 | old |\n| 3 | 2 | old |\n#+TBLFM: $3=if(\"$1\" == \"nan\", string(\"\"), $1+$2);E\n";
    let result = calculated(source).unwrap();
    assert_eq!(cell(&result, 0, 2), "");
    assert_eq!(cell(&result, 1, 2), "5");

    let source = "| 1 |  | 0 |\n#+TBLFM: $3=if(typeof(vsum($1..$2)) == 12, string(\"\"), 5);E\n";
    assert_eq!(cell(&calculated(source).unwrap(), 0, 2), "");

    let source = "| 0 | 0 | 0 |\n#+TBLFM: @1$1=4!::@1$2=25%*8::@1$3=if(3 != 4, 1, 0)\n";
    let result = calculated(source).unwrap();
    assert_eq!(cell(&result, 0, 0), "24");
    assert_eq!(cell(&result, 0, 1), "2");
    assert_eq!(cell(&result, 0, 2), "1");

    let source = "| 0 |\n#+TBLFM: $1=log10(1000)\n";
    assert_eq!(cell(&calculated(source).unwrap(), 0, 0), "3");
}

#[test]
fn unsupported_formula_fails_without_changing_source() {
    let source = "| 1 | 2 |\n| 3 | 4 |\n#+TBLFM: $2=unknown($1)\n";
    assert!(calculated(source).unwrap_err().contains("Unsupported"));
    assert!(
        calculated("| 0 |\n#+TBLFM: $1=sqrt(-1)\n")
            .unwrap_err()
            .contains("real-valued")
    );
    assert!(
        calculated("| 1 | 0 |\n#+TBLFM: $2=$+18446744073709551615\n")
            .unwrap_err()
            .contains("too large")
    );
}

#[gpui::test]
fn editor_recalculates_in_one_undo_step(cx: &mut gpui::TestAppContext) {
    cx.update(crate::editor::init);
    let source = "| A | 2 | 3 | 0 |\n| B | 4 | 5 | 0 |\n#+TBLFM: $4=$2*$3\n";
    let session = cx.new(|_| {
        DocumentSession::from_utf8("table.org".into(), source.as_bytes().to_vec()).unwrap()
    });
    let (editor, view) =
        cx.add_window_view(|_, cx| crate::editor::SemanticEditor::new(session.clone(), cx));
    view.run_until_parked();
    editor.update(view, |editor, cx| {
        editor.set_selection(
            crate::document::Selection::caret(ByteOffset(source.find("#+TBLFM:").unwrap() as u64)),
            cx,
        );
        assert!(editor.recalculate_table_at_selection(false, cx).unwrap());
    });
    view.read(|cx| {
        let snapshot = session.read(cx).snapshot();
        let text = snapshot.copy_range(ByteRange::new(0, snapshot.len_bytes()));
        assert_eq!(cell(&text, 0, 3), "6");
        assert_eq!(cell(&text, 1, 3), "20");
    });
    session.update(view, |session, cx| {
        session.undo(cx).unwrap();
    });
    view.read(|cx| {
        let snapshot = session.read(cx).snapshot();
        assert_eq!(
            snapshot.copy_range(ByteRange::new(0, snapshot.len_bytes())),
            source
        );
    });
}

#[gpui::test]
fn marked_row_recalculates_on_table_navigation_in_one_undo_step(cx: &mut gpui::TestAppContext) {
    cx.update(crate::editor::init);
    let source = "| # | 2 | 0 |\n#+TBLFM: $3=$2*2\n";
    let session = cx.new(|_| {
        DocumentSession::from_utf8("marked.org".into(), source.as_bytes().to_vec()).unwrap()
    });
    let editor = cx.new(|cx| crate::editor::SemanticEditor::new(session.clone(), cx));
    editor.update(cx, |editor, cx| {
        let caret = ByteOffset(source.find("2 | 0").unwrap() as u64);
        editor.set_selection(crate::document::Selection::caret(caret), cx);
        let snapshot = editor.snapshot(cx);
        let context =
            EditorCommandContext::at(std::path::Path::new("marked.org"), &snapshot, caret).unwrap();
        assert!(editor.align_table_from_context(
            &snapshot,
            &context,
            TableNavigation::NextCell,
            cx
        ));
    });
    let text = session.read_with(cx, |session, _| {
        let snapshot = session.snapshot();
        snapshot.copy_range(ByteRange::new(0, snapshot.len_bytes()))
    });
    assert_eq!(cell(&text, 0, 2), "4");
    assert_eq!(session.read_with(cx, |session, _| session.revision().0), 1);
    session.update(cx, |session, cx| {
        session.undo(cx).unwrap();
    });
    let text = session.read_with(cx, |session, _| {
        let snapshot = session.snapshot();
        snapshot.copy_range(ByteRange::new(0, snapshot.len_bytes()))
    });
    assert_eq!(text, source);
}
