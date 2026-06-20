use std::collections::HashSet;
use std::io::Read;
use std::path::{Path, PathBuf};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum FsError {
    #[error("IO error for '{path}': {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
}

impl FsError {
    fn io(path: impl Into<PathBuf>, source: std::io::Error) -> Self {
        Self::Io {
            path: path.into(),
            source,
        }
    }
}

/// Move a file from `src` to `dst`, creating destination parent directories as needed.
/// Falls back to copy-then-delete when `src` and `dst` are on different filesystems.
pub fn move_file(src: &Path, dst: &Path) -> Result<(), FsError> {
    if let Some(parent) = dst.parent() {
        std::fs::create_dir_all(parent).map_err(|e| FsError::io(parent, e))?;
    }
    match std::fs::rename(src, dst) {
        Ok(()) => Ok(()),
        Err(e) if is_cross_device(&e) => {
            std::fs::copy(src, dst).map_err(|e| FsError::io(dst, e))?;
            std::fs::remove_file(src).map_err(|e| FsError::io(src, e))?;
            Ok(())
        }
        Err(e) => Err(FsError::io(src, e)),
    }
}

/// Return true when the two files have identical content.
/// Compares sizes first, then reads both in 64 KiB chunks.
pub fn files_identical(a: &Path, b: &Path) -> Result<bool, FsError> {
    let meta_a = std::fs::metadata(a).map_err(|e| FsError::io(a, e))?;
    let meta_b = std::fs::metadata(b).map_err(|e| FsError::io(b, e))?;
    if meta_a.len() != meta_b.len() {
        return Ok(false);
    }
    // Same size: compare byte-by-byte in chunks.
    let mut fa = std::fs::File::open(a).map_err(|e| FsError::io(a, e))?;
    let mut fb = std::fs::File::open(b).map_err(|e| FsError::io(b, e))?;
    const CHUNK: usize = 64 * 1024;
    let mut buf_a = vec![0u8; CHUNK];
    let mut buf_b = vec![0u8; CHUNK];
    loop {
        let n_a = fa.read(&mut buf_a).map_err(|e| FsError::io(a, e))?;
        let n_b = fb.read(&mut buf_b).map_err(|e| FsError::io(b, e))?;
        if n_a != n_b || buf_a[..n_a] != buf_b[..n_b] {
            return Ok(false);
        }
        if n_a == 0 {
            return Ok(true);
        }
    }
}

/// Starting from `desired`, find a path that does not exist on disk and is not already
/// in `reserved`. Appends ` (2)`, ` (3)`, etc. to the stem until a free slot is found.
pub fn find_free_path(desired: &Path, reserved: &HashSet<PathBuf>) -> PathBuf {
    if !desired.exists() && !reserved.contains(desired) {
        return desired.to_path_buf();
    }
    let stem = desired
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("file");
    let ext = desired.extension().and_then(|s| s.to_str());
    let parent = desired.parent().unwrap_or(Path::new("."));

    for n in 2u64.. {
        let name = match ext {
            Some(e) => format!("{stem} ({n}).{e}"),
            None => format!("{stem} ({n})"),
        };
        let candidate = parent.join(&name);
        if !candidate.exists() && !reserved.contains(&candidate) {
            return candidate;
        }
    }
    unreachable!("exhausted candidate filenames")
}

/// Sanitise a name person name so it is safe to use as a directory name on all
/// platforms. Replaces characters that are forbidden or problematic on Windows, macOS,
/// and Linux with an underscore.
pub fn sanitise_dir_name(name: &str) -> String {
    name.chars()
        .map(|c| match c {
            '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|' => '_',
            c if c.is_control() => '_',
            c => c,
        })
        .collect()
}

fn is_cross_device(err: &std::io::Error) -> bool {
    // ErrorKind::CrossesDevices was stabilised in Rust 1.75 and covers EXDEV on
    // POSIX and ERROR_NOT_SAME_DEVICE on Windows.
    err.kind() == std::io::ErrorKind::CrossesDevices
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;
    use tempfile::tempdir;

    #[test]
    fn identical_files_detected() {
        let dir = tempdir().unwrap();
        let a = dir.path().join("a.jpg");
        let b = dir.path().join("b.jpg");
        std::fs::write(&a, b"hello world").unwrap();
        std::fs::write(&b, b"hello world").unwrap();
        assert!(files_identical(&a, &b).unwrap());
    }

    #[test]
    fn different_files_not_identical() {
        let dir = tempdir().unwrap();
        let a = dir.path().join("a.jpg");
        let b = dir.path().join("b.jpg");
        std::fs::write(&a, b"hello").unwrap();
        std::fs::write(&b, b"world").unwrap();
        assert!(!files_identical(&a, &b).unwrap());
    }

    #[test]
    fn find_free_path_no_clash() {
        let dir = tempdir().unwrap();
        let desired = dir.path().join("img.jpg");
        let reserved = HashSet::new();
        assert_eq!(find_free_path(&desired, &reserved), desired);
    }

    #[test]
    fn find_free_path_existing_file() {
        let dir = tempdir().unwrap();
        let desired = dir.path().join("img.jpg");
        std::fs::write(&desired, b"x").unwrap();
        let reserved = HashSet::new();
        let free = find_free_path(&desired, &reserved);
        assert_eq!(free, dir.path().join("img (2).jpg"));
    }

    #[test]
    fn find_free_path_reserved() {
        let dir = tempdir().unwrap();
        let desired = dir.path().join("img.jpg");
        let mut reserved = HashSet::new();
        reserved.insert(desired.clone());
        let free = find_free_path(&desired, &reserved);
        assert_eq!(free, dir.path().join("img (2).jpg"));
    }

    #[test]
    fn move_file_same_device() {
        let dir = tempdir().unwrap();
        let src = dir.path().join("src.jpg");
        let dst = dir.path().join("sub").join("dst.jpg");
        std::fs::write(&src, b"data").unwrap();
        move_file(&src, &dst).unwrap();
        assert!(!src.exists());
        assert_eq!(std::fs::read(&dst).unwrap(), b"data");
    }

    #[test]
    fn sanitise_dir_name_removes_forbidden() {
        assert_eq!(sanitise_dir_name("Joe/Bloggs:Test"), "Joe_Bloggs_Test");
        assert_eq!(sanitise_dir_name("Normal Name"), "Normal Name");
    }
}
