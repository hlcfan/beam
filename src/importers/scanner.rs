use std::fs;
use std::path::{Path, PathBuf};

use crate::error::BeamError;

/// Maximum depth `scan_folder` will descend into. Once a directory lives at
/// this depth it is listed but its subdirectories are not entered.
pub const MAX_SCAN_DEPTH: u32 = 10;

/// Hard upper bound on the number of files a single scan may return. Exceeding
/// this aborts the scan with `ScanError::TooManyFiles`.
pub const MAX_FILES_PER_SCAN: usize = 5_000;

/// Directory names that are always skipped during a scan, regardless of depth.
pub const SKIP_DIRS: &[&str] = &[".git", ".svn", ".hg", "node_modules"];

/// Errors emitted by `scan_folder`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ScanError {
    /// The provided root path does not exist or is not a directory.
    NotADirectory { path: PathBuf },
    /// The scan exceeded `MAX_FILES_PER_SCAN` files. The partial list is
    /// discarded — the caller is expected to surface a red banner to the user.
    TooManyFiles { limit: usize },
    /// A `std::fs` operation failed while walking the tree. The path is the
    /// entry that triggered the error.
    Io { path: PathBuf, message: String },
}

impl From<ScanError> for BeamError {
    fn from(err: ScanError) -> Self {
        match err {
            ScanError::NotADirectory { path } => BeamError::Validation {
                message: format!("not a directory: {}", path.display()),
            },
            ScanError::TooManyFiles { limit } => BeamError::Validation {
                message: format!("scan exceeded {limit} files"),
            },
            ScanError::Io { path, message } => BeamError::Validation {
                message: format!("scan io error at {}: {message}", path.display()),
            },
        }
    }
}

/// Recursively walk `root` depth-first, returning every regular file in a
/// stable order (sorted within each directory by filename, case-sensitive).
///
/// - Directories whose name starts with `.` or is in `SKIP_DIRS` are skipped.
/// - Descending stops once `depth == MAX_SCAN_DEPTH`; a `log::warn!` is
///   emitted and the walk continues with files at shallower depths.
/// - Symlinks and other special files are not returned.
/// - If the running file total exceeds `MAX_FILES_PER_SCAN`, the scan aborts
///   with `ScanError::TooManyFiles`.
pub fn scan_folder(root: &Path) -> Result<Vec<PathBuf>, ScanError> {
    if !root.is_dir() {
        return Err(ScanError::NotADirectory {
            path: root.to_path_buf(),
        });
    }

    let mut out: Vec<PathBuf> = Vec::new();
    walk(root, 0, &mut out)?;
    Ok(out)
}

fn walk(dir: &Path, depth: u32, out: &mut Vec<PathBuf>) -> Result<(), ScanError> {
    let entries = match fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(e) => {
            return Err(ScanError::Io {
                path: dir.to_path_buf(),
                message: e.to_string(),
            });
        }
    };

    let mut paths: Vec<PathBuf> = entries
        .filter_map(|e| match e {
            Ok(entry) => Some(entry.path()),
            Err(e) => {
                log::warn!("scan: readdir entry error at {}: {e}", dir.display());
                None
            }
        })
        .collect();
    paths.sort_by(|a, b| {
        a.file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("")
            .cmp(b.file_name().and_then(|n| n.to_str()).unwrap_or(""))
    });

    for path in paths {
        let file_name = path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("")
            .to_string();

        let meta = match fs::symlink_metadata(&path) {
            Ok(m) => m,
            Err(e) => {
                return Err(ScanError::Io {
                    path: path.clone(),
                    message: e.to_string(),
                });
            }
        };

        let file_type = meta.file_type();

        if file_type.is_symlink() {
            // Skip symlinks entirely — never follow them.
            continue;
        }

        if file_type.is_dir() {
            if should_skip_dir(&file_name) {
                continue;
            }
            if depth >= MAX_SCAN_DEPTH {
                log::warn!(
                    "scan: max depth {} reached at '{}' — not descending further",
                    MAX_SCAN_DEPTH,
                    path.display()
                );
                continue;
            }
            walk(&path, depth + 1, out)?;
            continue;
        }

        if file_type.is_file() {
            if out.len() >= MAX_FILES_PER_SCAN {
                return Err(ScanError::TooManyFiles {
                    limit: MAX_FILES_PER_SCAN,
                });
            }
            out.push(path);
            continue;
        }

        // Devices, sockets, fifos, etc. — skip silently.
    }

    Ok(())
}

fn should_skip_dir(name: &str) -> bool {
    if name.starts_with('.') {
        return true;
    }
    SKIP_DIRS.contains(&name)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::io::Write;
    use tempfile::TempDir;

    fn touch(path: &Path, contents: &str) {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        let mut f = fs::File::create(path).unwrap();
        f.write_all(contents.as_bytes()).unwrap();
    }

    fn relative(root: &Path, paths: &[PathBuf]) -> Vec<String> {
        paths
            .iter()
            .map(|p| p.strip_prefix(root).unwrap().to_string_lossy().to_string())
            .collect()
    }

    #[test]
    fn scans_mixed_known_and_unknown_files_in_nested_folders() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path();
        touch(&root.join("a.json"), "{}");
        touch(&root.join("sub/b.txt"), "b");
        touch(&root.join("sub/c.json"), "{}");
        touch(&root.join("sub/deeper/d.json"), "{}");

        let scanned = scan_folder(root).unwrap();
        let names = relative(root, &scanned);
        assert_eq!(
            names,
            vec!["a.json", "sub/b.txt", "sub/c.json", "sub/deeper/d.json"]
        );
    }

    #[test]
    fn skips_dot_dirs_and_skip_dirs_list() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path();
        touch(&root.join("keep.json"), "{}");
        // dot folder
        touch(&root.join(".git/config"), "x");
        // dot folder that is not in SKIP_DIRS
        touch(&root.join(".hidden/inside.json"), "{}");
        // skip-list folders
        touch(&root.join("node_modules/lib/index.js"), "x");
        touch(&root.join(".svn/wc.db"), "x");
        touch(&root.join(".hg/repo"), "x");

        let scanned = scan_folder(root).unwrap();
        let names = relative(root, &scanned);
        assert_eq!(names, vec!["keep.json"]);
    }

    #[test]
    fn respects_max_scan_depth() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path();
        // Build a tree deeper than MAX_SCAN_DEPTH.
        let mut depth = 0u32;
        let mut current = root.to_path_buf();
        loop {
            touch(&current.join(format!("file_at_{depth}.json")), "{}");
            if depth > MAX_SCAN_DEPTH + 1 {
                break;
            }
            current = current.join(format!("d{depth}"));
            depth += 1;
        }

        let scanned = scan_folder(root).unwrap();
        // All files up to and including depth MAX_SCAN_DEPTH should be present.
        // Files deeper than MAX_SCAN_DEPTH should be absent (we stopped
        // descending).
        for d in 0..=MAX_SCAN_DEPTH {
            assert!(
                scanned
                    .iter()
                    .any(|p| p.ends_with(format!("file_at_{d}.json"))),
                "expected file at depth {d} to be scanned"
            );
        }
        assert!(scanned
            .iter()
            .all(|p| !p.ends_with(format!("file_at_{}.json", MAX_SCAN_DEPTH + 1))));
    }

    #[test]
    fn returns_too_many_files_when_exceeding_limit() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path();
        // Create more than MAX_FILES_PER_SCAN files in a flat structure.
        for i in 0..=(MAX_FILES_PER_SCAN + 1) {
            touch(&root.join(format!("f{i:05}.json")), "{}");
        }
        let err = scan_folder(root).unwrap_err();
        assert_eq!(err, ScanError::TooManyFiles { limit: MAX_FILES_PER_SCAN });
    }

    #[test]
    fn returns_not_a_directory_for_missing_root() {
        let tmp = TempDir::new().unwrap();
        let missing = tmp.path().join("does-not-exist");
        let err = scan_folder(&missing).unwrap_err();
        assert!(matches!(err, ScanError::NotADirectory { .. }));
    }

    #[test]
    fn skips_symlinks() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path();
        touch(&root.join("real.json"), "{}");
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink(
                root.join("real.json"),
                root.join("link.json"),
            )
            .unwrap();
        }
        let scanned = scan_folder(root).unwrap();
        let names = relative(root, &scanned);
        assert_eq!(names, vec!["real.json"]);
    }

    #[test]
    fn stable_sorted_order_within_directory() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path();
        // Create in non-sorted order.
        for name in ["zeta.json", "alpha.json", "middle.json"] {
            touch(&root.join(name), "{}");
        }
        let scanned = scan_folder(root).unwrap();
        let names = relative(root, &scanned);
        assert_eq!(names, vec!["alpha.json", "middle.json", "zeta.json"]);
    }
}