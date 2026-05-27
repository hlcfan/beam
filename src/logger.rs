use std::fs::{self, File, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

use chrono::Local;
use log::{LevelFilter, Log, Metadata, Record};

const MAX_LOG_FILE_BYTES: u64 = 10 * 1024 * 1024;
const MAX_ROTATED_LOG_FILES: usize = 5;

#[derive(Default)]
struct LoggerState {
    file: Option<File>,
    file_path: Option<PathBuf>,
    current_size: u64,
}

struct BeamLogger {
    state: Mutex<LoggerState>,
}

impl BeamLogger {
    fn new() -> Self {
        Self {
            state: Mutex::new(LoggerState::default()),
        }
    }

    fn open_log_file(path: &Path) -> io::Result<File> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }

        OpenOptions::new().create(true).append(true).open(path)
    }

    fn set_log_file(&self, path: PathBuf) -> io::Result<()> {
        let file = Self::open_log_file(&path)?;
        let current_size = file.metadata()?.len();
        let mut state = self.state.lock().expect("logger state lock poisoned");
        state.file = Some(file);
        state.file_path = Some(path);
        state.current_size = current_size;
        Ok(())
    }

    fn rotate_logs(path: &Path) -> io::Result<()> {
        let oldest = rotated_log_path(path, MAX_ROTATED_LOG_FILES);
        if oldest.exists() {
            fs::remove_file(&oldest)?;
        }

        for index in (1..MAX_ROTATED_LOG_FILES).rev() {
            let source = rotated_log_path(path, index);
            if source.exists() {
                let target = rotated_log_path(path, index + 1);
                if target.exists() {
                    fs::remove_file(&target)?;
                }
                fs::rename(source, target)?;
            }
        }

        if path.exists() {
            let first_rotated = rotated_log_path(path, 1);
            if first_rotated.exists() {
                fs::remove_file(&first_rotated)?;
            }
            fs::rename(path, first_rotated)?;
        }

        Ok(())
    }

    fn rotate_if_needed(state: &mut LoggerState, incoming_bytes: usize) -> io::Result<()> {
        let Some(path) = state.file_path.clone() else {
            return Ok(());
        };

        if state.current_size + incoming_bytes as u64 <= MAX_LOG_FILE_BYTES {
            return Ok(());
        }

        if let Some(file) = state.file.as_mut() {
            file.flush()?;
        }
        state.file = None;

        Self::rotate_logs(&path)?;
        state.file = Some(Self::open_log_file(&path)?);
        state.current_size = 0;
        Ok(())
    }
}

impl Log for BeamLogger {
    fn enabled(&self, metadata: &Metadata) -> bool {
        metadata.level() <= log::max_level()
    }

    fn log(&self, record: &Record) {
        if !self.enabled(record.metadata()) {
            return;
        }

        let timestamp = Local::now().format("%Y-%m-%d %H:%M:%S%.3f");
        let line = format!(
            "{timestamp} [{:<5}] {}: {}",
            record.level(),
            record.target(),
            record.args()
        );

        let _ = writeln!(io::stderr(), "{line}");

        let mut state = self.state.lock().expect("logger state lock poisoned");
        if let Err(error) = Self::rotate_if_needed(&mut state, line.len() + 1) {
            let path = state
                .file_path
                .as_ref()
                .map(|path| path.display().to_string())
                .unwrap_or_else(|| "<unknown>".to_string());
            let _ = writeln!(
                io::stderr(),
                "failed to rotate Beam log file at {path}: {error}"
            );
            state.file = None;
            return;
        }
        let file_path = state.file_path.clone();
        if let Some(file) = state.file.as_mut() {
            if writeln!(file, "{line}").is_err() {
                let path = file_path
                    .as_ref()
                    .map(|path| path.display().to_string())
                    .unwrap_or_else(|| "<unknown>".to_string());
                let _ = writeln!(io::stderr(), "failed to write Beam log file at {path}");
                state.file = None;
            } else {
                state.current_size += (line.len() + 1) as u64;
            }
        }
    }

    fn flush(&self) {
        let _ = io::stderr().flush();
        let mut state = self.state.lock().expect("logger state lock poisoned");
        if let Some(file) = state.file.as_mut() {
            let _ = file.flush();
        }
    }
}

static LOGGER: OnceLock<BeamLogger> = OnceLock::new();
static LOGGER_INSTALLED: OnceLock<()> = OnceLock::new();

pub fn init_logging(log_file_path: PathBuf) -> Result<(), String> {
    let logger = LOGGER.get_or_init(BeamLogger::new);
    logger
        .set_log_file(log_file_path.clone())
        .map_err(|error| {
            format!(
                "Failed to open log file {}: {error}",
                log_file_path.display()
            )
        })?;

    if LOGGER_INSTALLED.get().is_none() {
        log::set_logger(logger).map_err(|error| format!("Failed to install logger: {error}"))?;
        log::set_max_level(LevelFilter::Debug);
        let _ = LOGGER_INSTALLED.set(());
    }

    log::info!("logging_initialized log_file={}", log_file_path.display());
    Ok(())
}

fn rotated_log_path(path: &Path, index: usize) -> PathBuf {
    let file_name = path
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| "beam.log".to_string());
    path.with_file_name(format!("{file_name}.{index}"))
}

#[cfg(test)]
mod tests {
    use std::fs;

    use tempfile::tempdir;

    use super::{BeamLogger, rotated_log_path};

    #[test]
    fn rotate_logs_shifts_existing_backups() {
        let dir = tempdir().expect("tempdir");
        let log_file = dir.path().join("beam.log");
        fs::write(&log_file, "current").expect("write current");
        fs::write(rotated_log_path(&log_file, 1), "first").expect("write rotated 1");
        fs::write(rotated_log_path(&log_file, 2), "second").expect("write rotated 2");

        BeamLogger::rotate_logs(&log_file).expect("rotate logs");

        assert!(!log_file.exists());
        assert_eq!(
            fs::read_to_string(rotated_log_path(&log_file, 1)).expect("read rotated 1"),
            "current"
        );
        assert_eq!(
            fs::read_to_string(rotated_log_path(&log_file, 2)).expect("read rotated 2"),
            "first"
        );
        assert_eq!(
            fs::read_to_string(rotated_log_path(&log_file, 3)).expect("read rotated 3"),
            "second"
        );
    }
}
