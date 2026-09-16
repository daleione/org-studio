pub(crate) const SIDEBAR_WIDTH: f32 = 220.0;
pub(crate) const SIDEBAR_MIN_WIDTH: f32 = 180.0;
pub(crate) const SIDEBAR_MAX_WIDTH: f32 = 420.0;
pub(crate) const SIDEBAR_RESIZE_HANDLE_WIDTH: f32 = 6.0;
pub(crate) const SIDEBAR_ITEM_HEIGHT: f32 = 39.0;
pub(crate) const SIDEBAR_ITEM_GAP: f32 = 4.0;
pub(crate) const SIDEBAR_SAVED_VIEW_HEIGHT: f32 = 36.0;
pub(crate) const SIDEBAR_TAG_HEIGHT: f32 = 35.0;
pub(crate) const SIDEBAR_SOURCE_HEIGHT: f32 = 32.0;
pub(crate) const SIDEBAR_SOURCE_BOTTOM_PADDING: f32 = 16.0;
pub(crate) const MIN_CONTENT_WIDTH: f32 = 360.0;
pub(crate) const TASK_ROW_HEIGHT: f32 = 45.0;
pub(crate) const DAY_HEADER_HEIGHT: f32 = 34.0;
pub(crate) const COLUMN_HEADER_HEIGHT: f32 = 44.0;
// Agenda colors route through the global theme so the whole surface follows
// the titlebar switch. Same-name functions keep every existing =style::X=
// call site unchanged while letting light and dark both resolve at paint time.
#[allow(non_snake_case)]
pub(crate) fn INK() -> u32 {
    palette(0x292b31, 0xe6edf3)
}

#[allow(non_snake_case)]
pub(crate) fn MUTED() -> u32 {
    palette(0x767a82, 0x8b98a7)
}

#[allow(non_snake_case)]
pub(crate) fn BORDER() -> u32 {
    palette(0xdedfe2, 0x253445)
}

#[allow(non_snake_case)]
pub(crate) fn SIDEBAR() -> u32 {
    palette(0xf5f6f6, 0x18232f)
}

#[allow(non_snake_case)]
pub(crate) fn TOOLBAR() -> u32 {
    palette(0xffffff, 0x18232f)
}

#[allow(non_snake_case)]
pub(crate) fn PURPLE() -> u32 {
    palette(0x70417c, 0xc792ea)
}

#[allow(non_snake_case)]
pub(crate) fn PURPLE_SELECTION() -> u32 {
    palette(0xf0ecf9, 0x2b2f42)
}

#[allow(non_snake_case)]
pub(crate) fn BLUE_SELECTION() -> u32 {
    palette(0xeff6ff, 0x1b3a5c)
}

// Agenda date-header purple family. The three light shades encode state
// (today / weekend / ordinary), so they keep dedicated dispatch entries
// instead of collapsing into one token; dark mode steps brightness, not hue.
#[allow(non_snake_case)]
pub(crate) fn HEADER_TODAY() -> u32 {
    palette(0xd291d8, 0xc792ea)
}

#[allow(non_snake_case)]
pub(crate) fn HEADER_WEEKEND() -> u32 {
    palette(0x732b79, 0xa673c9)
}

#[allow(non_snake_case)]
pub(crate) fn HEADER_DATE() -> u32 {
    palette(0xb54cbd, 0xc792ea)
}

fn palette(light: u32, dark: u32) -> u32 {
    if crate::theme::effective_theme_is_dark() {
        dark
    } else {
        light
    }
}
