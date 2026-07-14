//! Disposable project copies: mutants never touch the user's working tree.
//!
//! One sandbox is created per run (not per mutant): the whole project root
//! is copied once into a temp directory, then each mutant rewrites a single
//! file and restores the original afterwards. That keeps per-mutant cost at
//! one file write instead of one tree copy.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

/// Directories that are never copied into the sandbox.
const SKIP_DIRS: &[&str] = &[
    ".git",
    ".hg",
    ".svn",
    "target",
    "node_modules",
    ".venv",
    "venv",
    "__pycache__",
    ".idea",
    ".vscode",
];

/// Refuse to copy projects larger than this many files — mutation testing
/// a tree that size sequentially is a mistake, not a use case.
const MAX_FILES: usize = 20_000;

static COUNTER: AtomicU64 = AtomicU64::new(0);

/// A temp-dir copy of the project root, removed on drop.
#[derive(Debug)]
pub struct Sandbox {
    pub dir: PathBuf,
}

impl Sandbox {
    /// Copy `root` into a fresh temp directory.
    pub fn create(root: &Path) -> io::Result<Sandbox> {
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!("mutash-{}-{}", std::process::id(), n));
        fs::create_dir_all(&dir)?;
        let mut copied = 0usize;
        if let Err(e) = copy_tree(root, &dir, &mut copied) {
            let _ = fs::remove_dir_all(&dir);
            return Err(e);
        }
        Ok(Sandbox { dir })
    }

    /// The sandbox path of a project-relative file.
    pub fn path_of(&self, rel: &Path) -> PathBuf {
        self.dir.join(rel)
    }
}

impl Drop for Sandbox {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.dir);
    }
}

fn copy_tree(from: &Path, to: &Path, copied: &mut usize) -> io::Result<()> {
    for entry in fs::read_dir(from)? {
        let entry = entry?;
        let name = entry.file_name();
        let src = entry.path();
        let meta = fs::symlink_metadata(&src)?;
        if meta.is_symlink() {
            continue; // never follow links out of the project
        }
        if meta.is_dir() {
            if SKIP_DIRS.iter().any(|d| name == std::ffi::OsStr::new(d)) {
                continue;
            }
            let dst = to.join(&name);
            fs::create_dir_all(&dst)?;
            copy_tree(&src, &dst, copied)?;
        } else {
            *copied += 1;
            if *copied > MAX_FILES {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    format!("project has more than {MAX_FILES} files; point --root at the script's own directory"),
                ));
            }
            // fs::copy preserves permission bits, so executables stay executable.
            fs::copy(&src, to.join(&name))?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_project() -> PathBuf {
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        let dir =
            std::env::temp_dir().join(format!("mutash-test-src-{}-{}", std::process::id(), n));
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn copies_files_and_nested_directories() {
        let src = temp_project();
        fs::create_dir_all(src.join("tests")).unwrap();
        fs::write(src.join("run.sh"), "echo hi\n").unwrap();
        fs::write(src.join("tests/t.sh"), "true\n").unwrap();
        let sb = Sandbox::create(&src).unwrap();
        assert_eq!(
            fs::read_to_string(sb.dir.join("run.sh")).unwrap(),
            "echo hi\n"
        );
        assert_eq!(
            fs::read_to_string(sb.dir.join("tests/t.sh")).unwrap(),
            "true\n"
        );
        assert_eq!(
            sb.path_of(Path::new("tests/t.sh")),
            sb.dir.join("tests/t.sh")
        );
        fs::remove_dir_all(&src).unwrap();
    }

    #[test]
    fn skips_vcs_and_dependency_directories() {
        let src = temp_project();
        fs::create_dir_all(src.join(".git")).unwrap();
        fs::create_dir_all(src.join("node_modules/pkg")).unwrap();
        fs::write(src.join(".git/config"), "x").unwrap();
        fs::write(src.join("node_modules/pkg/index.js"), "x").unwrap();
        fs::write(src.join("keep.sh"), "true\n").unwrap();
        let sb = Sandbox::create(&src).unwrap();
        assert!(!sb.dir.join(".git").exists());
        assert!(!sb.dir.join("node_modules").exists());
        assert!(sb.dir.join("keep.sh").exists());
        fs::remove_dir_all(&src).unwrap();
    }

    #[test]
    fn preserves_the_executable_bit() {
        use std::os::unix::fs::PermissionsExt;
        let src = temp_project();
        fs::write(src.join("tool.sh"), "#!/bin/sh\ntrue\n").unwrap();
        fs::set_permissions(src.join("tool.sh"), fs::Permissions::from_mode(0o755)).unwrap();
        let sb = Sandbox::create(&src).unwrap();
        let mode = fs::metadata(sb.dir.join("tool.sh"))
            .unwrap()
            .permissions()
            .mode();
        assert_eq!(mode & 0o111, 0o111, "executable bits lost: {mode:o}");
        fs::remove_dir_all(&src).unwrap();
    }

    #[test]
    fn sandbox_directory_is_removed_on_drop() {
        let src = temp_project();
        fs::write(src.join("a.sh"), "true\n").unwrap();
        let dir;
        {
            let sb = Sandbox::create(&src).unwrap();
            dir = sb.dir.clone();
            assert!(dir.exists());
        }
        assert!(!dir.exists(), "sandbox not cleaned up");
        fs::remove_dir_all(&src).unwrap();
    }
}
