//! Retention operates only on explicitly listed application-owned data. It
//! never follows symlinks, removes user PDFs, or deletes pending edit journals.
use std::fs;
use std::path::Path;
#[cfg(windows)]
use std::path::PathBuf;
use std::time::{Duration, SystemTime};

const CACHE_FOLDERS: &[&str] = &[
    "performance-cache",
    "ocr-cache",
    "liquid2-cache",
    "liquid2-fast-cache",
    "liquid2-pymupdf-cache",
    "liquid2-pymupdf-work",
    "liquid2-ppdoclayout-cache",
    "liquid2-ppdoclayout-work",
];

#[derive(Default)]
pub struct StorageReport {
    pub removed_files: usize,
    pub removed_bytes: u64,
}

pub fn maintain(
    clear_caches: bool,
    days: u16,
    live_revisions: &[crate::document_store::FileRevision],
) -> Result<StorageReport, String> {
    let root = crate::settings::app_data_dir()
        .ok_or_else(|| "Could not find local application data.".to_owned())?;
    let age = Duration::from_secs(u64::from(days.clamp(1, 365)) * 86400);
    let mut report = StorageReport::default();
    for folder in CACHE_FOLDERS {
        prune_files(
            &root.join(folder),
            if clear_caches { Duration::ZERO } else { age },
            &mut report,
        )?;
    }
    #[cfg(windows)]
    if !cfg!(test) {
        if let Some(local) = std::env::var_os("LOCALAPPDATA") {
            prune_files(
                &PathBuf::from(local).join("LawPDF/performance-cache"),
                if clear_caches { Duration::ZERO } else { age },
                &mut report,
            )?;
        }
    }
    prune_files(
        &root.join("speech-cache"),
        Duration::from_secs(86400),
        &mut report,
    )?;
    // Backups and recovery sources follow retention, not the cache-clear action.
    prune_files(&root.join("backups"), age, &mut report)?;
    crate::document_store::DocumentStore::new()?.prune_sources(
        if clear_caches { Duration::ZERO } else { age },
        live_revisions,
    )?;
    Ok(report)
}

fn prune_files(root: &Path, age: Duration, report: &mut StorageReport) -> Result<(), String> {
    let mut pending = vec![root.to_path_buf()];
    let mut visited = 0usize;
    let now = SystemTime::now();
    while let Some(path) = pending.pop() {
        visited += 1;
        if visited > 200_000 {
            return Err("The cache contains too many entries to clear at once.".to_owned());
        }
        let metadata = match fs::symlink_metadata(&path) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(error) => return Err(format!("Could not inspect local cache: {error}")),
        };
        if metadata.file_type().is_symlink() {
            continue;
        }
        if metadata.is_dir() {
            for entry in fs::read_dir(path)
                .map_err(|error| format!("Could not read local cache: {error}"))?
            {
                pending.push(entry.map_err(|error| error.to_string())?.path());
            }
        } else if metadata.is_file()
            && metadata
                .modified()
                .ok()
                .and_then(|modified| now.duration_since(modified).ok())
                .is_some_and(|elapsed| elapsed >= age)
        {
            match fs::remove_file(path) {
                Ok(()) => {
                    report.removed_files += 1;
                    report.removed_bytes += metadata.len();
                }
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => return Err(format!("Could not clear a local cache file: {error}")),
            }
        }
    }
    Ok(())
}
