use std::{
    ffi::OsString,
    fs, io,
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
};

static TEMP_SEQUENCE: AtomicU64 = AtomicU64::new(0);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ConflictPolicy {
    Error,
    Skip,
    KeepBoth,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum OperationPlan {
    CreateFile {
        path: Arc<Path>,
    },
    CreateDirectory {
        path: Arc<Path>,
    },
    Rename {
        source: Arc<Path>,
        destination: Arc<Path>,
    },
    Copy {
        sources: Arc<[Arc<Path>]>,
        destination_directory: Arc<Path>,
        conflict: ConflictPolicy,
    },
    Move {
        sources: Arc<[Arc<Path>]>,
        destination_directory: Arc<Path>,
        conflict: ConflictPolicy,
    },
    Trash {
        sources: Arc<[Arc<Path>]>,
    },
}

impl OperationPlan {
    pub fn create_file(path: PathBuf) -> Self {
        Self::CreateFile { path: path.into() }
    }

    pub fn create_directory(path: PathBuf) -> Self {
        Self::CreateDirectory { path: path.into() }
    }

    pub fn rename(source: Arc<Path>, destination: PathBuf) -> Self {
        Self::Rename {
            source,
            destination: destination.into(),
        }
    }

    pub fn copy(
        sources: Arc<[Arc<Path>]>,
        destination_directory: PathBuf,
        conflict: ConflictPolicy,
    ) -> Self {
        Self::Copy {
            sources: normalize_sources(sources),
            destination_directory: destination_directory.into(),
            conflict,
        }
    }

    pub fn move_to(
        sources: Arc<[Arc<Path>]>,
        destination_directory: PathBuf,
        conflict: ConflictPolicy,
    ) -> Self {
        Self::Move {
            sources: normalize_sources(sources),
            destination_directory: destination_directory.into(),
            conflict,
        }
    }

    pub fn trash(sources: Arc<[Arc<Path>]>) -> Self {
        Self::Trash {
            sources: normalize_sources(sources),
        }
    }

    pub fn item_count(&self) -> usize {
        match self {
            Self::CreateFile { .. } | Self::CreateDirectory { .. } | Self::Rename { .. } => 1,
            Self::Copy { sources, .. } | Self::Move { sources, .. } | Self::Trash { sources } => {
                sources.len()
            }
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct OperationFailure {
    pub path: Arc<Path>,
    pub message: Arc<str>,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct OperationReport {
    pub completed: Arc<[Arc<Path>]>,
    pub skipped: Arc<[Arc<Path>]>,
    pub failures: Arc<[OperationFailure]>,
    pub destinations: Arc<[(Arc<Path>, Arc<Path>)]>,
}

impl OperationReport {
    pub fn succeeded(&self) -> bool {
        self.failures.is_empty()
    }

    pub fn summary(&self) -> Arc<str> {
        if self.failures.is_empty() {
            if self.skipped.is_empty() {
                Arc::from(format!("Completed {} item(s)", self.completed.len()))
            } else {
                Arc::from(format!(
                    "Completed {} item(s), skipped {}",
                    self.completed.len(),
                    self.skipped.len()
                ))
            }
        } else {
            let first = &self.failures[0];
            Arc::from(format!(
                "Completed {}; {} failed: {} ({})",
                self.completed.len(),
                self.failures.len(),
                first.path.display(),
                first.message
            ))
        }
    }
}

pub fn execute_operation(plan: &OperationPlan) -> OperationReport {
    let mut report = ReportBuilder::default();
    match plan {
        OperationPlan::CreateFile { path } => {
            report.record(path, None, create_file_exclusive(path));
        }
        OperationPlan::CreateDirectory { path } => {
            report.record(path, None, create_directory_exclusive(path));
        }
        OperationPlan::Rename {
            source,
            destination,
        } => {
            report.record(
                source,
                Some(destination),
                rename_exclusive(source, destination),
            );
        }
        OperationPlan::Copy {
            sources,
            destination_directory,
            conflict,
        } => {
            for source in sources.iter() {
                let result = destination_for(source, destination_directory, *conflict).and_then(
                    |destination| match destination {
                        Destination::Skip => Ok(None),
                        Destination::Path(destination) => {
                            copy_exclusive(source, &destination)?;
                            Ok(Some(destination))
                        }
                    },
                );
                report.record_optional_destination(source, result);
            }
        }
        OperationPlan::Move {
            sources,
            destination_directory,
            conflict,
        } => {
            for source in sources.iter() {
                let result = destination_for(source, destination_directory, *conflict).and_then(
                    |destination| match destination {
                        Destination::Skip => Ok(None),
                        Destination::Path(destination) => {
                            move_exclusive(source, &destination)?;
                            Ok(Some(destination))
                        }
                    },
                );
                report.record_optional_destination(source, result);
            }
        }
        OperationPlan::Trash { sources } => {
            for source in sources.iter() {
                let result = std::path::absolute(source)
                    .and_then(|path| trash::delete(path).map_err(io::Error::other));
                report.record(source, None, result);
            }
        }
    }
    report.finish()
}

#[derive(Default)]
struct ReportBuilder {
    completed: Vec<Arc<Path>>,
    skipped: Vec<Arc<Path>>,
    failures: Vec<OperationFailure>,
    destinations: Vec<(Arc<Path>, Arc<Path>)>,
}

impl ReportBuilder {
    fn record(
        &mut self,
        source: &Arc<Path>,
        destination: Option<&Arc<Path>>,
        result: io::Result<()>,
    ) {
        match result {
            Ok(()) => {
                self.completed.push(source.clone());
                if let Some(destination) = destination {
                    self.destinations
                        .push((source.clone(), destination.clone()));
                }
            }
            Err(error) => self.failures.push(OperationFailure {
                path: source.clone(),
                message: Arc::from(error.to_string()),
            }),
        }
    }

    fn record_optional_destination(
        &mut self,
        source: &Arc<Path>,
        result: io::Result<Option<PathBuf>>,
    ) {
        match result {
            Ok(Some(destination)) => {
                self.completed.push(source.clone());
                self.destinations
                    .push((source.clone(), Arc::from(destination)));
            }
            Ok(None) => self.skipped.push(source.clone()),
            Err(error) => self.failures.push(OperationFailure {
                path: source.clone(),
                message: Arc::from(error.to_string()),
            }),
        }
    }

    fn finish(self) -> OperationReport {
        OperationReport {
            completed: self.completed.into(),
            skipped: self.skipped.into(),
            failures: self.failures.into(),
            destinations: self.destinations.into(),
        }
    }
}

enum Destination {
    Path(PathBuf),
    Skip,
}

fn destination_for(
    source: &Path,
    directory: &Path,
    conflict: ConflictPolicy,
) -> io::Result<Destination> {
    if !directory.is_dir() {
        return Err(io::Error::new(
            io::ErrorKind::NotADirectory,
            format!("destination is not a directory: {}", directory.display()),
        ));
    }
    let name = source.file_name().ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("path has no file name: {}", source.display()),
        )
    })?;
    let destination = directory.join(name);
    if !destination.exists() {
        return Ok(Destination::Path(destination));
    }
    match conflict {
        ConflictPolicy::Error => Err(io::Error::new(
            io::ErrorKind::AlreadyExists,
            format!("destination already exists: {}", destination.display()),
        )),
        ConflictPolicy::Skip => Ok(Destination::Skip),
        ConflictPolicy::KeepBoth => Ok(Destination::Path(unique_destination(&destination))),
    }
}

fn unique_destination(path: &Path) -> PathBuf {
    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    let stem = path.file_stem().unwrap_or_default();
    let extension = path.extension();
    for index in 2_u64.. {
        let mut name = OsString::from(stem);
        name.push(format!(" {index}"));
        if let Some(extension) = extension {
            name.push(".");
            name.push(extension);
        }
        let candidate = parent.join(name);
        if !candidate.exists() {
            return candidate;
        }
    }
    unreachable!("the name space is finite but cannot be exhausted in practice")
}

fn create_file_exclusive(path: &Path) -> io::Result<()> {
    fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)?;
    Ok(())
}

fn create_directory_exclusive(path: &Path) -> io::Result<()> {
    fs::create_dir(path)
}

fn rename_exclusive(source: &Path, destination: &Path) -> io::Result<()> {
    validate_transfer(source, destination)?;
    rename_no_replace(source, destination)
}

fn copy_exclusive(source: &Path, destination: &Path) -> io::Result<()> {
    validate_transfer(source, destination)?;
    let temporary = temporary_sibling(destination)?;
    let result =
        copy_path(source, &temporary).and_then(|()| rename_no_replace(&temporary, destination));
    if result.is_err() {
        remove_temporary(&temporary);
    }
    result
}

fn move_exclusive(source: &Path, destination: &Path) -> io::Result<()> {
    validate_transfer(source, destination)?;
    match rename_no_replace(source, destination) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == io::ErrorKind::CrossesDevices => {
            copy_exclusive(source, destination)?;
            remove_source(source)
        }
        Err(error) => Err(error),
    }
}

fn validate_transfer(source: &Path, destination: &Path) -> io::Result<()> {
    let source_metadata = fs::symlink_metadata(source)?;
    if source == destination {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "source and destination are the same",
        ));
    }
    // A symlink is copied or renamed as a leaf, including when its target is
    // broken. It cannot recursively contain the destination.
    if source_metadata.file_type().is_symlink() {
        return Ok(());
    }
    let source = fs::canonicalize(source)?;
    let destination_parent = destination
        .parent()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "destination has no parent"))?;
    let destination_name = destination.file_name().ok_or_else(|| {
        io::Error::new(io::ErrorKind::InvalidInput, "destination has no file name")
    })?;
    let destination = fs::canonicalize(destination_parent)?.join(destination_name);
    if source == destination {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "source and destination are the same",
        ));
    }
    if source_metadata.is_dir() && destination.starts_with(&source) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "cannot place a directory inside itself",
        ));
    }
    Ok(())
}

fn temporary_sibling(destination: &Path) -> io::Result<PathBuf> {
    let parent = destination
        .parent()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "destination has no parent"))?;
    let name = destination.file_name().unwrap_or_default();
    for _ in 0..1_000 {
        let sequence = TEMP_SEQUENCE.fetch_add(1, Ordering::Relaxed);
        let candidate = parent.join(format!(
            ".org-studio-{}-{sequence}-{}",
            std::process::id(),
            name.to_string_lossy()
        ));
        if !candidate.exists() {
            return Ok(candidate);
        }
    }
    Err(io::Error::new(
        io::ErrorKind::AlreadyExists,
        "could not allocate a temporary destination",
    ))
}

fn copy_path(source: &Path, destination: &Path) -> io::Result<()> {
    let metadata = fs::symlink_metadata(source)?;
    if metadata.file_type().is_symlink() {
        let target = fs::read_link(source)?;
        #[cfg(unix)]
        std::os::unix::fs::symlink(target, destination)?;
        #[cfg(windows)]
        if source.is_dir() {
            std::os::windows::fs::symlink_dir(target, destination)?;
        } else {
            std::os::windows::fs::symlink_file(target, destination)?;
        }
        return Ok(());
    }
    if metadata.is_dir() {
        fs::create_dir(destination)?;
        for entry in fs::read_dir(source)? {
            let entry = entry?;
            copy_path(&entry.path(), &destination.join(entry.file_name()))?;
        }
        fs::set_permissions(destination, metadata.permissions())?;
    } else {
        let mut source_file = fs::File::open(source)?;
        let mut destination_file = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(destination)?;
        io::copy(&mut source_file, &mut destination_file)?;
        fs::set_permissions(destination, metadata.permissions())?;
    }
    Ok(())
}

fn normalize_sources(sources: Arc<[Arc<Path>]>) -> Arc<[Arc<Path>]> {
    let mut sources = sources.iter().cloned().collect::<Vec<_>>();
    sources.sort_by(|left, right| {
        left.components()
            .count()
            .cmp(&right.components().count())
            .then_with(|| left.cmp(right))
    });
    sources.dedup();
    let mut normalized: Vec<Arc<Path>> = Vec::with_capacity(sources.len());
    for source in sources {
        if normalized
            .iter()
            .any(|ancestor| source.starts_with(ancestor.as_ref()))
        {
            continue;
        }
        normalized.push(source);
    }
    normalized.into()
}

#[cfg(any(target_vendor = "apple", target_os = "linux", target_os = "redox"))]
fn rename_no_replace(source: &Path, destination: &Path) -> io::Result<()> {
    rustix::fs::renameat_with(
        rustix::fs::CWD,
        source,
        rustix::fs::CWD,
        destination,
        rustix::fs::RenameFlags::NOREPLACE,
    )
    .map_err(io::Error::from)
}

#[cfg(target_os = "windows")]
fn rename_no_replace(source: &Path, destination: &Path) -> io::Result<()> {
    // MoveFileEx without MOVEFILE_REPLACE_EXISTING, which backs std::fs::rename
    // on Windows, already has no-replace semantics.
    fs::rename(source, destination)
}

#[cfg(not(any(
    target_vendor = "apple",
    target_os = "linux",
    target_os = "redox",
    target_os = "windows"
)))]
fn rename_no_replace(_source: &Path, _destination: &Path) -> io::Result<()> {
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "atomic no-replace rename is unsupported on this platform",
    ))
}

fn remove_source(path: &Path) -> io::Result<()> {
    let metadata = fs::symlink_metadata(path)?;
    if metadata.is_dir() && !metadata.file_type().is_symlink() {
        fs::remove_dir_all(path)
    } else {
        fs::remove_file(path)
    }
}

fn remove_temporary(path: &Path) {
    let Ok(metadata) = fs::symlink_metadata(path) else {
        return;
    };
    if metadata.is_dir() && !metadata.file_type().is_symlink() {
        let _ = fs::remove_dir_all(path);
    } else {
        let _ = fs::remove_file(path);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn temporary_directory(name: &str) -> PathBuf {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let directory = std::env::temp_dir().join(format!(
            "org-studio-operation-{}-{nonce}-{name}",
            std::process::id()
        ));
        fs::create_dir(&directory).unwrap();
        directory
    }

    #[test]
    fn creates_renames_and_copies_without_overwriting() {
        let root = temporary_directory("basic");
        let original = root.join("note.org");
        let created = execute_operation(&OperationPlan::create_file(original.clone()));
        assert!(created.succeeded());

        let renamed = root.join("renamed.org");
        let report = execute_operation(&OperationPlan::rename(
            Arc::from(original.as_path()),
            renamed.clone(),
        ));
        assert!(report.succeeded());
        assert!(renamed.exists());

        let destination = root.join("copies");
        assert!(
            execute_operation(&OperationPlan::create_directory(destination.clone())).succeeded()
        );
        let plan = OperationPlan::copy(
            vec![Arc::from(renamed.as_path())].into(),
            destination.clone(),
            ConflictPolicy::Error,
        );
        assert!(execute_operation(&plan).succeeded());
        assert!(!execute_operation(&plan).succeeded());
        assert_eq!(fs::read_dir(&destination).unwrap().count(), 1);

        let moved_directory = root.join("moved");
        assert!(
            execute_operation(&OperationPlan::create_directory(moved_directory.clone()))
                .succeeded()
        );
        let move_plan = OperationPlan::move_to(
            vec![Arc::from(renamed.as_path())].into(),
            moved_directory.clone(),
            ConflictPolicy::Error,
        );
        assert!(execute_operation(&move_plan).succeeded());
        assert!(!renamed.exists());
        assert!(moved_directory.join("renamed.org").exists());

        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn recursively_copies_directories_and_keep_both_chooses_a_new_name() {
        let root = temporary_directory("directory");
        let source = root.join("source");
        fs::create_dir(&source).unwrap();
        fs::write(source.join("nested.org"), "hello").unwrap();
        let destination = root.join("destination");
        fs::create_dir(&destination).unwrap();
        let plan = OperationPlan::copy(
            vec![Arc::from(source.as_path())].into(),
            destination.clone(),
            ConflictPolicy::KeepBoth,
        );
        assert!(execute_operation(&plan).succeeded());
        assert!(execute_operation(&plan).succeeded());
        assert!(destination.join("source/nested.org").exists());
        assert!(destination.join("source 2/nested.org").exists());

        let skip = OperationPlan::copy(
            vec![Arc::from(source.as_path())].into(),
            destination.clone(),
            ConflictPolicy::Skip,
        );
        let report = execute_operation(&skip);
        assert!(report.succeeded());
        assert_eq!(report.skipped.as_ref(), &[Arc::from(source.as_path())]);

        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn refuses_to_copy_a_directory_inside_itself() {
        let root = temporary_directory("self-copy");
        let source = root.join("source");
        fs::create_dir(&source).unwrap();
        let plan = OperationPlan::copy(
            vec![Arc::from(source.as_path())].into(),
            source.clone(),
            ConflictPolicy::Error,
        );
        let report = execute_operation(&plan);
        assert!(!report.succeeded());
        assert!(source.exists());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn no_replace_commit_preserves_a_destination_that_already_exists() {
        let root = temporary_directory("no-replace");
        let source = root.join("source.org");
        let destination = root.join("destination.org");
        fs::write(&source, "new").unwrap();
        fs::write(&destination, "existing").unwrap();

        let error = copy_exclusive(&source, &destination).unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::AlreadyExists);
        assert_eq!(fs::read_to_string(&destination).unwrap(), "existing");
        assert_eq!(fs::read_to_string(&source).unwrap(), "new");
        assert!(fs::read_dir(&root).unwrap().all(|entry| {
            !entry
                .unwrap()
                .file_name()
                .to_string_lossy()
                .starts_with(".org-studio-")
        }));

        fs::remove_dir_all(root).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn refuses_self_copy_through_a_symlinked_destination_parent() {
        let root = temporary_directory("symlink-self-copy");
        let source = root.join("source");
        let nested = source.join("nested");
        let alias = root.join("alias");
        fs::create_dir(&source).unwrap();
        fs::create_dir(&nested).unwrap();
        std::os::unix::fs::symlink(&nested, &alias).unwrap();

        let report = execute_operation(&OperationPlan::copy(
            vec![Arc::from(source.as_path())].into(),
            alias,
            ConflictPolicy::Error,
        ));
        assert!(!report.succeeded());
        assert!(fs::read_dir(&nested).unwrap().next().is_none());

        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn plans_drop_duplicates_and_descendants_of_selected_directories() {
        let parent: Arc<Path> = Arc::from(Path::new("/tmp/project"));
        let child: Arc<Path> = Arc::from(Path::new("/tmp/project/notes/a.org"));
        let sibling: Arc<Path> = Arc::from(Path::new("/tmp/sibling.org"));
        let plan = OperationPlan::trash(
            vec![child, parent.clone(), sibling.clone(), parent.clone()].into(),
        );

        assert_eq!(plan.item_count(), 2);
        assert_eq!(
            plan,
            OperationPlan::Trash {
                sources: vec![parent, sibling].into()
            }
        );
    }

    #[cfg(unix)]
    #[test]
    fn broken_symlinks_can_be_renamed_without_following_their_target() {
        let root = temporary_directory("broken-symlink");
        let source = root.join("broken");
        let destination = root.join("renamed");
        std::os::unix::fs::symlink("missing-target", &source).unwrap();

        let report = execute_operation(&OperationPlan::rename(
            Arc::from(source.as_path()),
            destination.clone(),
        ));
        assert!(report.succeeded());
        assert_eq!(
            fs::read_link(destination).unwrap(),
            Path::new("missing-target")
        );

        fs::remove_dir_all(root).unwrap();
    }
}
