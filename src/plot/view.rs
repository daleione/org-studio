use super::{
    PlotDemand, PlotDocument, PlotEntry, PlotId, PlotPreparation, PlotRenderState, PlotService,
    PlotStyle,
};
use crate::components::image_frame::{CONTENT_RADIUS, ImageFrame};
use crate::document::{ByteRange, Revision, RevisionDelta, RevisionRange, SharedTextSnapshot};
use crate::document_preview::{img, record_box};
use gpui::{App, Context, Div, Entity, Subscription, div, prelude::*, px, rgb};
use std::{
    collections::{HashMap, HashSet},
    sync::Arc,
};

pub(crate) struct PlotHost {
    pub(crate) document: Entity<PlotDocument>,
    entries: Arc<[Arc<PlotEntry>]>,
    revision: Revision,
    current: bool,
    demands: HashMap<PlotId, PlotDemand>,
    _subscriptions: Vec<Subscription>,
}

impl PlotHost {
    pub(crate) fn new<T: 'static>(
        snapshot: SharedTextSnapshot,
        on_index: fn(&mut T, &mut Context<T>),
        cx: &mut Context<T>,
    ) -> Self {
        let revision = snapshot.revision();
        let document = PlotDocument::shared(snapshot, cx);
        let service = PlotService::shared(cx);
        let subscriptions = vec![
            cx.observe(&document, move |this, _, cx| {
                on_index(this, cx);
                cx.notify();
            }),
            cx.observe(&service, |_, _, cx| cx.notify()),
        ];
        Self {
            document,
            entries: Arc::from([]),
            revision,
            current: false,
            demands: HashMap::new(),
            _subscriptions: subscriptions,
        }
    }

    pub(crate) fn refresh(
        &mut self,
        snapshot: SharedTextSnapshot,
        delta: Option<RevisionDelta>,
        cx: &mut App,
    ) {
        if snapshot.revision() != self.revision {
            if let Some(delta) = delta.as_ref() {
                self.map_source_delta(delta);
            }
            if snapshot.revision() != self.revision {
                self.entries = Arc::from([]);
            }
            self.revision = snapshot.revision();
            self.current = false;
        }
        self.document
            .update(cx, |document, cx| document.refresh(snapshot, delta, cx));
    }

    pub(crate) fn map_source_delta(&mut self, delta: &RevisionDelta) {
        if self.revision != delta.before {
            return;
        }
        self.entries = self
            .entries
            .iter()
            .filter_map(|entry| {
                let map = |range: ByteRange| {
                    delta
                        .map_range(RevisionRange::new(delta.before, range))
                        .ok()
                        .map(|range| range.range)
                };
                // Keep exclusive ends before insertions on the following line.
                let source_range = |range: ByteRange| {
                    Some(ByteRange {
                        start: map(ByteRange::new(range.start.0, range.start.0 + 1))?.start,
                        end: map(ByteRange::new(range.end.0, range.end.0))?.start,
                    })
                };
                let mut entry = (**entry).clone();
                entry.source.declaration = source_range(entry.source.declaration)?;
                entry.source.table = source_range(entry.source.table)?;
                if let Some(range) = entry.source.image_link {
                    entry.source.image_link = Some(map(range)?);
                }
                Some(Arc::new(entry))
            })
            .collect::<Vec<_>>()
            .into();
        self.revision = delta.after;
        self.current = false;
    }

    pub(crate) fn entries(&mut self, revision: Revision, cx: &App) -> Arc<[Arc<PlotEntry>]> {
        let document = self.document.read(cx);
        if document.is_current(revision) {
            self.entries = document.entries.clone();
            self.current = true;
            self.revision = revision;
        }
        if self.revision != revision {
            return Arc::from([]);
        }
        self.entries.clone()
    }

    pub(crate) fn present(
        &mut self,
        entry: Arc<PlotEntry>,
        sizing: crate::preview::ImageSizing,
        style: PlotStyle,
        active: bool,
        cx: &mut App,
    ) -> PlotPresentation {
        let (width, height) = entry.image_size(&sizing);
        let mut demand = None;
        let mut state = None;
        if active && let PlotPreparation::Ready(plot) = &entry.prepared {
            let lease = self.demands.entry(entry.id).or_default();
            state = if self.current && !entry.display.lock().unwrap().resizing {
                lease.request(
                    super::PlotRenderRequest::for_display(
                        plot.clone(),
                        style.clone(),
                        (width, height),
                        sizing.font_size,
                    ),
                    cx,
                )
            } else {
                lease.state()
            };
            demand = Some(lease.clone());
        }
        PlotPresentation {
            document: self.document.clone(),
            output_error: self.document.read(cx).output_errors.get(&entry.id).cloned(),
            entry,
            state,
            demand,
            current: self.current,
            width,
            height,
            style,
        }
    }

    pub(crate) fn retain(&mut self, visible: &HashSet<PlotId>) {
        self.demands.retain(|id, _| visible.contains(id));
    }
    pub(crate) fn is_current(&self) -> bool {
        self.current
    }

    pub(crate) fn previews_ready(&self, revision: Revision, cx: &App) -> bool {
        self.document.read(cx).is_current(revision)
            && self.demands.values().all(|demand| {
                !matches!(demand.state(), None | Some(PlotRenderState::Pending { .. }))
            })
    }
}

#[derive(Clone)]
pub(crate) struct PlotPresentation {
    pub(crate) document: Entity<PlotDocument>,
    pub(crate) output_error: Option<Arc<super::PlotDiagnostic>>,
    pub(crate) entry: Arc<PlotEntry>,
    pub(crate) state: Option<PlotRenderState>,
    pub(crate) demand: Option<PlotDemand>,
    pub(crate) current: bool,
    pub(crate) width: f32,
    pub(crate) height: f32,
    pub(crate) style: PlotStyle,
}

pub(crate) fn render_plot_surface(
    presentation: &PlotPresentation,
    chinese: bool,
) -> gpui::Stateful<Div> {
    let height = presentation.height;
    let selected = presentation.entry.display.lock().unwrap().selected;
    let frame = ImageFrame::new(
        presentation.style.background,
        selected.then_some(presentation.style.colors[0]),
    );
    let mut surface = frame
        .surface()
        .size_full()
        .font_family(".SystemUIFont")
        .text_size(px(12.0))
        .text_color(rgb(presentation.style.muted));
    let message = if !presentation.current
        && presentation
            .state
            .as_ref()
            .and_then(PlotRenderState::rendered)
            .is_none()
    {
        Some(if chinese {
            "正在更新图表…".to_owned()
        } else {
            "Updating chart…".to_owned()
        })
    } else {
        match (&presentation.entry.prepared, &presentation.state) {
            (PlotPreparation::Ready(plot), Some(state)) if state.rendered().is_some() => {
                let rendered = state.rendered().expect("displayed chart");
                surface = surface.child(
                    img(rendered.image.clone())
                        .w_full()
                        .h_full()
                        .rounded(px(CONTENT_RADIUS)),
                );
                if !plot.plan.warnings.is_empty() {
                    surface = surface.child(
                        div()
                            .absolute()
                            .left(px(12.0))
                            .right(px(12.0))
                            .bottom(px(4.0))
                            .text_size(px(10.0))
                            .child(if chinese {
                                "绘图精度受限，精确值见表格"
                            } else {
                                "Approximate coordinates; exact values are in the table"
                            }),
                    );
                }
                None
            }
            (PlotPreparation::Empty, _) => Some(if chinese {
                "暂无可绘制数据".into()
            } else {
                "No plottable data".into()
            }),
            (PlotPreparation::Invalid(error), _) | (_, Some(PlotRenderState::Failed(error))) => {
                Some(error.message.clone())
            }
            _ => Some(if chinese {
                "正在生成图表…".into()
            } else {
                "Rendering chart…".into()
            }),
        }
    };
    surface = surface.children(render_plot_output_error(presentation));
    if let Some(message) = message {
        surface = surface.child(
            div()
                .size_full()
                .flex()
                .flex_col()
                .items_center()
                .justify_center()
                .px(px(24.0))
                .gap(px(10.0))
                .child(message)
                .when(
                    matches!(presentation.state, Some(PlotRenderState::Failed(_))),
                    |div| {
                        div.child(if chinese {
                            "点击重试"
                        } else {
                            "Click to retry"
                        })
                    },
                ),
        );
    }
    div()
        .id(("org-plot", presentation.entry.id.0))
        .w(px(presentation.width))
        .h(px(height))
        .child(record_box(surface))
}

pub(crate) fn render_plot_output_error(presentation: &PlotPresentation) -> Option<Div> {
    let error = presentation.output_error.as_ref()?;
    Some(
        div()
            .absolute()
            .bottom(px(6.0))
            .left(px(12.0))
            .right(px(12.0))
            .text_size(px(10.0))
            .child(error.message.clone()),
    )
}
