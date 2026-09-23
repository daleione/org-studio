use crate::i18n::Language;
use crate::theme::ThemeMode;
use std::{
    fs, io,
    io::Write,
    path::PathBuf,
    sync::{OnceLock, mpsc},
};

const SETTINGS_VERSION: u32 = 7;

// Keep the help shown in settings.conf beside the corresponding Rust fields.
// Doc comments are not available at runtime, so this small macro reuses them.
macro_rules! documented_settings {
    (
        $(#[$attribute:meta])*
        $visibility:vis struct $name:ident {
            $(#[doc = $help:literal] $field_visibility:vis $field:ident: $type:ty,)*
        }
    ) => {
        $(#[$attribute])*
        $visibility struct $name {
            $(#[doc = $help] $field_visibility $field: $type,)*
        }

        impl $name {
            fn description(field: &str) -> Option<&'static str> {
                $(if field == stringify!($field) { return Some($help.trim()); })*
                None
            }
        }
    };
}

documented_settings! {
    #[derive(Clone, Copy, Debug, Eq, PartialEq)]
    pub struct WorkspaceSettings {
        /// Left pane share, 1000..9000; 5000 means 50%.
        pub split_ratio: u16,
        /// en | zh-CN.
        pub language: Language,
        /// Show the minimap, true | false.
        pub minimap_enabled: bool,
        /// Minimap thumb, always | hover.
        pub minimap_thumb_visibility: MinimapThumbVisibility,
        /// auto or 48..480 logical pixels; preferred width before window clamping.
        pub minimap_width: Option<u16>,
        /// 180..420 logical pixels; preferred width before window clamping.
        pub sidebar_width: u16,
        /// Reading style, base | warm-clay.
        pub reading_style: crate::preview::PreviewStyleId,
        /// Status-line sections configured by the status_* keys.
        pub status_line: StatusLineSettings,
        /// auto (follow system) | light | dark.
        pub theme_mode: ThemeMode,
    }
}

documented_settings! {
    #[derive(Clone, Copy, Debug, Eq, PartialEq)]
    pub struct StatusLineSettings {
        /// Show the heading path, true | false.
        pub outline: bool,
        /// Show the cursor position, true | false.
        pub position: bool,
        /// Show reading progress, true | false.
        pub progress: bool,
        /// Show document statistics, true | false.
        pub statistics: bool,
        /// Show file format, true | false.
        pub format: bool,
    }
}

fn setting_description(key: &str) -> Option<&'static str> {
    if key == "version" {
        Some("Configuration format version; leave unchanged.")
    } else if let Some(field) = key.strip_prefix("status_") {
        StatusLineSettings::description(field)
    } else {
        WorkspaceSettings::description(key)
    }
}

fn append_setting(source: &mut String, key: &str, value: impl std::fmt::Display) {
    let description = setting_description(key).expect("every setting has a field description");
    source.push_str("# ");
    source.push_str(description);
    source.push('\n');
    source.push_str(key);
    source.push('=');
    source.push_str(&value.to_string());
    source.push('\n');
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
            language: Language::system(),
            minimap_enabled: true,
            minimap_thumb_visibility: MinimapThumbVisibility::Always,
            minimap_width: None,
            sidebar_width: 240,
            reading_style: crate::preview::PreviewStyleId::Base,
            status_line: StatusLineSettings::default(),
            theme_mode: ThemeMode::default(),
        }
    }
}

impl WorkspaceSettings {
    pub(crate) fn ensure_editable_file(self) -> io::Result<PathBuf> {
        let path = settings_path().ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::NotFound,
                "configuration directory is unavailable",
            )
        })?;
        self.ensure_editable_file_at(path)
    }

    fn ensure_editable_file_at(self, path: PathBuf) -> io::Result<PathBuf> {
        let parent = path.parent().expect("settings path has a parent");
        fs::create_dir_all(parent)?;
        match fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
        {
            Ok(mut file) => file.write_all(self.serialize().as_bytes())?,
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
            Err(error) => return Err(error),
        }
        Ok(path)
    }

    pub fn load() -> Self {
        let Some(path) = settings_path() else {
            return Self::default();
        };
        let Ok(source) = fs::read_to_string(path) else {
            return Self::default();
        };
        Self::parse(&source).unwrap_or_default()
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
        let mut theme_mode = None;
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
                // Legacy field retained as an ignored input. Soft wrap is a
                // document-local transient toggle and every newly opened
                // document starts wrapped.
                "soft_wrap" => {}
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
                "theme_mode" => theme_mode = ThemeMode::parse(value.trim()),
                _ => {}
            }
        }
        let version = version?;
        if !(5..=SETTINGS_VERSION).contains(&version) {
            return None;
        }
        Some(Self {
            split_ratio: split_ratio.unwrap_or(5_000),
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
            theme_mode: theme_mode.unwrap_or_default(),
        })
    }

    fn serialize(self) -> String {
        let mut source = String::from(
            "# Org Studio settings\n# Save and restart to apply changes. Invalid values use defaults.\n\n",
        );
        append_setting(&mut source, "version", SETTINGS_VERSION);
        append_setting(&mut source, "split_ratio", self.split_ratio);
        append_setting(
            &mut source,
            "language",
            match self.language {
                Language::English => "en",
                Language::Chinese => "zh-CN",
            },
        );
        append_setting(&mut source, "minimap_enabled", self.minimap_enabled);
        append_setting(
            &mut source,
            "minimap_thumb_visibility",
            match self.minimap_thumb_visibility {
                MinimapThumbVisibility::Always => "always",
                MinimapThumbVisibility::Hover => "hover",
            },
        );
        append_setting(
            &mut source,
            "minimap_width",
            self.minimap_width
                .map(|width| width.to_string())
                .unwrap_or_else(|| "auto".to_owned()),
        );
        append_setting(&mut source, "sidebar_width", self.sidebar_width);
        append_setting(&mut source, "reading_style", self.reading_style.as_str());
        append_setting(&mut source, "status_outline", self.status_line.outline);
        append_setting(&mut source, "status_position", self.status_line.position);
        append_setting(&mut source, "status_progress", self.status_line.progress);
        append_setting(
            &mut source,
            "status_statistics",
            self.status_line.statistics,
        );
        append_setting(&mut source, "status_format", self.status_line.format);
        append_setting(&mut source, "theme_mode", self.theme_mode.as_str());
        source
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
    #[cfg(target_os = "macos")]
    {
        std::env::var_os("HOME")
            .map(PathBuf::from)
            .map(|home| home.join(".config/org-studio/settings.conf"))
    }
    #[cfg(not(target_os = "macos"))]
    {
        application_support_dir().map(|path| path.join("settings.conf"))
    }
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

    fn test_directory() -> PathBuf {
        std::env::temp_dir().join(format!(
            "org-studio-settings-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("current time")
                .as_nanos()
        ))
    }

    #[test]
    fn settings_round_trip_and_reject_unknown_versions() {
        let settings = WorkspaceSettings {
            split_ratio: 5_360,
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
            theme_mode: ThemeMode::Dark,
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
    fn every_setting_has_its_description_directly_above_it() {
        let source = WorkspaceSettings::default().serialize();
        let mut previous = "";
        for line in source.lines() {
            if let Some((key, _)) = line.split_once('=')
                && !key.starts_with('#')
            {
                assert_eq!(
                    previous,
                    format!(
                        "# {}",
                        setting_description(key).expect("serialized setting has a description")
                    ),
                    "description must be directly above {key}"
                );
            }
            previous = line;
        }
    }

    #[test]
    fn missing_field_uses_product_default() {
        assert_eq!(
            WorkspaceSettings::parse("version=6\n"),
            Some(WorkspaceSettings::default())
        );
        assert_eq!(
            WorkspaceSettings::parse(
                "version=6\nminimap_width=auto\nminimap_thumb_visibility=always\n"
            )
            .expect("auto width settings"),
            WorkspaceSettings::default()
        );
        assert_eq!(
            WorkspaceSettings::parse("version=6\nminimap_width=480\n")
                .expect("manual width settings")
                .minimap_width,
            Some(480)
        );
        assert_eq!(
            WorkspaceSettings::parse("version=6\nminimap_width=999\n")
                .expect("invalid width falls back")
                .minimap_width,
            None
        );
        assert_eq!(
            WorkspaceSettings::parse("version=6\nsidebar_width=320\n")
                .expect("manual sidebar width settings")
                .sidebar_width,
            320
        );
        assert_eq!(
            WorkspaceSettings::parse("version=6\nsidebar_width=999\n")
                .expect("invalid sidebar width falls back")
                .sidebar_width,
            240
        );
        assert_eq!(
            WorkspaceSettings::parse("version=6\nreading_style=warm-clay\n")
                .expect("known reading style")
                .reading_style,
            crate::preview::PreviewStyleId::WarmClay
        );
        assert_eq!(
            WorkspaceSettings::parse("version=6\nreading_style=old-preview\n")
                .expect("unknown reading style falls back")
                .reading_style,
            crate::preview::PreviewStyleId::Base
        );
    }

    #[test]
    fn legacy_soft_wrap_is_ignored_without_resetting_other_settings() {
        let settings = WorkspaceSettings::parse(
            "version=5\nsplit_ratio=6200\nsoft_wrap=false\nlanguage=en\nminimap_enabled=false\nsidebar_width=312\nreading_style=warm-clay\n",
        )
        .expect("version five settings remain readable");

        assert_eq!(settings.split_ratio, 6_200);
        assert_eq!(settings.language, Language::English);
        assert!(!settings.minimap_enabled);
        assert_eq!(settings.sidebar_width, 312);
        assert_eq!(
            settings.reading_style,
            crate::preview::PreviewStyleId::WarmClay
        );
        assert!(
            !settings.serialize().contains("soft_wrap="),
            "an unrelated settings save must remove the legacy global soft-wrap value"
        );
    }

    #[test]
    fn editable_file_is_created_once_without_replacing_user_changes() {
        let directory = test_directory();
        let path = directory.join("settings.conf");
        let settings = WorkspaceSettings::default();
        assert_eq!(
            settings.ensure_editable_file_at(path.clone()).unwrap(),
            path
        );
        assert_eq!(fs::read_to_string(&path).unwrap(), settings.serialize());

        fs::write(&path, "version=7\ntheme_mode=dark\n# my own note").unwrap();
        settings.ensure_editable_file_at(path.clone()).unwrap();
        assert_eq!(
            fs::read_to_string(&path).unwrap(),
            "version=7\ntheme_mode=dark\n# my own note"
        );
        fs::remove_file(path).unwrap();
        fs::remove_dir(directory).unwrap();
    }
}
