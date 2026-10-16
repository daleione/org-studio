//! Opt-in native Metal check. Pixel readback exists only in this diagnostic
//! feature, never in the production gesture/render loop.
use super::*;
use crate::document::{
    ByteRange, DocumentCommand, EditOrigin, EditTransaction, Selection, TextEdit,
};
use gpui::{AsyncApp, WindowBounds, WindowHandle, WindowOptions};
use std::io::{BufWriter, Write};
use std::path::{Path, PathBuf};

/// Opt-in timestamps at application paint, not estimated display presentation.
/// Buffer writes so collecting a real trackpad trace does not add per-frame I/O.
pub(super) struct GestureTrace {
    writer: Option<BufWriter<std::fs::File>>,
    started: Instant,
    last_paint: Option<Instant>,
}

impl Default for GestureTrace {
    fn default() -> Self {
        let writer = std::env::var_os("ORG_STUDIO_OVERVIEW_TRACE").map(|path| {
            let mut writer = BufWriter::new(std::fs::File::create(path).expect("create gesture trace"));
            writeln!(writer, "elapsed_ms,kind,state,delta,target_progress,display_progress,progress_velocity,left,top,width,height,paint_interval_ms").expect("write trace header");
            writer
        });
        Self {
            writer,
            started: Instant::now(),
            last_paint: None,
        }
    }
}

impl GestureTrace {
    pub fn enabled(&self) -> bool {
        self.writer.is_some()
    }

    pub fn input(&mut self, event: &gpui::PinchEvent) {
        if let Some(writer) = &mut self.writer {
            writeln!(
                writer,
                "{:.3},input,{:?},{:.8},,,,,,,,",
                self.started.elapsed().as_secs_f64() * 1000.,
                event.phase,
                event.delta
            )
            .expect("write gesture input");
            if matches!(event.phase, TouchPhase::Ended | TouchPhase::Cancelled) {
                writer.flush().expect("flush gesture trace");
            }
        }
    }

    pub fn paint(
        &mut self,
        phase: &str,
        target: f32,
        progress: f32,
        velocity: f32,
        bounds: Bounds<Pixels>,
    ) {
        if let Some(writer) = &mut self.writer {
            let now = Instant::now();
            let interval = self.last_paint.replace(now).map_or(f64::NAN, |last| {
                now.duration_since(last).as_secs_f64() * 1000.
            });
            writeln!(writer, "{:.3},paint,{phase},,{target:.6},{progress:.6},{velocity:.6},{:.4},{:.4},{:.4},{:.4},{interval:.3}", now.duration_since(self.started).as_secs_f64() * 1000., f32::from(bounds.left()), f32::from(bounds.top()), f32::from(bounds.size.width), f32::from(bounds.size.height)).expect("write gesture paint");
        }
    }

    fn flush(&mut self) {
        if let Some(writer) = &mut self.writer {
            writer.flush().expect("flush gesture trace");
        }
    }
}

pub fn run_visual_check(output: PathBuf) {
    std::fs::create_dir_all(&output).expect("create diagnostic output directory");
    gpui_platform::application().run(move |cx| {
        crate::editor::init(cx);
        crate::typography::init(crate::typography::FontSettings::default(), cx.text_system());
        crate::theme::set_theme_mode(crate::theme::ThemeMode::Light);
        let window = cx.open_window(WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(Bounds::new(point(px(30.), px(40.)), size(px(1280.), px(900.))))),
            titlebar: Some(gpui::TitlebarOptions { appears_transparent: true, ..Default::default() }),
            ..Default::default()
        }, |_, cx| cx.new(|cx| {
            let mut workspace = WorkspaceWindow::new();
            workspace.language = crate::i18n::Language::Chinese;
            workspace.theme_mode = crate::theme::ThemeMode::Light;
            workspace.minimap_visible = true;
            for (name, text) in [
                ("项目计划.org", "* 项目计划\n\n** TODO 完成文件总览\n\n- [X] 原布局预览\n- [ ] 测试手势\n- [ ] 完善细节\n\n* 里程碑\n\n| 阶段 | 结果 |\n|------+------|\n| 原型 | 完成 |\n| 实现 | 进行中 |\n"),
                ("研究笔记.md", concat!("# Org mode 的未来\n\nOrg mode 已经发展了十多年，依然充满活力。\n\n## 值得关注的方向\n\n- 更好的移动端体验\n- 与现代工具链的集成\n- 更友好的新手引导\n\n> Org 不只是一个工具，更是一种思考与组织的方式。\n\n![SVG preview](", env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/images/typst-demo.svg)\n")),
                ("收件箱.org", "* 收件箱\n\n** TODO 回复邮件\n** TODO 阅读文章\n** TODO 整理书桌\n"),
                ("README.md", "# Org Studio\n\n原生 Org mode 与 Markdown 工作区。\n\n## Features\n\n- Fast editing\n- Reading mode\n- Open Files overview\n\n\u{60}\u{60}\u{60}sh\ncargo test\n\u{60}\u{60}\u{60}\n"),
                ("每周回顾.org", "* 每周回顾\n\n** 本周完成\n\n- [X] 完成核心功能\n- [X] 与团队同步\n- [ ] 整理用户反馈\n\n** 下周计划\n\n- [ ] 发布测试版本\n- [ ] 准备演示\n"),
                ("未命名 2", "记录一些想法\n\n这是一个临时的草稿。\n- 这里可以写下任何内容\n- 之后再整理到正式的文件中\n"),
            ] {
                workspace.create_buffer(name.into(), None, cx);
                let doc = workspace.document_session().unwrap().clone();
                doc.update(cx, |doc, cx| {
                    doc.edit(DocumentCommand::new(
                        EditTransaction::new(doc.revision(), vec![TextEdit::new(ByteRange::new(0, 0), text)]),
                        Selection::default(), Selection::default(), EditOrigin::Other,
                    ), cx).unwrap();
                });
            }
            workspace
        })).expect("open native diagnostic window");
        cx.activate(true);
        cx.spawn(async move |cx| {
            if let Some(seconds) = std::env::var("ORG_STUDIO_OVERVIEW_INTERACTIVE").ok().and_then(|s| s.parse::<u64>().ok()) {
                eprintln!("interactive trackpad capture: {seconds} seconds; use the overview normally");
                pause(cx, seconds * 1000).await;
                let _ = window.update(cx, |w, _, _| w.file_overview.trace.flush());
                let _ = cx.update(|cx| cx.quit());
                return;
            }
            match scenario(window, &output, cx).await {
                Ok(()) => eprintln!("file-overview native paint check: passed; {}", output.display()),
                Err(error) => {
                    eprintln!("file-overview native paint check: FAILED: {error}");
                    std::process::exit(1);
                }
            }
            let _ = window.update(cx, |w, _, _| w.file_overview.trace.flush());
            let _ = cx.update(|cx| cx.quit());
        }).detach();
    });
}

async fn pause(cx: &AsyncApp, ms: u64) {
    cx.background_executor()
        .timer(Duration::from_millis(ms))
        .await;
}

fn pump_headless_frame(
    window: WindowHandle<WorkspaceWindow>,
    cx: &mut AsyncApp,
) -> Result<(), String> {
    cx.update_window(window.into(), |_, window, cx| {
        // Remote macOS can render with Metal while AppKit has no display link.
        // Supply its missing clock only in this opt-in diagnostic.
        if manual_clock() || window.display(cx).is_none() {
            window.simulate_next_frame(cx);
        }
    })
    .map_err(|e| e.to_string())
}

fn manual_clock() -> bool {
    std::env::var_os("ORG_STUDIO_OVERVIEW_DIAGNOSTIC_CLOCK").is_some()
}

async fn advance(
    window: WindowHandle<WorkspaceWindow>,
    cx: &mut AsyncApp,
    ms: u64,
) -> Result<(), String> {
    let until = Instant::now() + Duration::from_millis(ms);
    while Instant::now() < until {
        pump_headless_frame(window, cx)?;
        pause(cx, 16).await;
    }
    Ok(())
}

async fn wait_for(
    window: WindowHandle<WorkspaceWindow>,
    cx: &mut AsyncApp,
    label: &str,
    condition: impl Fn(&WorkspaceWindow) -> bool,
) -> Result<(), String> {
    for _ in 0..100 {
        pump_headless_frame(window, cx)?;
        if window
            .update(cx, |w, _, _| condition(w))
            .map_err(|e| e.to_string())?
        {
            return Ok(());
        }
        pause(cx, 30).await;
    }
    let state = window
        .update(cx, |w, window, cx| {
            let state = w
                .file_overview
                .session
                .as_ref()
                .map(|s| (s.phase, s.progress, !s.is_preparing(), s.motion.is_some()));
            (
                state,
                window.display(cx).map(|d| d.id()),
                window.is_window_active(),
                window
                    .frame_duration_snapshot()
                    .draw_duration_histogram
                    .len(),
            )
        })
        .map_err(|e| e.to_string())?;
    Err(format!("{label} timed out: {state:?}"))
}

fn screenshot(
    window: WindowHandle<WorkspaceWindow>,
    output: &Path,
    name: &str,
    cx: &mut AsyncApp,
) -> Result<(), String> {
    frame(window, cx)?
        .save(output.join(name))
        .map_err(|e| e.to_string())
}

fn frame(
    window: WindowHandle<WorkspaceWindow>,
    cx: &mut AsyncApp,
) -> Result<image::RgbaImage, String> {
    window
        .update(cx, |_, window, _| {
            window.render_to_image().map_err(|e| e.to_string())
        })
        .map_err(|e| e.to_string())?
}

async fn scenario(
    window: WindowHandle<WorkspaceWindow>,
    output: &Path,
    cx: &mut AsyncApp,
) -> Result<(), String> {
    advance(window, cx, 250).await?;
    let ids = window
        .update(cx, |w, _, cx| {
            w.buffer_sessions()
                .map(|doc| doc.read(cx).id())
                .collect::<Vec<_>>()
        })
        .map_err(|e| e.to_string())?;
    for id in ids.iter().copied() {
        window
            .update(cx, |w, _, cx| {
                w.activate_buffer(id, cx);
                let name = w.document_session().unwrap().read(cx).display_name();
                if name == "研究笔记.md" || name == "README.md" {
                    w.show_reading(cx);
                } else if name == "每周回顾.org" {
                    w.show_split(cx);
                } else {
                    w.show_editor(cx);
                }
            })
            .map_err(|e| e.to_string())?;
        advance(window, cx, 180).await?;
        let reading = window
            .update(cx, |w, _, cx| {
                w.document_session().unwrap().read(cx).display_name() == "研究笔记.md"
            })
            .map_err(|e| e.to_string())?;
        if reading {
            screenshot(window, output, "00-reading-svg.png", cx)?;
        }
        let probe = window
            .update(cx, |w, _, cx| {
                match w
                    .document_session()
                    .unwrap()
                    .read(cx)
                    .display_name()
                    .as_str()
                {
                    "研究笔记.md" => Some("reading"),
                    "每周回顾.org" => Some("split"),
                    _ => None,
                }
            })
            .map_err(|e| e.to_string())?;
        if let Some(probe) = probe {
            // Allow the SVG/image loader to finish before freezing its layout.
            advance(window, cx, 500).await?;
            screenshot(window, output, &format!("00-{probe}-native.png"), cx)?;
            window
                .update(cx, |w, window, cx| {
                    w.open_file_overview(window, cx);
                    let s = w.file_overview.session.as_mut().unwrap();
                    s.motion = None;
                    s.progress = 0.;
                })
                .map_err(|e| e.to_string())?;
            advance(window, cx, 40).await?;
            screenshot(window, output, &format!("00-{probe}-layout-1x.png"), cx)?;
            window
                .update(cx, |w, window, cx| w.close_file_overview_now(window, cx))
                .map_err(|e| e.to_string())?;
            advance(window, cx, 60).await?;
        }
    }
    let origin = window
        .update(cx, |w, _, cx| w.document_session().unwrap().read(cx).id())
        .map_err(|e| e.to_string())?;
    screenshot(window, output, "01-document.png", cx)?;
    let preparation_start = Instant::now();
    let prepared = window
        .update(cx, |w, window, cx| {
            w.open_file_overview(window, cx);
            let state = w.file_overview.session.as_mut().unwrap();
            state.phase = Phase::Entering;
            state.motion = None;
            state.progress = 0.;
            state.origin_frame.is_some()
        })
        .map_err(|e| e.to_string())?;
    if !prepared {
        return Err("production preview did not produce a document frame".into());
    }
    eprintln!(
        "file-overview source preparation CPU time: {:.3} ms",
        preparation_start.elapsed().as_secs_f64() * 1000.
    );
    advance(window, cx, 40).await?;
    screenshot(window, output, "01b-layout-original-scale.png", cx)?;
    for (progress, name) in [
        (0.15, "02-pinch-15.png"),
        (0.45, "02-pinch-midpoint.png"),
        (0.75, "02-pinch-75.png"),
        (1., "02-pinch-100.png"),
    ] {
        window
            .update(cx, |w, _, cx| {
                w.file_overview.session.as_mut().unwrap().progress = progress;
                cx.notify();
            })
            .map_err(|e| e.to_string())?;
        advance(window, cx, 60).await?;
        screenshot(window, output, name, cx)?;
    }
    window
        .update(cx, |w, _, cx| {
            let state = w.file_overview.session.as_mut().unwrap();
            state.phase = Phase::Visible;
            state.progress = 1.;
            cx.notify();
        })
        .map_err(|e| e.to_string())?;
    advance(window, cx, 100).await?;
    screenshot(window, output, "03-overview-light.png", cx)?;
    let target = ids.iter().copied().find(|id| *id != origin).unwrap();
    window
        .update(cx, |w, window, cx| {
            w.file_overview.cache.clear();
            w.select_overview_item(target, window, cx);
        })
        .map_err(|e| e.to_string())?;
    wait_for(window, cx, "cold original-layout preparation", |w| {
        w.file_overview
            .session
            .as_ref()
            .is_some_and(|state| !state.is_preparing() && state.target_frame.is_some())
    })
    .await?;
    window
        .update(cx, |w, _, cx| {
            if w.document_session().unwrap().read(cx).id() != origin {
                return Err("preparing a cold document changed the active session".to_owned());
            }
            let state = w.file_overview.session.as_mut().unwrap();
            state.motion = None;
            state.progress = 0.55;
            cx.notify();
            Ok(())
        })
        .map_err(|e| e.to_string())??;
    eprintln!("cold original-layout preparation: real frame ready; active document preserved");
    advance(window, cx, 100).await?;
    screenshot(window, output, "04-expand-midpoint.png", cx)?;
    window
        .update(cx, |w, window, cx| w.cancel_file_overview(window, cx))
        .map_err(|e| e.to_string())?;
    wait_for(window, cx, "cancel expansion", |w| {
        w.file_overview
            .session
            .as_ref()
            .is_some_and(|s| s.phase == Phase::Visible)
    })
    .await?;
    let cancelled = window
        .update(cx, |w, _, cx| {
            w.document_session().unwrap().read(cx).id() == origin
        })
        .map_err(|e| e.to_string())?;
    if !cancelled {
        return Err("cancelling expansion changed the active document".into());
    }
    window
        .update(cx, |w, window, cx| {
            w.select_overview_item(target, window, cx)
        })
        .map_err(|e| e.to_string())?;
    wait_for(window, cx, "commit selection", |w| {
        !w.file_overview_is_open()
    })
    .await?;
    let committed = window
        .update(cx, |w, _, cx| {
            !w.file_overview_is_open() && w.document_session().unwrap().read(cx).id() == target
        })
        .map_err(|e| e.to_string())?;
    if !committed {
        return Err("expansion did not commit exactly the selected document".into());
    }
    screenshot(window, output, "05-document-after-selection.png", cx)?;
    window
        .update(cx, |w, _, cx| {
            crate::theme::set_theme_mode(crate::theme::ThemeMode::Dark);
            w.theme_mode = crate::theme::ThemeMode::Dark;
            cx.notify();
        })
        .map_err(|e| e.to_string())?;
    advance(window, cx, 150).await?;
    window
        .update(cx, |w, window, cx| w.open_file_overview(window, cx))
        .map_err(|e| e.to_string())?;
    wait_for(window, cx, "dark overview entry", |w| {
        w.file_overview
            .session
            .as_ref()
            .is_some_and(|s| s.phase == Phase::Visible)
    })
    .await?;
    screenshot(window, output, "06-overview-dark.png", cx)?;
    window
        .update(cx, |_, window, _| window.resize(size(px(390.), px(800.))))
        .map_err(|e| e.to_string())?;
    advance(window, cx, 200).await?;
    window
        .update(cx, |w, window, cx| w.open_file_overview(window, cx))
        .map_err(|e| e.to_string())?;
    wait_for(window, cx, "narrow overview entry", |w| {
        w.file_overview
            .session
            .as_ref()
            .is_some_and(|s| s.phase == Phase::Visible)
    })
    .await?;
    screenshot(window, output, "07-overview-narrow.png", cx)?;
    // Reproduce two tall document frames, including one cached in the
    // background. Their borders must follow the original document ratio.
    window
        .update(cx, |w, window, cx| {
            w.close_file_overview_now(window, cx);
            crate::theme::set_theme_mode(crate::theme::ThemeMode::Light);
            w.theme_mode = crate::theme::ThemeMode::Light;
            window.resize(size(px(900.), px(1050.)));
            for id in ids.iter().copied().skip(2) {
                w.remove_buffer(id, cx);
            }
            cx.notify();
        })
        .map_err(|e| e.to_string())?;
    for id in ids.iter().copied().take(2) {
        window
            .update(cx, |w, _, cx| {
                w.activate_buffer(id, cx);
                w.show_editor(cx);
            })
            .map_err(|e| e.to_string())?;
        advance(window, cx, 180).await?;
    }
    window
        .update(cx, |w, window, cx| {
            w.open_file_overview(window, cx);
            let items = w.overview_items(cx);
            let grid = w.overview_grid(window, &items);
            for (index, item) in items.iter().enumerate() {
                let source = w.overview_item_size(item, window);
                let preview = grid.document_preview(index, px(0.), source);
                assert!(
                    (f32::from(preview.size.width / preview.size.height)
                        - f32::from(source.width / source.height))
                    .abs()
                        < 0.0001
                );
            }
        })
        .map_err(|e| e.to_string())?;
    wait_for(window, cx, "portrait overview", |w| {
        w.file_overview
            .session
            .as_ref()
            .is_some_and(|s| s.phase == Phase::Visible)
    })
    .await?;
    screenshot(window, output, "08-overview-portrait.png", cx)?;
    window
        .update(cx, |w, _, cx| {
            let state = w.file_overview.session.as_mut().unwrap();
            state.phase = Phase::Entering;
            state.motion = None;
            state.progress = 0.5;
            cx.notify();
        })
        .map_err(|e| e.to_string())?;
    advance(window, cx, 60).await?;
    screenshot(window, output, "09-pinch-portrait.png", cx)?;
    check_handoff_and_motion(window, output, &ids[..2], cx).await?;
    check_pinch_follow(window, output, cx).await?;
    window.update(cx, |_, window, _| {
        let timings = window.frame_duration_snapshot();
        eprintln!(
            "file-overview diagnostic frames={} draw_p50_ms={:.3} draw_p95_ms={:.3} present_interval_p95_ms={:.3}",
            timings.draw_duration_histogram.len(),
            timings.draw_duration_histogram.value_at_quantile(0.50) as f64 / 1_000_000.,
            timings.draw_duration_histogram.value_at_quantile(0.95) as f64 / 1_000_000.,
            timings.present_interval_histogram.value_at_quantile(0.95) as f64 / 1_000_000.,
        );
    }).map_err(|e| e.to_string())?;
    Ok(())
}

/// Call our handler with irregular GPUI input intervals, without posting OS
/// events. Held samples send no input, so convergence requires frame ticks.
async fn trace_pinch(
    window: WindowHandle<WorkspaceWindow>,
    position: gpui::Point<Pixels>,
    label: &str,
    inputs: &[(Option<f32>, TouchPhase)],
    trace: &mut String,
    cx: &mut AsyncApp,
) -> Result<Vec<[f32; 3]>, String> {
    let start = Instant::now();
    let mut samples = Vec::new();
    for (index, &(delta, phase)) in inputs.iter().enumerate() {
        pause(cx, [6, 8, 12, 7][index % 4]).await;
        pump_headless_frame(window, cx)?;
        let sample = window
            .update(cx, |w, window, cx| {
                if let Some(delta) = delta {
                    w.file_overview_pinch(
                        &gpui::PinchEvent {
                            position,
                            delta,
                            phase,
                            modifiers: Default::default(),
                        },
                        window,
                        cx,
                    );
                }
                w.file_overview.session.as_ref().map(|s| {
                    let target = s.gesture.as_ref().map_or(f32::NAN, Gesture::progress);
                    let velocity = s.follow.map_or(0., |f| f.sample(Instant::now()).velocity);
                    [target, s.progress, velocity]
                })
            })
            .map_err(|e| e.to_string())?;
        if let Some([target, displayed, velocity]) = sample {
            trace.push_str(&format!(
                "{label},{:.3},{target:.6},{displayed:.6},{velocity:.6}\n",
                start.elapsed().as_secs_f64() * 1000.
            ));
            samples.push([target, displayed, velocity]);
        }
    }
    Ok(samples)
}

async fn check_pinch_follow(
    window: WindowHandle<WorkspaceWindow>,
    output: &Path,
    cx: &mut AsyncApp,
) -> Result<(), String> {
    let (origin, position) = window
        .update(cx, |w, window, cx| {
            (
                w.document_session().unwrap().read(cx).id(),
                w.overview_stage_bounds(window).center(),
            )
        })
        .map_err(|e| e.to_string())?;
    let mut trace = "sequence,elapsed_ms,target_progress,rendered_progress,velocity\n".to_owned();
    let movement = vec![(Some(-0.014), TouchPhase::Moved); 20];
    let moving = trace_pinch(window, position, "inward", &movement, &mut trace, cx).await?;
    let last = moving
        .last()
        .ok_or("pinch did not claim the source document")?;
    if last[0] < 0.25 || last[1] >= last[0] - 0.01 {
        return Err(format!("pinch did not retain a small visual lag: {last:?}"));
    }
    let held = trace_pinch(
        window,
        position,
        "hold-without-input",
        &vec![(None, TouchPhase::Moved); 24],
        &mut trace,
        cx,
    )
    .await?;
    let held = held.last().ok_or("lost the held gesture")?;
    if (held[0] - held[1]).abs() > 0.005 {
        return Err(format!(
            "held pinch did not converge without input: {held:?}"
        ));
    }
    let mut reverse = vec![(Some(0.022), TouchPhase::Moved); 5];
    reverse.push((Some(0.), TouchPhase::Ended));
    trace_pinch(
        window,
        position,
        "reverse-and-release",
        &reverse,
        &mut trace,
        cx,
    )
    .await?;
    wait_for(window, cx, "reversed inward gesture", |w| {
        !w.file_overview_is_open()
    })
    .await?;
    let preserved = window
        .update(cx, |w, _, cx| {
            w.document_session().unwrap().read(cx).id() == origin
        })
        .map_err(|e| e.to_string())?;
    if !preserved {
        return Err("reversing a filtered pinch changed the document".into());
    }
    let mut commit = vec![(Some(-0.014), TouchPhase::Moved); 24];
    commit.push((Some(0.), TouchPhase::Ended));
    trace_pinch(
        window,
        position,
        "inward-release-to-regrab",
        &commit,
        &mut trace,
        cx,
    )
    .await?;
    let mut regrab = vec![(Some(0.), TouchPhase::Started)];
    regrab.extend(vec![(Some(0.03), TouchPhase::Moved); 5]);
    regrab.push((Some(0.), TouchPhase::Ended));
    trace_pinch(
        window,
        position,
        "inward-regrab-and-reverse",
        &regrab,
        &mut trace,
        cx,
    )
    .await?;
    wait_for(window, cx, "regrab inward settling", |w| {
        !w.file_overview_is_open()
    })
    .await?;
    trace_pinch(window, position, "inward-commit", &commit, &mut trace, cx).await?;
    wait_for(window, cx, "filtered overview entry", |w| {
        w.file_overview
            .session
            .as_ref()
            .is_some_and(|s| s.phase == Phase::Visible)
    })
    .await?;
    let (target, position) = window
        .update(cx, |w, window, cx| {
            let items = w.overview_items(cx);
            let (index, item) = items
                .iter()
                .enumerate()
                .find(|(_, item)| item.id != origin)
                .unwrap();
            let grid = w.overview_grid(window, &items);
            let source = w.overview_item_size(item, window);
            (
                item.id,
                grid.document_preview(index, px(0.), source).center(),
            )
        })
        .map_err(|e| e.to_string())?;
    let mut expansion = vec![(Some(0.018), TouchPhase::Moved); 24];
    expansion.push((Some(0.), TouchPhase::Ended));
    trace_pinch(
        window,
        position,
        "outward-release-to-regrab",
        &expansion,
        &mut trace,
        cx,
    )
    .await?;
    let mut regrab = vec![(Some(0.), TouchPhase::Started)];
    regrab.extend(vec![(Some(-0.03), TouchPhase::Moved); 5]);
    regrab.push((Some(0.), TouchPhase::Ended));
    trace_pinch(
        window,
        position,
        "outward-regrab-and-reverse",
        &regrab,
        &mut trace,
        cx,
    )
    .await?;
    wait_for(window, cx, "regrab outward settling", |w| {
        w.file_overview
            .session
            .as_ref()
            .is_some_and(|s| s.phase == Phase::Visible)
    })
    .await?;
    if !window
        .update(cx, |w, _, cx| {
            w.document_session().unwrap().read(cx).id() == origin
        })
        .map_err(|e| e.to_string())?
    {
        return Err("regrabbing an expansion activated its target".into());
    }
    for cancel in [true, false] {
        let mut expansion = vec![(Some(0.018), TouchPhase::Moved); 24];
        if !cancel {
            expansion.push((Some(0.), TouchPhase::Ended));
        }
        trace_pinch(
            window,
            position,
            if cancel {
                "outward-escape"
            } else {
                "outward-commit"
            },
            &expansion,
            &mut trace,
            cx,
        )
        .await?;
        if cancel {
            window
                .update(cx, |w, window, cx| {
                    w.overview_key_command("escape", Default::default(), window, cx)
                })
                .map_err(|e| e.to_string())?;
            wait_for(window, cx, "cancel filtered expansion", |w| {
                w.file_overview
                    .session
                    .as_ref()
                    .is_some_and(|s| s.phase == Phase::Visible)
            })
            .await?;
            let preserved = window
                .update(cx, |w, _, cx| {
                    w.document_session().unwrap().read(cx).id() == origin
                })
                .map_err(|e| e.to_string())?;
            if !preserved {
                return Err("Escape committed a filtered expansion".into());
            }
        } else {
            wait_for(window, cx, "commit filtered expansion", |w| {
                !w.file_overview_is_open()
            })
            .await?;
            let committed = window
                .update(cx, |w, _, cx| {
                    w.document_session().unwrap().read(cx).id() == target
                })
                .map_err(|e| e.to_string())?;
            if !committed {
                return Err("filtered expansion committed the wrong document".into());
            }
        }
    }
    std::fs::write(output.join("pinch-trace.csv"), trace).map_err(|e| e.to_string())?;
    eprintln!(
        "scripted native pinch: filtered input, hold without events, reversal, Escape and selection passed"
    );
    Ok(())
}

async fn check_handoff_and_motion(
    window: WindowHandle<WorkspaceWindow>,
    output: &Path,
    ids: &[DocumentId],
    cx: &mut AsyncApp,
) -> Result<(), String> {
    let mut colour_report = String::new();
    for (name, theme) in [
        ("light", crate::theme::ThemeMode::Light),
        ("dark", crate::theme::ThemeMode::Dark),
    ] {
        window
            .update(cx, |w, window, cx| {
                w.close_file_overview_now(window, cx);
                crate::theme::set_theme_mode(theme);
                w.theme_mode = theme;
                w.invalidate_file_overview_frames();
                w.apply_reading_style(crate::preview::PreviewStyleId::WarmClay, cx);
                cx.notify();
            })
            .map_err(|e| e.to_string())?;
        for id in ids.iter().copied() {
            window
                .update(cx, |w, _, cx| {
                    w.activate_buffer(id, cx);
                    w.show_reading(cx);
                })
                .map_err(|e| e.to_string())?;
            advance(window, cx, 180).await?;
        }
        for returning in [false, true] {
            let action = if returning { "return" } else { "open" };
            window
                .update(cx, |w, window, cx| w.open_file_overview(window, cx))
                .map_err(|e| e.to_string())?;
            wait_for(window, cx, "warm reading overview", |w| {
                w.file_overview
                    .session
                    .as_ref()
                    .is_some_and(|s| s.phase == Phase::Visible)
            })
            .await?;
            window
                .update(cx, |w, window, cx| {
                    if returning {
                        w.cancel_file_overview(window, cx);
                    } else {
                        w.select_overview_item(ids[0], window, cx);
                    }
                })
                .map_err(|e| e.to_string())?;
            wait_for(window, cx, "prepared expansion endpoint", |w| {
                w.file_overview
                    .session
                    .as_ref()
                    .is_some_and(|s| !s.is_preparing())
            })
            .await?;
            window
                .update(cx, |w, _, cx| {
                    let state = w.file_overview.session.as_mut().unwrap();
                    state.motion = None;
                    state.progress = if returning { 0. } else { 1. };
                    cx.notify();
                })
                .map_err(|e| e.to_string())?;
            advance(window, cx, 40).await?;
            let endpoint = frame(window, cx)?;
            endpoint
                .save(output.join(format!("10-handoff-{name}-{action}-zoom-end.png")))
                .map_err(|e| e.to_string())?;
            window
                .update(cx, |w, _, cx| {
                    let state = w.file_overview.session.as_mut().unwrap();
                    state.motion = Some(Motion::settle(
                        Instant::now(),
                        state.progress,
                        state.progress,
                        0.,
                        true,
                    ));
                    cx.notify();
                })
                .map_err(|e| e.to_string())?;
            wait_for(window, cx, "live document handoff", |w| {
                w.file_overview
                    .session
                    .as_ref()
                    .is_some_and(|s| s.phase == Phase::Handoff)
            })
            .await?;
            let (bounds, scale) = window
                .update(cx, |w, window, _| {
                    (
                        w.file_overview.session.as_ref().unwrap().target_to,
                        window.scale_factor(),
                    )
                })
                .map_err(|e| e.to_string())?;
            let mut probes = Vec::new();
            let mut frames = Vec::new();
            for index in 0..6 {
                let pixels = frame(window, cx)?;
                let pixel = pixels
                    .get_pixel(pixels.width() * 3 / 5, pixels.height() * 3 / 4)
                    .0;
                probes.push(pixel);
                pixels
                    .save(output.join(format!("10-handoff-{name}-{action}-{index}.png")))
                    .map_err(|e| e.to_string())?;
                frames.push(pixels);
                advance(window, cx, 16).await?;
            }
            wait_for(window, cx, "finish live document handoff", |w| {
                !w.file_overview_is_open()
            })
            .await?;
            let pixels = frame(window, cx)?;
            pixels
                .save(output.join(format!("10-handoff-{name}-{action}-final.png")))
                .map_err(|e| e.to_string())?;
            let baseline = pixels
                .get_pixel(pixels.width() * 3 / 5, pixels.height() * 3 / 4)
                .0;
            let largest_change = probes
                .iter()
                .flat_map(|pixel| {
                    (0..3).map(move |channel| pixel[channel].abs_diff(baseline[channel]))
                })
                .max()
                .unwrap_or(0);
            colour_report.push_str(&format!("- {name} Warm Clay / {action}: final RGBA={baseline:?}, maximum background change={largest_change}/255 across 6 handoff frames.\n"));
            let (maximum, changed, _) =
                handoff_pixel_difference(&endpoint, &pixels, bounds, scale, 70.);
            colour_report.push_str(&format!(
                "  - Zoom endpoint: body max={maximum}/255, body pixels >8/255={changed}.\n"
            ));
            if maximum > 2 {
                return Err(format!(
                    "{name}/{action} zoom endpoint differs from the live document by {maximum}/255"
                ));
            }
            for (index, sample) in frames.iter().enumerate() {
                // Check text and the minimap as well as blank background. Also
                // report the status area, whose live navigation state may change
                // when a document is activated.
                let (maximum, changed, mean) =
                    handoff_pixel_difference(sample, &pixels, bounds, scale, 70.);
                let (all_maximum, all_changed, _) =
                    handoff_pixel_difference(sample, &pixels, bounds, scale, 0.);
                colour_report.push_str(&format!("  - Frame {index}: body max={maximum}/255, body pixels >8/255={changed}, body mean={mean:.6}/255, complete document max={all_maximum}/255, complete document pixels >8/255={all_changed}.\n"));
                if maximum > 2 {
                    std::fs::write(output.join("handoff-pixels.org"), &colour_report)
                        .map_err(|e| e.to_string())?;
                    return Err(format!(
                        "{name}/{action} handoff changed document text by {maximum}/255 in frame {index}"
                    ));
                }
            }
            if largest_change > 1 {
                return Err(format!(
                    "{name} handoff changed the page background by {largest_change}: {probes:?}, final={baseline:?}"
                ));
            }
        }
    }
    std::fs::write(output.join("handoff-pixels.org"), &colour_report).map_err(|e| e.to_string())?;
    // Exclude startup, screenshot readback, font loading and the colour probes
    // above. Measure actual automatic motion with the production frame source.
    let before = window
        .update(cx, |_, window, _| window.frame_duration_snapshot())
        .map_err(|e| e.to_string())?;
    for _ in 0..3 {
        window
            .update(cx, |w, window, cx| w.open_file_overview(window, cx))
            .map_err(|e| e.to_string())?;
        wait_for(window, cx, "measured overview entry", |w| {
            w.file_overview
                .session
                .as_ref()
                .is_some_and(|s| s.phase == Phase::Visible)
        })
        .await?;
        let target = window
            .update(cx, |w, _, cx| {
                let current = w.document_session().unwrap().read(cx).id();
                *ids.iter().find(|id| **id != current).unwrap()
            })
            .map_err(|e| e.to_string())?;
        window
            .update(cx, |w, window, cx| {
                w.select_overview_item(target, window, cx)
            })
            .map_err(|e| e.to_string())?;
        wait_for(window, cx, "measured overview expansion", |w| {
            !w.file_overview_is_open()
        })
        .await?;
    }
    let (mut after, display) = window
        .update(cx, |_, window, cx| {
            (
                window.frame_duration_snapshot(),
                window.display(cx).map(|display| display.id()),
            )
        })
        .map_err(|e| e.to_string())?;
    after
        .draw_duration_histogram
        .subtract(&before.draw_duration_histogram)
        .map_err(|e| e.to_string())?;
    after
        .present_interval_histogram
        .subtract(&before.present_interval_histogram)
        .map_err(|e| e.to_string())?;
    let metrics = format!(
        "* Open Files motion check\n\nBuild: {}. Display: {display:?}. Three entry / expansion cycles.\nClock: {}.\n\n- Frames: {}\n- CPU draw P50: {:.3} ms\n- CPU draw P95: {:.3} ms\n- Presentation interval P95: {:.3} ms\n\n{}\nThese measurements exclude startup and pixel readback. They do not measure physical trackpad input latency or establish 120 Hz performance. Manually driven intervals are diagnostic only and must not be treated as native frame pacing.\n",
        if cfg!(debug_assertions) {
            "debug"
        } else {
            "release"
        },
        if manual_clock() {
            "manually supplied diagnostic callbacks"
        } else {
            "native display callbacks (headless fallback if no display)"
        },
        after.draw_duration_histogram.len(),
        after.draw_duration_histogram.value_at_quantile(0.50) as f64 / 1_000_000.,
        after.draw_duration_histogram.value_at_quantile(0.95) as f64 / 1_000_000.,
        after.present_interval_histogram.value_at_quantile(0.95) as f64 / 1_000_000.,
        colour_report
    );
    eprintln!("{metrics}");
    std::fs::write(output.join("motion-metrics.org"), metrics).map_err(|e| e.to_string())?;
    Ok(())
}

fn handoff_pixel_difference(
    sample: &image::RgbaImage,
    reference: &image::RgbaImage,
    bounds: Bounds<Pixels>,
    scale: f32,
    bottom_inset: f32,
) -> (u8, usize, f64) {
    let left = (f32::from(bounds.left()) * scale).max(0.) as u32;
    let top = (f32::from(bounds.top()) * scale).max(0.) as u32;
    let right = ((f32::from(bounds.right()) * scale) as u32).min(reference.width());
    let bottom = (((f32::from(bounds.bottom()) - bottom_inset) * scale).max(top as f32) as u32)
        .min(reference.height());
    let mut maximum = 0;
    let mut changed = 0;
    let mut total = 0u64;
    for y in top..bottom {
        for x in left..right {
            let a = sample.get_pixel(x, y);
            let b = reference.get_pixel(x, y);
            let difference = std::array::from_fn::<_, 3, _>(|c| a[c].abs_diff(b[c]));
            let largest = *difference.iter().max().unwrap();
            maximum = maximum.max(largest);
            changed += usize::from(largest > 8);
            total += difference.into_iter().map(u64::from).sum::<u64>();
        }
    }
    let channels = (right - left) as u64 * (bottom - top) as u64 * 3;
    (maximum, changed, total as f64 / channels.max(1) as f64)
}
