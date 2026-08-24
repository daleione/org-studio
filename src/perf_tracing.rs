use std::{
    cell::RefCell,
    collections::HashMap,
    sync::{
        Arc, Mutex, OnceLock,
        atomic::{AtomicU64, Ordering},
    },
    time::{Duration, Instant},
};

use tracing::{
    Event, Metadata, Subscriber,
    span::{Attributes, Id, Record},
};

thread_local! {
    static ENTERED: RefCell<Vec<(u64, Instant)>> = const { RefCell::new(Vec::new()) };
}

#[derive(Default)]
struct TraceState {
    next_id: AtomicU64,
    spans: Mutex<HashMap<u64, &'static str>>,
    samples: Mutex<HashMap<&'static str, Vec<Duration>>>,
}

#[derive(Clone)]
struct PerfSubscriber {
    state: Arc<TraceState>,
}

pub struct PerfTrace {
    state: Arc<TraceState>,
}

static TRACE_STATE: OnceLock<Arc<TraceState>> = OnceLock::new();

impl PerfTrace {
    pub fn install() -> Option<Self> {
        std::env::var_os("ORG_STUDIO_PROFILE_FRAMES")?;
        let state = Arc::new(TraceState::default());
        tracing::subscriber::set_global_default(PerfSubscriber {
            state: state.clone(),
        })
        .ok()?;
        TRACE_STATE.set(state.clone()).ok()?;
        Some(Self { state })
    }

    pub fn report(&self) {
        let samples = self.state.samples.lock().unwrap();
        let mut names: Vec<_> = samples.keys().copied().collect();
        names.sort_unstable();
        for name in names {
            let values = &samples[name];
            if values.is_empty() {
                continue;
            }
            let mut values = values.clone();
            values.sort_unstable();
            let percentile = |p: f64| {
                let index = ((values.len() - 1) as f64 * p).ceil() as usize;
                values[index].as_secs_f64() * 1000.0
            };
            eprintln!(
                "org_preview_profile span={name:?} samples={} p50_ms={:.3} p95_ms={:.3} p99_ms={:.3} max_ms={:.3}",
                values.len(),
                percentile(0.50),
                percentile(0.95),
                percentile(0.99),
                values.last().unwrap().as_secs_f64() * 1000.0,
            );
        }
    }
}

pub fn report() {
    if let Some(state) = TRACE_STATE.get() {
        PerfTrace {
            state: state.clone(),
        }
        .report();
    }
}

impl Subscriber for PerfSubscriber {
    fn enabled(&self, _: &Metadata<'_>) -> bool {
        true
    }

    fn new_span(&self, attributes: &Attributes<'_>) -> Id {
        let id = self.state.next_id.fetch_add(1, Ordering::Relaxed) + 1;
        self.state
            .spans
            .lock()
            .unwrap()
            .insert(id, attributes.metadata().name());
        Id::from_u64(id)
    }

    fn record(&self, _: &Id, _: &Record<'_>) {}

    fn record_follows_from(&self, _: &Id, _: &Id) {}

    fn event(&self, _: &Event<'_>) {}

    fn enter(&self, id: &Id) {
        ENTERED.with(|entered| entered.borrow_mut().push((id.into_u64(), Instant::now())));
    }

    fn exit(&self, id: &Id) {
        let elapsed = ENTERED.with(|entered| {
            let mut entered = entered.borrow_mut();
            let index = entered.iter().rposition(|(entered_id, _)| *entered_id == id.into_u64())?;
            Some(entered.remove(index).1.elapsed())
        });
        let Some(elapsed) = elapsed else {
            return;
        };
        let Some(name) = self.state.spans.lock().unwrap().get(&id.into_u64()).copied() else {
            return;
        };
        self.state
            .samples
            .lock()
            .unwrap()
            .entry(name)
            .or_default()
            .push(elapsed);
    }

    fn clone_span(&self, id: &Id) -> Id {
        id.clone()
    }

    fn try_close(&self, id: Id) -> bool {
        self.state.spans.lock().unwrap().remove(&id.into_u64());
        true
    }
}
