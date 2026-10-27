//! Immutable application paint data in the document's original coordinate space.
//! Only public GPUI layout/paint APIs are used; no window pixels or Scene access.
use gpui::{
    AnyElement, App, Background, Bounds, BoxShadow, ContentMask, Corners, Edges, Element,
    ElementId, Fill, GlobalElementId, HighlightStyle, Hitbox, Hsla, ImageSource, Img,
    ImgLayoutState, ImgResourceLoader, InspectorElementId, IntoElement, LayoutId, LineLayout,
    ObjectFit, PaintQuad, ParentElement, PathVertex, Pixels, Point, Refineable, RenderImage,
    ShapedGlyph, ShapedLine, SharedString, Style, StyleRefinement, Styled, TextAlign, TextLayout,
    TextRun, Window, WrappedLine, WrappedLineLayout, fill, px, rgb, size,
};
use std::{cell::RefCell, ops::Range, rc::Rc, sync::Arc};

#[cfg(test)]
mod tests;

#[derive(Clone, Default)]
pub(crate) struct FrameSlot(Rc<RefCell<PublishedFrame>>);

#[derive(Default)]
struct PublishedFrame {
    frame: Option<Rc<PreviewFrame>>,
    paints: u8,
}

impl FrameSlot {
    pub fn latest(&self) -> Option<Rc<PreviewFrame>> {
        self.0.borrow().frame.clone()
    }

    pub fn paint_count(&self) -> u8 {
        self.0.borrow().paints
    }

    fn clear(&self) {
        *self.0.borrow_mut() = PublishedFrame::default();
    }

    fn publish(&self, frame: PreviewFrame) {
        let mut published = self.0.borrow_mut();
        published.frame = Some(Rc::new(frame));
        published.paints = published.paints.saturating_add(1);
    }
}

thread_local! {
    static RECORDING: RefCell<Option<Recording>> = const { RefCell::new(None) };
}

#[derive(Default)]
struct Recording {
    commands: Vec<Command>,
    pending_images: bool,
}

pub(crate) struct PreviewFrame {
    pub bounds: Bounds<Pixels>,
    commands: Vec<Command>,
}

struct Command {
    mask: Bounds<Pixels>,
    paint: Paint,
}

enum Paint {
    Quad(PaintQuad),
    Path {
        path: gpui::Path<Pixels>,
        color: Background,
    },
    Shadows {
        bounds: Bounds<Pixels>,
        radii: Corners<Pixels>,
        shadows: Vec<BoxShadow>,
    },
    // GPUI embeds large inline buffers; keep them out of every command's size.
    Shaped(Box<ShapedPaint>),
    Wrapped {
        line: RefCell<WrappedLine>,
        layout: Arc<WrappedLineLayout>,
        scaled: RefCell<Option<(f32, Arc<WrappedLineLayout>)>>,
        origin: Point<Pixels>,
        height: Pixels,
        align: TextAlign,
        bounds: Option<Bounds<Pixels>>,
        background: bool,
    },
    Image {
        image: Arc<RenderImage>,
        bounds: Bounds<Pixels>,
        radii: Corners<Pixels>,
        frame: usize,
    },
    Svg {
        bounds: Bounds<Pixels>,
        key: SharedString,
        data: &'static [u8],
        color: Hsla,
    },
}

struct ShapedPaint {
    line: ShapedLine,
    scaled: RefCell<Option<(f32, ShapedLine)>>,
    origin: Point<Pixels>,
    height: Pixels,
    align: TextAlign,
    width: Option<Pixels>,
    background: bool,
}

fn recording() -> bool {
    RECORDING.with(|state| state.borrow().is_some())
}
fn push(window: &Window, paint: Paint) {
    RECORDING.with(|state| {
        if let Some(recording) = state.borrow_mut().as_mut() {
            recording.commands.push(Command {
                mask: window.content_mask().bounds,
                paint,
            });
        }
    });
}

pub(crate) fn quad(window: &mut Window, quad: PaintQuad) {
    if recording() {
        push(window, Paint::Quad(quad.clone()));
    }
    window.paint_quad(quad);
}

pub(crate) fn drop_shadow(
    window: &mut Window,
    bounds: Bounds<Pixels>,
    radii: Corners<Pixels>,
    shadow: &BoxShadow,
) {
    if recording() {
        push(
            window,
            Paint::Shadows {
                bounds,
                radii,
                shadows: vec![shadow.clone()],
            },
        );
    }
    window.paint_drop_shadows(bounds, radii, std::slice::from_ref(shadow));
}

pub(crate) fn path(window: &mut Window, path: gpui::Path<Pixels>, color: impl Into<Background>) {
    let color = color.into();
    if recording() {
        push(
            window,
            Paint::Path {
                path: path.clone(),
                color,
            },
        );
    }
    window.paint_path(path, color);
}

pub(crate) fn image(
    window: &mut Window,
    bounds: Bounds<Pixels>,
    image_bounds: Bounds<Pixels>,
    radii: Corners<Pixels>,
    data: Arc<RenderImage>,
    frame: usize,
    grayscale: bool,
) -> gpui::Result<()> {
    if recording() {
        // The source clip remains separate from the image geometry.
        let mask = window.content_mask().bounds.intersect(&bounds);
        RECORDING.with(|state| {
            if let Some(recording) = state.borrow_mut().as_mut() {
                recording.commands.push(Command {
                    mask,
                    paint: Paint::Image {
                        image: data.clone(),
                        bounds: image_bounds,
                        radii,
                        frame,
                    },
                });
            }
        });
    }
    window.paint_image(bounds, image_bounds, radii, data, frame, grayscale)
}

pub(crate) fn shaped(
    line: &ShapedLine,
    origin: Point<Pixels>,
    height: Pixels,
    align: TextAlign,
    width: Option<Pixels>,
    background: bool,
    window: &mut Window,
) {
    if recording() {
        push(
            window,
            Paint::Shaped(Box::new(ShapedPaint {
                line: line.clone(),
                scaled: RefCell::default(),
                origin,
                height,
                align,
                width,
                background,
            })),
        );
    }
}

pub(crate) fn paint_shaped(
    line: &ShapedLine,
    origin: Point<Pixels>,
    height: Pixels,
    align: TextAlign,
    width: Option<Pixels>,
    window: &mut Window,
    cx: &mut App,
) -> gpui::Result<()> {
    shaped(line, origin, height, align, width, false, window);
    line.paint(origin, height, align, width, window, cx)
}
pub(crate) fn paint_wrapped(
    line: &WrappedLine,
    runs: &[TextRun],
    origin: Point<Pixels>,
    height: Pixels,
    window: &mut Window,
    cx: &mut App,
) -> gpui::Result<()> {
    record_wrapped(line, runs, origin, height, window);
    line.paint(origin, height, TextAlign::Left, None, window, cx)
}

/// Recreate decorations once from the same style runs, retaining the original
/// shaped glyphs and wrap boundaries even if the source contains custom offsets.
fn record_wrapped(
    line: &WrappedLine,
    runs: &[TextRun],
    origin: Point<Pixels>,
    height: Pixels,
    window: &mut Window,
) {
    if !recording() {
        return;
    }
    let Some(mut copy) = window
        .text_system()
        .shape_text(
            line.text.clone(),
            line.font_size(),
            runs,
            line.wrap_width,
            None,
        )
        .ok()
        .and_then(|mut lines| (!lines.is_empty()).then(|| lines.remove(0)))
    else {
        return;
    };
    let layout: Arc<WrappedLineLayout> = std::ops::Deref::deref(line).clone();
    *copy = layout.clone();
    push(
        window,
        Paint::Wrapped {
            line: RefCell::new(copy),
            layout,
            scaled: RefCell::default(),
            origin,
            height,
            align: TextAlign::Left,
            bounds: None,
            background: false,
        },
    );
}

fn scale_layout(layout: &LineLayout, scale: f32) -> Arc<LineLayout> {
    let mut runs = layout.runs.clone();
    for run in &mut runs {
        for glyph in &mut run.glyphs {
            glyph.position *= scale;
        }
    }
    Arc::new(LineLayout {
        font_size: layout.font_size * scale,
        width: layout.width * scale,
        ascent: layout.ascent * scale,
        descent: layout.descent * scale,
        runs,
        len: layout.len,
    })
}

fn rescale_layout(original: &LineLayout, scaled: &mut Arc<LineLayout>, scale: f32) {
    let Some(scaled) = Arc::get_mut(scaled) else {
        *scaled = scale_layout(original, scale);
        return;
    };
    scaled.font_size = original.font_size * scale;
    scaled.width = original.width * scale;
    scaled.ascent = original.ascent * scale;
    scaled.descent = original.descent * scale;
    for (run, source) in scaled.runs.iter_mut().zip(&original.runs) {
        for (glyph, source) in run.glyphs.iter_mut().zip(&source.glyphs) {
            glyph.position = source.position * scale;
        }
    }
}

fn scaled_wrapped(
    original: &Arc<WrappedLineLayout>,
    cache: &mut Option<(f32, Arc<WrappedLineLayout>)>,
    scale: f32,
) -> Arc<WrappedLineLayout> {
    if scale == 1. {
        return original.clone();
    }
    let (previous_scale, scaled) = cache.get_or_insert_with(|| {
        (
            scale,
            Arc::new(WrappedLineLayout {
                unwrapped_layout: scale_layout(&original.unwrapped_layout, scale),
                wrap_boundaries: original.wrap_boundaries.clone(),
                wrap_width: original.wrap_width.map(|width| width * scale),
            }),
        )
    });
    if *previous_scale != scale {
        if let Some(layout) = Arc::get_mut(scaled) {
            rescale_layout(
                &original.unwrapped_layout,
                &mut layout.unwrapped_layout,
                scale,
            );
            layout.wrap_width = original.wrap_width.map(|width| width * scale);
        } else {
            *scaled = Arc::new(WrappedLineLayout {
                unwrapped_layout: scale_layout(&original.unwrapped_layout, scale),
                wrap_boundaries: original.wrap_boundaries.clone(),
                wrap_width: original.wrap_width.map(|width| width * scale),
            });
        }
        *previous_scale = scale;
    }
    scaled.clone()
}

impl PreviewFrame {
    #[cfg(test)]
    pub(crate) fn image_count(&self) -> usize {
        self.commands
            .iter()
            .filter(|command| matches!(command.paint, Paint::Image { .. }))
            .count()
    }

    pub fn estimated_bytes(&self) -> usize {
        self.commands
            .iter()
            .map(|command| match &command.paint {
                Paint::Shaped(paint) => {
                    256 + paint
                        .line
                        .runs
                        .iter()
                        .map(|r| 2 * r.glyphs.len() * std::mem::size_of::<ShapedGlyph>())
                        .sum::<usize>()
                }
                Paint::Wrapped { layout, .. } => {
                    256 + layout
                        .runs()
                        .iter()
                        .map(|r| 2 * r.glyphs.len() * std::mem::size_of::<ShapedGlyph>())
                        .sum::<usize>()
                }
                Paint::Image { image, frame, .. } => {
                    image.as_bytes(*frame).map_or(0, |data| data.len())
                }
                Paint::Path { path, .. } => {
                    std::mem::size_of::<Command>()
                        + path.vertices.len() * std::mem::size_of::<PathVertex<Pixels>>()
                }
                Paint::Shadows { shadows, .. } => {
                    std::mem::size_of::<Command>()
                        + shadows.len() * std::mem::size_of::<BoxShadow>()
                }
                _ => std::mem::size_of::<Command>(),
            })
            .sum()
    }

    pub fn paint(&self, target: Bounds<Pixels>, window: &mut Window, cx: &mut App) {
        let source = self.bounds;
        let scale =
            (target.size.width / source.size.width).min(target.size.height / source.size.height);
        let position = |point: Point<Pixels>| target.origin + (point - source.origin) * scale;
        let rect = |bounds: Bounds<Pixels>| {
            Bounds::new(
                position(bounds.origin),
                size(bounds.size.width * scale, bounds.size.height * scale),
            )
        };
        window.with_content_mask(Some(ContentMask { bounds: target }), |window| {
            for command in &self.commands {
                let mask = rect(command.mask.intersect(&source)).intersect(&target);
                if mask.is_empty() {
                    continue;
                }
                window.with_content_mask(
                    Some(ContentMask { bounds: mask }),
                    |window| match &command.paint {
                        Paint::Quad(quad) => {
                            let mut quad = quad.clone();
                            quad.bounds = rect(quad.bounds);
                            quad.corner_radii = quad.corner_radii.map(|p| *p * scale);
                            quad.border_widths = quad.border_widths.map(|p| *p * scale);
                            window.paint_quad(quad);
                        }
                        Paint::Path { path, color } => {
                            let mut path = path.clone();
                            path.bounds = rect(path.bounds);
                            for vertex in &mut path.vertices {
                                vertex.xy_position = position(vertex.xy_position);
                                vertex.content_mask.bounds = rect(vertex.content_mask.bounds);
                            }
                            window.paint_path(path, *color);
                        }
                        Paint::Svg {
                            bounds,
                            key,
                            data,
                            color,
                        } => {
                            let _ = window.paint_svg(
                                rect(*bounds),
                                key.clone(),
                                Some(data),
                                Default::default(),
                                *color,
                                cx,
                            );
                        }
                        Paint::Shadows {
                            bounds,
                            radii,
                            shadows,
                        } => {
                            let shadows = shadows
                                .iter()
                                .cloned()
                                .map(|mut shadow| {
                                    shadow.offset *= scale;
                                    shadow.blur_radius *= scale;
                                    shadow.spread_radius *= scale;
                                    shadow
                                })
                                .collect::<Vec<_>>();
                            let radii = radii.map(|p| *p * scale);
                            window.paint_drop_shadows(rect(*bounds), radii, &shadows);
                            window.paint_inset_shadows(rect(*bounds), radii, &shadows);
                        }
                        Paint::Shaped(paint) => {
                            let ShapedPaint {
                                line,
                                scaled,
                                origin,
                                height,
                                align,
                                width,
                                background,
                            } = paint.as_ref();
                            let mut cache = scaled.borrow_mut();
                            let scaled = if scale == 1. {
                                line
                            } else {
                                let (previous_scale, scaled) = cache.get_or_insert_with(|| {
                                    let mut scaled = line.clone();
                                    *scaled = scale_layout(line, scale);
                                    (scale, scaled)
                                });
                                if *previous_scale != scale {
                                    rescale_layout(line, scaled, scale);
                                    *previous_scale = scale;
                                }
                                &*scaled
                            };
                            if *background {
                                let _ = scaled.paint_background(
                                    position(*origin),
                                    *height * scale,
                                    *align,
                                    width.map(|w| w * scale),
                                    window,
                                    cx,
                                );
                            } else {
                                let _ = scaled.paint(
                                    position(*origin),
                                    *height * scale,
                                    *align,
                                    width.map(|w| w * scale),
                                    window,
                                    cx,
                                );
                            }
                        }
                        Paint::Wrapped {
                            line,
                            layout,
                            scaled,
                            origin,
                            height,
                            align,
                            bounds,
                            background,
                        } => {
                            let mut line = line.borrow_mut();
                            **line = scaled_wrapped(layout, &mut scaled.borrow_mut(), scale);
                            if *background {
                                let _ = line.paint_background(
                                    position(*origin),
                                    *height * scale,
                                    *align,
                                    bounds.map(rect),
                                    window,
                                    cx,
                                );
                            }
                            let _ = line.paint(
                                position(*origin),
                                *height * scale,
                                *align,
                                bounds.map(rect),
                                window,
                                cx,
                            );
                            **line = layout.clone();
                        }
                        Paint::Image {
                            image,
                            bounds,
                            radii,
                            frame,
                        } => {
                            let bounds = rect(*bounds);
                            let _ = window.paint_image(
                                bounds,
                                bounds,
                                radii.map(|p| *p * scale),
                                image.clone(),
                                *frame,
                                false,
                            );
                        }
                    },
                );
            }
        });
    }
}

type MediaReady = Box<dyn Fn(&App) -> bool>;

pub(crate) struct DocumentStage {
    content: AnyElement,
    slot: FrameSlot,
    enabled: bool,
    ready: bool,
    media_ready: Option<MediaReady>,
}
impl DocumentStage {
    pub fn new(content: impl IntoElement, slot: FrameSlot, enabled: bool) -> Self {
        Self {
            content: content.into_any_element(),
            slot,
            enabled,
            ready: true,
            media_ready: None,
        }
    }

    pub fn with_ready(mut self, ready: bool) -> Self {
        self.ready = ready;
        self
    }

    pub fn with_media_ready(mut self, ready: impl Fn(&App) -> bool + 'static) -> Self {
        self.media_ready = Some(Box::new(ready));
        self
    }
}

impl Element for DocumentStage {
    type RequestLayoutState = ();
    type PrepaintState = ();
    fn id(&self) -> Option<ElementId> {
        None
    }
    fn source_location(&self) -> Option<&'static core::panic::Location<'static>> {
        None
    }
    fn request_layout(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, ()) {
        (self.content.request_layout(window, cx), ())
    }
    fn prepaint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        _: Bounds<Pixels>,
        _: &mut (),
        window: &mut Window,
        cx: &mut App,
    ) {
        self.content.prepaint(window, cx);
    }
    fn paint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _: &mut (),
        _: &mut (),
        window: &mut Window,
        cx: &mut App,
    ) {
        if !self.enabled {
            self.content.paint(window, cx);
            return;
        }
        if !self.ready || self.media_ready.as_ref().is_some_and(|ready| !ready(cx)) {
            // A loading pane must never inherit a previous document's frame
            // merely because it has the same viewport dimensions.
            self.slot.clear();
            self.content.paint(window, cx);
            return;
        }
        let scope = RecordingScope(Some(
            RECORDING.with(|state| state.replace(Some(Recording::default()))),
        ));
        // The base color belongs to this immutable frame, just like its text.
        // Replaying must not mix the recorded palette with the current theme.
        quad(
            window,
            fill(bounds, rgb(crate::theme::current_theme().background)),
        );
        self.content.paint(window, cx);
        let recording = scope.finish();
        if recording.pending_images {
            self.slot.clear();
        } else {
            self.slot.publish(PreviewFrame {
                bounds,
                commands: recording.commands,
            });
        }
    }
}
impl IntoElement for DocumentStage {
    type Element = Self;
    fn into_element(self) -> Self {
        self
    }
}

struct RecordingScope(Option<Option<Recording>>);
impl RecordingScope {
    fn finish(mut self) -> Recording {
        RECORDING
            .with(|state| state.replace(self.0.take().unwrap()))
            .unwrap_or_default()
    }
}
impl Drop for RecordingScope {
    fn drop(&mut self) {
        if let Some(previous) = self.0.take() {
            RECORDING.with(|state| state.replace(previous));
        }
    }
}

/// A normal StyledText with the style recipe retained alongside its public layout.
/// Native GPUI still owns measurement and normal document painting.
pub(crate) struct RecordedText {
    native: gpui::StyledText,
    highlights: Vec<(Range<usize>, HighlightStyle)>,
}
impl RecordedText {
    pub fn new(text: impl Into<SharedString>) -> Self {
        Self {
            native: gpui::StyledText::new(text),
            highlights: Vec::new(),
        }
    }
    pub fn layout(&self) -> &TextLayout {
        self.native.layout()
    }
    pub fn with_highlights(
        mut self,
        highlights: impl IntoIterator<Item = (Range<usize>, HighlightStyle)>,
    ) -> Self {
        self.highlights = highlights.into_iter().collect();
        self.native = self.native.with_highlights(self.highlights.clone());
        self
    }
    pub fn with_font_family_overrides(
        mut self,
        overrides: impl IntoIterator<Item = (Range<usize>, SharedString)>,
    ) -> Self {
        self.native = self.native.with_font_family_overrides(overrides);
        self
    }
}
impl Element for RecordedText {
    type RequestLayoutState = ();
    type PrepaintState = ();
    fn id(&self) -> Option<ElementId> {
        None
    }
    fn source_location(&self) -> Option<&'static core::panic::Location<'static>> {
        None
    }
    fn request_layout(
        &mut self,
        id: Option<&GlobalElementId>,
        inspector: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, ()) {
        self.native.request_layout(id, inspector, window, cx)
    }
    fn prepaint(
        &mut self,
        id: Option<&GlobalElementId>,
        inspector: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        state: &mut (),
        window: &mut Window,
        cx: &mut App,
    ) {
        self.native
            .prepaint(id, inspector, bounds, state, window, cx);
    }
    fn paint(
        &mut self,
        id: Option<&GlobalElementId>,
        inspector: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        state: &mut (),
        prepaint: &mut (),
        window: &mut Window,
        cx: &mut App,
    ) {
        if recording() {
            let layout = self.native.layout();
            let text: SharedString = layout.text().into();
            let style = window.text_style();
            let mut runs = Vec::new();
            let mut start = 0;
            for (range, highlight) in &self.highlights {
                if start < range.start {
                    runs.push(style.to_run(range.start - start));
                }
                runs.push(style.clone().highlight(*highlight).to_run(range.len()));
                start = range.end;
            }
            if start < text.len() {
                runs.push(style.to_run(text.len() - start));
            }
            let originals = layout.line_layouts();
            let font = originals.first().map_or(px(14.), |line| line.font_size());
            if let Ok(lines) = window.text_system().shape_text(
                text,
                font,
                &runs,
                originals.first().and_then(|line| line.wrap_width),
                None,
            ) {
                let mut origin = bounds.origin;
                for (mut line, original) in lines.into_iter().zip(originals) {
                    *line = original.clone();
                    let height = line.size(layout.line_height()).height;
                    push(
                        window,
                        Paint::Wrapped {
                            line: RefCell::new(line),
                            layout: original,
                            scaled: RefCell::default(),
                            origin,
                            height: layout.line_height(),
                            align: window.text_style().text_align,
                            bounds: Some(bounds),
                            background: true,
                        },
                    );
                    origin.y += height;
                }
            }
        }
        self.native
            .paint(id, inspector, bounds, state, prepaint, window, cx);
    }
}
impl IntoElement for RecordedText {
    type Element = Self;
    fn into_element(self) -> Self {
        self
    }
}

pub(crate) struct RecordedBox {
    content: AnyElement,
    style: Style,
}
pub(crate) fn record_box(mut content: impl IntoElement + Styled) -> RecordedBox {
    let mut style = Style::default();
    style.refine(content.style());
    RecordedBox {
        content: content.into_any_element(),
        style,
    }
}
impl Element for RecordedBox {
    type RequestLayoutState = ();
    type PrepaintState = ();
    fn id(&self) -> Option<ElementId> {
        None
    }
    fn source_location(&self) -> Option<&'static core::panic::Location<'static>> {
        None
    }
    fn request_layout(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        w: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, ()) {
        (self.content.request_layout(w, cx), ())
    }
    fn prepaint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        _: Bounds<Pixels>,
        _: &mut (),
        w: &mut Window,
        cx: &mut App,
    ) {
        self.content.prepaint(w, cx);
    }
    fn paint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _: &mut (),
        _: &mut (),
        w: &mut Window,
        cx: &mut App,
    ) {
        let radii = self
            .style
            .corner_radii
            .to_pixels(w.rem_size())
            .clamp_radii_for_quad_size(bounds.size);
        if recording() {
            if !self.style.box_shadow.is_empty() {
                push(
                    w,
                    Paint::Shadows {
                        bounds,
                        radii,
                        shadows: self.style.box_shadow.clone(),
                    },
                );
            }
            if let Some(background) = self.style.background.as_ref().and_then(Fill::color) {
                let mut border = background.as_solid().unwrap_or_default();
                border.a = 0.;
                push(
                    w,
                    Paint::Quad(gpui::quad(
                        bounds,
                        radii,
                        background,
                        Edges::default(),
                        border,
                        self.style.border_style,
                    )),
                );
            }
        }
        self.content.paint(w, cx);
        if recording() {
            let widths = self.style.border_widths.to_pixels(w.rem_size());
            if let Some(color) = self.style.border_color {
                let mut background = color;
                background.a = 0.;
                push(
                    w,
                    Paint::Quad(gpui::quad(
                        bounds,
                        radii,
                        background,
                        widths,
                        color,
                        self.style.border_style,
                    )),
                );
            }
        }
    }
}
impl IntoElement for RecordedBox {
    type Element = Self;
    fn into_element(self) -> Self {
        self
    }
}

/// Fixed-size status icons share the same public painter in live and preview views.
pub(crate) fn svg(key: &'static str, data: &'static [u8]) -> gpui::Div {
    let key: SharedString = format!("org-studio-preview/{key}").into();
    gpui::div().child(
        gpui::canvas(
            |_, _, _| (),
            move |bounds, _, window, cx| {
                let color = window.text_style().color;
                if recording() {
                    push(
                        window,
                        Paint::Svg {
                            bounds,
                            key: key.clone(),
                            data,
                            color,
                        },
                    );
                }
                let _ = window.paint_svg(
                    bounds,
                    key.clone(),
                    Some(data),
                    Default::default(),
                    color,
                    cx,
                );
            },
        )
        .size_full(),
    )
}

pub(crate) struct RecordedImage {
    native: Img,
    source: ImageSource,
}
pub(crate) fn img(source: impl Into<ImageSource>) -> RecordedImage {
    let source = source.into();
    RecordedImage {
        native: gpui::img(source.clone()),
        source,
    }
}
impl Styled for RecordedImage {
    fn style(&mut self) -> &mut StyleRefinement {
        self.native.style()
    }
}
impl IntoElement for RecordedImage {
    type Element = Self;
    fn into_element(self) -> Self {
        self
    }
}
impl Element for RecordedImage {
    type RequestLayoutState = ImgLayoutState;
    type PrepaintState = Option<Hitbox>;
    fn id(&self) -> Option<ElementId> {
        Element::id(&self.native)
    }
    fn source_location(&self) -> Option<&'static core::panic::Location<'static>> {
        None
    }
    fn request_layout(
        &mut self,
        id: Option<&GlobalElementId>,
        inspector: Option<&InspectorElementId>,
        w: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, Self::RequestLayoutState) {
        self.native.request_layout(id, inspector, w, cx)
    }
    fn prepaint(
        &mut self,
        id: Option<&GlobalElementId>,
        inspector: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        state: &mut Self::RequestLayoutState,
        w: &mut Window,
        cx: &mut App,
    ) -> Self::PrepaintState {
        self.native.prepaint(id, inspector, bounds, state, w, cx)
    }
    fn paint(
        &mut self,
        id: Option<&GlobalElementId>,
        inspector: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        state: &mut Self::RequestLayoutState,
        prepaint: &mut Self::PrepaintState,
        w: &mut Window,
        cx: &mut App,
    ) {
        if recording() {
            let data = match &self.source {
                ImageSource::Resource(resource) => w.use_asset::<ImgResourceLoader>(resource, cx),
                ImageSource::Render(image) => Some(Ok(image.clone())),
                ImageSource::Image(image) => image.clone().use_render_image(w, cx).map(Ok),
                ImageSource::Custom(loader) => loader(w, cx),
            };
            if data.is_none() {
                RECORDING.with(|state| {
                    if let Some(recording) = state.borrow_mut().as_mut() {
                        recording.pending_images = true;
                    }
                });
            }
            if let Some(Ok(image)) = data {
                let dimensions = image.size(0);
                let fitted = ObjectFit::Contain.get_bounds(bounds, dimensions);
                let mut style = Style::default();
                style.refine(self.native.style());
                push(
                    w,
                    Paint::Image {
                        image,
                        bounds: fitted,
                        radii: style.corner_radii.to_pixels(w.rem_size()),
                        frame: 0,
                    },
                );
            }
        }
        self.native
            .paint(id, inspector, bounds, state, prepaint, w, cx);
    }
}
