use std::fs::{self, File, OpenOptions};
use std::io::{self, BufWriter, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

static NONCE: AtomicU64 = AtomicU64::new(0);

struct TemporaryFile(PathBuf);

impl Drop for TemporaryFile {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.0);
    }
}

pub fn write(path: &Path, bytes: &[u8]) -> io::Result<()> {
    replace_with(path, |file| file.write_all(bytes))
}

/// Write and flush a sibling file before replacing the destination. A failed
/// write or replacement leaves the existing destination intact.
pub fn replace_with(
    path: &Path,
    write: impl FnOnce(&mut BufWriter<File>) -> io::Result<()>,
) -> io::Result<()> {
    replace_with_guard(path, write, || Ok(()))
}

/// The guard runs after the replacement has been flushed, immediately before
/// rename. Callers use it to reject a changed destination instead of replacing
/// somebody else's edits. The destination remains untouched on guard failure.
pub fn replace_with_guard(
    path: &Path,
    write: impl FnOnce(&mut BufWriter<File>) -> io::Result<()>,
    before_replace: impl FnOnce() -> io::Result<()>,
) -> io::Result<()> {
    let destination = if path.is_symlink() {
        fs::canonicalize(path)?
    } else {
        path.to_path_buf()
    };
    let permissions = match fs::metadata(&destination) {
        Ok(metadata) => {
            if metadata.permissions().readonly() {
                return Err(io::Error::new(
                    io::ErrorKind::PermissionDenied,
                    "destination is read-only",
                ));
            }
            Some(metadata.permissions())
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => None,
        Err(error) => return Err(error),
    };
    let parent = destination
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    fs::create_dir_all(parent)?;
    let (temporary, file) = loop {
        let id = NONCE.fetch_add(1, Ordering::Relaxed);
        let candidate = parent.join(format!(".lawpdf-{}-{id}.tmp", std::process::id()));
        let mut options = OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        match options.open(&candidate) {
            Ok(file) => break (TemporaryFile(candidate), file),
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(error),
        }
    };
    let mut writer = BufWriter::new(file);
    write(&mut writer)?;
    writer.flush()?;
    if let Some(permissions) = permissions {
        writer.get_ref().set_permissions(permissions)?;
    }
    writer.get_ref().sync_all()?;
    drop(writer);
    before_replace()?;
    fs::rename(&temporary.0, &destination)?;
    sync_directory(parent)
}

/// Persist directory-entry changes as well as file contents on Unix. Windows
/// does not expose directory fsync through std; file data is flushed above.
pub fn sync_directory(path: &Path) -> io::Result<()> {
    #[cfg(unix)]
    {
        File::open(path)?.sync_all()?;
    }
    #[cfg(not(unix))]
    let _ = path;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture() -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "lawpdf-atomic-test-{}-{}",
            std::process::id(),
            NONCE.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(&dir).unwrap();
        dir.join("saved.pdf")
    }

    #[test]
    fn interrupted_write_preserves_the_original_and_cleans_temporary_file() {
        let path = fixture();
        fs::write(&path, b"original document").unwrap();
        let result = replace_with(&path, |file| {
            file.write_all(b"incomplete replacement")?;
            Err(io::Error::other("injected write failure"))
        });
        assert!(result.is_err());
        assert_eq!(fs::read(&path).unwrap(), b"original document");
        assert_eq!(fs::read_dir(path.parent().unwrap()).unwrap().count(), 1);
        fs::remove_dir_all(path.parent().unwrap()).unwrap();
    }

    #[test]
    fn replacement_failure_preserves_destination_and_cleans_temporary_file() {
        let path = fixture();
        fs::create_dir(&path).unwrap();
        fs::write(path.join("keep.txt"), b"original").unwrap();
        assert!(write(&path, b"replacement").is_err());
        assert_eq!(fs::read(path.join("keep.txt")).unwrap(), b"original");
        assert_eq!(fs::read_dir(path.parent().unwrap()).unwrap().count(), 1);
        fs::remove_dir_all(path.parent().unwrap()).unwrap();
    }

    #[test]
    fn existing_destination_is_replaced_without_leaving_a_temporary_file() {
        let path = fixture();
        write(&path, b"old").unwrap();
        write(&path, b"new complete document").unwrap();
        assert_eq!(fs::read(&path).unwrap(), b"new complete document");
        assert_eq!(fs::read_dir(path.parent().unwrap()).unwrap().count(), 1);
        fs::remove_dir_all(path.parent().unwrap()).unwrap();
    }
}
