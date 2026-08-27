use serde::{Deserialize, Serialize};
use std::{
    fs, io,
    path::{Path, PathBuf},
    sync::{OnceLock, mpsc},
    time::{SystemTime, UNIX_EPOCH},
};

const STORE_VERSION: u32 = 1;
const MAX_RECENT_DOCUMENTS: usize = 10;

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct RecentDocument {
    pub path: PathBuf,
    pub opened_at: u64,
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
struct StoredRecentDocuments {
    version: u32,
    documents: Vec<RecentDocument>,
}

pub fn load() -> Vec<RecentDocument> {
    let Some(path) = store_path() else {
        return Vec::new();
    };
    let Ok(source) = fs::read_to_string(path) else {
        return Vec::new();
    };
    let Ok(mut store) = serde_json::from_str::<StoredRecentDocuments>(&source) else {
        return Vec::new();
    };
    if store.version != STORE_VERSION {
        return Vec::new();
    }
    normalize(&mut store.documents, true);
    store.documents
}

pub fn record_success(documents: &mut Vec<RecentDocument>, path: PathBuf) {
    documents.retain(|document| document.path != path);
    documents.insert(
        0,
        RecentDocument {
            path,
            opened_at: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs(),
        },
    );
    normalize(documents, false);
    save_async(documents.clone());
}

pub fn remove(documents: &mut Vec<RecentDocument>, path: &Path) {
    documents.retain(|document| document.path != path);
    save_async(documents.clone());
}

pub fn clear(documents: &mut Vec<RecentDocument>) {
    documents.clear();
    save_async(Vec::new());
}

fn normalize(documents: &mut Vec<RecentDocument>, remove_missing: bool) {
    documents.sort_by_key(|document| std::cmp::Reverse(document.opened_at));
    let mut seen = std::collections::HashSet::new();
    documents.retain(|document| {
        seen.insert(document.path.clone()) && (!remove_missing || document.path.is_file())
    });
    documents.truncate(MAX_RECENT_DOCUMENTS);
}

fn save_async(documents: Vec<RecentDocument>) {
    static SENDER: OnceLock<mpsc::Sender<Vec<RecentDocument>>> = OnceLock::new();
    let sender = SENDER.get_or_init(|| {
        let (sender, receiver) = mpsc::channel::<Vec<RecentDocument>>();
        std::thread::Builder::new()
            .name("org-studio-recents".into())
            .spawn(move || {
                while let Ok(mut latest) = receiver.recv() {
                    while let Ok(newer) = receiver.try_recv() {
                        latest = newer;
                    }
                    if let Err(error) = save(&latest) {
                        eprintln!("org_studio_recents_save_failed error={error}");
                    }
                }
            })
            .expect("recent document writer thread must start");
        sender
    });
    let _ = sender.send(documents);
}

fn save(documents: &[RecentDocument]) -> io::Result<()> {
    let Some(path) = store_path() else {
        return Ok(());
    };
    let Some(parent) = path.parent() else {
        return Ok(());
    };
    fs::create_dir_all(parent)?;
    let temporary = path.with_extension(format!("tmp-{}", std::process::id()));
    let source = serde_json::to_vec_pretty(&StoredRecentDocuments {
        version: STORE_VERSION,
        documents: documents.to_vec(),
    })?;
    fs::write(&temporary, source)?;
    fs::rename(temporary, path)
}

fn store_path() -> Option<PathBuf> {
    crate::settings::application_support_dir().map(|path| path.join("recent-documents.json"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalize_orders_deduplicates_and_limits_documents() {
        let mut documents = (0..12)
            .map(|index| RecentDocument {
                path: PathBuf::from(format!("/{index}.org")),
                opened_at: index,
            })
            .collect::<Vec<_>>();
        documents.push(RecentDocument {
            path: PathBuf::from("/11.org"),
            opened_at: 100,
        });
        normalize(&mut documents, false);
        assert_eq!(documents.len(), MAX_RECENT_DOCUMENTS);
        assert_eq!(documents[0].path, PathBuf::from("/11.org"));
        assert_eq!(
            documents
                .iter()
                .filter(|document| document.path == Path::new("/11.org"))
                .count(),
            1
        );
    }
}
