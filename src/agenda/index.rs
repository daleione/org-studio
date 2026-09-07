use std::{collections::HashMap, path::PathBuf, sync::Arc};

use super::{AgendaDiagnostic, FileId, SourceVersion, TaskRecord};

#[derive(Clone)]
pub(crate) struct FileAgendaShard {
    pub(crate) file: FileId,
    pub(crate) path: Arc<PathBuf>,
    pub(crate) version: SourceVersion,
    pub(crate) tasks: Arc<[TaskRecord]>,
    pub(crate) diagnostics: Arc<[AgendaDiagnostic]>,
}

impl FileAgendaShard {
    pub(crate) fn new(
        file: FileId,
        path: Arc<PathBuf>,
        version: SourceVersion,
        tasks: Vec<TaskRecord>,
    ) -> Self {
        Self {
            file,
            path,
            version,
            tasks: tasks.into(),
            diagnostics: Arc::from([]),
        }
    }

    pub(crate) fn with_diagnostics(mut self, diagnostics: Vec<AgendaDiagnostic>) -> Self {
        self.diagnostics = diagnostics.into();
        self
    }
}

#[derive(Clone, Default)]
pub(crate) struct AgendaIndexSnapshot {
    pub(crate) generation: u64,
    pub(crate) files: Arc<[Arc<FileAgendaShard>]>,
    pub(crate) file_positions: Arc<HashMap<FileId, usize>>,
}

#[derive(Default)]
pub(crate) struct AgendaIndex {
    snapshot: Arc<AgendaIndexSnapshot>,
}

impl AgendaIndex {
    pub(crate) fn snapshot(&self) -> Arc<AgendaIndexSnapshot> {
        self.snapshot.clone()
    }

    pub(crate) fn replace(&mut self, shard: FileAgendaShard) -> Arc<AgendaIndexSnapshot> {
        let mut files = self.snapshot.files.to_vec();
        if let Some(position) = self.snapshot.file_positions.get(&shard.file).copied() {
            files[position] = Arc::new(shard);
        } else {
            files.push(Arc::new(shard));
        }
        self.publish(files)
    }

    pub(crate) fn remove(&mut self, file: FileId) -> Arc<AgendaIndexSnapshot> {
        let files = self
            .snapshot
            .files
            .iter()
            .filter(|shard| shard.file != file)
            .cloned()
            .collect();
        self.publish(files)
    }

    fn publish(&mut self, files: Vec<Arc<FileAgendaShard>>) -> Arc<AgendaIndexSnapshot> {
        let generation = self
            .snapshot
            .generation
            .checked_add(1)
            .expect("Agenda generation exhausted");
        let file_positions = files
            .iter()
            .enumerate()
            .map(|(index, shard)| (shard.file, index))
            .collect();
        self.snapshot = Arc::new(AgendaIndexSnapshot {
            generation,
            files: files.into(),
            file_positions: Arc::new(file_positions),
        });
        self.snapshot.clone()
    }
}
