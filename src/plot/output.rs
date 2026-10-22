use std::{
    io::Write,
    path::{Component, Path, PathBuf},
    sync::{Arc, Mutex},
};

use super::{PlotDiagnostic, PlotOutputTarget};

pub(crate) fn normalize_path(path: &Path) -> PathBuf {
    let mut result = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir if result.file_name().is_some_and(|name| name != "..") => {
                result.pop();
            }
            _ => result.push(component.as_os_str()),
        }
    }
    result
}

pub(crate) fn destination(
    target: &PlotOutputTarget,
    document: Option<&Path>,
) -> Result<Option<PathBuf>, PlotDiagnostic> {
    let PlotOutputTarget::File(path) = target else {
        return Ok(None);
    };
    if path.is_absolute() {
        return Ok(Some(normalize_path(path)));
    }
    let root = document_directory(document)?;
    Ok(Some(normalize_path(&root.join(path))))
}

pub(crate) fn document_directory(document: Option<&Path>) -> Result<PathBuf, PlotDiagnostic> {
    let document = document.filter(|path| !path.as_os_str().is_empty())
        .ok_or_else(|| PlotDiagnostic::new("unsaved-output", "Save the Org document before using a relative plot file, or specify an absolute output path"))?;
    let absolute = if document.is_absolute() {
        document.to_path_buf()
    } else {
        std::env::current_dir()
            .map_err(|error| PlotDiagnostic::new("plot-output", error.to_string()))?
            .join(document)
    };
    absolute.parent().map(ToOwned::to_owned).ok_or_else(|| {
        PlotDiagnostic::new("plot-output", "The Org document has no parent directory")
    })
}

/// The epoch is invalidated before an edit or path change can publish a new job.
/// Holding it across the atomic rename prevents an obsolete render from committing.
pub(crate) fn write_file(
    path: &Path,
    bytes: &[u8],
    epoch: &Arc<Mutex<u64>>,
    expected: u64,
) -> Result<(), PlotDiagnostic> {
    let write = || -> std::io::Result<()> {
        if *epoch.lock().unwrap() != expected {
            return Ok(());
        }
        let parent = path.parent().unwrap_or_else(|| Path::new("."));
        std::fs::create_dir_all(parent)?;
        let mut file = tempfile::NamedTempFile::new_in(parent)?;
        file.write_all(bytes)?;
        let guard = epoch.lock().unwrap();
        if *guard != expected {
            return Ok(());
        }
        file.persist(path).map_err(|error| error.error)?;
        Ok(())
    };
    write().map_err(|error| {
        PlotDiagnostic::new(
            "plot-output",
            format!("Could not save {}: {error}", path.display()),
        )
    })
}
