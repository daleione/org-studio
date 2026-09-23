use std::{
    cell::RefCell,
    path::{Path, PathBuf},
    rc::Rc,
    sync::Arc,
};

use gpui::{
    App, Asset, DevicePixels, Entity, ImageCacheError, IntoElement, MouseButton, ObjectFit,
    RenderImage, ScrollHandle, SvgSize, Window, div, img, prelude::*, px, rgb, size, svg,
};

use super::{ContentRoute, WorkspaceWindow};

pub(crate) struct ImageViewerState {
    pub(crate) path: Option<PathBuf>,
    pub(crate) size: Option<(u32, u32)>,
    pub(crate) zoom: f32,
    pub(crate) raster_zoom: f32,
    pub(crate) scroll: ScrollHandle,
    pub(crate) return_route: ContentRoute,
    raster_cache: SvgRasterCache,
}

impl Default for ImageViewerState {
    fn default() -> Self {
        Self {
            path: None,
            size: None,
            zoom: 1.0,
            raster_zoom: 1.0,
            scroll: ScrollHandle::new(),
            return_route: ContentRoute::Document,
            raster_cache: SvgRasterCache::default(),
        }
    }
}

impl ImageViewerState {
    pub(crate) fn clear(&mut self, cx: &mut App) {
        self.path = None;
        self.size = None;
        self.raster_cache.clear(cx);
    }
}

pub(super) fn zoomed_scroll_offset(
    offset: f32,
    focal: f32,
    ratio: f32,
    old_origin: f32,
    new_origin: f32,
) -> f32 {
    focal - new_origin - (focal - offset - old_origin) * ratio
}

#[derive(Clone, Hash, PartialEq, Eq)]
struct SvgRasterSource {
    path: PathBuf,
    width: i32,
    height: i32,
}

struct SvgRasterAsset;

#[derive(Default)]
struct SvgRasterCacheState {
    active_source: Option<SvgRasterSource>,
    last_ready: Option<Arc<RenderImage>>,
}

#[derive(Clone, Default)]
struct SvgRasterCache(Rc<RefCell<SvgRasterCacheState>>);

impl SvgRasterCache {
    fn clear(&self, cx: &mut App) {
        let mut cache = self.0.borrow_mut();
        let source = cache.active_source.take();
        cache.last_ready = None;
        drop(cache);
        if let Some(source) = source {
            cx.remove_asset::<SvgRasterAsset>(&source);
        }
    }

    fn load_or_last(
        &self,
        source: &SvgRasterSource,
        window: &mut Window,
        cx: &mut App,
    ) -> Option<Result<Arc<RenderImage>, ImageCacheError>> {
        let previous_source = {
            let mut cache = self.0.borrow_mut();
            if cache.active_source.as_ref() == Some(source) {
                None
            } else {
                cache.active_source.replace(source.clone())
            }
        };
        if let Some(previous_source) = previous_source {
            cx.remove_asset::<SvgRasterAsset>(&previous_source);
        }
        match window.use_asset::<SvgRasterAsset>(source, cx) {
            Some(Ok(image)) => {
                self.0.borrow_mut().last_ready = Some(image.clone());
                Some(Ok(image))
            }
            Some(Err(error)) => self
                .0
                .borrow()
                .last_ready
                .clone()
                .map(Ok)
                .or(Some(Err(error))),
            None => self.0.borrow().last_ready.clone().map(Ok),
        }
    }
}

impl Asset for SvgRasterAsset {
    type Source = SvgRasterSource;
    type Output = Result<Arc<RenderImage>, ImageCacheError>;

    fn load(
        source: Self::Source,
        cx: &mut App,
    ) -> impl Future<Output = Self::Output> + Send + 'static {
        let renderer = cx.svg_renderer();
        async move {
            let bytes = std::fs::read(&source.path)?;
            let svg = renderer.parse_svg(&bytes)?;
            let target = size(DevicePixels(source.width), DevicePixels(source.height));
            Ok(renderer.render_parsed(&svg, SvgSize::ExactSize(target))?)
        }
    }
}

fn is_svg(path: &Path) -> bool {
    path.extension()
        .is_some_and(|extension| extension.eq_ignore_ascii_case("svg"))
}

fn svg_raster_source(path: &Path, width: f32, height: f32, scale_factor: f32) -> SvgRasterSource {
    let width = (width * scale_factor).max(1.0);
    let height = (height * scale_factor).max(1.0);
    // Quantize one axis, then derive the other from the same scale so SVGs
    // retain their aspect ratio. Bound the final texture for large images.
    let target_width = (width / 32.0).ceil() * 32.0;
    let target_height = height * target_width / width;
    let limit = (8192.0 / target_width)
        .min(8192.0 / target_height)
        .min((16_000_000.0 / (target_width * target_height)).sqrt())
        .min(1.0);
    SvgRasterSource {
        path: path.to_path_buf(),
        width: (target_width * limit).round().max(1.0) as i32,
        height: (target_height * limit).round().max(1.0) as i32,
    }
}

#[derive(Debug, PartialEq)]
struct ImageLayout {
    image_width: f32,
    image_height: f32,
    canvas_width: f32,
    canvas_height: f32,
}

fn image_layout(
    image_size: (u32, u32),
    viewport_width: f32,
    viewport_height: f32,
    zoom: f32,
) -> ImageLayout {
    let viewport_width = viewport_width.max(1.0);
    let viewport_height = viewport_height.max(1.0);
    let fit = ((viewport_width - 32.0).max(1.0) / image_size.0.max(1) as f32)
        .min((viewport_height - 32.0).max(1.0) / image_size.1.max(1) as f32)
        .min(1.0);
    let image_width = (image_size.0 as f32 * fit).min((viewport_width - 32.0).max(1.0)) * zoom;
    let image_height = (image_size.1 as f32 * fit).min((viewport_height - 32.0).max(1.0)) * zoom;
    ImageLayout {
        image_width,
        image_height,
        canvas_width: viewport_width.max(image_width + 32.0),
        canvas_height: viewport_height.max(image_height + 32.0),
    }
}

pub(super) fn image_origin(
    image_size: (u32, u32),
    viewport_width: f32,
    viewport_height: f32,
    zoom: f32,
) -> (f32, f32) {
    let layout = image_layout(image_size, viewport_width, viewport_height, zoom);
    (
        (layout.canvas_width - layout.image_width) / 2.0,
        (layout.canvas_height - layout.image_height) / 2.0,
    )
}

pub(super) fn render_image_viewer(
    workspace: Entity<WorkspaceWindow>,
    path: &Path,
    image_size: (u32, u32),
    state: &ImageViewerState,
    titlebar_inset: f32,
    window: &Window,
) -> impl IntoElement {
    let theme = crate::theme::current_theme();
    let viewport = window.viewport_size();
    let viewport_width = f32::from(viewport.width);
    let viewport_height = (f32::from(viewport.height) - crate::app::TITLEBAR_HEIGHT).max(1.0);
    let layout = image_layout(image_size, viewport_width, viewport_height, state.zoom);
    let raster_layout = image_layout(
        image_size,
        viewport_width,
        viewport_height,
        state.raster_zoom,
    );
    let zoom = state.zoom;
    let raster_cache = state.raster_cache.clone();
    let filename = path
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.display().to_string());
    let close_workspace = workspace.clone();
    let zoom_out_workspace = workspace.clone();
    let reset_workspace = workspace.clone();
    let zoom_in_workspace = workspace.clone();
    let title_inset = titlebar_inset + 156.0;
    let center_title = viewport_width >= title_inset * 2.0 + 80.0;
    div()
        .id("image-viewer")
        .size_full()
        .flex()
        .flex_col()
        .bg(rgb(theme.background))
        .child(
            div()
                .debug_selector(|| "image-viewer-titlebar".to_owned())
                .h(px(crate::app::TITLEBAR_HEIGHT))
                .flex_none()
                .relative()
                .pl(px(titlebar_inset))
                .pr(px(crate::app::TITLEBAR_TRAILING_INSET))
                .flex()
                .items_center()
                .border_b_1()
                .border_color(rgb(theme.border))
                .child(
                    div()
                        .id("image-viewer-close")
                        .debug_selector(|| "image-viewer-back".to_owned())
                        .size(px(32.0))
                        .flex()
                        .items_center()
                        .justify_center()
                        .cursor_pointer()
                        .rounded_full()
                        .bg(rgb(theme.background_alt))
                        .hover(move |style| style.bg(rgb(theme.code_active_background)))
                        .child(
                            svg()
                                .data(include_bytes!("../../assets/icons/agenda/caret-left.svg"))
                                .size(px(16.0))
                                .text_color(rgb(theme.foreground)),
                        )
                        .on_mouse_down(MouseButton::Left, move |_, _, cx| {
                            cx.stop_propagation();
                            close_workspace
                                .update(cx, |workspace, cx| workspace.close_image_viewer(cx));
                        }),
                )
                .child(
                    div()
                        .id("image-viewer-zoom-out")
                        .debug_selector(|| "image-viewer-zoom-out".to_owned())
                        .size(px(28.0))
                        .flex()
                        .items_center()
                        .justify_center()
                        .cursor_pointer()
                        .rounded_full()
                        .hover(move |style| style.bg(rgb(theme.background_alt)))
                        .child("−")
                        .on_mouse_down(MouseButton::Left, move |_, window, cx| {
                            cx.stop_propagation();
                            zoom_out_workspace.update(cx, |workspace, cx| {
                                workspace.zoom_image(0.8, None, window, cx)
                            });
                        }),
                )
                .child(
                    div()
                        .id("image-viewer-zoom-reset")
                        .debug_selector(|| "image-viewer-zoom-reset".to_owned())
                        .h(px(28.0))
                        .min_w(px(52.0))
                        .flex()
                        .items_center()
                        .justify_center()
                        .cursor_pointer()
                        .rounded(px(6.0))
                        .hover(move |style| style.bg(rgb(theme.background_alt)))
                        .child(format!("{:.0}%", zoom * 100.0))
                        .on_mouse_down(MouseButton::Left, move |_, _, cx| {
                            cx.stop_propagation();
                            reset_workspace
                                .update(cx, |workspace, cx| workspace.reset_image_zoom(cx));
                        }),
                )
                .child(
                    div()
                        .id("image-viewer-zoom-in")
                        .debug_selector(|| "image-viewer-zoom-in".to_owned())
                        .size(px(28.0))
                        .flex()
                        .items_center()
                        .justify_center()
                        .cursor_pointer()
                        .rounded_full()
                        .hover(move |style| style.bg(rgb(theme.background_alt)))
                        .child("+")
                        .on_mouse_down(MouseButton::Left, move |_, window, cx| {
                            cx.stop_propagation();
                            zoom_in_workspace.update(cx, |workspace, cx| {
                                workspace.zoom_image(1.25, None, window, cx)
                            });
                        }),
                )
                .when(center_title, |bar| {
                    bar.child(div().flex_1()).child(
                        div()
                            .absolute()
                            .top_0()
                            .left(px(title_inset))
                            .right(px(title_inset))
                            .h_full()
                            .min_w_0()
                            .flex()
                            .items_center()
                            .justify_center()
                            .child(
                                div()
                                    .id("image-viewer-filename")
                                    .debug_selector(|| "image-viewer-filename".to_owned())
                                    .min_w_0()
                                    .truncate()
                                    .text_size(px(13.0))
                                    .text_color(rgb(theme.foreground))
                                    .child(filename.clone()),
                            ),
                    )
                })
                .when(!center_title, |bar| {
                    bar.child(
                        div()
                            .id("image-viewer-filename")
                            .debug_selector(|| "image-viewer-filename".to_owned())
                            .flex_1()
                            .min_w_0()
                            .ml(px(8.0))
                            .truncate()
                            .text_size(px(13.0))
                            .text_color(rgb(theme.foreground))
                            .child(filename),
                    )
                }),
        )
        .child(
            div()
                .id("image-viewer-scroll")
                .debug_selector(|| "image-viewer-scroll".to_owned())
                .flex_1()
                .min_h_0()
                .min_w_0()
                .overflow_scroll()
                .track_scroll(&state.scroll)
                .on_pinch(move |event, window, cx| {
                    workspace.update(cx, |workspace, cx| {
                        workspace.zoom_image_gesture(
                            (1.0 + event.delta).max(0.1),
                            event.position,
                            event.phase,
                            window,
                            cx,
                        )
                    });
                    cx.stop_propagation();
                })
                .child(
                    div()
                        .w(px(layout.canvas_width))
                        .h(px(layout.canvas_height))
                        .flex()
                        .items_center()
                        .justify_center()
                        .child(if is_svg(path) {
                            let source = svg_raster_source(
                                path,
                                raster_layout.image_width,
                                raster_layout.image_height,
                                window.scale_factor(),
                            );
                            img(move |window: &mut Window, cx: &mut App| {
                                raster_cache.load_or_last(&source, window, cx)
                            })
                            .w(px(layout.image_width))
                            .h(px(layout.image_height))
                            .object_fit(ObjectFit::Contain)
                        } else {
                            img(path.to_path_buf())
                                .w(px(layout.image_width))
                                .h(px(layout.image_height))
                                .object_fit(ObjectFit::Contain)
                        }),
                ),
        )
}

#[cfg(test)]
mod tests {
    use super::{
        SvgRasterAsset, SvgRasterCache, image_layout, image_origin, is_svg, svg_raster_source,
        zoomed_scroll_offset,
    };
    use crate::app::{WorkspaceWindow, image_viewer::render_image_viewer};
    use gpui::{AppContext, Context, Entity, IntoElement, Render, Window, px};
    use std::path::Path;

    struct ImageViewerHarness(Entity<WorkspaceWindow>);

    impl Render for ImageViewerHarness {
        fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
            render_image_viewer(
                self.0.clone(),
                Path::new(concat!(
                    env!("CARGO_MANIFEST_DIR"),
                    "/assets/screenshots/org-studio.jpg"
                )),
                (100, 100),
                &self.0.read(cx).image_viewer,
                crate::app::TITLEBAR_LEADING_INSET,
                window,
            )
        }
    }

    #[gpui::test]
    fn image_viewer_uses_one_titlebar_row(cx: &mut gpui::TestAppContext) {
        let workspace = cx.new(|_| WorkspaceWindow::with_split_layout(false));
        let (_, cx) = cx.add_window_view(move |_, _| ImageViewerHarness(workspace));
        let bar = cx.debug_bounds("image-viewer-titlebar").unwrap();
        let back = cx.debug_bounds("image-viewer-back").unwrap();
        let title = cx.debug_bounds("image-viewer-filename").unwrap();
        let zoom = cx.debug_bounds("image-viewer-zoom-reset").unwrap();
        let content = cx.debug_bounds("image-viewer-scroll").unwrap();

        assert_eq!(bar.size.height, px(crate::app::TITLEBAR_HEIGHT));
        assert_eq!(content.top(), bar.bottom());
        for control in [back, title, zoom] {
            assert!((f32::from(control.center().y) - f32::from(bar.center().y)).abs() <= 1.0);
        }
        assert!((f32::from(title.center().x) - f32::from(bar.center().x)).abs() <= 1.0);
        assert!(back.right() <= zoom.left());
        assert!(zoom.right() < title.left());
    }

    #[gpui::test]
    fn narrow_image_viewer_keeps_filename_visible(cx: &mut gpui::TestAppContext) {
        let workspace = cx.new(|_| WorkspaceWindow::with_split_layout(false));
        let (_, cx) = cx.add_window_view(move |_, _| ImageViewerHarness(workspace));
        cx.simulate_resize(gpui::size(px(390.0), px(600.0)));

        let title = cx.debug_bounds("image-viewer-filename").unwrap();
        let zoom = cx.debug_bounds("image-viewer-zoom-in").unwrap();
        assert!(title.size.width > px(0.0));
        assert!(title.left() >= zoom.right());
    }

    #[test]
    fn large_image_fits_then_expands_into_scrollable_canvas() {
        let fit = image_layout((4000, 3000), 1200.0, 800.0, 1.0);
        assert!(fit.image_width <= 1168.0);
        assert!(fit.image_height <= 768.0);
        assert_eq!(fit.canvas_width, 1200.0);
        assert_eq!(fit.canvas_height, 800.0);

        let enlarged = image_layout((4000, 3000), 1200.0, 800.0, 2.0);
        assert!(enlarged.canvas_width > 1200.0);
        assert!(enlarged.canvas_height > 800.0);
        assert_eq!(enlarged.image_height, fit.image_height * 2.0);
    }

    #[test]
    fn zoom_keeps_the_focal_point_stationary() {
        let image = (800, 400);
        let old = image_origin(image, 1200.0, 800.0, 1.0);
        let enlarged = image_origin(image, 1200.0, 800.0, 2.0);
        let offset = zoomed_scroll_offset(0.0, 600.0, 2.0, old.0, enlarged.0);
        assert_eq!(offset, -216.0);
        assert_eq!(
            zoomed_scroll_offset(offset, 600.0, 0.5, enlarged.0, old.0),
            0.0
        );
    }

    #[test]
    fn svg_uses_larger_render_target_when_zoomed() {
        let path = Path::new("drawing.SVG");
        assert!(is_svg(path));
        let fit = svg_raster_source(path, 100.0, 50.0, 2.0);
        let zoomed = svg_raster_source(path, 300.0, 150.0, 2.0);
        assert!(zoomed.width > fit.width);
        assert!(zoomed.height > fit.height);
        assert!(zoomed.width >= 600);
        assert_eq!((fit.width, fit.height), (224, 112));
        assert_eq!(zoomed.width, zoomed.height * 2);

        let huge = svg_raster_source(path, 20_000.0, 20_000.0, 2.0);
        assert!(huge.width <= 8192);
        assert!(huge.height <= 8192);
    }

    #[gpui::test]
    fn closing_image_evicts_svg_raster(cx: &mut gpui::TestAppContext) {
        let cache = SvgRasterCache::default();
        let source = svg_raster_source(Path::new("drawing.svg"), 100.0, 50.0, 2.0);
        cx.update(|cx| {
            let _ = cx.fetch_asset::<SvgRasterAsset>(&source);
            assert!(cx.has_asset::<SvgRasterAsset>(&source));
            cache.0.borrow_mut().active_source = Some(source.clone());
            cache.clear(cx);
            assert!(!cx.has_asset::<SvgRasterAsset>(&source));
            assert!(cx.fetch_asset::<SvgRasterAsset>(&source).1);
        });
    }
}
