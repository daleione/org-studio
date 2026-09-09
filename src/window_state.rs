//! Persists the main window's placement so the next launch reopens on the
//! same display and at the same size and position, mirroring the frame
//! autosave behavior of conventional macOS apps.
//!
//! Without this, `preview_window_options` always re-centers on GPUI's primary
//! display (the screen with the menu bar), which is commonly an external
//! monitor: the window kept appearing there no matter where the user clicked
//! the Dock icon. Once the user moves the window, this state makes the app
//! remember that placement across launches and window reopens.
//!
//! GPUI places windows in *display-relative* coordinates: `Window::bounds()`
//! is relative to the top-left of the window's current display, and the
//! display itself is tracked separately by id. The stored frame therefore
//! pairs a display-relative bounds with the id of the display the window's
//! center was on at save time, and only means something again if that display
//! is still connected.

use gpui::{App, Bounds, DisplayId, Pixels, Size, point, px, size};
use serde::{Deserialize, Serialize};
use std::{
    fs, io,
    path::PathBuf,
    sync::{Mutex, OnceLock, mpsc},
};

const STORE_VERSION: u32 = 1;
const STORE_FILE: &str = "window-frame.json";
/// Plausibility guards for loaded state. Real displays are far smaller, so any
/// value beyond these limits is treated as corrupt state rather than a window
/// placement.
const MAX_ABSOLUTE_COORDINATE: f32 = 1_000_000.0;
const MIN_WINDOW_EXTENT: f32 = 40.0;
const MAX_WINDOW_EXTENT: f32 = 100_000.0;

/// The last-known placement of the main window.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct WindowFrame {
    /// Display-relative top-left origin, in pixels.
    origin: [f32; 2],
    /// Window size, in pixels.
    size: [f32; 2],
    /// Id of the display the window was on at save time.
    display_id: Option<u64>,
}

impl WindowFrame {
    pub fn from_window(bounds: Bounds<Pixels>, display_id: Option<DisplayId>) -> Self {
        Self {
            origin: [bounds.origin.x.as_f32(), bounds.origin.y.as_f32()],
            size: [bounds.size.width.as_f32(), bounds.size.height.as_f32()],
            display_id: display_id.map(u64::from),
        }
    }

    pub fn bounds(&self) -> Bounds<Pixels> {
        Bounds::new(
            point(px(self.origin[0]), px(self.origin[1])),
            size(px(self.size[0]), px(self.size[1])),
        )
    }

    pub fn display_id(&self) -> Option<DisplayId> {
        self.display_id.map(DisplayId::new)
    }

    fn is_plausible(&self) -> bool {
        let finite = |value: f32| value.is_finite() && value.abs() <= MAX_ABSOLUTE_COORDINATE;
        self.origin.iter().all(|value| finite(*value))
            && self
                .size
                .iter()
                .all(|value| (MIN_WINDOW_EXTENT..=MAX_WINDOW_EXTENT).contains(value))
    }
}

/// Plain-data snapshot of a display's visible area, so placement logic stays
/// testable without GPUI platform handles. Bounds are display-relative, the
/// same space `Window::bounds()` is reported in.
#[derive(Clone, Copy, Debug)]
pub struct DisplayInfo {
    pub id: DisplayId,
    pub visible_bounds: Bounds<Pixels>,
}

/// Restores a saved frame onto a currently connected display.
///
/// The saved bounds are relative to the display the window last lived on, so
/// they only make sense again when that display is still connected; otherwise
/// `None` is returned and the caller falls back to a centered placement. The
/// restored bounds are always clamped into the display's visible area, so a
/// saved placement can never reopen off-screen.
pub fn restore_frame_placement(
    displays: &[DisplayInfo],
    frame: &WindowFrame,
) -> Option<(Bounds<Pixels>, DisplayId)> {
    if !frame.is_plausible() || displays.is_empty() {
        return None;
    }
    let display = frame
        .display_id()
        .and_then(|id| displays.iter().find(|display| display.id == id))?;
    Some((
        clamp_to_visible(frame.bounds(), display.visible_bounds),
        display.id,
    ))
}

/// Clamps a window so it fits fully inside a display's visible area, keeping
/// its size when it already fits.
pub fn clamp_to_visible(bounds: Bounds<Pixels>, visible: Bounds<Pixels>) -> Bounds<Pixels> {
    let visible_origin = visible.origin;
    let width = bounds.size.width.as_f32().min(visible.size.width.as_f32());
    let height = bounds
        .size
        .height
        .as_f32()
        .min(visible.size.height.as_f32());
    let max_x = visible_origin.x.as_f32() + visible.size.width.as_f32() - width;
    let max_y = visible_origin.y.as_f32() + visible.size.height.as_f32() - height;
    let origin_x = if max_x < visible_origin.x.as_f32() {
        visible_origin.x.as_f32()
    } else {
        bounds
            .origin
            .x
            .as_f32()
            .clamp(visible_origin.x.as_f32(), max_x)
    };
    let origin_y = if max_y < visible_origin.y.as_f32() {
        visible_origin.y.as_f32()
    } else {
        bounds
            .origin
            .y
            .as_f32()
            .clamp(visible_origin.y.as_f32(), max_y)
    };
    Bounds::new(
        point(px(origin_x), px(origin_y)),
        size(px(width), px(height)),
    )
}

/// Returns the placement for the main window, preferring the persisted frame:
///
/// - the saved display is still connected → restored bounds on it;
/// - a saved frame exists but its display is gone → centered on the primary
///   display at the saved size;
/// - no saved frame → centered on the primary display at `default_size`.
pub fn restored_placement(
    cx: &App,
    default_size: Size<Pixels>,
) -> (Bounds<Pixels>, Option<DisplayId>) {
    let displays = cx
        .displays()
        .iter()
        .map(|display| DisplayInfo {
            id: display.id(),
            visible_bounds: display.visible_bounds(),
        })
        .collect::<Vec<_>>();
    let Some(frame) = load() else {
        return (Bounds::centered(None, default_size, cx), None);
    };
    match restore_frame_placement(&displays, &frame) {
        Some((bounds, display_id)) => (bounds, Some(display_id)),
        None => {
            let bounds = Bounds::centered(None, frame.bounds().size, cx);
            let bounds = cx
                .primary_display()
                .map(|display| clamp_to_visible(bounds, display.visible_bounds()))
                .unwrap_or(bounds);
            (bounds, None)
        }
    }
}

/// Records the window's current placement and schedules a coalesced disk
/// write. Cheap enough to call from a move/resize observer on every frame of
/// a drag; the writer thread coalesces back-to-back updates.
pub fn remember(bounds: Bounds<Pixels>, display_id: Option<DisplayId>) {
    *last_frame().lock().unwrap() = Some(WindowFrame::from_window(bounds, display_id));
    save_async();
}

/// Synchronously persists the last remembered placement. Called when the main
/// window closes so the next launch restores it even if the process exits
/// before the background writer drains.
pub fn flush() {
    let Some(frame) = *last_frame().lock().unwrap() else {
        return;
    };
    if let Err(error) = save(&frame) {
        eprintln!("org_studio_window_frame_save_failed error={error}");
    }
}

fn last_frame() -> &'static Mutex<Option<WindowFrame>> {
    static LAST_FRAME: OnceLock<Mutex<Option<WindowFrame>>> = OnceLock::new();
    LAST_FRAME.get_or_init(|| Mutex::new(None))
}

fn load() -> Option<WindowFrame> {
    let path = store_path()?;
    let source = fs::read_to_string(path).ok()?;
    let stored = serde_json::from_str::<StoredWindowFrame>(&source).ok()?;
    (stored.version == STORE_VERSION)
        .then_some(stored.frame)
        .filter(WindowFrame::is_plausible)
}

fn save_async() {
    static SENDER: OnceLock<mpsc::Sender<()>> = OnceLock::new();
    let sender = SENDER.get_or_init(|| {
        let (sender, receiver) = mpsc::channel::<()>();
        std::thread::Builder::new()
            .name("org-studio-window-frame".into())
            .spawn(move || {
                while receiver.recv().is_ok() {
                    while receiver.try_recv().is_ok() {}
                    let Some(frame) = *last_frame().lock().unwrap() else {
                        continue;
                    };
                    if let Err(error) = save(&frame) {
                        eprintln!("org_studio_window_frame_save_failed error={error}");
                    }
                }
            })
            .expect("window frame writer thread must start");
        sender
    });
    let _ = sender.send(());
}

fn save(frame: &WindowFrame) -> io::Result<()> {
    let Some(path) = store_path() else {
        return Ok(());
    };
    let Some(parent) = path.parent() else {
        return Ok(());
    };
    fs::create_dir_all(parent)?;
    let temporary = path.with_extension(format!("tmp-{}", std::process::id()));
    let source = serde_json::to_vec_pretty(&StoredWindowFrame {
        version: STORE_VERSION,
        frame: *frame,
    })?;
    fs::write(&temporary, source)?;
    fs::rename(temporary, path)
}

fn store_path() -> Option<PathBuf> {
    crate::settings::application_support_dir().map(|path| path.join(STORE_FILE))
}

#[derive(Serialize, Deserialize)]
struct StoredWindowFrame {
    version: u32,
    frame: WindowFrame,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn display(id: u64, width: f32, height: f32) -> DisplayInfo {
        DisplayInfo {
            id: DisplayId::new(id),
            visible_bounds: Bounds::new(point(px(0.), px(0.)), size(px(width), px(height))),
        }
    }

    fn frame(display_id: u64, x: f32, y: f32, width: f32, height: f32) -> WindowFrame {
        WindowFrame {
            origin: [x, y],
            size: [width, height],
            display_id: Some(display_id),
        }
    }

    #[test]
    fn restores_a_frame_unchanged_on_its_display() {
        let displays = [display(1, 1920., 1080.), display(2, 2560., 1600.)];
        let saved = frame(2, 100., 200., 1200., 800.);
        let (bounds, display_id) =
            restore_frame_placement(&displays, &saved).expect("display 2 is connected");
        assert_eq!(display_id, DisplayId::new(2));
        assert_eq!(bounds.origin.x, px(100.));
        assert_eq!(bounds.origin.y, px(200.));
        assert_eq!(bounds.size.width, px(1200.));
        assert_eq!(bounds.size.height, px(800.));
    }

    #[test]
    fn returns_none_when_the_saved_display_is_gone() {
        let displays = [display(1, 1920., 1080.)];
        let saved = frame(2, 100., 200., 1200., 800.);
        assert_eq!(restore_frame_placement(&displays, &saved), None);
    }

    #[test]
    fn clamps_an_offscreen_frame_into_the_visible_area() {
        let displays = [display(1, 1920., 1080.)];
        // Window hangs off the right and bottom edges of display 1.
        let saved = frame(1, 1700., 900., 1200., 800.);
        let (bounds, _) = restore_frame_placement(&displays, &saved).expect("display 1 connected");
        assert_eq!(bounds.origin.x, px(720.)); // 1920 - 1200
        assert_eq!(bounds.origin.y, px(280.)); // 1080 - 800
    }

    #[test]
    fn shrinks_a_frame_larger_than_the_visible_area() {
        let displays = [display(1, 1920., 1080.)];
        let saved = frame(1, 0., 0., 4000., 3000.);
        let (bounds, _) = restore_frame_placement(&displays, &saved).expect("display 1 connected");
        assert_eq!(bounds.size.width, px(1920.));
        assert_eq!(bounds.size.height, px(1080.));
        assert_eq!(bounds.origin, point(px(0.), px(0.)));
    }

    #[test]
    fn rejects_implausible_frames() {
        let displays = [display(1, 1920., 1080.)];
        let not_finite = WindowFrame {
            origin: [f32::NAN, 0.],
            size: [1200., 800.],
            display_id: Some(1),
        };
        assert_eq!(restore_frame_placement(&displays, &not_finite), None);

        let tiny = WindowFrame {
            origin: [0., 0.],
            size: [1., 800.],
            display_id: Some(1),
        };
        assert_eq!(restore_frame_placement(&displays, &tiny), None);
    }

    #[test]
    fn serde_round_trip_preserves_bounds_and_display() {
        let saved = frame(2, 100., 200., 1200., 800.);
        let stored = StoredWindowFrame {
            version: STORE_VERSION,
            frame: saved,
        };
        let encoded = serde_json::to_vec_pretty(&stored).unwrap();
        let decoded: StoredWindowFrame = serde_json::from_slice(&encoded).unwrap();
        assert_eq!(decoded.frame, saved);
        assert_eq!(decoded.frame.display_id(), Some(DisplayId::new(2)));
        assert_eq!(decoded.frame.bounds().size.width, px(1200.));
    }
}
