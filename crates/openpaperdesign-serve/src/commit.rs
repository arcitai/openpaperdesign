//! Writing several project files as one operation: either all of them change, or (after an
//! error) every one is byte-identical to before.
//!
//! 1. The current bytes of every target are read (the pre-images; a missing file is "none").
//! 2. Every new content is written to a temporary file next to its target and synced.
//! 3. The temporary files are renamed over their targets, in the order given.
//! 4. Each directory containing a target is synced, so the renames are durable.
//!
//! If anything fails (a write, a file or directory sync, a rename), the temporary files are
//! removed and each target already replaced is put back: its pre-image is written the same way
//! (temporary file, sync, rename), or the file is removed if it didn't exist, and the
//! directories are synced again. If any of that fails too, the project can no longer be known
//! to be consistent: the error is `Broken`, listing the files, and the caller must refuse every
//! further write (see `Broken` in lib.rs).
//!
//! This covers errors the process sees. It is not crash-atomic: there is no journal, the
//! pre-images live only in memory, so a crash between two renames leaves some targets new and
//! some old (each file itself complete). Temporary files are named like upstream's
//! (`.<name>.<pid>-<random>.tmp`), so the ones a crash leaves are cleaned by upstream
//! `Project::open`.

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
    /// A failed write could not be undone: `files` may now be inconsistent (some new, some
    /// old, or not durably restored).
    Broken { error: String, files: Vec<String> },
}

/// Errors before anything is written.
impl From<String> for CommitError {
    fn from(e: String) -> CommitError {
        CommitError::Failed(e)
    }
}

impl CommitError {
    /// For the person or agent: what happened, and whether anything may have changed.
    pub fn describe(&self) -> String {
        match self {
            CommitError::Failed(e) => format!("not saved; nothing was written ({e})"),
            CommitError::Broken { error, files } => format!(
                "not saved, and undoing the partial save failed ({error}): these files may now \
                 be inconsistent: {}. Writes to this project are disabled until those files \
                 are checked or restored and the server is restarted",
                files.join(", ")
            ),
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

/// Sync every distinct directory holding one of `paths`.
fn sync_dirs<'a>(paths: impl Iterator<Item = &'a Path>) -> std::io::Result<()> {
    let mut done: Vec<&Path> = Vec::new();
    for p in paths {
        let d = p.parent().unwrap_or(Path::new("."));
        if done.contains(&d) {
            continue;
        }
        done.push(d);
        fault::check()?;
        std::fs::File::open(d)?.sync_all()?;
    }
    Ok(())
}

/// Put the first `n` targets back (latest first) and sync their directories. Returns the
/// files that couldn't be put back durably.
fn roll_back(writes: &[Write], pre: &[Option<Vec<u8>>], n: usize) -> Vec<String> {
    let mut failed = Vec::new();
    for j in (0..n).rev() {
        if let Err(e) = restore(&writes[j].path, &pre[j]) {
            failed.push(format!("{} ({e})", label(&writes[j].path)));
        }
    }
    if let Err(e) = sync_dirs(writes[..n].iter().map(|w| w.path.as_path())) {
        // Every restored file's directory entry may not be durable.
        for w in &writes[..n] {
            let l = label(&w.path);
            if !failed.iter().any(|f| f.starts_with(&l)) {
                failed.push(format!("{l} (directory sync: {e})"));
            }
        }
    }
    failed
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
    let outcome = |what: String, n: usize| {
        let failed = roll_back(writes, &pre, n);
        if failed.is_empty() {
            CommitError::Failed(what)
        } else {
            CommitError::Broken {
                error: what,
                files: failed,
            }
        }
    };
    for (i, w) in writes.iter().enumerate() {
        if let Err(e) = replace(&temps[i], &w.path) {
            for t in &temps[i..] {
                let _ = std::fs::remove_file(t);
            }
            return Err(outcome(format!("{}: {e}", label(&w.path)), i));
        }
    }
    // The renames are only durable once their directories are synced.
    if let Err(e) = sync_dirs(writes.iter().map(|w| w.path.as_path())) {
        return Err(outcome(format!("directory sync: {e}"), writes.len()));
    }
    Ok(())
}

/// Test-only failure injection: fail the Nth fallible step (staging a file, a rename, a
/// directory sync, a restore) on this thread. Compiled out of non-test builds.
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
