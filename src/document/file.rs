use std::{
    fs::{self, File, OpenOptions, Permissions},
    io::{self, BufReader, Read, Write},
    os::unix::fs::MetadataExt,
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
    time::SystemTime,
};

#[cfg(not(unix))]
compile_error!("org-studio file persistence requires Unix atomic rename semantics");

use super::{ByteOffset, DocumentId, DocumentSnapshot, Revision, SavePoint, TextSnapshot};

const UTF8_BOM: &[u8] = b"\xef\xbb\xbf";
const FNV_OFFSET: u64 = 0xcbf2_9ce4_8422_2325;
const FNV_PRIME: u64 = 0x0000_0100_0000_01b3;
static NEXT_TEMP_FILE: AtomicU64 = AtomicU64::new(1);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct FileResourceId {
    device: u64,
    file: u64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FileStamp {
    pub resource_id: Option<FileResourceId>,
    pub modified: Option<SystemTime>,
    pub len: u64,
    pub content_hash: u64,
}

impl FileStamp {
    pub fn read(path: &Path) -> io::Result<Self> {
        for _ in 0..3 {
            let file = File::open(path)?;
            let before = file.metadata()?;
            let mut reader = BufReader::new(file);
            let mut buffer = [0_u8; 64 * 1024];
            let mut hash = FNV_OFFSET;
            let mut len = 0_u64;
            loop {
                let read = reader.read(&mut buffer)?;
                if read == 0 {
                    break;
                }
                hash = extend_hash(hash, &buffer[..read]);
                len += read as u64;
            }
            let after = reader.get_ref().metadata()?;
            let current_path = fs::metadata(path)?;
            if same_metadata(&before, &after)
                && same_metadata(&after, &current_path)
                && current_path.len() == len
            {
                return Ok(Self::from_metadata(&current_path, len, hash));
            }
        }
        Err(io::Error::new(
            io::ErrorKind::WouldBlock,
            "file changed repeatedly while its stamp was being calculated",
        ))
    }

    pub fn from_loaded(path: &Path, bytes: &[u8]) -> io::Result<Self> {
        let metadata = fs::metadata(path)?;
        Ok(Self::from_metadata(
            &metadata,
            bytes.len() as u64,
            hash_bytes(bytes),
        ))
    }

    pub(crate) fn detached(bytes: &[u8]) -> Self {
        Self {
            resource_id: None,
            modified: None,
            len: bytes.len() as u64,
            content_hash: hash_bytes(bytes),
        }
    }

    fn from_metadata(metadata: &fs::Metadata, len: u64, content_hash: u64) -> Self {
        Self {
            resource_id: resource_id(metadata),
            modified: metadata.modified().ok(),
            len,
            content_hash,
        }
    }
}

fn same_metadata(left: &fs::Metadata, right: &fs::Metadata) -> bool {
    resource_id(left) == resource_id(right)
        && left.len() == right.len()
        && left.modified().ok() == right.modified().ok()
}

fn resource_id(metadata: &fs::Metadata) -> Option<FileResourceId> {
    Some(FileResourceId {
        device: metadata.dev(),
        file: metadata.ino(),
    })
}

pub(crate) fn resolve_symlink_target(path: &Path) -> PathBuf {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_symlink() => {
            fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf())
        }
        _ => path.to_path_buf(),
    }
}

fn hash_bytes(bytes: &[u8]) -> u64 {
    extend_hash(FNV_OFFSET, bytes)
}

fn extend_hash(hash: u64, bytes: &[u8]) -> u64 {
    bytes.iter().fold(hash, |hash, byte| {
        (hash ^ u64::from(*byte)).wrapping_mul(FNV_PRIME)
    })
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum OriginalNewline {
    None,
    Lf,
    CrLf,
    Mixed,
}

#[derive(Clone, Debug)]
pub struct FileMetadata {
    pub utf8_bom: bool,
    pub newline: OriginalNewline,
    pub permissions: Option<Permissions>,
}

impl FileMetadata {
    pub(crate) fn from_loaded(path: &Path, bytes: &[u8]) -> Self {
        Self {
            utf8_bom: bytes.starts_with(UTF8_BOM),
            newline: detect_newline(bytes),
            permissions: fs::metadata(path)
                .ok()
                .map(|metadata| metadata.permissions()),
        }
    }
}

fn detect_newline(bytes: &[u8]) -> OriginalNewline {
    let mut lf = false;
    let mut crlf = false;
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'\n' {
            if index > 0 && bytes[index - 1] == b'\r' {
                crlf = true;
            } else {
                lf = true;
            }
        }
        index += 1;
    }
    match (lf, crlf) {
        (false, false) => OriginalNewline::None,
        (true, false) => OriginalNewline::Lf,
        (false, true) => OriginalNewline::CrLf,
        (true, true) => OriginalNewline::Mixed,
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum SyncState {
    InSync {
        stamp: FileStamp,
    },
    Dirty {
        base: FileStamp,
    },
    Conflict {
        base: FileStamp,
        external: FileStamp,
    },
    Missing {
        base: FileStamp,
    },
}

impl SyncState {
    pub fn base(&self) -> &FileStamp {
        match self {
            Self::InSync { stamp } => stamp,
            Self::Dirty { base } | Self::Conflict { base, .. } | Self::Missing { base } => base,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum SaveState {
    Idle,
    Saving { revision: Revision, target: PathBuf },
}

#[derive(Clone)]
pub struct SaveRequest {
    pub(crate) document_id: DocumentId,
    pub(crate) save_point: SavePoint,
    pub(crate) snapshot: DocumentSnapshot,
    pub(crate) source_path: PathBuf,
    pub(crate) target_path: PathBuf,
    pub(crate) expected_target: TargetExpectation,
    pub(crate) metadata: FileMetadata,
}

#[derive(Clone)]
pub(crate) enum TargetExpectation {
    Exact(Option<FileStamp>),
    CaptureAtStart,
}

impl SaveRequest {
    pub fn target_path(&self) -> &Path {
        &self.target_path
    }

    pub fn revision(&self) -> Revision {
        self.save_point.revision()
    }
}

#[derive(Clone, Debug)]
pub struct SaveOutcome {
    pub(crate) document_id: DocumentId,
    pub(crate) save_point: SavePoint,
    pub(crate) source_path: PathBuf,
    pub(crate) target_path: PathBuf,
    pub(crate) stamp: FileStamp,
    pub(crate) metadata: FileMetadata,
    warning: Option<Arc<str>>,
}

impl SaveOutcome {
    pub fn target_path(&self) -> &Path {
        &self.target_path
    }

    pub fn warning(&self) -> Option<&Arc<str>> {
        self.warning.as_ref()
    }
}

#[derive(Debug)]
pub enum SaveError {
    Io {
        path: PathBuf,
        source: io::Error,
    },
    Conflict {
        path: PathBuf,
        external: Option<FileStamp>,
    },
    Injected(&'static str),
}

impl std::fmt::Display for SaveError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Io { path, source } => write!(formatter, "{}: {source}", path.display()),
            Self::Conflict { path, .. } => {
                write!(formatter, "{} changed on disk before save", path.display())
            }
            Self::Injected(point) => write!(formatter, "injected save failure at {point}"),
        }
    }
}

impl std::error::Error for SaveError {}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum FaultPoint {
    AfterCreate,
    AfterWrite,
    BeforeReplace,
    AfterReplace,
}

pub fn write_atomic(request: SaveRequest) -> Result<SaveOutcome, SaveError> {
    write_atomic_inner(request, None)
}

fn write_atomic_inner(
    request: SaveRequest,
    fault: Option<FaultPoint>,
) -> Result<SaveOutcome, SaveError> {
    let expected_target = match request.expected_target.clone() {
        TargetExpectation::Exact(stamp) => stamp,
        TargetExpectation::CaptureAtStart => match FileStamp::read(&request.target_path) {
            Ok(stamp) => Some(stamp),
            Err(error) if error.kind() == io::ErrorKind::NotFound => None,
            Err(source) => {
                return Err(SaveError::Io {
                    path: request.target_path.clone(),
                    source,
                });
            }
        },
    };
    let parent = request
        .target_path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    let mut temporary = TemporaryFile::create(parent, &request.target_path)?;
    fail_at(fault, FaultPoint::AfterCreate)?;

    let mut written_hash = FNV_OFFSET;
    let mut written_len = 0_u64;
    if request.metadata.utf8_bom {
        temporary.write_all(UTF8_BOM)?;
        written_hash = extend_hash(written_hash, UTF8_BOM);
        written_len += UTF8_BOM.len() as u64;
    }
    let mut offset = 0_u64;
    while offset < request.snapshot.len_bytes() {
        let chunk = request
            .snapshot
            .chunk_at(ByteOffset(offset))
            .expect("offset inside a document snapshot has a chunk");
        temporary.write_all(chunk.text.as_bytes())?;
        written_hash = extend_hash(written_hash, chunk.text.as_bytes());
        written_len += chunk.text.len() as u64;
        offset = chunk.start.0 + chunk.text.len() as u64;
    }
    let permissions = fs::metadata(&request.target_path)
        .ok()
        .map(|metadata| metadata.permissions())
        .or_else(|| request.metadata.permissions.clone());
    if let Some(permissions) = permissions {
        temporary.set_permissions(permissions)?;
    }
    temporary.sync_all()?;
    fail_at(fault, FaultPoint::AfterWrite)?;

    let observed = match FileStamp::read(&request.target_path) {
        Ok(stamp) => Some(stamp),
        Err(error) if error.kind() == io::ErrorKind::NotFound => None,
        Err(source) => {
            return Err(SaveError::Io {
                path: request.target_path.clone(),
                source,
            });
        }
    };
    if observed != expected_target {
        return Err(SaveError::Conflict {
            path: request.target_path.clone(),
            external: observed,
        });
    }
    fail_at(fault, FaultPoint::BeforeReplace)?;
    let pre_replace_metadata = temporary.metadata()?;
    temporary.replace(&request.target_path)?;
    let mut warnings = Vec::new();
    if fault == Some(FaultPoint::AfterReplace) {
        warnings.push("injected post-commit failure".to_owned());
    }
    if let Err(error) = sync_directory(parent) {
        warnings.push(format!(
            "the file was replaced, but its directory could not be synced: {error}"
        ));
    }
    let written_metadata = fs::metadata(&request.target_path).unwrap_or_else(|error| {
        warnings.push(format!(
            "the file was replaced, but its final metadata could not be read: {error}"
        ));
        pre_replace_metadata
    });
    let stamp = FileStamp::from_metadata(&written_metadata, written_len, written_hash);
    let mut metadata = request.metadata;
    metadata.permissions = Some(written_metadata.permissions());
    Ok(SaveOutcome {
        document_id: request.document_id,
        save_point: request.save_point,
        source_path: request.source_path,
        target_path: request.target_path,
        stamp,
        metadata,
        warning: (!warnings.is_empty()).then(|| Arc::from(warnings.join("; "))),
    })
}

fn fail_at(fault: Option<FaultPoint>, point: FaultPoint) -> Result<(), SaveError> {
    if fault == Some(point) {
        Err(SaveError::Injected(match point {
            FaultPoint::AfterCreate => "after-create",
            FaultPoint::AfterWrite => "after-write",
            FaultPoint::BeforeReplace => "before-replace",
            FaultPoint::AfterReplace => "after-replace",
        }))
    } else {
        Ok(())
    }
}

struct TemporaryFile {
    path: PathBuf,
    file: Option<File>,
}

impl TemporaryFile {
    fn create(parent: &Path, target: &Path) -> Result<Self, SaveError> {
        let file_name = target
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("document");
        for _ in 0..128 {
            let sequence = NEXT_TEMP_FILE.fetch_add(1, Ordering::Relaxed);
            let path = parent.join(format!(
                ".{file_name}.org-studio-{}-{sequence}.tmp",
                std::process::id()
            ));
            match OpenOptions::new().write(true).create_new(true).open(&path) {
                Ok(file) => {
                    return Ok(Self {
                        path,
                        file: Some(file),
                    });
                }
                Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
                Err(source) => {
                    return Err(SaveError::Io {
                        path: path.clone(),
                        source,
                    });
                }
            }
        }
        Err(SaveError::Io {
            path: parent.to_path_buf(),
            source: io::Error::new(
                io::ErrorKind::AlreadyExists,
                "could not allocate a unique save file",
            ),
        })
    }

    fn write_all(&mut self, bytes: &[u8]) -> Result<(), SaveError> {
        self.file
            .as_mut()
            .expect("temporary file remains open before replace")
            .write_all(bytes)
            .map_err(|source| SaveError::Io {
                path: self.path.clone(),
                source,
            })
    }

    fn sync_all(&mut self) -> Result<(), SaveError> {
        self.file
            .as_mut()
            .expect("temporary file remains open before replace")
            .sync_all()
            .map_err(|source| SaveError::Io {
                path: self.path.clone(),
                source,
            })
    }

    fn set_permissions(&mut self, permissions: Permissions) -> Result<(), SaveError> {
        self.file
            .as_mut()
            .expect("temporary file remains open before replace")
            .set_permissions(permissions)
            .map_err(|source| SaveError::Io {
                path: self.path.clone(),
                source,
            })
    }

    fn metadata(&self) -> Result<fs::Metadata, SaveError> {
        self.file
            .as_ref()
            .expect("temporary file remains open before replace")
            .metadata()
            .map_err(|source| SaveError::Io {
                path: self.path.clone(),
                source,
            })
    }

    fn replace(mut self, target: &Path) -> Result<(), SaveError> {
        self.file.take();
        fs::rename(&self.path, target).map_err(|source| SaveError::Io {
            path: target.to_path_buf(),
            source,
        })?;
        self.path.clear();
        Ok(())
    }
}

impl Drop for TemporaryFile {
    fn drop(&mut self) {
        if !self.path.as_os_str().is_empty() {
            let _ = fs::remove_file(&self.path);
        }
    }
}

fn sync_directory(path: &Path) -> io::Result<()> {
    File::open(path)?.sync_all()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::DocumentSession;
    use std::os::unix::fs::{PermissionsExt, symlink};

    fn test_directory(name: &str) -> PathBuf {
        let sequence = NEXT_TEMP_FILE.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "org-studio-save-{name}-{}-{sequence}",
            std::process::id()
        ));
        fs::create_dir(&path).unwrap();
        path
    }

    fn request(path: &Path, contents: &[u8]) -> SaveRequest {
        fs::write(path, contents).unwrap();
        let session = DocumentSession::from_utf8(path.to_path_buf(), contents.to_vec()).unwrap();
        session.save_request(Some(path.to_path_buf())).unwrap()
    }

    #[test]
    fn atomic_writer_preserves_bom_and_cleans_temporary_files() {
        let directory = test_directory("bom");
        let path = directory.join("notes.org");
        let outcome = write_atomic(request(&path, b"\xef\xbb\xbfhello\r\n")).unwrap();
        assert_eq!(fs::read(&path).unwrap(), b"\xef\xbb\xbfhello\r\n");
        assert_eq!(outcome.metadata.newline, OriginalNewline::CrLf);
        assert_eq!(fs::read_dir(&directory).unwrap().count(), 1);
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn every_fault_point_keeps_original_and_removes_temporary_file() {
        for fault in [
            FaultPoint::AfterCreate,
            FaultPoint::AfterWrite,
            FaultPoint::BeforeReplace,
        ] {
            let directory = test_directory("fault");
            let path = directory.join("notes.org");
            let save = request(&path, b"original");
            assert!(matches!(
                write_atomic_inner(save, Some(fault)),
                Err(SaveError::Injected(_))
            ));
            assert_eq!(fs::read(&path).unwrap(), b"original");
            assert_eq!(fs::read_dir(&directory).unwrap().count(), 1);
            fs::remove_dir_all(directory).unwrap();
        }
    }

    #[test]
    fn post_replace_failure_is_a_committed_outcome_not_a_save_error() {
        let directory = test_directory("post-commit");
        let path = directory.join("notes.org");
        let save = request(&path, b"committed");
        let outcome = write_atomic_inner(save, Some(FaultPoint::AfterReplace)).unwrap();
        assert_eq!(fs::read(&path).unwrap(), b"committed");
        assert!(outcome.warning().is_some());
        assert_eq!(fs::read_dir(&directory).unwrap().count(), 1);
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn changed_target_is_never_overwritten() {
        let directory = test_directory("conflict");
        let path = directory.join("notes.org");
        let save = request(&path, b"base");
        fs::write(&path, b"external").unwrap();
        assert!(matches!(
            write_atomic(save),
            Err(SaveError::Conflict { .. })
        ));
        assert_eq!(fs::read(&path).unwrap(), b"external");
        assert_eq!(fs::read_dir(&directory).unwrap().count(), 1);
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn saving_through_a_symlink_updates_its_target_without_replacing_the_link() {
        let directory = test_directory("symlink");
        let target = directory.join("target.org");
        let link = directory.join("notes.org");
        fs::write(&target, b"linked").unwrap();
        symlink(&target, &link).unwrap();

        let outcome = write_atomic(request(&link, b"linked")).unwrap();

        assert_eq!(outcome.target_path(), target.canonicalize().unwrap());
        assert!(
            fs::symlink_metadata(&link)
                .unwrap()
                .file_type()
                .is_symlink()
        );
        assert_eq!(fs::read(&target).unwrap(), b"linked");
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn atomic_writer_preserves_existing_unix_permissions() {
        let directory = test_directory("permissions");
        let path = directory.join("notes.org");
        fs::write(&path, b"private").unwrap();
        fs::set_permissions(&path, Permissions::from_mode(0o640)).unwrap();

        write_atomic(request(&path, b"private")).unwrap();

        assert_eq!(
            fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o640
        );
        fs::remove_dir_all(directory).unwrap();
    }
}
