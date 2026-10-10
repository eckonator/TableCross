//! Log file in `<settings>/logs/tablecross.log`, rotated at 1 MB (three old files are kept).

use std::fs::{File, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

const MAX_SIZE: u64 = 1024 * 1024;
const KEEP: usize = 3;

pub struct RotatingFile {
    path: PathBuf,
    state: Mutex<(File, u64)>,
}

impl RotatingFile {
    pub fn open(dir: &Path) -> io::Result<Self> {
        std::fs::create_dir_all(dir)?;
        let path = dir.join("tablecross.log");
        let file = OpenOptions::new().create(true).append(true).open(&path)?;
        let size = file.metadata()?.len();
        Ok(Self {
            path,
            state: Mutex::new((file, size)),
        })
    }

    fn rotate(&self, state: &mut (File, u64)) -> io::Result<()> {
        for i in (1..KEEP).rev() {
            let _ = std::fs::rename(self.numbered(i), self.numbered(i + 1));
        }
        std::fs::rename(&self.path, self.numbered(1))?;
        *state = (OpenOptions::new().create(true).append(true).open(&self.path)?, 0);
        Ok(())
    }

    fn numbered(&self, i: usize) -> PathBuf {
        self.path.with_extension(format!("log.{i}"))
    }

    pub fn path(&self) -> &Path {
        &self.path
    }
}

impl Write for &RotatingFile {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        let mut state = self.state.lock().map_err(|_| io::Error::other("log lock"))?;
        if state.1 + buf.len() as u64 > MAX_SIZE {
            let _ = self.rotate(&mut state);
        }
        let n = state.0.write(buf)?;
        state.1 += n as u64;
        // For computers that collect this log (troubleshooting)
        tc_session::peer_logs::record(&buf[..n]);
        Ok(n)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.state.lock().map_err(|_| io::Error::other("log lock"))?.0.flush()
    }
}

/// Logs to the file (and in debug builds to the terminal). Returns the log file's path.
pub fn init(dir: &Path) -> Option<PathBuf> {
    use tracing_subscriber::EnvFilter;
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info,quinn=warn,mdns_sd=warn"));
    match RotatingFile::open(&dir.join("logs")) {
        Ok(file) => {
            let file: &'static RotatingFile = Box::leak(Box::new(file));
            let path = file.path().to_path_buf();
            tracing_subscriber::fmt()
                .with_env_filter(filter)
                .with_ansi(false)
                .with_writer(move || file)
                .init();
            Some(path)
        }
        Err(e) => {
            tracing_subscriber::fmt().with_env_filter(filter).init();
            tracing::warn!("cannot open the log file: {e}");
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rotates_and_keeps_three() {
        let dir = std::env::temp_dir().join(format!("tablecross-log-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let log = RotatingFile::open(&dir).unwrap();
        let line = vec![b'x'; 100 * 1024];
        for _ in 0..60 {
            (&log).write_all(&line).unwrap();
        }
        assert!(std::fs::metadata(log.path()).unwrap().len() <= MAX_SIZE);
        for i in 1..=KEEP {
            assert!(log.numbered(i).exists(), "{i}");
        }
        assert!(!log.numbered(KEEP + 1).exists());
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
