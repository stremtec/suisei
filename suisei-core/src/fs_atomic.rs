//! Crash-safe full-file writes (tmp + fsync + rename).

use std::fs::{self, File};
use std::io::Write;
use std::path::Path;

/// Write `data` to `path` atomically: sibling temp → write → fsync → rename.
/// Never uses in-place `fs::write` on the original path (that can truncate mid-crash).
pub fn atomic_write_file(path: &Path, data: impl AsRef<[u8]>) -> std::io::Result<()> {
    let data = data.as_ref();
    let parent = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    let name = path
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("buffer");
    let tmp = parent.join(format!(
        ".{}.suisei-tmp-{}-{}",
        name,
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0)
    ));

    let write_result = (|| {
        let mut f = File::create(&tmp)?;
        f.write_all(data)?;
        f.sync_all()?;
        Ok::<(), std::io::Error>(())
    })();

    if let Err(e) = write_result {
        let _ = fs::remove_file(&tmp);
        return Err(e);
    }

    match fs::rename(&tmp, path) {
        Ok(()) => Ok(()),
        Err(e) => {
            // Windows may refuse rename-over-existing; fall back to remove+rename.
            let _ = fs::remove_file(path);
            match fs::rename(&tmp, path) {
                Ok(()) => Ok(()),
                Err(e2) => {
                    let _ = fs::remove_file(&tmp);
                    Err(e2.raw_os_error().map(|_| e2).unwrap_or(e))
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn atomic_write_replaces_and_matches_bytes() {
        let dir = std::env::temp_dir().join(format!(
            "suisei-fs-atomic-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0)
        ));
        let _ = fs::create_dir_all(&dir);
        let path = dir.join("doc.txt");
        fs::write(&path, "OLD").unwrap();
        atomic_write_file(&path, "NEW FULL CONTENT").unwrap();
        assert_eq!(fs::read_to_string(&path).unwrap(), "NEW FULL CONTENT");
        let leftovers: Vec<_> = fs::read_dir(&dir)
            .unwrap()
            .filter_map(|e| e.ok())
            .filter(|e| e.file_name().to_string_lossy().contains("suisei-tmp"))
            .collect();
        assert!(leftovers.is_empty());
        let _ = fs::remove_dir_all(&dir);
    }
}
