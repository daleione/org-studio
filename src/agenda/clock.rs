use super::TaskRecord;
use serde::{Deserialize, Serialize};
use std::{fs, io, path::Path, time::SystemTime};

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub(crate) struct ActiveClock {
    pub(crate) file: std::path::PathBuf,
    pub(crate) heading_title: String,
    pub(crate) started_unix: u64,
}
#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub(crate) struct ClockStore {
    pub(crate) active: Option<ActiveClock>,
    #[serde(default)]
    pub(crate) pending: Vec<ClockLog>,
}
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub(crate) struct ClockLog {
    pub(crate) file: std::path::PathBuf,
    pub(crate) heading_title: String,
    pub(crate) started_unix: u64,
    pub(crate) stopped_unix: u64,
}

impl ClockStore {
    pub(crate) fn load(path: &Path) -> io::Result<Self> {
        match fs::read(path) {
            Ok(bytes) => serde_json::from_slice(&bytes)
                .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error)),
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(Self::default()),
            Err(error) => Err(error),
        }
    }
    pub(crate) fn save(&self, path: &Path) -> io::Result<()> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        let mut temporary = path.to_path_buf();
        temporary.set_extension(format!("clock.{}.tmp", std::process::id()));
        fs::write(
            &temporary,
            serde_json::to_vec_pretty(self).map_err(io::Error::other)?,
        )?;
        fs::rename(temporary, path)
    }
    pub(crate) fn start(&mut self, task: &TaskRecord, now: SystemTime) -> Option<ClockLog> {
        let previous = self.stop(now);
        self.active = Some(ActiveClock {
            file: task.source.path.as_ref().clone(),
            heading_title: task.title.to_string(),
            started_unix: unix(now),
        });
        previous
    }
    pub(crate) fn stop(&mut self, now: SystemTime) -> Option<ClockLog> {
        let active = self.active.take()?;
        let log = ClockLog {
            file: active.file,
            heading_title: active.heading_title,
            started_unix: active.started_unix,
            stopped_unix: unix(now).max(active.started_unix),
        };
        self.pending.push(log.clone());
        Some(log)
    }
}
fn unix(time: SystemTime) -> u64 {
    time.duration_since(SystemTime::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}
impl ClockLog {
    pub(crate) fn duration_seconds(&self) -> u64 {
        self.stopped_unix.saturating_sub(self.started_unix)
    }
    pub(crate) fn org_line(&self) -> String {
        let timestamp = |seconds: u64| {
            i64::try_from(seconds)
                .ok()
                .and_then(|seconds| jiff::Timestamp::from_second(seconds).ok())
                .map(|timestamp| {
                    timestamp
                        .to_zoned(jiff::tz::TimeZone::system())
                        .strftime("%Y-%m-%d %a %H:%M:%S")
                        .to_string()
                })
                .unwrap_or_else(|| "1970-01-01 Thu 00:00:00".to_owned())
        };
        format!(
            "CLOCK: [{}]--[{}] => {:02}:{:02}",
            timestamp(self.started_unix),
            timestamp(self.stopped_unix),
            self.duration_seconds() / 3600,
            self.duration_seconds() % 3600 / 60
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn stopped_clock_is_durable_until_log_is_acknowledged() {
        let path =
            std::env::temp_dir().join(format!("agenda-clock-pending-{}.json", std::process::id()));
        let mut store = ClockStore {
            active: Some(ActiveClock {
                file: "a.org".into(),
                heading_title: "A".into(),
                started_unix: 1,
            }),
            pending: Vec::new(),
        };
        let log = store
            .stop(SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(120))
            .unwrap();
        store.save(&path).unwrap();
        let restored = ClockStore::load(&path).unwrap();
        assert!(restored.active.is_none());
        assert_eq!(restored.pending, vec![log]);
        std::fs::remove_file(path).unwrap();
    }
    #[test]
    fn previous_clock_format_migrates_without_losing_active_timer() {
        let store: ClockStore = serde_json::from_str(
            r#"{"active":{"file":"a.org","heading_title":"A","started_unix":1}}"#,
        )
        .unwrap();
        assert!(store.active.is_some());
        assert!(store.pending.is_empty());
    }
}
