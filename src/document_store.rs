//! Durable editing state, independent of native PDF handles and UI lifetimes.
//!
//! A journal records a complete annotation revision and references an immutable
//! source PDF. Recovery never has to guess which version of a cloud file the
//! annotation geometry belonged to. Source snapshots are private local data.
use std::fs::{self, File};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::model::{AnnotationKind, EditorAnnotation};

pub const MAX_DOCUMENT_BYTES: u64 = 512 * 1024 * 1024;
const MAX_JOURNAL_BYTES: u64 = 32 * 1024 * 1024;
const JOURNAL_VERSION: u32 = 1;
const MAX_RECOVERY_BYTES: u64 = 4 * 1024 * 1024 * 1024;
static STORE_ACCESS: Mutex<()> = Mutex::new(());

fn store_access() -> MutexGuard<'static, ()> {
    STORE_ACCESS
        .lock()
        .unwrap_or_else(|error| error.into_inner())
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FileRevision {
    pub sha256: String,
    pub bytes: u64,
}

impl FileRevision {
    pub fn from_bytes(bytes: &[u8]) -> Self {
        Self {
            sha256: format!("{:x}", Sha256::digest(bytes)),
            bytes: bytes.len() as u64,
        }
    }

    pub fn read(path: &Path) -> Result<Self, String> {
        let mut file = File::open(path).map_err(|error| format!("Could not read PDF: {error}"))?;
        let mut digest = Sha256::new();
        let mut bytes = 0u64;
        let mut buffer = [0u8; 64 * 1024];
        loop {
            let read = file
                .read(&mut buffer)
                .map_err(|error| format!("Could not read PDF: {error}"))?;
            if read == 0 {
                break;
            }
            bytes += read as u64;
            if bytes > MAX_DOCUMENT_BYTES {
                return Err("This PDF exceeds the 512 MB document limit.".to_owned());
            }
            digest.update(&buffer[..read]);
        }
        Ok(Self {
            sha256: format!("{:x}", digest.finalize()),
            bytes,
        })
    }

    pub fn require_current(&self, path: &Path) -> Result<(), String> {
        if Self::read(path)? == *self {
            Ok(())
        } else {
            Err("This PDF changed in another application or cloud sync. The other version was kept. Save your recovered annotations as a separate copy.".to_owned())
        }
    }

    fn valid(&self) -> bool {
        self.bytes <= MAX_DOCUMENT_BYTES && valid_digest(&self.sha256)
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RecoveryRecord {
    pub schema_version: u32,
    pub original_path: PathBuf,
    pub base_revision: FileRevision,
    pub generation: u64,
    pub annotations: Vec<EditorAnnotation>,
}

#[derive(Debug, Clone)]
pub struct RecoveryItem {
    pub journal_path: PathBuf,
    pub record: RecoveryRecord,
    pub snapshot_path: PathBuf,
}

#[derive(Debug, Default)]
pub struct RecoveryScan {
    pub items: Vec<RecoveryItem>,
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct DocumentStore {
    root: PathBuf,
}

impl DocumentStore {
    pub fn new() -> Result<Self, String> {
        let root = crate::settings::app_data_dir()
            .ok_or_else(|| "Could not find the recovery folder.".to_owned())?
            .join("document-recovery");
        Ok(Self { root })
    }

    pub fn prune_sources(
        &self,
        age: std::time::Duration,
        live: &[FileRevision],
    ) -> Result<(), String> {
        let _access = store_access();
        let mut keep = live
            .iter()
            .map(|revision| revision.sha256.clone())
            .collect::<std::collections::HashSet<_>>();
        let pending = self.root.join("pending");
        if pending.exists() {
            for entry in fs::read_dir(pending).map_err(|error| error.to_string())? {
                let path = entry.map_err(|error| error.to_string())?.path();
                if path
                    .extension()
                    .is_some_and(|extension| extension == "json")
                {
                    // A damaged journal prevents pruning: its source might be
                    // the only remaining copy of a document.
                    keep.insert(self.read_record(&path)?.base_revision.sha256);
                }
            }
        }
        let sources = self.root.join("sources");
        if !sources.exists() {
            return Ok(());
        }
        let now = std::time::SystemTime::now();
        for entry in fs::read_dir(sources).map_err(|error| error.to_string())? {
            let path = entry.map_err(|error| error.to_string())?.path();
            let Some(digest) = path.file_stem().and_then(|stem| stem.to_str()) else {
                continue;
            };
            if !valid_digest(digest) || keep.contains(digest) {
                continue;
            }
            let metadata = fs::symlink_metadata(&path).map_err(|error| error.to_string())?;
            if metadata.is_file()
                && metadata
                    .modified()
                    .ok()
                    .and_then(|modified| now.duration_since(modified).ok())
                    .is_some_and(|elapsed| elapsed >= age.max(std::time::Duration::from_secs(120)))
            {
                remove_durable(&path)?;
            }
        }
        Ok(())
    }

    /// Capture before editing, on the document worker. The content hash also
    /// acts as the compare-before-replace token for all subsequent PDF writes.
    pub fn capture(&self, source: &Path) -> Result<FileRevision, String> {
        let bytes = read_limited(source, MAX_DOCUMENT_BYTES)?;
        let revision = FileRevision::from_bytes(&bytes);
        revision.require_current(source)?;
        let _access = store_access();
        self.write_snapshot(&revision, &bytes)?;
        Ok(revision)
    }

    /// Reserve and flush the next recovery source before changing the user's
    /// file, so a full recovery volume cannot turn a successful write into an
    /// untracked document revision.
    pub fn preserve_bytes(&self, bytes: &[u8]) -> Result<FileRevision, String> {
        if bytes.len() as u64 > MAX_DOCUMENT_BYTES {
            return Err("This PDF exceeds the document limit.".to_owned());
        }
        let revision = FileRevision::from_bytes(bytes);
        let _access = store_access();
        self.write_snapshot(&revision, bytes)?;
        Ok(revision)
    }

    fn snapshot_path(&self, revision: &FileRevision) -> Result<PathBuf, String> {
        if !revision.valid() {
            return Err("Invalid recovery snapshot identifier.".to_owned());
        }
        Ok(self
            .root
            .join("sources")
            .join(format!("{}.pdf", revision.sha256)))
    }

    fn journal_path(&self, source: &Path) -> PathBuf {
        // The path is canonicalized when opening, but must remain stable after
        // a file is moved or its former symlink resolves to a different target.
        let key = source.to_string_lossy();
        self.root
            .join("pending")
            .join(format!("{:x}.json", Sha256::digest(key.as_bytes())))
    }

    fn write_snapshot(&self, revision: &FileRevision, bytes: &[u8]) -> Result<PathBuf, String> {
        if FileRevision::from_bytes(bytes) != *revision {
            return Err("Recovery snapshot did not match the document revision.".to_owned());
        }
        let path = self.snapshot_path(revision)?;
        if path.is_file() {
            if FileRevision::read(&path)? == *revision {
                // Opening a deduplicated, old snapshot renews the pruning
                // grace period before the UI has received its live revision.
                File::open(&path)
                    .and_then(|file| file.set_modified(std::time::SystemTime::now()))
                    .map_err(|error| error.to_string())?;
                return Ok(path);
            }
            return Err(
                "The recovery snapshot is damaged; the PDF was not overwritten.".to_owned(),
            );
        }
        private_directory(path.parent().expect("snapshot parent"))?;
        let mut used = 0u64;
        for entry in fs::read_dir(path.parent().expect("snapshot parent"))
            .map_err(|error| error.to_string())?
        {
            let metadata = entry
                .map_err(|error| error.to_string())?
                .metadata()
                .map_err(|error| error.to_string())?;
            used = used.saturating_add(metadata.len());
        }
        if used.saturating_add(bytes.len() as u64) > MAX_RECOVERY_BYTES {
            return Err("Local recovery storage reached its 4 GB limit. Use Settings to clear unused cached data, then try saving again. Pending recovery edits are preserved.".to_owned());
        }
        crate::atomic_file::write(&path, bytes)
            .map_err(|error| format!("Could not preserve the recovery source: {error}"))?;
        Ok(path)
    }

    /// This small, flushed write happens when an edit is accepted, before the
    /// debounced whole-PDF save. Failure leaves the UI dirty and is surfaced.
    pub fn journal(
        &self,
        source: &Path,
        revision: &FileRevision,
        generation: u64,
        annotations: &[EditorAnnotation],
    ) -> Result<(), String> {
        validate_annotations(annotations)?;
        let _access = store_access();
        let snapshot = self.snapshot_path(revision)?;
        if !snapshot.is_file() {
            return Err(
                "No recovery source is available. Keep the document open and save a copy."
                    .to_owned(),
            );
        }
        let record = RecoveryRecord {
            schema_version: JOURNAL_VERSION,
            original_path: source.to_path_buf(),
            base_revision: revision.clone(),
            generation,
            annotations: annotations.to_vec(),
        };
        self.write_record(&record)
    }

    fn write_record(&self, record: &RecoveryRecord) -> Result<(), String> {
        let bytes = serde_json::to_vec(record)
            .map_err(|error| format!("Could not encode recovery data: {error}"))?;
        if bytes.len() as u64 > MAX_JOURNAL_BYTES {
            return Err("The annotation recovery limit was reached. Save a copy before adding more annotations.".to_owned());
        }
        let path = self.journal_path(&record.original_path);
        private_directory(path.parent().expect("journal parent"))?;
        crate::atomic_file::write(&path, &bytes)
            .map_err(|error| format!("Could not protect this edit for recovery: {error}"))
    }

    /// A completed save may lag behind newer edits. Rebase their journal onto
    /// the newly saved immutable source instead of deleting the newer revision.
    pub fn acknowledge(
        &self,
        source: &Path,
        generation: u64,
        saved_bytes: &[u8],
    ) -> Result<FileRevision, String> {
        let revision = FileRevision::from_bytes(saved_bytes);
        let _access = store_access();
        self.write_snapshot(&revision, saved_bytes)?;
        let path = self.journal_path(source);
        match self.read_record(&path) {
            Ok(mut record) if record.generation > generation => {
                record.base_revision = revision.clone();
                self.write_record(&record)?;
            }
            Ok(_) => remove_durable(&path)?,
            Err(_) if !path.exists() => {}
            Err(error) => return Err(error),
        }
        Ok(revision)
    }

    pub fn acknowledge_saved_file(
        &self,
        source: &Path,
        generation: u64,
        expected: &FileRevision,
    ) -> Result<(), String> {
        let bytes = read_limited(source, MAX_DOCUMENT_BYTES)?;
        if FileRevision::from_bytes(&bytes) != *expected {
            return Err("The PDF changed again before saving was confirmed. Your recovery record was kept; save a separate copy.".to_owned());
        }
        self.acknowledge(source, generation, &bytes)?;
        Ok(())
    }

    pub fn discard(&self, source: &Path) -> Result<(), String> {
        let _access = store_access();
        remove_durable(&self.journal_path(source))
    }

    pub fn discard_recovery(&self, item: &RecoveryItem) -> Result<(), String> {
        let _access = store_access();
        let current = self.read_record(&item.journal_path)?;
        if current != item.record {
            return Err("These recovered edits changed while the dialog was open. Refresh recovery before deciding what to keep.".to_owned());
        }
        remove_durable(&item.journal_path)
    }

    pub fn pending_for(&self, source: &Path) -> Result<Option<RecoveryItem>, String> {
        let _access = store_access();
        let path = self.journal_path(source);
        if !path.exists() {
            return Ok(None);
        }
        self.read_item(path).map(Some)
    }

    pub fn pending(&self) -> Result<RecoveryScan, String> {
        let _access = store_access();
        let directory = self.root.join("pending");
        if !directory.exists() {
            return Ok(RecoveryScan::default());
        }
        let mut scan = RecoveryScan::default();
        for entry in fs::read_dir(directory)
            .map_err(|error| format!("Could not read recovery files: {error}"))?
        {
            let path = entry.map_err(|error| error.to_string())?.path();
            if path
                .extension()
                .is_some_and(|extension| extension == "json")
            {
                match self.read_item(path) {
                    Ok(item) => scan.items.push(item),
                    Err(error) => scan.warnings.push(error),
                }
            }
        }
        scan.items
            .sort_by(|a, b| a.record.original_path.cmp(&b.record.original_path));
        Ok(scan)
    }

    fn read_record(&self, path: &Path) -> Result<RecoveryRecord, String> {
        let bytes = read_limited(path, MAX_JOURNAL_BYTES)?;
        let record: RecoveryRecord = serde_json::from_slice(&bytes).map_err(|_| "A recovery record is damaged. It was kept for recovery; it will not overwrite a PDF.".to_owned())?;
        if record.schema_version != JOURNAL_VERSION || !record.base_revision.valid() {
            return Err(
                "This recovery record needs a compatible LawPDF version. It was kept unchanged."
                    .to_owned(),
            );
        }
        validate_annotations(&record.annotations)?;
        if self.journal_path(&record.original_path) != path {
            return Err("Recovery record identity does not match its file.".to_owned());
        }
        Ok(record)
    }

    fn read_item(&self, path: PathBuf) -> Result<RecoveryItem, String> {
        let record = self.read_record(&path)?;
        let snapshot_path = self.snapshot_path(&record.base_revision)?;
        record.base_revision.require_current(&snapshot_path)?;
        Ok(RecoveryItem {
            journal_path: path,
            record,
            snapshot_path,
        })
    }

    /// Recover against the immutable source, even if the original was moved,
    /// removed, or replaced. The user always chooses the output destination.
    pub fn export_recovery(&self, item: &RecoveryItem, destination: &Path) -> Result<(), String> {
        if crate::settings::pdf_document_key(destination)
            == crate::settings::pdf_document_key(&item.record.original_path)
        {
            return Err("Choose a different filename to preserve both versions.".to_owned());
        }
        let parent = destination
            .parent()
            .ok_or_else(|| "Choose a PDF filename.".to_owned())?;
        let resolved_parent = fs::canonicalize(parent)
            .map_err(|error| format!("Could not open the destination folder: {error}"))?;
        let resolved_root = fs::canonicalize(&self.root).map_err(|error| error.to_string())?;
        let resolved_destination = fs::canonicalize(destination).ok();
        if resolved_parent.starts_with(&resolved_root)
            || resolved_destination
                .as_ref()
                .is_some_and(|path| path.starts_with(&resolved_root))
        {
            return Err("Choose a location outside LawPDF's recovery folder.".to_owned());
        }
        item.record
            .base_revision
            .require_current(&item.snapshot_path)?;
        crate::pdf_backend::save_with_annotations(
            &item.snapshot_path,
            destination,
            &item.record.annotations,
        )
        .map_err(|error| format!("Could not create the recovery copy: {error:#}"))?;
        Ok(())
    }

    pub fn export_annotations(
        &self,
        source: &Path,
        revision: &FileRevision,
        annotations: &[EditorAnnotation],
        destination: &Path,
    ) -> Result<(), String> {
        let item = RecoveryItem {
            journal_path: self.journal_path(source),
            snapshot_path: self.snapshot_path(revision)?,
            record: RecoveryRecord {
                schema_version: JOURNAL_VERSION,
                original_path: source.to_path_buf(),
                base_revision: revision.clone(),
                generation: 0,
                annotations: annotations.to_vec(),
            },
        };
        self.export_recovery(&item, destination)
    }
}

fn valid_digest(value: &str) -> bool {
    value.len() == 64 && value.bytes().all(|byte| byte.is_ascii_hexdigit())
}

pub(crate) fn read_limited(path: &Path, limit: u64) -> Result<Vec<u8>, String> {
    let file =
        File::open(path).map_err(|error| format!("Could not read {}: {error}", path.display()))?;
    if file.metadata().map_err(|error| error.to_string())?.len() > limit {
        return Err("This file exceeds the supported size limit.".to_owned());
    }
    let mut bytes = Vec::new();
    file.take(limit + 1)
        .read_to_end(&mut bytes)
        .map_err(|error| error.to_string())?;
    if bytes.len() as u64 > limit {
        return Err("This file grew beyond the supported size limit.".to_owned());
    }
    Ok(bytes)
}

fn private_directory(path: &Path) -> Result<(), String> {
    fs::create_dir_all(path)
        .map_err(|error| format!("Could not create recovery folder: {error}"))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o700))
            .map_err(|error| error.to_string())?;
    }
    Ok(())
}

fn remove_durable(path: &Path) -> Result<(), String> {
    match fs::remove_file(path) {
        Ok(()) => crate::atomic_file::sync_directory(path.parent().expect("journal parent"))
            .map_err(|error| error.to_string()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(format!("Could not finish clearing recovery state: {error}")),
    }
}

pub fn validate_annotations(annotations: &[EditorAnnotation]) -> Result<(), String> {
    if annotations.len() > 100_000 {
        return Err("Too many annotations for one document.".to_owned());
    }
    for annotation in annotations {
        let rect = annotation.rect;
        if annotation.page_index >= 20_000
            || rect.right < rect.left
            || rect.top < rect.bottom
            || ![rect.left, rect.bottom, rect.right, rect.top]
                .iter()
                .all(|value| value.is_finite() && value.abs() <= 10_000_000.0)
        {
            return Err("An annotation has invalid page coordinates.".to_owned());
        }
        let finite_color = |color: &[f32; 3]| {
            color
                .iter()
                .all(|value| value.is_finite() && (0.0..=1.0).contains(value))
        };
        let valid = match &annotation.kind {
            AnnotationKind::Marker {
                color_rgb, opacity, ..
            } => finite_color(color_rgb) && opacity.is_finite() && (0.0..=1.0).contains(opacity),
            AnnotationKind::TextBox {
                text,
                font_size,
                color_rgb,
            } => {
                text.len() <= 1_000_000
                    && font_size.is_finite()
                    && *font_size > 0.0
                    && finite_color(color_rgb)
            }
            AnnotationKind::Comment {
                text,
                id,
                created_at,
                updated_at,
                color_rgb,
                anchor,
            } => {
                text.len() <= 1_000_000
                    && id.len() <= 1024
                    && created_at.len() <= 1024
                    && updated_at.len() <= 1024
                    && finite_color(color_rgb)
                    && anchor.0.is_finite()
                    && anchor.1.is_finite()
            }
            AnnotationKind::Signature {
                signer,
                signed_at,
                strokes,
            } => {
                signer.len() <= 4096
                    && signed_at.len() <= 1024
                    && strokes.len() <= 10_000
                    && strokes.iter().map(Vec::len).sum::<usize>() <= 100_000
                    && strokes
                        .iter()
                        .flatten()
                        .all(|point| point.0.is_finite() && point.1.is_finite())
            }
        };
        if !valid {
            return Err("An annotation contains invalid or excessive data.".to_owned());
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn fixture() -> (tempfile::TempDir, DocumentStore, PathBuf, FileRevision) {
        let directory = tempfile::tempdir().unwrap();
        let source = directory.path().join("original.pdf");
        fs::write(&source, b"original PDF revision").unwrap();
        let store = DocumentStore {
            root: directory.path().join("recovery"),
        };
        let revision = store.capture(&source).unwrap();
        (directory, store, source, revision)
    }
    fn annotation(text: &str) -> EditorAnnotation {
        EditorAnnotation {
            page_index: 0,
            rect: crate::model::PdfRect::new(10.0, 20.0, 100.0, 40.0),
            kind: AnnotationKind::TextBox {
                text: text.to_owned(),
                font_size: 12.0,
                color_rgb: [0.0; 3],
            },
        }
    }
    #[test]
    fn newer_edits_survive_an_older_save_acknowledgment() {
        let (_directory, store, source, revision) = fixture();
        store
            .journal(&source, &revision, 2, &[annotation("newer")])
            .unwrap();
        let saved = store
            .acknowledge(&source, 1, b"saved first revision")
            .unwrap();
        let recovered = store.pending_for(&source).unwrap().unwrap();
        assert_eq!(recovered.record.generation, 2);
        assert_eq!(recovered.record.base_revision, saved);
        assert_eq!(recovered.record.annotations, vec![annotation("newer")]);
    }
    #[test]
    fn external_changes_are_rejected_and_original_snapshot_survives() {
        let (_directory, store, source, revision) = fixture();
        store
            .journal(&source, &revision, 1, &[annotation("keep me")])
            .unwrap();
        fs::write(&source, b"external editor revision").unwrap();
        assert!(revision.require_current(&source).is_err());
        let recovered = store.pending_for(&source).unwrap().unwrap();
        assert_eq!(
            fs::read(recovered.snapshot_path).unwrap(),
            b"original PDF revision"
        );
        assert_eq!(fs::read(source).unwrap(), b"external editor revision");
    }
    #[test]
    fn a_stale_recovery_dialog_cannot_discard_newer_edits() {
        let (_directory, store, source, revision) = fixture();
        store
            .journal(&source, &revision, 1, &[annotation("first")])
            .unwrap();
        let stale = store.pending_for(&source).unwrap().unwrap();
        store
            .journal(&source, &revision, 2, &[annotation("newer")])
            .unwrap();
        assert!(store.discard_recovery(&stale).is_err());
        assert_eq!(
            store
                .pending_for(&source)
                .unwrap()
                .unwrap()
                .record
                .generation,
            2
        );
    }
    #[test]
    fn damaged_record_does_not_hide_other_recoverable_documents() {
        let (directory, store, source, revision) = fixture();
        store
            .journal(&source, &revision, 1, &[annotation("recover")])
            .unwrap();
        fs::write(
            directory.path().join("recovery/pending/broken.json"),
            b"not json",
        )
        .unwrap();
        let scan = store.pending().unwrap();
        assert_eq!(scan.items.len(), 1);
        assert_eq!(scan.warnings.len(), 1);
        assert!(store.prune_sources(std::time::Duration::ZERO, &[]).is_err());
    }
    #[test]
    fn quota_failure_happens_before_any_source_change() {
        let (directory, store, source, revision) = fixture();
        let allocation =
            File::create(directory.path().join("recovery/sources/quota-placeholder")).unwrap();
        allocation.set_len(MAX_RECOVERY_BYTES).unwrap();
        assert!(store.preserve_bytes(b"new revision").is_err());
        assert_eq!(FileRevision::read(&source).unwrap(), revision);
    }
    #[test]
    fn current_and_pending_sources_survive_retention() {
        let (_directory, store, source, revision) = fixture();
        store
            .journal(&source, &revision, 1, &[annotation("keep")])
            .unwrap();
        let snapshot = store.snapshot_path(&revision).unwrap();
        File::open(&snapshot)
            .unwrap()
            .set_modified(std::time::SystemTime::UNIX_EPOCH)
            .unwrap();
        store.prune_sources(std::time::Duration::ZERO, &[]).unwrap();
        assert!(snapshot.exists());
        store.discard(&source).unwrap();
        store
            .prune_sources(std::time::Duration::ZERO, &[revision])
            .unwrap();
        assert!(snapshot.exists());
    }
}
