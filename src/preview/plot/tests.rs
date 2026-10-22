use super::*;
use crate::document::{DocumentSnapshot, TextSnapshot};
use gpui::AppContext;
#[gpui::test]
fn reading_plot_is_a_zero_source_range_after_the_visible_table(cx: &mut gpui::TestAppContext) {
    fn entry(panel: &ReadingPreviewPanel) -> Arc<PlotEntry> {
        panel
            .document()
            .projection
            .rows
            .iter()
            .find_map(|row| {
                if let super::super::projection::VisualRowKind::Plot(plot) = &row.kind {
                    Some(plot.entry.clone())
                } else {
                    None
                }
            })
            .unwrap()
    }
    let source =
        "#+PLOT: ind:1 with:histograms\n| x | y |\n|---+---|\n| a | 2 |\n| b | 3 |\n\nAfter.\n";
    let mut buffer =
        crate::document::DocumentBuffer::from_utf8(source.as_bytes().to_vec()).unwrap();
    let snapshot = buffer.snapshot();
    let preview = Arc::new(super::super::loading::derive_preview(
        std::path::PathBuf::from("plot.org"),
        snapshot.clone(),
    ));
    let base_count = preview.projection.rows.len();
    let panel = cx.new(|cx| {
        let mut panel = ReadingPreviewPanel::new(preview.clone(), 80.0);
        panel.attach_plots(None, cx);
        panel
    });
    cx.run_until_parked();
    cx.executor()
        .advance_clock(std::time::Duration::from_millis(81));
    cx.run_until_parked();
    cx.read(|cx| {
        let document = panel.read(cx).document();
        assert_eq!(document.projection.rows.len(), base_count + 1);
        let (index, row) = document
            .projection
            .rows
            .iter()
            .enumerate()
            .find(|(_, row)| matches!(row.kind, super::super::projection::VisualRowKind::Plot(_)))
            .unwrap();
        let range = document.projection.source_row(index).unwrap().content.range;
        assert!(range.is_empty());
        assert_eq!(range.start.0, source.find("\n\nAfter").unwrap() as u64 + 1);
        assert!(matches!(
            document.projection.rows.get(index - 1).unwrap().kind,
            super::super::projection::VisualRowKind::Table(_)
        ));
        assert_eq!(row.source.revision, snapshot.revision());
        assert_eq!(document.text.len_bytes(), snapshot.len_bytes());
        assert!(Arc::ptr_eq(panel.read(cx).derived_document(), &preview));
    });
    let replacement = Arc::new((*preview).clone());
    panel.update(cx, |panel, cx| {
        panel.replace_document_with_style(
            replacement.clone(),
            *super::super::preview_style(super::super::PreviewStyleId::Base),
            cx,
        );
        panel.attach_plots(None, cx);
        let augmented = panel.document().clone();
        assert_eq!(augmented.projection.rows.len(), base_count + 1);
        assert!(Arc::ptr_eq(panel.derived_document(), &replacement));
        panel.attach_plots(None, cx);
        assert!(Arc::ptr_eq(panel.document(), &augmented));
    });
    let before = panel.read_with(cx, |panel, _| entry(panel));
    let start = source.find("| a | 2").unwrap() as u64 + 6;
    let delta = buffer
        .commit(crate::document::EditTransaction::new(
            snapshot.revision(),
            vec![crate::document::TextEdit::new(
                crate::document::ByteRange::new(start, start + 1),
                "20 |\n| extra | 7",
            )],
        ))
        .unwrap();
    let updated = Arc::new(super::super::loading::derive_preview(
        "plot.org".into(),
        buffer.snapshot(),
    ));
    panel.update(cx, |panel, cx| {
        panel.map_viewport_through_delta(&delta);
        panel.replace_document_with_style(
            updated,
            *super::super::preview_style(super::super::PreviewStyleId::Base),
            cx,
        );
        panel.attach_plots(None, cx);
        assert!(!panel.plots.as_ref().unwrap().is_current());
        assert_eq!(
            entry(panel).id,
            before.id,
            "keep the existing image lease until the new plan is ready"
        );
    });
    cx.run_until_parked();
    cx.executor()
        .advance_clock(std::time::Duration::from_millis(81));
    cx.run_until_parked();
    panel.read_with(cx, |panel, _| {
        let after = entry(panel);
        assert_eq!(after.id, before.id);
        let crate::plot::PlotPreparation::Ready(old_plan) = &before.prepared else {
            panic!("initial chart")
        };
        let crate::plot::PlotPreparation::Ready(new_plan) = &after.prepared else {
            panic!("updated chart")
        };
        assert_ne!(
            old_plan.content_key, new_plan.content_key,
            "publish the new plan even when plot identities stay the same"
        );
    });
}

#[gpui::test]
fn linked_plot_uses_the_real_image_row_and_resizes_through_org_attributes(
    cx: &mut gpui::TestAppContext,
) {
    let source = "#+PLOT: ind:1 file:\"images/chart.svg\"\n| x | y |\n|---+---|\n| 1 | 2 |\n\n#+ATTR_ORG: :width 320\n[[file:./images/chart.svg]]\n";
    let snapshot = DocumentSnapshot::from_utf8(source.as_bytes().to_vec()).unwrap();
    let preview = Arc::new(super::super::loading::derive_preview(
        "plot.org".into(),
        snapshot.clone(),
    ));
    let base_count = preview.projection.rows.len();
    let panel = cx.new(|cx| {
        let mut panel = ReadingPreviewPanel::new(preview, 80.0);
        panel.attach_plots(None, cx);
        panel
    });
    cx.run_until_parked();
    cx.executor()
        .advance_clock(std::time::Duration::from_millis(81));
    cx.run_until_parked();
    panel.update(cx, |panel, cx| {
        assert_eq!(panel.document().projection.rows.len(), base_count);
        let (index, visual) = panel
            .document()
            .projection
            .rows
            .iter()
            .enumerate()
            .find(|(_, row)| matches!(row.kind, super::super::projection::VisualRowKind::Plot(_)))
            .unwrap();
        let super::super::projection::VisualRowKind::Plot(projection) = &visual.kind else {
            unreachable!()
        };
        let projection = projection.clone();
        assert_eq!(
            snapshot.copy_range(visual.source.range).trim(),
            "[[file:./images/chart.svg]]"
        );
        let target = super::super::source_action_target(panel.document(), visual);
        let plot = panel
            .plot_presentation(
                &projection,
                false,
                900.0,
                *super::super::preview_style(super::super::PreviewStyleId::Base),
                cx,
            )
            .unwrap();
        assert_eq!(plot.width, 320.0);
        panel.start_plot_resize(&plot, 100.0, 900.0, Some(target));
        panel.move_plot_resize(180.0, cx);
        let action = panel.finish_plot_resize(cx).unwrap();
        assert!(matches!(
            action,
            super::super::PreviewAction::ResizeImage {
                width: Some(400),
                ..
            }
        ));
        assert_eq!(panel.document().revision, snapshot.revision());
        assert_eq!(
            panel
                .document()
                .display_map
                .as_ref()
                .unwrap()
                .image_size(index, 900.0)
                .unwrap()
                .0,
            400.0
        );
    });
}
