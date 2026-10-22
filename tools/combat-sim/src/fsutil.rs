//! Atomic file writes shared by the tools.
//!
//! Every save-file store goes through [`atomic_write`]: write a temporary file
//! in the same directory, flush and sync it, then rename it over the target.
//! A crash or a full disk therefore leaves the previous file intact instead of
//! a truncated YAML/JSON document. The temporary file is removed on failure.

use anyhow::{Context, Result};
use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

/// Write `bytes` to `path` atomically (same-directory temporary file + rename).
pub fn atomic_write(path: impl AsRef<Path>, bytes: impl AsRef<[u8]>) -> Result<()> {
    let path = path.as_ref();
    let parent = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    fs::create_dir_all(parent).with_context(|| format!("create {}", parent.display()))?;
    let tmp = temp_path(path);
    let written = (|| -> Result<()> {
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&tmp)
            .with_context(|| format!("create {}", tmp.display()))?;
        file.write_all(bytes.as_ref())
            .with_context(|| format!("write {}", tmp.display()))?;
        file.flush()?;
        file.sync_all()?;
        Ok(())
    })();
    if let Err(err) = written {
        let _ = fs::remove_file(&tmp);
        return Err(err);
    }
    if let Err(err) = fs::rename(&tmp, path)
        .with_context(|| format!("rename {} -> {}", tmp.display(), path.display()))
    {
        let _ = fs::remove_file(&tmp);
        return Err(err);
    }
    sync_parent(parent);
    Ok(())
}

/// Unique temporary sibling name (pid + nanoseconds) so concurrent writers do
/// not share a temp file.
fn temp_path(path: &Path) -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let mut name = path
        .file_name()
        .map(|n| n.to_os_string())
        .unwrap_or_default();
    name.push(format!(".tmp-{}-{nanos}", std::process::id()));
    path.with_file_name(name)
}

/// Best-effort directory sync so the rename itself is durable on POSIX.
fn sync_parent(dir: &Path) {
    #[cfg(unix)]
    {
        if let Ok(handle) = File::open(dir) {
            let _ = handle.sync_all();
        }
    }
    #[cfg(not(unix))]
    {
        let _ = dir;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dir(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("fsutil-{tag}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&d);
        d
    }

    #[test]
    fn writes_creates_parents_and_leaves_no_temp_file() {
        let d = dir("write");
        let p = d.join("a/b.yaml");
        atomic_write(&p, b"x: 1\n").unwrap();
        assert_eq!(fs::read_to_string(&p).unwrap(), "x: 1\n");
        let names: Vec<String> = fs::read_dir(d.join("a"))
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().to_string())
            .collect();
        assert_eq!(names, vec!["b.yaml".to_string()]);
        let _ = fs::remove_dir_all(&d);
    }

    #[test]
    fn overwrite_replaces_content() {
        let d = dir("overwrite");
        let p = d.join("x.json");
        atomic_write(&p, b"old").unwrap();
        atomic_write(&p, b"new").unwrap();
        assert_eq!(fs::read_to_string(&p).unwrap(), "new");
        let _ = fs::remove_dir_all(&d);
    }
}
