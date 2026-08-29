use std::{
    fs,
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

use super::{ExportError, ExportFormat};

pub(super) fn write(
    destination: &Path,
    format: ExportFormat,
    artifacts: &[Vec<u8>],
) -> Result<Vec<PathBuf>, ExportError> {
    if artifacts.is_empty() {
        return Err(ExportError::Render("export produced no artifacts".into()));
    }
    if artifacts.len() == 1 {
        return write_one(destination, &artifacts[0]).map(|path| vec![path]);
    }
    write_pages(destination, format, artifacts)
}

fn write_one(destination: &Path, bytes: &[u8]) -> Result<PathBuf, ExportError> {
    let parent = destination.parent().unwrap_or_else(|| Path::new("."));
    let name = destination
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("export");
    let staging = parent.join(format!(".{name}.org-studio-{}.tmp", nonce()));
    fs::write(&staging, bytes).map_err(|error| io_error(&staging, error))?;
    if let Err(error) = fs::rename(&staging, destination) {
        let _ = fs::remove_file(&staging);
        return Err(io_error(destination, error));
    }
    Ok(destination.to_path_buf())
}

fn write_pages(
    destination: &Path,
    format: ExportFormat,
    artifacts: &[Vec<u8>],
) -> Result<Vec<PathBuf>, ExportError> {
    let parent = destination.parent().unwrap_or_else(|| Path::new("."));
    let stem = destination
        .file_stem()
        .and_then(|stem| stem.to_str())
        .unwrap_or("export");
    let final_dir = parent.join(format!("{stem}-pages"));
    let staging = parent.join(format!(".{stem}-pages.org-studio-{}.tmp", nonce()));
    fs::create_dir(&staging).map_err(|error| io_error(&staging, error))?;
    let width = artifacts.len().to_string().len().max(2);
    let mut staged = Vec::with_capacity(artifacts.len());
    for (index, bytes) in artifacts.iter().enumerate() {
        let path = staging.join(format!(
            "{stem}-{:0width$}.{}",
            index + 1,
            format.extension(),
            width = width
        ));
        if let Err(error) = fs::write(&path, bytes) {
            let _ = fs::remove_dir_all(&staging);
            return Err(io_error(&path, error));
        }
        staged.push(path);
    }
    if final_dir.exists() {
        let _ = fs::remove_dir_all(&staging);
        return Err(ExportError::Io {
            path: final_dir,
            message: "page output directory already exists".into(),
        });
    }
    if let Err(error) = fs::rename(&staging, &final_dir) {
        let _ = fs::remove_dir_all(&staging);
        return Err(io_error(&final_dir, error));
    }
    Ok(staged
        .into_iter()
        .map(|path| final_dir.join(path.file_name().expect("page filename")))
        .collect())
}

fn nonce() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos()
}

fn io_error(path: &Path, error: std::io::Error) -> ExportError {
    ExportError::Io {
        path: path.to_path_buf(),
        message: error.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn writes_single_and_paged_outputs() {
        let root = std::env::temp_dir().join(format!("org-studio-export-test-{}", nonce()));
        fs::create_dir(&root).unwrap();
        let single = root.join("one.pdf");
        let written = write(&single, ExportFormat::Pdf, &[b"pdf".to_vec()]).unwrap();
        assert_eq!(fs::read(&written[0]).unwrap(), b"pdf");

        let base = root.join("many.png");
        let written = write(
            &base,
            ExportFormat::Png,
            &[b"one".to_vec(), b"two".to_vec()],
        )
        .unwrap();
        assert_eq!(written[0].file_name().unwrap(), "many-01.png");
        assert_eq!(fs::read(&written[1]).unwrap(), b"two");
        fs::remove_dir_all(root).unwrap();
    }
}
