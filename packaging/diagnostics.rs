//! Local, private, bounded diagnostics for desktop launches without a console.
use std::io::{self, Seek, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

const LOG_LIMIT: usize = 1024 * 1024;
const ERROR_LIMIT: usize = 64 * 1024;

struct BoundedLog {
    file: std::fs::File,
    remaining: usize,
    truncated: bool,
}
impl Write for BoundedLog {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        let count = bytes.len().min(self.remaining);
        self.file.write_all(&bytes[..count])?;
        self.remaining -= count;
        if count < bytes.len() && !self.truncated {
            self.file.write_all(
                b"\n[Diagnostic log limit reached; latest error is in the adjacent .error file.]\n",
            )?;
            self.truncated = true;
        }
        Ok(bytes.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        self.file.flush()
    }
}
struct SharedLog(Arc<Mutex<BoundedLog>>);
impl Write for SharedLog {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.0
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .write(bytes)
    }
    fn flush(&mut self) -> io::Result<()> {
        self.0.lock().unwrap_or_else(|p| p.into_inner()).flush()
    }
}

/// Replaces only this launch's owned error handle, never a path opened by a later write.
fn last_error(file: &mut std::fs::File, message: &[u8]) -> io::Result<()> {
    file.rewind()?;
    file.write_all(&message[..message.len().min(ERROR_LIMIT)])?;
    file.set_len(message.len().min(ERROR_LIMIT) as u64)?;
    file.flush()
}

fn create_log(directory: &Path) -> io::Result<(PathBuf, std::fs::File)> {
    let mut builder = std::fs::DirBuilder::new();
    builder.recursive(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        builder.mode(0o700);
    }
    builder.create(directory)?;
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let path = directory.join(format!("app-{now}-{}.log", std::process::id()));
    Ok((path.clone(), private_file(&path)?))
}
fn private_file(path: &Path) -> io::Result<std::fs::File> {
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    options.open(path)
}

pub fn start(gui: bool) -> Option<PathBuf> {
    let filter = if gui {
        "warn,paper_rs=info,openpaperdesign=info"
    } else {
        "warn"
    };
    let mut logger =
        env_logger::Builder::from_env(env_logger::Env::default().default_filter_or(filter));
    let mut path = None;
    let mut error_sink = None;
    if gui {
        let files = dirs::data_local_dir()
            .ok_or_else(|| {
                io::Error::new(io::ErrorKind::NotFound, "No local application-data folder")
            })
            .and_then(|root| create_log(&root.join("OpenPaperDesign/logs")));
        match files {
            Ok((name, file)) => {
                let error = private_file(&name.with_extension("error"));
                match error {
                    Ok(file) => error_sink = Some(Arc::new(Mutex::new(file))),
                    Err(error) => eprintln!("Cannot create latest-error log: {error}"),
                }
                logger.target(env_logger::Target::Pipe(Box::new(SharedLog(Arc::new(
                    Mutex::new(BoundedLog {
                        file,
                        remaining: LOG_LIMIT,
                        truncated: false,
                    }),
                )))));
                path = Some(name);
            }
            Err(error) => eprintln!("Cannot create a local diagnostic log: {error}"),
        }
    }
    if let Some(sink) = error_sink.clone() {
        logger.format(move |buffer, record| {
            let message = format!(
                "{} {}: {}\n",
                buffer.timestamp_millis(),
                record.level(),
                record.args()
            );
            if record.level() == log::Level::Error {
                let _ = last_error(
                    &mut sink.lock().unwrap_or_else(|p| p.into_inner()),
                    message.as_bytes(),
                );
            }
            buffer.write_all(message.as_bytes())
        });
    }
    if logger.try_init().is_err() {
        return None;
    }
    if let Some(sink) = error_sink {
        let previous = std::panic::take_hook();
        std::panic::set_hook(Box::new(move |info| {
            // Independent from the general logger, so its quota or lock cannot hide a panic.
            let message = format!(
                "Unrecoverable application error: {info}\n{}",
                std::backtrace::Backtrace::force_capture()
            );
            if let Ok(mut file) = sink.try_lock() {
                let _ = last_error(&mut file, message.as_bytes());
            }
            previous(info);
        }));
    }
    log::info!(
        "OpenPaperDesign {} starting on {}/{} (process {})",
        env!("CARGO_PKG_VERSION"),
        std::env::consts::OS,
        std::env::consts::ARCH,
        std::process::id()
    );
    path
}

#[cfg(test)]
mod tests {
    use super::*;
    struct Scratch(PathBuf);
    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
    fn scratch() -> Scratch {
        let root =
            std::env::temp_dir().join(format!("opd-log-test-{}", paper_rs::project::new_id()));
        std::fs::create_dir(&root).unwrap();
        Scratch(root)
    }
    #[test]
    fn exhaustion_keeps_latest_error_and_reports_full_writes() {
        let root = scratch();
        let (path, file) = create_log(&root.0).unwrap();
        let error_path = path.with_extension("error");
        let mut error = private_file(&error_path).unwrap();
        let mut log = BoundedLog {
            file,
            remaining: 4,
            truncated: false,
        };
        assert_eq!(log.write(b"first").unwrap(), 5);
        assert_eq!(log.write(b"more").unwrap(), 4);
        last_error(&mut error, b"first fatal error").unwrap();
        last_error(&mut error, b"last error").unwrap();
        log.flush().unwrap();
        let text = std::fs::read_to_string(path).unwrap();
        assert!(text.starts_with("firs"));
        assert_eq!(text.matches("limit reached").count(), 1);
        assert_eq!(std::fs::read(error_path).unwrap(), b"last error");
    }
    #[test]
    fn creates_private_new_files_without_replacing_existing_files() {
        let root = scratch();
        let unrelated = root.0.join("existing.log");
        std::fs::write(&unrelated, b"keep").unwrap();
        assert!(private_file(&unrelated).is_err());
        let (path, _) = create_log(&root.0).unwrap();
        assert_eq!(std::fs::read(unrelated).unwrap(), b"keep");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                std::fs::metadata(path).unwrap().permissions().mode() & 0o777,
                0o600
            );
        }
    }
}
