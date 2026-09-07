use std::{fs, io, path::PathBuf};

use serde::{Deserialize, Serialize};

const CURRENT_VERSION: u32 = 1;

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub(crate) struct InboxConfig {
    pub(crate) file: PathBuf,
    pub(crate) heading: Option<String>,
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub(crate) struct StructuredQuery {
    pub(crate) text: Option<String>,
    pub(crate) todo: Option<String>,
    pub(crate) tag: Option<String>,
    pub(crate) source: Option<PathBuf>,
    pub(crate) scheduled: Option<bool>,
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub(crate) struct SavedViewConfig {
    pub(crate) name: String,
    #[serde(default)]
    pub(crate) query: StructuredQuery,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub(crate) struct AgendaConfig {
    pub(crate) version: u32,
    #[serde(default)]
    pub(crate) sources: Vec<PathBuf>,
    pub(crate) inbox: Option<InboxConfig>,
    #[serde(default)]
    pub(crate) saved_views: Vec<SavedViewConfig>,
}

impl Default for AgendaConfig {
    fn default() -> Self {
        Self {
            version: CURRENT_VERSION,
            sources: Vec::new(),
            inbox: None,
            saved_views: Vec::new(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn version_one_config_migrates_missing_saved_views() {
        let config: AgendaConfig =
            serde_json::from_str(r#"{"version":1,"sources":[],"inbox":null}"#).unwrap();
        assert!(config.saved_views.is_empty());
    }

    #[test]
    fn saved_views_round_trip_structured_query() {
        let config = AgendaConfig {
            saved_views: vec![SavedViewConfig {
                name: "Waiting docs".into(),
                query: StructuredQuery {
                    todo: Some("WAITING".into()),
                    tag: Some("docs".into()),
                    ..Default::default()
                },
            }],
            ..Default::default()
        };
        let decoded: AgendaConfig =
            serde_json::from_slice(&serde_json::to_vec(&config).unwrap()).unwrap();
        assert_eq!(decoded, config);
    }
}

pub(crate) struct AgendaConfigStore {
    path: PathBuf,
}

impl AgendaConfigStore {
    pub(crate) fn new(path: PathBuf) -> Self {
        Self { path }
    }

    pub(crate) fn load(&self) -> io::Result<AgendaConfig> {
        match fs::read(&self.path) {
            Ok(bytes) => {
                let config: AgendaConfig = serde_json::from_slice(&bytes)
                    .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
                if config.version != CURRENT_VERSION {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidData,
                        format!("unsupported Agenda config version {}", config.version),
                    ));
                }
                Ok(config)
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(AgendaConfig::default()),
            Err(error) => Err(error),
        }
    }
}
