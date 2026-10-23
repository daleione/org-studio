use super::*;
use crate::{
    document::{
        ByteRange, DocumentBuffer, DocumentSnapshot, EditTransaction, TextEdit, TextSnapshot,
    },
    plot::{PlotRenderRequest, PlotStyle, output, render_plot},
};

#[gpui::test]
fn resized_plots_keep_their_sizes_when_sql_results_are_replaced(cx: &mut gpui::TestAppContext) {
    for (named, with_delta) in [(true, true), (true, false), (false, true), (false, false)] {
        let result = |name: &str, hash: &str, rows: &str| {
            let declaration = if named {
                format!("#+name: {name}\n")
            } else {
                String::new()
            };
            let marker = if named { name } else { "" };
            format!(
                "{declaration}#+begin_src sql\nSELECT '{name}' AS label;\n#+end_src\n\n#+RESULTS{hash}: {marker}\n#+PLOT: ind:1 with:linespoints\n| x | y |\n|---+---|\n{rows}\n"
            )
        };
        let first = result("sample-a", "[old]", "| 1 | 2 |\n");
        let second = result("sample-b", "[old]", "| 1 | 2 |\n");
        let mut buffer =
            DocumentBuffer::from_utf8(format!("{first}{second}").into_bytes()).unwrap();
        let document = cx.update(|cx| PlotDocument::shared(Arc::new(buffer.snapshot()), cx));
        cx.run_until_parked();
        cx.executor().advance_clock(Duration::from_millis(81));
        cx.run_until_parked();
        let entries = cx.read(|cx| document.read(cx).entries.clone());
        for (entry, (width, height)) in entries.iter().zip([(620.0, 370.0), (750.0, 420.0)]) {
            document.update(cx, |document, cx| {
                document.resize_canvas(entry.id, Some(width), Some(height), false, cx);
            });
        }
        let replacement = result("sample-b", "", "| 1 | 20 |\n| 2 | 30 |\n");
        let delta = buffer
            .commit(EditTransaction::new(
                buffer.revision(),
                vec![TextEdit::new(
                    ByteRange::new(first.len() as u64, (first.len() + second.len()) as u64),
                    replacement,
                )],
            ))
            .unwrap();
        document.update(cx, |document, cx| {
            document.refresh(Arc::new(buffer.snapshot()), with_delta.then_some(delta), cx);
        });
        cx.run_until_parked();
        cx.executor().advance_clock(Duration::from_millis(81));
        cx.run_until_parked();
        document.read_with(cx, |document, _| {
            assert_eq!(document.entries.len(), 2);
            for (entry, previous) in document.entries.iter().zip(entries.iter()) {
                assert_eq!(
                    entry.id, previous.id,
                    "updating results must retain chart identity, with_delta={with_delta}"
                );
                assert!(Arc::ptr_eq(&entry.display, &previous.display));
            }
            let display = document.entries[1].display.lock().unwrap();
            assert_eq!(
                (display.requested_width, display.requested_height),
                (Some(750.0), Some(420.0))
            );
            let PlotPreparation::Ready(plot) = &document.entries[1].prepared else {
                panic!("updated chart must be ready");
            };
            assert_eq!(plot.plan.rows, 2);
            let PlotPreparation::Ready(previous) = &entries[1].prepared else {
                panic!("original chart must be ready");
            };
            assert_ne!(plot.content_key, previous.content_key);
        });
        // Reusing another chart's old data must not also take its size.
        let replacement = format!(
            "{}{}",
            result("sample-a", "", "| 1 | 20 |\n| 2 | 30 |\n"),
            result("sample-b", "", "| 1 | 2 |\n")
        );
        let delta = buffer
            .commit(EditTransaction::new(
                buffer.revision(),
                vec![TextEdit::new(
                    ByteRange::new(0, buffer.snapshot().len_bytes()),
                    replacement,
                )],
            ))
            .unwrap();
        document.update(cx, |document, cx| {
            document.refresh(Arc::new(buffer.snapshot()), with_delta.then_some(delta), cx);
        });
        cx.run_until_parked();
        cx.executor().advance_clock(Duration::from_millis(81));
        cx.run_until_parked();
        document.read_with(cx, |document, _| {
            for (entry, previous) in document.entries.iter().zip(entries.iter()) {
                assert_eq!(
                    entry.id, previous.id,
                    "equal data must not transfer a chart's size"
                );
                assert!(Arc::ptr_eq(&entry.display, &previous.display));
            }
        });
        let delta = buffer
            .commit(EditTransaction::new(
                buffer.revision(),
                vec![TextEdit::new(
                    ByteRange::new(0, 0),
                    result("sample-new", "", "| 1 | 2 |\n"),
                )],
            ))
            .unwrap();
        document.update(cx, |document, cx| {
            document.refresh(Arc::new(buffer.snapshot()), with_delta.then_some(delta), cx);
        });
        cx.run_until_parked();
        cx.executor().advance_clock(Duration::from_millis(81));
        cx.run_until_parked();
        document.read_with(cx, |document, _| {
            let added = &document.entries[0];
            assert!(
                entries.iter().all(|entry| entry.id != added.id),
                "new charts must not take another chart's identity"
            );
            let display = added.display.lock().unwrap();
            assert_eq!(
                (display.requested_width, display.requested_height),
                (None, None)
            );
            for (entry, previous) in document.entries[1..].iter().zip(entries.iter()) {
                assert_eq!(entry.id, previous.id);
                assert!(Arc::ptr_eq(&entry.display, &previous.display));
            }
        });
    }
}

#[gpui::test]
fn plot_file_output_survives_unrelated_edits_and_discards_obsolete_writes(
    cx: &mut gpui::TestAppContext,
) {
    let directory = tempfile::tempdir().unwrap();
    let source = "#+PLOT: ind:1 file:\"images/chart.svg\"\n| x | y |\n|---+---|\n| 1 | 2 |\n\n#+ATTR_ORG: :width 320\n[[file:./images/chart.svg]]\n";
    let mut buffer =
        crate::document::DocumentBuffer::from_utf8(source.as_bytes().to_vec()).unwrap();
    let snapshot = buffer.snapshot();
    let document = cx.update(|cx| PlotDocument::shared(Arc::new(snapshot.clone()), cx));
    document.update(cx, |document, _| {
        document.set_file_path(Some(directory.path().join("doc.org")))
    });
    cx.run_until_parked();
    cx.executor()
        .advance_clock(std::time::Duration::from_millis(81));
    cx.run_until_parked();
    assert_eq!(
        std::fs::read_dir(directory.path()).unwrap().count(),
        0,
        "opening must not write images"
    );
    let entry = cx.read(|cx| document.read(cx).entries[0].clone());
    assert_eq!(
        entry.source.image_link.unwrap().start.0 as usize,
        source.find("[[file:").unwrap()
    );
    document.update(cx, |document, cx| document.write_output(entry.id, cx));
    for text in ["\n", "unrelated text\n"] {
        let end = buffer.snapshot().len_bytes();
        let delta = buffer
            .commit(crate::document::EditTransaction::new(
                buffer.revision(),
                vec![crate::document::TextEdit::new(
                    crate::document::ByteRange::new(end, end),
                    text,
                )],
            ))
            .unwrap();
        document.update(cx, |document, cx| {
            document.refresh(Arc::new(buffer.snapshot()), Some(delta), cx)
        });
    }
    cx.run_until_parked();
    cx.executor()
        .advance_clock(std::time::Duration::from_millis(81));
    cx.run_until_parked();
    let path = directory.path().join("images/chart.svg");
    assert!(std::fs::read_to_string(&path).unwrap().contains("<svg"));
    let PlotPreparation::Ready(plot) = &entry.prepared else {
        panic!("valid plot")
    };
    let artifact = render_plot(
        crate::typst_runtime::shared_engine(),
        &PlotRenderRequest::natural(
            plot.clone(),
            PlotStyle::for_theme(&crate::theme::ORG_STUDIO_LIGHT, 12.0),
        ),
    )
    .unwrap();
    let cancelled = directory.path().join("cancelled/chart.svg");
    output::write_file(
        &cancelled,
        &artifact.svg,
        &Arc::new(std::sync::Mutex::new(2)),
        1,
    )
    .unwrap();
    assert!(
        !cancelled.parent().unwrap().exists(),
        "obsolete output must not create directories or files"
    );
    let before = std::fs::read(&path).unwrap();
    document.update(cx, |document, cx| {
        document.resize_canvas(entry.id, Some(600.0), Some(400.0), false, cx)
    });
    cx.run_until_parked();
    assert_eq!(std::fs::read(&path).unwrap(), before);
    assert_eq!(snapshot.len_bytes(), source.len() as u64);
    for (extension, signature) in [("png", &b"\x89PNG\r\n\x1a\n"[..]), ("pdf", &b"%PDF-"[..])] {
        let source = format!("#+PLOT: ind:1 file:\"chart.{extension}\"\n| 1 | 2 |\n| 2 | 3 |\n");
        let snapshot = DocumentSnapshot::from_utf8(source.into_bytes()).unwrap();
        let document = cx.update(|cx| PlotDocument::shared(Arc::new(snapshot), cx));
        document.update(cx, |document, _| {
            document.set_file_path(Some(directory.path().join("doc.org")))
        });
        cx.run_until_parked();
        cx.executor()
            .advance_clock(std::time::Duration::from_millis(81));
        cx.run_until_parked();
        let path = directory.path().join(format!("chart.{extension}"));
        assert!(!path.exists(), "opening must not write {extension} files");
        let id = cx.read(|cx| document.read(cx).entries[0].id);
        document.update(cx, |document, cx| document.write_output(id, cx));
        cx.run_until_parked();
        assert!(std::fs::read(&path).unwrap().starts_with(signature));
    }
}
