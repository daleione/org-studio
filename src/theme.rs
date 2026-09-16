#[derive(Clone, Copy)]
pub struct Theme {
    pub search_current: u32,
    pub search_match: u32,
    // Surface hierarchy: window chrome, document body, panels, popovers and
    // interaction states. Light and dark keep the same layering order so
    // callers can swap themes without per-surface conditionals.
    pub window_background: u32,
    pub editor_background: u32,
    pub surface: u32,
    pub elevated: u32,
    pub hover: u32,
    pub selected: u32,
    pub background: u32,
    pub background_alt: u32,
    pub foreground: u32,
    pub foreground_dim: u32,
    pub foreground_muted: u32,
    pub foreground_disabled: u32,
    pub line_number: u32,
    pub border: u32,
    pub border_hover: u32,
    pub divider: u32,
    pub heading: [u32; 4],
    pub heading_bullets: [&'static str; 4],
    pub accent: u32,
    pub accent_bright: u32,
    pub accent_bg: u32,
    pub accent_border: u32,
    pub success: u32,
    pub info: u32,
    pub warning: u32,
    pub error: u32,
    pub status_bar: u32,
    pub echo_area: u32,
    pub code_background: u32,
    pub code_active_background: u32,
    pub code_boundary_background: u32,
    pub code_block_accent: u32,
    pub code_foreground: u32,
    pub code_boundary: u32,
    pub quote: u32,
    pub link: u32,
    pub link_external: u32,
    pub link_file: u32,
    pub link_internal: u32,
    pub link_mail: u32,
    pub link_warn: u32,
    pub link_other: u32,
    pub meta: u32,
    pub inline_code: u32,
    pub verbatim: u32,
    pub inline_code_background: u32,
    pub date: u32,
    pub todo: u32,
    pub todo_active: u32,
    pub todo_project: u32,
    pub waiting: u32,
    pub done: u32,
    pub keyword: u32,
    pub string: u32,
    pub comment: u32,
    pub type_name: u32,
    pub function: u32,
    pub constant: u32,
    pub number: u32,
    pub variable: u32,
    pub operator: u32,
    pub attribute: u32,
}

// The light palette used by orgmode.org's feature illustrations. The website
// assets only define the core Org faces, so the remaining editor faces extend
// the same red/orange/green/blue vocabulary.
pub static ORG_STUDIO_LIGHT: Theme = Theme {
    search_current: 0xe6a00088,
    search_match: 0xe6cc0044,
    window_background: 0xffffff,
    editor_background: 0xffffff,
    surface: 0xf7f7f8,
    elevated: 0xf0f0f2,
    hover: 0xe9ebf0,
    selected: 0xdce4ee,
    background: 0xffffff,
    background_alt: 0xf7f7f8,
    foreground: 0x373942,
    foreground_dim: 0x92949d,
    foreground_muted: 0x777b82,
    foreground_disabled: 0xaab0b8,
    line_number: 0x92949d,
    border: 0xdedfe3,
    border_hover: 0xb8bcc4,
    divider: 0xe4e5e8,
    heading: [0xe45549, 0xd98547, 0x4fa14e, 0x3f78f2],
    heading_bullets: ["*", "**", "***", "****"],
    accent: 0x3f78f2,
    accent_bright: 0x1688ff,
    accent_bg: 0xdce4ee,
    accent_border: 0x8eb9df,
    success: 0x4fa14e,
    info: 0x3f78f2,
    warning: 0x986801,
    error: 0xe45549,
    status_bar: 0xf0f0f2,
    echo_area: 0xf7faff,
    code_background: 0xe6e6e6,
    code_active_background: 0xf0f0f0,
    code_boundary_background: 0xc8c8c8,
    code_block_accent: 0xbdbdbd,
    code_foreground: 0x373942,
    code_boundary: 0x373942,
    quote: 0x686b75,
    link: 0x3f78f2,
    link_external: 0x3f78f2,
    link_file: 0x4fa14e,
    link_internal: 0xb751b6,
    link_mail: 0x0084a0,
    link_warn: 0xe45549,
    link_other: 0x84888b,
    meta: 0xd98547,
    inline_code: 0xd98547,
    verbatim: 0x4fa14e,
    inline_code_background: 0xffffff,
    date: 0x3f78f2,
    todo: 0x50a14f,
    todo_active: 0xb751b6,
    todo_project: 0x84888b,
    waiting: 0x986801,
    done: 0x383a42,
    keyword: 0xe45549,
    string: 0x4fa14e,
    comment: 0xa0a1a7,
    type_name: 0x986801,
    function: 0xa626a4,
    constant: 0x986801,
    number: 0xd98547,
    variable: 0xa626a4,
    operator: 0x373942,
    attribute: 0x3f78f2,
};

// Org Studio Dark follows the product dark palette: a low-saturation cold
// blue-grey canvas where layering is expressed through luminance steps and a
// single blue accent. Semantic faces reuse the accent/status vocabulary so no
// face needs its own bespoke dark constant.
pub static ORG_STUDIO_DARK: Theme = Theme {
    search_current: 0x66b3ff66,
    search_match: 0x4da3ff33,
    window_background: 0x0f1823,
    editor_background: 0x121d28,
    surface: 0x18232f,
    elevated: 0x1e2b39,
    hover: 0x253444,
    selected: 0x1b3a5c,
    background: 0x121d28,
    background_alt: 0x18232f,
    foreground: 0xe6edf3,
    foreground_dim: 0x8b98a7,
    foreground_muted: 0x667484,
    foreground_disabled: 0x465362,
    line_number: 0x536171,
    border: 0x253445,
    border_hover: 0x33455c,
    divider: 0x182430,
    heading: [0x5cb8ff, 0x36d399, 0xffcb6b, 0xc792ea],
    heading_bullets: ["*", "**", "***", "****"],
    accent: 0x4da3ff,
    accent_bright: 0x66b3ff,
    accent_bg: 0x1b3a5c,
    accent_border: 0x2f6394,
    success: 0x36d399,
    info: 0x4da3ff,
    warning: 0xf5b84b,
    error: 0xf06a7a,
    status_bar: 0x0f1823,
    echo_area: 0x18232f,
    code_background: 0x16222e,
    code_active_background: 0x1e2b39,
    code_boundary_background: 0x1a2635,
    code_block_accent: 0x2f6394,
    code_foreground: 0xe6edf3,
    code_boundary: 0x5f6d7c,
    quote: 0xc5ced8,
    link: 0x4da3ff,
    link_external: 0x4da3ff,
    link_file: 0x36d399,
    link_internal: 0xc792ea,
    link_mail: 0x66b3ff,
    link_warn: 0xf06a7a,
    link_other: 0x667484,
    meta: 0xf5b84b,
    inline_code: 0xffcb6b,
    verbatim: 0x36d399,
    inline_code_background: 0x18232f,
    date: 0x4da3ff,
    todo: 0x36d399,
    todo_active: 0xc792ea,
    todo_project: 0x8b98a7,
    waiting: 0xf5b84b,
    done: 0x637587,
    keyword: 0xc792ea,
    string: 0xc3e88d,
    comment: 0x637587,
    type_name: 0xffcb6b,
    function: 0x82aaff,
    constant: 0xf78c6c,
    number: 0xf78c6c,
    variable: 0xe6edf3,
    operator: 0x89ddff,
    attribute: 0x82aaff,
};

/// User-selectable theme mode. `Auto` follows the macOS system appearance,
/// the other two pin the palette regardless of the system setting.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum ThemeMode {
    #[default]
    Auto,
    Light,
    Dark,
}

impl ThemeMode {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Auto => "auto",
            Self::Light => "light",
            Self::Dark => "dark",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value.trim() {
            "auto" => Some(Self::Auto),
            "light" => Some(Self::Light),
            "dark" => Some(Self::Dark),
            _ => None,
        }
    }

    /// Clicking the titlebar toggle rotates all three modes in a fixed order
    /// (Auto -> Light -> Dark -> Auto). Every state stays reachable; a click
    /// that does not change the palette still changes the icon, which is the
    /// signal that the mode left/returned to Auto.
    pub fn next(self) -> Self {
        match self {
            Self::Auto => Self::Light,
            Self::Light => Self::Dark,
            Self::Dark => Self::Auto,
        }
    }
}

const MODE_AUTO: u8 = 0;
const MODE_LIGHT: u8 = 1;
const MODE_DARK: u8 = 2;

#[cfg(test)]
pub(crate) static THEME_TEST_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

static THEME_MODE: std::sync::atomic::AtomicU8 = std::sync::atomic::AtomicU8::new(MODE_AUTO);
/// Whether the operating system currently reports a dark appearance. Only
/// meaningful while `THEME_MODE` is `Auto`; explicit modes override it.
static SYSTEM_DARK: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
/// Monotonic counter bumped whenever the effective palette can change. Color
/// caches that bake palette colors into their keys (editor shaped-line runs,
/// editor minimap rasters) mix this generation in so a theme switch invalidates
/// them without tearing down the panels.
static THEME_GENERATION: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

pub fn theme_generation() -> u64 {
    THEME_GENERATION.load(std::sync::atomic::Ordering::Relaxed)
}

/// The raw system appearance, independent of the selected mode. Toggle logic
/// needs this (not the effective appearance) to know what `Auto` would render.
pub fn system_dark() -> bool {
    SYSTEM_DARK.load(std::sync::atomic::Ordering::Relaxed)
}

fn bump_theme_generation() {
    THEME_GENERATION.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
}

pub fn theme_mode() -> ThemeMode {
    match THEME_MODE.load(std::sync::atomic::Ordering::Relaxed) {
        MODE_LIGHT => ThemeMode::Light,
        MODE_DARK => ThemeMode::Dark,
        _ => ThemeMode::Auto,
    }
}

pub fn set_theme_mode(mode: ThemeMode) {
    let encoded = match mode {
        ThemeMode::Auto => MODE_AUTO,
        ThemeMode::Light => MODE_LIGHT,
        ThemeMode::Dark => MODE_DARK,
    };
    if THEME_MODE.swap(encoded, std::sync::atomic::Ordering::Relaxed) != encoded {
        bump_theme_generation();
    }
}

/// Feed the system appearance (from gpui's window appearance observer) into
/// the theme state so `Auto` mode can follow it.
pub fn set_system_dark(dark: bool) {
    if SYSTEM_DARK.swap(dark, std::sync::atomic::Ordering::Relaxed) != dark {
        bump_theme_generation();
    }
}

/// Appearance events fired while an explicit mode pins the native chrome
/// reflect our own pin, not the system, so they must not corrupt the recorded
/// system appearance. Only trust them while following the system.
pub fn note_system_appearance(dark: bool) {
    if theme_mode() == ThemeMode::Auto {
        set_system_dark(dark);
    }
}

pub fn effective_theme_is_dark() -> bool {
    match theme_mode() {
        ThemeMode::Dark => true,
        ThemeMode::Light => false,
        ThemeMode::Auto => SYSTEM_DARK.load(std::sync::atomic::Ordering::Relaxed),
    }
}

pub fn current_theme() -> &'static Theme {
    if effective_theme_is_dark() {
        &ORG_STUDIO_DARK
    } else {
        &ORG_STUDIO_LIGHT
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn theme_mode_round_trips_and_cycles() {
        for mode in [ThemeMode::Auto, ThemeMode::Light, ThemeMode::Dark] {
            assert_eq!(ThemeMode::parse(mode.as_str()), Some(mode));
        }
        assert_eq!(ThemeMode::parse("midnight"), None);
        assert_eq!(ThemeMode::Auto.next(), ThemeMode::Light);
        assert_eq!(ThemeMode::Light.next(), ThemeMode::Dark);
        assert_eq!(ThemeMode::Dark.next(), ThemeMode::Auto);
        // A full rotation visits every mode exactly once.
        let mut visited = vec![ThemeMode::Auto];
        for _ in 0..2 {
            visited.push(visited.last().unwrap().next());
        }
        assert_eq!(
            visited,
            vec![ThemeMode::Auto, ThemeMode::Light, ThemeMode::Dark]
        );
    }

    #[test]
    fn explicit_mode_overrides_system_appearance() {
        let _guard = THEME_TEST_LOCK
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        set_theme_mode(ThemeMode::Auto);
        set_system_dark(false);
        assert!(std::ptr::eq(current_theme(), &ORG_STUDIO_LIGHT));
        set_system_dark(true);
        assert!(std::ptr::eq(current_theme(), &ORG_STUDIO_DARK));
        set_theme_mode(ThemeMode::Light);
        assert!(std::ptr::eq(current_theme(), &ORG_STUDIO_LIGHT));
        set_theme_mode(ThemeMode::Dark);
        set_system_dark(false);
        assert!(std::ptr::eq(current_theme(), &ORG_STUDIO_DARK));
        set_theme_mode(ThemeMode::Auto);
        set_system_dark(false);
    }

    #[test]
    fn explicit_modes_ignore_pinned_appearance_events() {
        let _guard = THEME_TEST_LOCK
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        set_theme_mode(ThemeMode::Auto);
        note_system_appearance(true);
        assert!(system_dark());
        // Pinning explicit Dark fires an appearance event with dark — it must
        // not overwrite the recorded system appearance.
        set_theme_mode(ThemeMode::Dark);
        note_system_appearance(false);
        assert!(system_dark());
        // Back to Auto trusts the next real system event again.
        set_theme_mode(ThemeMode::Auto);
        note_system_appearance(false);
        assert!(!system_dark());
        set_theme_mode(ThemeMode::Auto);
        set_system_dark(false);
    }

    #[test]
    fn dark_palette_stays_on_the_cold_blue_grey_ramp() {
        let dark = &ORG_STUDIO_DARK;
        assert_eq!(dark.window_background, 0x0f1823);
        assert_eq!(dark.editor_background, 0x121d28);
        assert_eq!(dark.accent, 0x4da3ff);
        // Layering must stay monotonic: window < editor < surface < elevated,
        // and the border must remain dark enough to separate surfaces.
        assert!(dark.window_background < dark.editor_background);
        assert!(dark.editor_background < dark.surface);
        assert!(dark.surface < dark.elevated);
        assert!(dark.hover > dark.surface);
        assert!(dark.divider < dark.border);
        // Text ramp keeps readable contrast against the editor background.
        assert!(dark.foreground > dark.foreground_dim);
        assert!(dark.foreground_dim > dark.foreground_muted);
        assert!(dark.foreground_muted > dark.foreground_disabled);
    }
}
