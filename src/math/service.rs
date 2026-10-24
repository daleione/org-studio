use super::{MathKey, render::render_batch};
use gpui::{App, AppContext, Context, Entity, Global, RenderImage};
use std::{
    collections::{HashMap, VecDeque},
    sync::{Arc, Mutex, OnceLock, Weak},
};

const CACHE_BYTES: usize = 32 * 1024 * 1024;
const CACHE_ENTRIES: usize = 128;
const BATCH_SIZE: usize = 24;

#[derive(Debug)]
pub(crate) struct RenderedFormula {
    pub(crate) image: Arc<RenderImage>,
    pub(crate) width: f32,
    pub(crate) height: f32,
    pub(crate) baseline: f32,
}

impl RenderedFormula {
    fn size_bytes(&self) -> usize {
        self.image.as_bytes(0).map_or(0, <[u8]>::len)
    }
}

type Outcome = Result<Arc<RenderedFormula>, Arc<str>>;

#[derive(Debug, Default)]
struct DemandState {
    key: Option<MathKey>,
    result: Option<Outcome>,
    previous: Option<Arc<RenderedFormula>>,
    version: u64,
}

/// A viewport lease; queued jobs never keep a document or its images alive.
#[derive(Clone, Debug, Default)]
pub(crate) struct MathDemand(Arc<Mutex<DemandState>>);

impl MathDemand {
    pub(crate) fn rendered(&self) -> Option<Arc<RenderedFormula>> {
        let state = self.0.lock().unwrap();
        match &state.result {
            Some(Ok(image)) => Some(image.clone()),
            Some(Err(_)) => None,
            None => state.previous.clone(),
        }
    }
    pub(crate) fn error(&self) -> Option<Arc<str>> {
        self.0
            .lock()
            .unwrap()
            .result
            .as_ref()?
            .as_ref()
            .err()
            .cloned()
    }
    pub(crate) fn ready_image(&self) -> Option<Arc<RenderedFormula>> {
        self.0
            .lock()
            .unwrap()
            .result
            .as_ref()?
            .as_ref()
            .ok()
            .cloned()
    }
    pub(crate) fn version(&self) -> u64 {
        self.0.lock().unwrap().version
    }
    pub(crate) fn is_pending(&self) -> bool {
        self.0.lock().unwrap().result.is_none()
    }
    pub(crate) fn request(&self, key: MathKey, cx: &mut App) {
        MathService::shared(cx).update(cx, |service, cx| service.request(self, key, cx));
    }
    pub(crate) fn inherit(&self, previous: &Self) {
        self.0.lock().unwrap().previous = previous.rendered();
    }
}

struct Job {
    waiters: Vec<Weak<Mutex<DemandState>>>,
}

#[derive(Default)]
pub(crate) struct MathService {
    jobs: HashMap<MathKey, Job>,
    queue: VecDeque<MathKey>,
    running: bool,
    scheduled: bool,
    task: Option<gpui::Task<()>>,
}

struct SharedService(Entity<MathService>);
impl Global for SharedService {}

impl MathService {
    pub(crate) fn shared(cx: &mut App) -> Entity<Self> {
        if let Some(service) = cx.try_global::<SharedService>() {
            return service.0.clone();
        }
        let service = cx.new(|_| Self::default());
        cx.set_global(SharedService(service.clone()));
        service
    }
    fn request(&mut self, demand: &MathDemand, key: MathKey, cx: &mut Context<Self>) {
        let _span = tracing::info_span!("math.request").entered();
        let mut state = demand.0.lock().unwrap();
        if state.key.as_ref() == Some(&key) {
            return;
        }
        let same_style = state.key.as_ref().is_some_and(|old| old.style == key.style);
        if state.key.is_some() {
            state.previous = if same_style {
                state
                    .result
                    .as_ref()
                    .and_then(|result| result.as_ref().ok())
                    .cloned()
                    .or_else(|| state.previous.take())
            } else {
                None
            };
        }
        state.key = Some(key.clone());
        state.result = cache().lock().unwrap().get(&key);
        state.version = state.version.wrapping_add(1);
        if state.result.is_some() {
            state.previous = None;
            return;
        }
        drop(state);
        self.jobs
            .entry(key.clone())
            .or_insert_with(|| {
                self.queue.push_back(key.clone());
                Job {
                    waiters: Vec::new(),
                }
            })
            .waiters
            .push(Arc::downgrade(&demand.0));
        // Coalesce all requests made during this frame before starting the compiler.
        if !self.running && !self.scheduled {
            self.scheduled = true;
            let service = cx.entity().downgrade();
            cx.defer(move |cx| {
                let _ = service.update(cx, |this, cx| {
                    this.scheduled = false;
                    this.start_next(cx);
                });
            });
        }
    }
    fn start_next(&mut self, cx: &mut Context<Self>) {
        if self.running {
            return;
        }
        let mut keys = Vec::with_capacity(BATCH_SIZE);
        while keys.len() < BATCH_SIZE {
            let Some(key) = self.queue.pop_front() else {
                break;
            };
            let Some(job) = self.jobs.get_mut(&key) else {
                continue;
            };
            job.waiters.retain(|waiter| {
                waiter
                    .upgrade()
                    .is_some_and(|waiter| waiter.lock().unwrap().key.as_ref() == Some(&key))
            });
            if job.waiters.is_empty() {
                self.jobs.remove(&key);
            } else {
                keys.push(key);
            }
        }
        if keys.is_empty() {
            return;
        }
        self.running = true;
        let executor = cx.background_executor().clone();
        let renderer = cx.svg_renderer();
        self.task = Some(cx.spawn(async move |this, cx| {
            let (keys, results) = executor
                .spawn(async move {
                    let _span = tracing::info_span!("math.compile_batch").entered();
                    let results = render_batch(crate::typst_runtime::shared_engine(), &keys)
                        .into_iter()
                        .map(|result| {
                            result
                                .and_then(|fragment| {
                                    let image = renderer
                                        .render_single_frame(&fragment.svg, 1.0)
                                        .map_err(|error| error.to_string())?;
                                    // Typst uses 72 points/inch; the text system uses 96 logical pixels/inch.
                                    Ok(Arc::new(RenderedFormula {
                                        image,
                                        width: fragment.width / 0.75,
                                        height: fragment.height / 0.75,
                                        baseline: fragment.baseline / 0.75,
                                    }))
                                })
                                .map_err(Arc::<str>::from)
                        })
                        .collect::<Vec<_>>();
                    (keys, results)
                })
                .await;
            let _ = this.update(cx, |this, cx| {
                let _span = tracing::info_span!("math.publish_batch").entered();
                for (key, result) in keys.into_iter().zip(results) {
                    cache()
                        .lock()
                        .unwrap()
                        .insert_cache(key.clone(), result.clone());
                    if let Some(job) = this.jobs.remove(&key) {
                        for waiter in job.waiters {
                            if let Some(waiter) = waiter.upgrade() {
                                let mut state = waiter.lock().unwrap();
                                if state.key.as_ref() == Some(&key) {
                                    state.result = Some(result.clone());
                                    state.previous = None;
                                    state.version = state.version.wrapping_add(1);
                                }
                            }
                        }
                    }
                }
                this.running = false;
                cx.notify();
                this.start_next(cx);
            });
        }));
    }
}

#[derive(Default)]
struct MathCache {
    cache: HashMap<MathKey, Outcome>,
    lru: VecDeque<MathKey>,
    cache_bytes: usize,
}

fn cache() -> &'static Mutex<MathCache> {
    static CACHE: OnceLock<Mutex<MathCache>> = OnceLock::new();
    CACHE.get_or_init(Mutex::default)
}

/// Measurement may reuse existing pixels, but cannot schedule work or extend the LRU.
pub(crate) fn cached_formula(key: &MathKey) -> Option<Arc<RenderedFormula>> {
    cache()
        .lock()
        .unwrap()
        .cache
        .get(key)?
        .as_ref()
        .ok()
        .cloned()
}

impl MathCache {
    fn get(&mut self, key: &MathKey) -> Option<Outcome> {
        let outcome = self.cache.get(key)?.clone();
        self.lru.retain(|old| old != key);
        self.lru.push_back(key.clone());
        Some(outcome)
    }
    fn insert_cache(&mut self, key: MathKey, result: Outcome) {
        if let Some(previous) = self.cache.remove(&key) {
            self.cache_bytes -= cache_bytes(&key, &previous);
            self.lru.retain(|old| old != &key);
        }
        let bytes = cache_bytes(&key, &result);
        self.cache_bytes += bytes;
        self.lru.push_back(key.clone());
        self.cache.insert(key, result);
        while self.cache_bytes > CACHE_BYTES || self.cache.len() > CACHE_ENTRIES {
            let Some(expired) = self.lru.pop_front() else {
                break;
            };
            if let Some(result) = self.cache.remove(&expired) {
                self.cache_bytes -= cache_bytes(&expired, &result);
            }
        }
    }
}

fn cache_bytes(key: &MathKey, result: &Outcome) -> usize {
    key.formula.body.len()
        + result
            .as_ref()
            .map_or_else(|error| error.len(), |image| image.size_bytes())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::org_syntax::math::{Formula, MathMode};

    fn key(body: &str) -> MathKey {
        MathKey {
            formula: Formula::new(body, MathMode::Inline),
            style: crate::math::MathStyle::new(16.0, 0x202020),
        }
    }

    #[gpui::test]
    fn math_service_coalesces_demands_cancels_dropped_consumers_and_reuses_images(
        cx: &mut gpui::TestAppContext,
    ) {
        let first = MathDemand::default();
        let second = MathDemand::default();
        let abandoned = MathDemand::default();
        cx.update(|cx| {
            first.request(key(r"x_{9512}^2"), cx);
            second.request(key(r"x_{9512}^2"), cx);
            abandoned.request(key(r"\frac{15123}{7}"), cx);
            assert!(MathService::shared(cx).read(cx).scheduled);
            assert_eq!(MathService::shared(cx).read(cx).jobs.len(), 2);
            drop(abandoned);
        });
        cx.run_until_parked();
        assert!(Arc::ptr_eq(
            &first.rendered().unwrap(),
            &second.rendered().unwrap()
        ));
        cx.update(|cx| {
            let service = MathService::shared(cx);
            assert!(cached_formula(&key(r"x_{9512}^2")).is_some());
            assert!(cached_formula(&key(r"\frac{15123}{7}")).is_none());
            let cached = MathDemand::default();
            cached.request(key(r"x_{9512}^2"), cx);
            assert!(!cached.is_pending());
            assert!(Arc::ptr_eq(
                &first.rendered().unwrap(),
                &cached.rendered().unwrap()
            ));
            assert!(service.read(cx).jobs.is_empty());
        });
    }

    #[test]
    fn math_cache_bounds_failures_and_source_bytes_too() {
        let mut service = MathCache::default();
        for index in 0..1000 {
            service.insert_cache(
                key(&format!("x_{{{index}}}")),
                Err("unsupported command".into()),
            );
        }
        assert_eq!(service.cache.len(), CACHE_ENTRIES);
        assert_eq!(service.lru.len(), CACHE_ENTRIES);
        assert!(!service.cache.contains_key(&key("x_{0}")));
        assert_eq!(
            service.cache_bytes,
            service
                .cache
                .iter()
                .map(|(key, result)| cache_bytes(key, result))
                .sum::<usize>()
        );
        service.insert_cache(key("huge-error"), Err("e".repeat(CACHE_BYTES + 1).into()));
        assert!(service.cache_bytes <= CACHE_BYTES && service.cache.len() <= CACHE_ENTRIES);
    }
}
