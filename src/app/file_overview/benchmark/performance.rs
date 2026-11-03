//! Repeatable native-window measurements without screenshot readback.
use super::*;

pub(super) fn run(output: PathBuf, count: usize) {
    assert!(
        count >= 2,
        "overview measurements need at least two documents"
    );
    std::fs::create_dir_all(&output).unwrap();
    let paths = if let Some(inputs) = std::env::var_os("ORG_STUDIO_OVERVIEW_PERF_INPUTS") {
        let paths: Vec<PathBuf> =
            serde_json::from_reader(std::fs::File::open(inputs).unwrap()).unwrap();
        assert!(paths.len() >= count, "not enough input documents");
        paths.into_iter().take(count).collect::<Vec<_>>()
    } else {
        let directory = output.join("workspace");
        std::fs::create_dir_all(&directory).unwrap();
        let text = (0..180)
            .map(|index| {
                format!(
                    "* Topic {index}\n\nNotes for the next review, with links and examples.\n\n- TODO Review the draft\n- DONE Collect references\n\n"
                )
            })
            .collect::<String>();
        (0..count)
            .map(|index| {
                let path = directory.join(format!("document-{index:03}.org"));
                std::fs::write(&path, &text).unwrap();
                path
            })
            .collect::<Vec<_>>()
    };
    let inputs = paths.clone();
    let trace = crate::perf_tracing::PerfTrace::install();
    gpui_platform::application().run(move |cx| {
        crate::editor::init(cx);
        crate::typography::init(Default::default(), cx.text_system());
        crate::theme::set_theme_mode(crate::theme::ThemeMode::Light);
        let window = cx
            .open_window(
                WindowOptions {
                    window_bounds: Some(WindowBounds::Windowed(Bounds::new(
                        point(px(30.), px(40.)),
                        size(px(1280.), px(900.)),
                    ))),
                    inactive_frame_interval: None,
                    ..Default::default()
                },
                |window, cx| {
                    cx.new(|cx| {
                        let mut workspace = WorkspaceWindow::new();
                        for path in paths {
                            let loaded =
                                crate::preview::load_workspace_document(path, false).unwrap();
                            assert!(workspace.apply_load_result(0, Ok(loaded), cx));
                        }
                        let first = workspace.buffers.opened_ids().next().unwrap();
                        workspace.activate_buffer(first, cx);
                        window.activate_window();
                        workspace
                    })
                },
            )
            .unwrap();
        cx.activate(true);
        cx.spawn(async move |cx| {
            match measure(window, cx).await {
                Ok(metrics) => {
                    let report = serde_json::json!({
                        "documents": count,
                        "release": !cfg!(debug_assertions),
                        "manual_clock": manual_clock(),
                        "inputs": inputs,
                        "measurements": metrics,
                    });
                    let report = serde_json::to_string_pretty(&report).unwrap();
                    std::fs::write(output.join("performance.json"), &report).unwrap();
                    eprintln!("{report}");
                }
                Err(error) => {
                    eprintln!("overview performance check failed: {error}");
                    std::process::exit(1);
                }
            }
            cx.update(|cx| cx.quit());
        })
        .detach();
    });
    if let Some(trace) = trace {
        trace.report();
    }
}

async fn measure(
    window: WindowHandle<WorkspaceWindow>,
    cx: &mut AsyncApp,
) -> Result<Vec<serde_json::Value>, String> {
    advance(window, cx, 250).await?;
    let inputs = window
        .update(cx, |w, _, cx| {
            w.buffer_sessions()
                .map(|session| {
                    let revision = session.read(cx).revision();
                    (session, revision)
                })
                .collect::<Vec<_>>()
        })
        .map_err(|error| error.to_string())?;
    let ids = window
        .update(cx, |w, _, cx| {
            w.overview_items(cx)
                .iter()
                .map(|item| item.id)
                .collect::<Vec<_>>()
        })
        .map_err(|error| error.to_string())?;
    let mut results = Vec::new();
    for warm in [false, true] {
        if warm {
            for id in &ids {
                window
                    .update(cx, |w, _, cx| w.activate_buffer(*id, cx))
                    .map_err(|error| error.to_string())?;
                advance(window, cx, 80).await?;
            }
        }
        for cycle in 0..4 {
            let before = window
                .update(cx, |_, window, _| window.frame_duration_snapshot())
                .map_err(|error| error.to_string())?;
            window
                .update(cx, |w, window, cx| {
                    window.activate_window();
                    cx.activate(true);
                    w.open_file_overview(window, cx);
                })
                .map_err(|error| error.to_string())?;
            wait_for(window, cx, "entry", |w| {
                w.file_overview
                    .session
                    .as_ref()
                    .is_some_and(|state| state.phase == Phase::Visible)
            })
            .await?;
            let after = window
                .update(cx, |_, window, _| window.frame_duration_snapshot())
                .map_err(|error| error.to_string())?;
            results.push(sample("entry", warm, before, after.clone())?);

            for step in 0..24 {
                window
                    .update(cx, |w, _, cx| {
                        let state = w.file_overview.session.as_mut().unwrap();
                        state
                            .scroll
                            .set_offset(point(px(0.), px(-(step as f32) * 60.)));
                        cx.notify();
                    })
                    .map_err(|error| error.to_string())?;
                advance(window, cx, 16).await?;
            }
            let before_open = window
                .update(cx, |w, window, cx| {
                    let before = window.frame_duration_snapshot();
                    w.file_overview
                        .session
                        .as_mut()
                        .unwrap()
                        .scroll
                        .set_offset(point(px(0.), px(0.)));
                    cx.notify();
                    before
                })
                .map_err(|error| error.to_string())?;
            results.push(sample("scroll", warm, after, before_open.clone())?);
            advance(window, cx, 32).await?;
            window
                .update(cx, |w, window, cx| {
                    w.select_overview_item(ids[cycle % 2], window, cx)
                })
                .map_err(|error| error.to_string())?;
            wait_for(window, cx, "open", |w| !w.file_overview_is_open()).await?;
            let after_open = window
                .update(cx, |_, window, _| window.frame_duration_snapshot())
                .map_err(|error| error.to_string())?;
            results.push(sample("open", warm, before_open, after_open)?);
            let unchanged = cx.update(|cx| {
                inputs
                    .iter()
                    .all(|(session, revision)| session.read(cx).revision() == *revision)
            });
            if !unchanged {
                return Err("input document changed during measurement; discard this run".into());
            }
        }
    }
    Ok(results)
}

fn sample(
    action: &str,
    warm: bool,
    before: gpui::FrameDurationSnapshot,
    mut after: gpui::FrameDurationSnapshot,
) -> Result<serde_json::Value, String> {
    after
        .draw_duration_histogram
        .subtract(&before.draw_duration_histogram)
        .map_err(|error| error.to_string())?;
    after
        .present_interval_histogram
        .subtract(&before.present_interval_histogram)
        .map_err(|error| error.to_string())?;
    Ok(serde_json::json!({
        "action": action,
        "warm": warm,
        "frames": after.draw_duration_histogram.len(),
        "draw_p50_ms": after.draw_duration_histogram.value_at_quantile(0.5) as f64 / 1_000_000.,
        "draw_p95_ms": after.draw_duration_histogram.value_at_quantile(0.95) as f64 / 1_000_000.,
        "draw_max_ms": after.draw_duration_histogram.max() as f64 / 1_000_000.,
        "present_p95_ms": after.present_interval_histogram.value_at_quantile(0.95) as f64 / 1_000_000.,
    }))
}
