use super::{Context, Duration, Instant, PreviewApp, Window, px};

pub(in crate::preview) struct ScrollBenchmark {
    pub(in crate::preview) target_frames: usize,
    pub(in crate::preview) warmup_remaining: usize,
    pub(in crate::preview) sampling_started: bool,
    pub(in crate::preview) scroll_pixels: f32,
    pub(in crate::preview) samples: Vec<Duration>,
    pub(in crate::preview) last_frame: Instant,
}

impl PreviewApp {
    pub(in crate::preview) fn schedule_scroll_sample(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        cx.on_next_frame(window, |this, window, cx| {
            let now = Instant::now();
            let Some(benchmark) = this.scroll_benchmark.as_mut() else {
                return;
            };
            if benchmark.warmup_remaining > 0 {
                benchmark.warmup_remaining -= 1;
                if benchmark.warmup_remaining % 60 == 0 {
                    eprintln!(
                        "org_preview_scroll_warmup remaining={} display={:?} active={}",
                        benchmark.warmup_remaining,
                        window.display(cx).map(|display| display.id()),
                        window.is_window_active(),
                    );
                    // AppKit may briefly create the window on the current display
                    // before applying the requested benchmark bounds. Reactivate
                    // after migration so ProMotion does not throttle an otherwise
                    // deterministic, unattended sample as a background window.
                    window.activate_window();
                }
                benchmark.last_frame = now;
                this.schedule_scroll_sample(window, cx);
                cx.notify();
                return;
            }
            if !benchmark.sampling_started {
                benchmark.sampling_started = true;
                crate::perf_tracing::reset_samples();
                benchmark.last_frame = now;
                this.list_state.scroll_by(px(benchmark.scroll_pixels));
                this.schedule_scroll_sample(window, cx);
                cx.notify();
                return;
            }
            benchmark
                .samples
                .push(now.duration_since(benchmark.last_frame));
            benchmark.last_frame = now;
            if benchmark.samples.len() >= benchmark.target_frames {
                let mut samples = benchmark.samples.clone();
                samples.sort_unstable();
                let percentile = |p: f64| {
                    let index = ((samples.len() - 1) as f64 * p).ceil() as usize;
                    samples[index].as_secs_f64() * 1000.0
                };
                let cadence_ms = percentile(0.50);
                let late_frame_threshold_ms = cadence_ms * 1.5;
                let late_frames = samples
                    .iter()
                    .filter(|sample| {
                        sample.as_secs_f64() * 1000.0 > late_frame_threshold_ms
                    })
                    .count();
                let estimated_missed_vsyncs: u64 = samples
                    .iter()
                    .map(|sample| {
                        let elapsed_ms = sample.as_secs_f64() * 1000.0;
                        (elapsed_ms / cadence_ms).round().max(1.0) as u64 - 1
                    })
                    .sum();
                let over_12_5 = samples
                    .iter()
                    .filter(|sample| sample.as_secs_f64() * 1000.0 > 12.5)
                    .count();
                let over_16_67 = samples
                    .iter()
                    .filter(|sample| sample.as_secs_f64() * 1000.0 > 16.67)
                    .count();
                eprintln!(
                    "org_preview_scroll frames={} pixels_per_frame={:.1} cadence_ms={:.3} cadence_hz={:.2} p50_ms={:.3} p95_ms={:.3} p99_ms={:.3} max_ms={:.3} late_frames={} late_rate_pct={:.3} estimated_missed_vsyncs={} over_12_5={} over_16_67={}",
                    samples.len(),
                    benchmark.scroll_pixels,
                    cadence_ms,
                    1000.0 / cadence_ms,
                    percentile(0.50),
                    percentile(0.95),
                    percentile(0.99),
                    samples.last().unwrap().as_secs_f64() * 1000.0,
                    late_frames,
                    late_frames as f64 * 100.0 / samples.len() as f64,
                    estimated_missed_vsyncs,
                    over_12_5,
                    over_16_67,
                );
                crate::perf_tracing::report();
                this.scroll_benchmark = None;
                cx.quit();
            } else {
                this.list_state.scroll_by(px(benchmark.scroll_pixels));
                this.schedule_scroll_sample(window, cx);
                cx.notify();
            }
        });
    }
}
