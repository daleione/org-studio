use std::{
    fs, io,
    path::PathBuf,
    sync::{OnceLock, mpsc},
};

const SETTINGS_VERSION: u32 = 1;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PreviewSettings {
    pub minimap_enabled: bool,
    pub minimap_thumb_visibility: MinimapThumbVisibility,
    /// `None` follows the adaptive width; `Some` is the user's preferred
    /// logical-pixel width before the current window's safety clamp.
    pub minimap_width: Option<u16>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MinimapThumbVisibility {
    Always,
    Hover,
}

impl Default for PreviewSettings {
    fn default() -> Self {
        Self {
            minimap_enabled: true,
            minimap_thumb_visibility: MinimapThumbVisibility::Always,
            minimap_width: None,
        }
    }
}

impl PreviewSettings {
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
        static SENDER: OnceLock<mpsc::Sender<PreviewSettings>> = OnceLock::new();
        let sender = SENDER.get_or_init(|| {
            let (sender, receiver) = mpsc::channel::<PreviewSettings>();
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
        let mut minimap_thumb_visibility = None;
        let mut minimap_width = None;
        for line in source.lines() {
            let Some((key, value)) = line.split_once('=') else {
                continue;
            };
            match key.trim() {
                "version" => version = value.trim().parse::<u32>().ok(),
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
                _ => {}
            }
        }
        (version == Some(SETTINGS_VERSION)).then_some(Self {
            minimap_enabled: minimap_enabled.unwrap_or(true),
            minimap_thumb_visibility: minimap_thumb_visibility
                .unwrap_or(MinimapThumbVisibility::Always),
            minimap_width: minimap_width.unwrap_or(None),
        })
    }

    fn serialize(self) -> String {
        format!(
            "version={SETTINGS_VERSION}\nminimap_enabled={}\nminimap_thumb_visibility={}\nminimap_width={}\n",
            self.minimap_enabled,
            match self.minimap_thumb_visibility {
                MinimapThumbVisibility::Always => "always",
                MinimapThumbVisibility::Hover => "hover",
            },
            self.minimap_width
                .map(|width| width.to_string())
                .unwrap_or_else(|| "auto".to_owned()),
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
        .unwrap_or_else(|| PreviewSettings::load().minimap_enabled)
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

fn settings_path() -> Option<PathBuf> {
    #[cfg(target_os = "macos")]
    {
        std::env::var_os("HOME")
            .map(PathBuf::from)
            .map(|home| home.join("Library/Application Support/Org Studio/settings.conf"))
    }
    #[cfg(target_os = "windows")]
    {
        return std::env::var_os("APPDATA")
            .map(PathBuf::from)
            .map(|root| root.join("Org Studio/settings.conf"));
    }
    #[cfg(all(not(target_os = "macos"), not(target_os = "windows")))]
    {
        if let Some(root) = std::env::var_os("XDG_CONFIG_HOME") {
            return Some(PathBuf::from(root).join("org-studio/settings.conf"));
        }
        std::env::var_os("HOME")
            .map(PathBuf::from)
            .map(|home| home.join(".config/org-studio/settings.conf"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn settings_round_trip_and_reject_unknown_versions() {
        let settings = PreviewSettings {
            minimap_enabled: false,
            minimap_thumb_visibility: MinimapThumbVisibility::Hover,
            minimap_width: Some(176),
        };
        assert_eq!(
            PreviewSettings::parse(&settings.serialize()),
            Some(settings)
        );
        assert_eq!(
            PreviewSettings::parse("version=99\nminimap_enabled=false\n"),
            None
        );
    }

    #[test]
    fn missing_field_uses_product_default() {
        assert_eq!(
            PreviewSettings::parse("version=1\n"),
            Some(PreviewSettings::default())
        );
        assert_eq!(
            PreviewSettings::parse(
                "version=1\nminimap_width=auto\nminimap_thumb_visibility=always\n"
            )
            .expect("auto width settings"),
            PreviewSettings::default()
        );
        assert_eq!(
            PreviewSettings::parse("version=1\nminimap_width=480\n")
                .expect("manual width settings")
                .minimap_width,
            Some(480)
        );
        assert_eq!(
            PreviewSettings::parse("version=1\nminimap_width=999\n")
                .expect("invalid width falls back")
                .minimap_width,
            None
        );
    }
}
