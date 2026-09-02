use crate::i18n::Language;
use std::{
    fs, io,
    path::PathBuf,
    sync::{OnceLock, mpsc},
};

const SETTINGS_VERSION: u32 = 5;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct WorkspaceSettings {
    /// Preferred left-pane share in basis points when the workspace is split.
    pub split_ratio: u16,
    pub soft_wrap: bool,
    pub language: Language,
    pub minimap_enabled: bool,
    pub minimap_thumb_visibility: MinimapThumbVisibility,
    /// `None` follows the adaptive width; `Some` is the user's preferred
    /// logical-pixel width before the current window's safety clamp.
    pub minimap_width: Option<u16>,
    /// Preferred Sidebar width. Rendering applies the current window clamp
    /// without overwriting this value.
    pub sidebar_width: u16,
    pub reading_style: crate::preview::PreviewStyleId,
    pub status_line: StatusLineSettings,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct StatusLineSettings {
    pub outline: bool,
    pub position: bool,
    pub progress: bool,
    pub statistics: bool,
    pub format: bool,
}

impl Default for StatusLineSettings {
    fn default() -> Self {
        Self {
            outline: true,
            position: true,
            progress: true,
            statistics: true,
            format: true,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MinimapThumbVisibility {
    Always,
    Hover,
}

impl Default for WorkspaceSettings {
    fn default() -> Self {
        Self {
            split_ratio: 5_000,
            soft_wrap: true,
            language: Language::system(),
            minimap_enabled: true,
            minimap_thumb_visibility: MinimapThumbVisibility::Always,
            minimap_width: None,
            sidebar_width: 240,
            reading_style: crate::preview::PreviewStyleId::Base,
            status_line: StatusLineSettings::default(),
        }
    }
}

impl WorkspaceSettings {
    pub fn load() -> Self {
        settings_path()
            .and_then(|path| fs::read_to_string(path).ok())
            .and_then(|source| Self::parse(&source))
            .unwrap_or_default()
    }

    pub fn save(self) -> io::Result<()> {
        let Some(path) = settings_path() else {
            return Ok(());
        };
        let Some(parent) = path.parent() else {
            return Ok(());
        };
        fs::create_dir_all(parent)?;
        let temporary = path.with_extension(format!("tmp-{}", std::process::id()));
        fs::write(&temporary, self.serialize())?;
        fs::rename(temporary, path)
    }

    pub fn save_async(self) {
        static SENDER: OnceLock<mpsc::Sender<WorkspaceSettings>> = OnceLock::new();
        let sender = SENDER.get_or_init(|| {
            let (sender, receiver) = mpsc::channel::<WorkspaceSettings>();
            std::thread::Builder::new()
                .name("org-studio-settings".into())
                .spawn(move || {
                    while let Ok(mut latest) = receiver.recv() {
                        while let Ok(newer) = receiver.try_recv() {
                            latest = newer;
                        }
                        if let Err(error) = latest.save() {
                            eprintln!("org_studio_settings_save_failed error={error}");
                        }
                    }
                })
                .expect("settings writer thread must start");
            sender
        });
        let _ = sender.send(self);
    }

    fn parse(source: &str) -> Option<Self> {
        let mut version = None;
        let mut minimap_enabled = None;
        let mut split_ratio = None;
        let mut soft_wrap = None;
        let mut language = None;
        let mut minimap_thumb_visibility = None;
        let mut minimap_width = None;
        let mut sidebar_width = None;
        let mut reading_style = None;
        let mut status_outline = None;
        let mut status_position = None;
        let mut status_progress = None;
        let mut status_statistics = None;
        let mut status_format = None;
        for line in source.lines() {
            let Some((key, value)) = line.split_once('=') else {
                continue;
            };
            match key.trim() {
                "version" => version = value.trim().parse::<u32>().ok(),
                "language" => {
                    language = match value.trim() {
                        "en" => Some(Language::English),
                        "zh-CN" => Some(Language::Chinese),
                        _ => None,
                    }
                }
                "split_ratio" => {
                    split_ratio = value
                        .trim()
                        .parse::<u16>()
                        .ok()
                        .filter(|ratio| (1_000..=9_000).contains(ratio));
                }
                "soft_wrap" => soft_wrap = value.trim().parse::<bool>().ok(),
                "minimap_enabled" => minimap_enabled = value.trim().parse::<bool>().ok(),
                "minimap_thumb_visibility" => {
                    minimap_thumb_visibility = match value.trim() {
                        "always" => Some(MinimapThumbVisibility::Always),
                        "hover" => Some(MinimapThumbVisibility::Hover),
                        _ => None,
                    }
                }
                "minimap_width" => {
                    minimap_width = match value.trim() {
                        "auto" => Some(None),
                        value => value
                            .parse::<u16>()
                            .ok()
                            .filter(|width| (48..=480).contains(width))
                            .map(Some),
                    }
                }
                "sidebar_width" => {
                    sidebar_width = value
                        .trim()
                        .parse::<u16>()
                        .ok()
                        .filter(|width| (180..=420).contains(width));
                }
                "reading_style" => {
                    reading_style = crate::preview::PreviewStyleId::parse(value.trim());
                }
                "status_outline" => status_outline = value.trim().parse::<bool>().ok(),
                "status_position" => status_position = value.trim().parse::<bool>().ok(),
                "status_progress" => status_progress = value.trim().parse::<bool>().ok(),
                "status_statistics" => status_statistics = value.trim().parse::<bool>().ok(),
                "status_format" => status_format = value.trim().parse::<bool>().ok(),
                _ => {}
            }
        }
        (version == Some(SETTINGS_VERSION)).then_some(Self {
            split_ratio: split_ratio.unwrap_or(5_000),
            soft_wrap: soft_wrap.unwrap_or(true),
            language: language.unwrap_or_else(Language::system),
            minimap_enabled: minimap_enabled.unwrap_or(true),
            minimap_thumb_visibility: minimap_thumb_visibility
                .unwrap_or(MinimapThumbVisibility::Always),
            minimap_width: minimap_width.unwrap_or(None),
            sidebar_width: sidebar_width.unwrap_or(240),
            reading_style: reading_style.unwrap_or_default(),
            status_line: StatusLineSettings {
                outline: status_outline.unwrap_or(true),
                position: status_position.unwrap_or(true),
                progress: status_progress.unwrap_or(true),
                statistics: status_statistics.unwrap_or(true),
                format: status_format.unwrap_or(true),
            },
        })
    }

    fn serialize(self) -> String {
        format!(
            "version={SETTINGS_VERSION}\nsplit_ratio={}\nsoft_wrap={}\nlanguage={}\nminimap_enabled={}\nminimap_thumb_visibility={}\nminimap_width={}\nsidebar_width={}\nreading_style={}\nstatus_outline={}\nstatus_position={}\nstatus_progress={}\nstatus_statistics={}\nstatus_format={}\n",
            self.split_ratio,
            self.soft_wrap,
            match self.language {
                Language::English => "en",
                Language::Chinese => "zh-CN",
            },
            self.minimap_enabled,
            match self.minimap_thumb_visibility {
                MinimapThumbVisibility::Always => "always",
                MinimapThumbVisibility::Hover => "hover",
            },
            self.minimap_width
                .map(|width| width.to_string())
                .unwrap_or_else(|| "auto".to_owned()),
            self.sidebar_width,
            self.reading_style.as_str(),
            self.status_line.outline,
            self.status_line.position,
            self.status_line.progress,
            self.status_line.statistics,
            self.status_line.format,
        )
    }
}

pub fn initial_minimap_enabled() -> bool {
    std::env::var("ORG_STUDIO_MINIMAP")
        .ok()
        .and_then(|value| match value.as_str() {
            "1" | "true" | "on" => Some(true),
            "0" | "false" | "off" => Some(false),
            _ => None,
        })
        .unwrap_or_else(|| WorkspaceSettings::load().minimap_enabled)
}

pub fn initial_minimap_thumb_visibility(
    fallback: MinimapThumbVisibility,
) -> MinimapThumbVisibility {
    std::env::var("ORG_STUDIO_MINIMAP_THUMB")
        .ok()
        .and_then(|value| match value.as_str() {
            "always" => Some(MinimapThumbVisibility::Always),
            "hover" => Some(MinimapThumbVisibility::Hover),
            _ => None,
        })
        .unwrap_or(fallback)
}

pub fn initial_minimap_width(fallback: Option<u16>) -> Option<u16> {
    std::env::var("ORG_STUDIO_MINIMAP_WIDTH")
        .ok()
        .and_then(|value| match value.as_str() {
            "auto" => Some(None),
            value => value
                .parse::<u16>()
                .ok()
                .filter(|width| (48..=480).contains(width))
                .map(Some),
        })
        .unwrap_or(fallback)
}

pub fn initial_sidebar_width(fallback: u16) -> u16 {
    std::env::var("ORG_STUDIO_SIDEBAR_WIDTH")
        .ok()
        .and_then(|value| value.parse::<u16>().ok())
        .filter(|width| (180..=420).contains(width))
        .unwrap_or(fallback)
}

fn settings_path() -> Option<PathBuf> {
    application_support_dir().map(|path| path.join("settings.conf"))
}

pub(crate) fn application_support_dir() -> Option<PathBuf> {
    #[cfg(target_os = "macos")]
    {
        std::env::var_os("HOME")
            .map(PathBuf::from)
            .map(|home| home.join("Library/Application Support/Org Studio"))
    }
    #[cfg(target_os = "windows")]
    {
        return std::env::var_os("APPDATA")
            .map(PathBuf::from)
            .map(|root| root.join("Org Studio"));
    }
    #[cfg(all(not(target_os = "macos"), not(target_os = "windows")))]
    {
        if let Some(root) = std::env::var_os("XDG_CONFIG_HOME") {
            return Some(PathBuf::from(root).join("org-studio"));
        }
        std::env::var_os("HOME")
            .map(PathBuf::from)
            .map(|home| home.join(".config/org-studio"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn settings_round_trip_and_reject_unknown_versions() {
        let settings = WorkspaceSettings {
            split_ratio: 5_360,
            soft_wrap: false,
            language: Language::English,
            minimap_enabled: false,
            minimap_thumb_visibility: MinimapThumbVisibility::Hover,
            minimap_width: Some(176),
            sidebar_width: 312,
            reading_style: crate::preview::PreviewStyleId::WarmClay,
            status_line: StatusLineSettings {
                outline: false,
                position: true,
                progress: false,
                statistics: true,
                format: false,
            },
        };
        assert_eq!(
            WorkspaceSettings::parse(&settings.serialize()),
            Some(settings)
        );
        assert_eq!(
            WorkspaceSettings::parse("version=99\nminimap_enabled=false\n"),
            None
        );
    }

    #[test]
    fn missing_field_uses_product_default() {
        assert_eq!(
            WorkspaceSettings::parse("version=5\n"),
            Some(WorkspaceSettings::default())
        );
        assert_eq!(
            WorkspaceSettings::parse(
                "version=5\nminimap_width=auto\nminimap_thumb_visibility=always\n"
            )
            .expect("auto width settings"),
            WorkspaceSettings::default()
        );
        assert_eq!(
            WorkspaceSettings::parse("version=5\nminimap_width=480\n")
                .expect("manual width settings")
                .minimap_width,
            Some(480)
        );
        assert_eq!(
            WorkspaceSettings::parse("version=5\nminimap_width=999\n")
                .expect("invalid width falls back")
                .minimap_width,
            None
        );
        assert_eq!(
            WorkspaceSettings::parse("version=5\nsidebar_width=320\n")
                .expect("manual sidebar width settings")
                .sidebar_width,
            320
        );
        assert_eq!(
            WorkspaceSettings::parse("version=5\nsidebar_width=999\n")
                .expect("invalid sidebar width falls back")
                .sidebar_width,
            240
        );
        assert_eq!(
            WorkspaceSettings::parse("version=5\nreading_style=warm-clay\n")
                .expect("known reading style")
                .reading_style,
            crate::preview::PreviewStyleId::WarmClay
        );
        assert_eq!(
            WorkspaceSettings::parse("version=5\nreading_style=old-preview\n")
                .expect("unknown reading style falls back")
                .reading_style,
            crate::preview::PreviewStyleId::Base
        );
    }
}
