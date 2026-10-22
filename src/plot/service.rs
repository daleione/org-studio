use super::{PlotArtifact, PlotDiagnostic, PlotRenderKey, PlotRenderRequest};
use gpui::{App, AppContext, Context, Entity, Global, RenderImage};
use std::{
    collections::{HashMap, VecDeque},
    sync::{Arc, Mutex, Weak},
};

const CACHE_BYTES: usize = 32 * 1024 * 1024;

#[derive(Clone, Debug)]
pub(crate) struct PlotRendered {
    pub(crate) artifact: PlotArtifact,
    pub(crate) image: Arc<RenderImage>,
}

impl PlotRendered {
    fn size_bytes(&self) -> usize {
        self.artifact.svg.len()
            + self.artifact.key.content.len()
            + self.image.as_bytes(0).map_or(0, <[u8]>::len)
    }
}

#[derive(Clone, Debug)]
pub(crate) enum PlotRenderState {
    Pending { previous: Option<Arc<PlotRendered>> },
    Ready(Arc<PlotRendered>),
    Failed(Arc<PlotDiagnostic>),
}

impl PlotRenderState {
    pub(crate) fn rendered(&self) -> Option<&Arc<PlotRendered>> {
        match self {
            Self::Ready(rendered) => Some(rendered),
            Self::Pending { previous } => previous.as_ref(),
            Self::Failed(_) => None,
        }
    }
}

#[derive(Default)]
struct DemandState {
    key: Option<PlotRenderKey>,
    state: Option<PlotRenderState>,
}

// Jobs hold only weak leases: closing or scrolling away releases the consumer,
// including its displayed artifact, without retaining a view in a global cache.
#[derive(Clone, Default)]
pub(crate) struct PlotDemand(Arc<Mutex<DemandState>>);

impl PlotDemand {
    pub(crate) fn request(
        &self,
        request: PlotRenderRequest,
        cx: &mut App,
    ) -> Option<PlotRenderState> {
        PlotService::shared(cx).update(cx, |service, cx| service.request(self, request, cx));
        self.state()
    }

    pub(crate) fn state(&self) -> Option<PlotRenderState> {
        self.0.lock().ok()?.state.clone()
    }
    pub(crate) fn retry(&self) {
        if let Ok(mut state) = self.0.lock()
            && matches!(state.state, Some(PlotRenderState::Failed(_)))
        {
            state.key = None;
            state.state = None;
        }
    }
}

struct Job {
    request: PlotRenderRequest,
    waiters: Vec<Weak<Mutex<DemandState>>>,
}

#[derive(Default)]
pub(crate) struct PlotService {
    jobs: HashMap<PlotRenderKey, Job>,
    queue: VecDeque<PlotRenderKey>,
    running: Option<PlotRenderKey>,
    task: Option<gpui::Task<()>>,
    cache: HashMap<PlotRenderKey, Arc<PlotRendered>>,
    lru: VecDeque<PlotRenderKey>,
    cache_bytes: usize,
}

struct SharedService(Entity<PlotService>);
impl Global for SharedService {}

impl PlotService {
    pub(crate) fn shared(cx: &mut App) -> Entity<Self> {
        if let Some(service) = cx.try_global::<SharedService>() {
            return service.0.clone();
        }
        let service = cx.new(|_| Self::default());
        cx.set_global(SharedService(service.clone()));
        service
    }

    fn request(&mut self, demand: &PlotDemand, request: PlotRenderRequest, cx: &mut Context<Self>) {
        let mut state = demand.0.lock().expect("plot demand lock");
        if state.key.as_ref() == Some(&request.key) {
            return;
        }
        state.key = Some(request.key.clone());
        if let Some(cached) = self.cache.get(&request.key) {
            state.state = Some(PlotRenderState::Ready(cached.clone()));
            self.lru.retain(|key| key != &request.key);
            self.lru.push_back(request.key);
            return;
        }
        let previous = state
            .state
            .as_ref()
            .and_then(PlotRenderState::rendered)
            .filter(|rendered| rendered.artifact.key.style == request.key.style)
            .cloned();
        state.state = Some(PlotRenderState::Pending { previous });
        drop(state);
        let key = request.key.clone();
        if let Some(job) = self.jobs.get_mut(&key) {
            if !job
                .waiters
                .iter()
                .any(|waiter| waiter.ptr_eq(&Arc::downgrade(&demand.0)))
            {
                job.waiters.push(Arc::downgrade(&demand.0));
            }
        } else {
            self.jobs.insert(
                key.clone(),
                Job {
                    request,
                    waiters: vec![Arc::downgrade(&demand.0)],
                },
            );
            self.queue.push_back(key);
        }
        self.start_next(cx);
    }

    fn start_next(&mut self, cx: &mut Context<Self>) {
        if self.running.is_some() {
            return;
        }
        while let Some(key) = self.queue.pop_front() {
            let Some(job) = self.jobs.get_mut(&key) else {
                continue;
            };
            job.waiters.retain(|waiter| {
                waiter.upgrade().is_some_and(|waiter| {
                    waiter
                        .lock()
                        .is_ok_and(|state| state.key.as_ref() == Some(&key))
                })
            });
            if job.waiters.is_empty() {
                self.jobs.remove(&key);
                continue;
            }
            let request = job.request.clone();
            self.running = Some(key.clone());
            let executor = cx.background_executor().clone();
            let renderer = cx.svg_renderer();
            self.task = Some(cx.spawn(async move |this, cx| {
                let result = executor
                    .spawn(async move {
                        let artifact =
                            super::render_plot(crate::typst_runtime::shared_engine(), &request)?;
                        // Ready means paintable: keep the old image through SVG decoding too.
                        let image =
                            renderer
                                .render_single_frame(&artifact.svg, 1.0)
                                .map_err(|error| {
                                    tracing::warn!(%error, "Org Plot image decoding failed");
                                    PlotDiagnostic::new(
                                        "decode-failed",
                                        "Could not load the chart image. Click to retry.",
                                    )
                                })?;
                        Ok(Arc::new(PlotRendered { artifact, image }))
                    })
                    .await;
                let _ = this.update(cx, |this, cx| {
                    let state = match result {
                        Ok(rendered) => {
                            this.insert_cache(key.clone(), rendered.clone());
                            PlotRenderState::Ready(rendered)
                        }
                        Err(error) => PlotRenderState::Failed(Arc::new(error)),
                    };
                    if let Some(job) = this.jobs.remove(&key) {
                        for waiter in job.waiters {
                            if let Some(waiter) = waiter.upgrade()
                                && let Ok(mut waiter) = waiter.lock()
                                && waiter.key.as_ref() == Some(&key)
                            {
                                waiter.state = Some(state.clone());
                            }
                        }
                    }
                    this.running = None;
                    cx.notify();
                    this.start_next(cx);
                });
            }));
            return;
        }
    }

    fn insert_cache(&mut self, key: PlotRenderKey, rendered: Arc<PlotRendered>) {
        self.cache_bytes += rendered.size_bytes();
        self.lru.push_back(key.clone());
        self.cache.insert(key, rendered);
        while self.cache_bytes > CACHE_BYTES || self.cache.len() > 128 {
            let Some(key) = self.lru.pop_front() else {
                break;
            };
            if let Some(expired) = self.cache.remove(&key) {
                self.cache_bytes = self.cache_bytes.saturating_sub(expired.size_bytes());
            }
        }
    }
}
