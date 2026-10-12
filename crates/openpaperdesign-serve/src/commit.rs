//! Writing several project files as one operation: either all of them change, or (after an
//! error) every one is byte-identical to before.
//!
//! 1. The current bytes of every target are read (the pre-images; a missing file is "none").
//! 2. Every new content is written to a temporary file next to its target and synced.
//! 3. The temporary files are renamed over their targets, in the order given.
//!
//! If anything fails, the temporary files are removed and each target already replaced is put
//! back: its pre-image is written the same way (temporary file, sync, rename), or the file is
//! removed if it didn't exist. If putting a file back fails too, the project can no longer be
//! known to be consistent: the error is `Broken`, naming the files, and the caller must refuse
//! every further write (see `Broken` in lib.rs).
//!
//! Temporary files are named like upstream's (`.<name>.<pid>-<random>.tmp`), so a crash
//! leaves files that upstream `Project::open` already cleans up.

use std::io::Write as _;
use std::path::{Path, PathBuf};

pub(crate) struct Write {
    pub path: PathBuf,
    pub data: Vec<u8>,
}

#[derive(Debug)]
pub(crate) enum CommitError {
    /// Nothing changed: every target is as it was before the call.
    Failed(String),
    /// A failed write could not be undone; the files named may be inconsistent.
    Broken(String),
}

impl CommitError {
    pub fn message(&self) -> &str {
        match self {
            CommitError::Failed(m) | CommitError::Broken(m) => m,
        }
    }
}

fn temp_path(path: &Path) -> PathBuf {
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| "file".into());
    use std::hash::{BuildHasher, RandomState};
    let n = RandomState::new().hash_one(std::time::SystemTime::now()) as u32;
    path.with_file_name(format!(".{name}.{}-{n:08x}.tmp", std::process::id()))
}

/// Write `data` to a new temporary file next to `path` and sync it.
fn stage(path: &Path, data: &[u8]) -> std::io::Result<PathBuf> {
    fault::check()?;
    let tmp = temp_path(path);
    let r = (|| {
        let mut f = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&tmp)?;
        f.write_all(data)?;
        f.sync_all()
    })();
    match r {
        Ok(()) => Ok(tmp),
        Err(e) => {
            let _ = std::fs::remove_file(&tmp);
            Err(e)
        }
    }
}

fn replace(tmp: &Path, path: &Path) -> std::io::Result<()> {
    fault::check()?;
    std::fs::rename(tmp, path)
}

fn sync_parent(path: &Path) {
    if let Some(d) = path.parent()
        && let Ok(f) = std::fs::File::open(d)
    {
        let _ = f.sync_all();
    }
}

/// Put `path` back to `pre` (its bytes before, or absent).
fn restore(path: &Path, pre: &Option<Vec<u8>>) -> std::io::Result<()> {
    match pre {
        Some(bytes) => {
            let tmp = stage(path, bytes)?;
            replace(&tmp, path).inspect_err(|_| {
                let _ = std::fs::remove_file(&tmp);
            })
        }
        None => {
            fault::check()?;
            std::fs::remove_file(path)
        }
    }
}

fn label(path: &Path) -> String {
    path.file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default()
}

/// Write every file in `writes` (in that order), or none of them.
pub(crate) fn commit(writes: &[Write]) -> Result<(), CommitError> {
    let mut pre = Vec::with_capacity(writes.len());
    for w in writes {
        pre.push(match std::fs::read(&w.path) {
            Ok(b) => Some(b),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
            Err(e) => return Err(CommitError::Failed(format!("{}: {e}", label(&w.path)))),
        });
    }
    let mut temps: Vec<PathBuf> = Vec::with_capacity(writes.len());
    for w in writes {
        match stage(&w.path, &w.data) {
            Ok(t) => temps.push(t),
            Err(e) => {
                for t in &temps {
                    let _ = std::fs::remove_file(t);
                }
                return Err(CommitError::Failed(format!("{}: {e}", label(&w.path))));
            }
        }
    }
    for (i, w) in writes.iter().enumerate() {
        if let Err(e) = replace(&temps[i], &w.path) {
            for t in &temps[i..] {
                let _ = std::fs::remove_file(t);
            }
            let mut unrestored = Vec::new();
            for j in (0..i).rev() {
                if let Err(re) = restore(&writes[j].path, &pre[j]) {
                    unrestored.push(format!("{} ({re})", label(&writes[j].path)));
                }
            }
            let what = format!("{}: {e}", label(&w.path));
            if unrestored.is_empty() {
                return Err(CommitError::Failed(what));
            }
            return Err(CommitError::Broken(format!(
                "{what}; restoring the files already written failed too: {}",
                unrestored.join(", ")
            )));
        }
    }
    for w in writes {
        sync_parent(&w.path);
    }
    Ok(())
}

/// Test-only failure injection: fail the Nth fallible step (staging a file, a rename, a
/// restore) on this thread. Compiled out of non-test builds.
pub(crate) mod fault {
    #[cfg(test)]
    thread_local! {
        static STATE: std::cell::RefCell<(usize, Vec<usize>)> =
            const { std::cell::RefCell::new((0, Vec::new())) };
    }

    /// Fail the steps numbered in `at` (counting from 0) from now on.
    #[cfg(test)]
    pub fn fail_at(at: &[usize]) {
        STATE.with(|s| *s.borrow_mut() = (0, at.to_vec()));
    }

    #[cfg(test)]
    pub fn check() -> std::io::Result<()> {
        STATE.with(|s| {
            let mut s = s.borrow_mut();
            let n = s.0;
            s.0 += 1;
            if s.1.contains(&n) {
                Err(std::io::Error::other(format!(
                    "injected failure at step {n}"
                )))
            } else {
                Ok(())
            }
        })
    }

    #[cfg(not(test))]
    #[inline(always)]
    pub fn check() -> std::io::Result<()> {
        Ok(())
    }
}
