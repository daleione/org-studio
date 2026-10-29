//! Shared immutable directory data. Navigation state belongs to each consumer.

use std::{
    collections::{HashMap, HashSet},
    path::{Path, PathBuf},
    sync::Arc,
};

use crate::navigation::CancellationToken;

use super::{FileEntry, ScanError, ScanResult};

const MAX_CACHED_DIRECTORIES: usize = 128;
const MAX_CACHED_ENTRIES: usize = 100_000;

#[derive(Clone, Debug)]
pub struct DirectorySnapshot {
    pub directory: PathBuf,
    pub entries: Arc<[Arc<FileEntry>]>,
    pub revision: u64,
}

#[derive(Clone, Debug)]
pub struct DirectoryRequest {
    pub directory: PathBuf,
    pub cancellation: CancellationToken,
    revision: u64,
}

#[derive(Default)]
struct DirectoryState {
    snapshot: Option<Arc<DirectorySnapshot>>,
    request: Option<DirectoryRequest>,
    error: Option<Arc<str>>,
    dirty: bool,
    accessed: u64,
}

#[derive(Default)]
pub struct DirectoryStore {
    directories: HashMap<PathBuf, DirectoryState>,
    revision: u64,
}

impl DirectoryStore {
    pub fn snapshot(&self, directory: &Path) -> Option<&Arc<DirectorySnapshot>> {
        self.directories.get(directory)?.snapshot.as_ref()
    }

    pub fn error(&self, directory: &Path) -> Option<&Arc<str>> {
        self.directories.get(directory)?.error.as_ref()
    }

    pub fn is_loading(&self, directory: &Path) -> bool {
        self.directories
            .get(directory)
            .is_some_and(|state| state.request.is_some())
    }

    pub fn is_current(&self, directory: &Path) -> bool {
        self.directories.get(directory).is_some_and(|state| {
            state.snapshot.is_some()
                && !state.dirty
                && state.request.is_none()
                && state.error.is_none()
        })
    }

    /// Coalesce consumers of the same directory, including a refresh arriving
    /// while a read is in flight. Different directories never cancel each other.
    pub fn request(&mut self, directory: PathBuf, refresh: bool) -> Option<DirectoryRequest> {
        self.revision = self.revision.wrapping_add(1);
        let state = self.directories.entry(directory.clone()).or_default();
        state.accessed = self.revision;
        if state.request.is_some() {
            state.dirty |= refresh;
            return None;
        }
        if !refresh && (state.snapshot.is_some() || state.error.is_some()) && !state.dirty {
            return None;
        }
        let request = DirectoryRequest {
            directory,
            cancellation: CancellationToken::new(),
            revision: self.revision,
        };
        state.request = Some(request.clone());
        state.error = None;
        state.dirty = false;
        Some(request)
    }

    pub fn commit(
        &mut self,
        request: &DirectoryRequest,
        result: Result<ScanResult, ScanError>,
    ) -> bool {
        let Some(state) = self.directories.get_mut(&request.directory) else {
            return false;
        };
        if request.cancellation.is_cancelled()
            || state.request.as_ref().map(|pending| pending.revision) != Some(request.revision)
            || result
                .as_ref()
                .is_ok_and(|result| result.directory != request.directory)
        {
            return false;
        }
        state.request = None;
        match result {
            Ok(result) => {
                state.snapshot = Some(Arc::new(DirectorySnapshot {
                    directory: result.directory,
                    entries: result.entries.into_iter().map(Arc::new).collect(),
                    revision: request.revision,
                }));
                state.error = None;
            }
            Err(error) => {
                state.error = Some(error.source.to_string().into());
            }
        }
        true
    }

    pub fn needs_refresh(&self, directory: &Path) -> bool {
        self.directories
            .get(directory)
            .is_some_and(|state| state.dirty && state.request.is_none())
    }

    pub fn invalidate(&mut self, directory: &Path) {
        if let Some(state) = self.directories.get_mut(directory) {
            state.dirty = true;
        }
    }

    pub fn retain_active(&mut self, active: &HashSet<PathBuf>) {
        for (path, state) in &mut self.directories {
            if !active.contains(path) {
                if let Some(request) = state.request.take() {
                    request.cancellation.cancel();
                }
                state.dirty = true;
            }
        }
        // Bound inactive data without discarding the much smaller UI memories.
        let mut entries = self
            .directories
            .values()
            .filter_map(|state| state.snapshot.as_ref())
            .map(|snapshot| snapshot.entries.len())
            .sum::<usize>();
        if self.directories.len() <= MAX_CACHED_DIRECTORIES && entries <= MAX_CACHED_ENTRIES {
            return;
        }
        let mut inactive = self
            .directories
            .iter()
            .filter(|(path, _)| !active.contains(*path))
            .map(|(path, state)| (state.accessed, path.clone()))
            .collect::<Vec<_>>();
        inactive.sort_unstable();
        for (_, path) in inactive {
            if self.directories.len() <= MAX_CACHED_DIRECTORIES && entries <= MAX_CACHED_ENTRIES {
                break;
            }
            if let Some(state) = self.directories.remove(&path) {
                entries -= state
                    .snapshot
                    .as_ref()
                    .map_or(0, |snapshot| snapshot.entries.len());
            }
        }
    }
}
