use std::{
    fs::{self, OpenOptions},
    io::Write,
    path::{Component, Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
};

pub(super) static NEXT_TEMP_OUTPUT: AtomicU64 = AtomicU64::new(1);

pub(super) struct PendingArtifact {
    pub(super) temporary: PathBuf,
    pub(super) target: PathBuf,
    pub(super) allowed_root: PathBuf,
}

impl PendingArtifact {
    pub(super) fn publish(&mut self) -> Result<(), String> {
        validate_output_parent(&self.target, &self.allowed_root)?;
        validate_publish_target(&self.target)?;
        fs::rename(&self.temporary, &self.target)
            .map_err(|error| format!("Could not publish {}: {error}", self.target.display()))?;
        self.temporary = PathBuf::new();
        Ok(())
    }
}

pub(super) fn validate_publish_target(target: &Path) -> Result<(), String> {
    match fs::symlink_metadata(target) {
        Ok(metadata) if metadata.file_type().is_file() || metadata.file_type().is_symlink() => {
            Ok(())
        }
        Ok(_) => Err(format!(
            "Could not publish {}: the target is not a file",
            target.display()
        )),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(format!("Could not inspect {}: {error}", target.display())),
    }
}

impl Drop for PendingArtifact {
    fn drop(&mut self) {
        if !self.temporary.as_os_str().is_empty() {
            let _ = fs::remove_file(&self.temporary);
        }
    }
}

pub(super) fn resolve_output_target(document_root: &Path, file: &str) -> Result<PathBuf, String> {
    let file = Path::new(file);
    if file.is_absolute() {
        return Err("Babel :file must be relative to the Org document directory".to_owned());
    }
    let mut relative = PathBuf::new();
    for component in file.components() {
        match component {
            Component::Normal(part) => relative.push(part),
            Component::CurDir => {}
            Component::ParentDir => {
                return Err("Babel :file must not leave the Org document directory".to_owned());
            }
            Component::RootDir | Component::Prefix(_) => {
                return Err("Babel :file must be relative to the Org document directory".to_owned());
            }
        }
    }
    if relative.file_name().is_none() {
        return Err("The :file target has no valid filename".to_owned());
    }
    Ok(document_root.join(relative))
}

pub(super) fn validate_output_parent(
    target: &Path,
    allowed_root: &Path,
) -> Result<PathBuf, String> {
    let parent = target.parent().unwrap_or_else(|| Path::new("."));
    let parent = fs::canonicalize(parent)
        .map_err(|error| format!("Could not resolve {}: {error}", parent.display()))?;
    if !parent.starts_with(allowed_root) {
        return Err(format!(
            "Babel :file resolves outside the Org document directory: {}",
            target.display()
        ));
    }
    Ok(parent)
}

pub(super) fn write_temporary_output(
    target: &Path,
    document_root: &Path,
    bytes: &[u8],
) -> Result<PendingArtifact, String> {
    let allowed_root = fs::canonicalize(document_root).map_err(|error| {
        format!(
            "Could not resolve document directory {}: {error}",
            document_root.display()
        )
    })?;
    let relative = target.strip_prefix(document_root).map_err(|_| {
        format!(
            "Output target is outside the document directory: {}",
            target.display()
        )
    })?;
    let mut parent = allowed_root.clone();
    for component in relative
        .parent()
        .unwrap_or_else(|| Path::new(""))
        .components()
    {
        let Component::Normal(name) = component else {
            return Err("Output path has an invalid directory component".to_owned());
        };
        parent.push(name);
        match fs::create_dir(&parent) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
            Err(error) => return Err(format!("Could not create {}: {error}", parent.display())),
        }
        parent = fs::canonicalize(&parent)
            .map_err(|error| format!("Could not resolve {}: {error}", parent.display()))?;
        if !parent.starts_with(&allowed_root) {
            return Err(format!(
                "Babel :file resolves outside the Org document directory: {}",
                target.display()
            ));
        }
    }
    let name = target
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| "The :file target has no valid filename".to_owned())?;
    for _ in 0..16 {
        let sequence = NEXT_TEMP_OUTPUT.fetch_add(1, Ordering::Relaxed);
        let temporary = parent.join(format!(
            ".{name}.org-studio-{}-{sequence}.tmp",
            std::process::id()
        ));
        match OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&temporary)
        {
            Ok(mut file) => {
                if let Err(error) = file.write_all(bytes).and_then(|_| file.sync_all()) {
                    let _ = fs::remove_file(&temporary);
                    return Err(format!("Could not write {}: {error}", target.display()));
                }
                return Ok(PendingArtifact {
                    temporary,
                    target: target.to_path_buf(),
                    allowed_root,
                });
            }
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(error) => {
                return Err(format!("Could not create {}: {error}", target.display()));
            }
        }
    }
    Err(format!(
        "Could not reserve a temporary file beside {}",
        target.display()
    ))
}
