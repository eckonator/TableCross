//! Running file transfers: sending a listing over a stream, receiving it into the download folder.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use tc_net::{Fingerprint, InStream, Node};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

use crate::files::{self, Kind, Listing};

const CHUNK: usize = 256 * 1024;
const PROGRESS_EVERY: Duration = Duration::from_millis(150);

#[derive(Debug, Clone, PartialEq)]
pub enum TransferState {
    Running,
    /// Received into this folder (or sent).
    Done(Option<PathBuf>),
    Failed(String),
    Cancelled,
}

#[derive(Debug, Clone, PartialEq)]
pub struct TransferView {
    pub id: u64,
    pub peer: String,
    pub incoming: bool,
    /// "Fotos, notes.txt"
    pub label: String,
    pub count: u64,
    pub done: u64,
    pub total: u64,
    pub state: TransferState,
}

/// Reports progress now and then.
pub struct Progress<F: Fn(u64)> {
    report: F,
    done: u64,
    last: Instant,
}

impl<F: Fn(u64)> Progress<F> {
    pub fn new(report: F) -> Self {
        Self {
            report,
            done: 0,
            last: Instant::now(),
        }
    }

    fn add(&mut self, n: usize) {
        self.done += n as u64;
        if self.last.elapsed() >= PROGRESS_EVERY {
            self.last = Instant::now();
            (self.report)(self.done);
        }
    }

    fn finish(&self) {
        (self.report)(self.done);
    }
}

/// Sends every entry of `listing` on a new stream to `peer`; without `links` (a peer before 0.4.26)
/// as plain files and folders only.
pub async fn send(
    node: &Node,
    peer: &Fingerprint,
    id: u64,
    listing: &Listing,
    links: bool,
    cancel: &AtomicBool,
    progress: &mut Progress<impl Fn(u64)>,
) -> Result<(), String> {
    let mut out = node
        .open_stream(peer, &files::header(id))
        .await
        .map_err(|e| e.to_string())?;
    let mut buf = vec![0; CHUNK];
    for (entry, path) in &listing.entries {
        if cancel.load(Ordering::SeqCst) {
            out.cancel();
            return Err("cancelled".into());
        }
        let plain;
        let entry = if links {
            entry
        } else {
            match entry.plain() {
                Some(e) => {
                    plain = e;
                    &plain
                }
                None => continue,
            }
        };
        if entry.has_content() {
            let mut file = tokio::fs::File::open(path)
                .await
                .map_err(|e| format!("{}: {e}", path.display()))?;
            // The size announced is what goes on the wire, even if the file changed meanwhile
            out.write_all(&files::encode_entry(entry))
                .await
                .map_err(|e| e.to_string())?;
            let mut left = entry.size;
            while left > 0 {
                if cancel.load(Ordering::SeqCst) {
                    out.cancel();
                    return Err("cancelled".into());
                }
                let want = (left as usize).min(buf.len());
                let n = file.read(&mut buf[..want]).await.map_err(|e| e.to_string())?;
                if n == 0 {
                    // The file got shorter: fill up, the receiver checks nothing but the size
                    buf[..want].fill(0);
                    out.write_all(&buf[..want]).await.map_err(|e| e.to_string())?;
                    left -= want as u64;
                    progress.add(want);
                    continue;
                }
                out.write_all(&buf[..n]).await.map_err(|e| e.to_string())?;
                left -= n as u64;
                progress.add(n);
            }
        } else {
            out.write_all(&files::encode_entry(entry))
                .await
                .map_err(|e| e.to_string())?;
        }
    }
    progress.finish();
    out.finish().await.map_err(|e| e.to_string())
}

async fn read_exact(stream: &mut InStream, buf: &mut [u8]) -> Result<bool, String> {
    let mut filled = 0;
    while filled < buf.len() {
        let n = stream.read(&mut buf[filled..]).await.map_err(|e| e.to_string())?;
        if n == 0 {
            return if filled == 0 {
                Ok(false)
            } else {
                Err("stream ended early".into())
            };
        }
        filled += n;
    }
    Ok(true)
}

/// Receives into a part folder inside `target` and moves the result into `target` when complete.
/// Returns the top-level files and folders as they are named now.
pub async fn receive(
    mut stream: InStream,
    id: u64,
    target: &Path,
    cancel: &AtomicBool,
    progress: &mut Progress<impl Fn(u64)>,
) -> Result<Vec<PathBuf>, String> {
    let partial = target.join(format!(".tablecross-part-{id:x}"));
    let result = receive_into(&mut stream, &partial, cancel, progress).await;
    let roots = match result {
        Ok(roots) => roots,
        Err(e) => {
            if cancel.load(Ordering::SeqCst) {
                stream.cancel();
            }
            let _ = tokio::fs::remove_dir_all(&partial).await;
            return Err(e);
        }
    };
    let mut placed = Vec::new();
    for root in roots {
        let destination = files::unique_name(target, &root);
        tokio::fs::rename(partial.join(&root), &destination)
            .await
            .map_err(|e| e.to_string())?;
        placed.push(destination);
    }
    let _ = tokio::fs::remove_dir_all(&partial).await;
    progress.finish();
    Ok(placed)
}

async fn receive_into(
    stream: &mut InStream,
    partial: &Path,
    cancel: &AtomicBool,
    progress: &mut Progress<impl Fn(u64)>,
) -> Result<Vec<String>, String> {
    tokio::fs::create_dir_all(partial).await.map_err(|e| e.to_string())?;
    let mut roots: Vec<String> = Vec::new();
    // Nothing is written below or onto a link: following it could lead elsewhere
    let mut links: Vec<String> = Vec::new();
    let mut buf = vec![0; CHUNK];
    loop {
        if cancel.load(Ordering::SeqCst) {
            return Err("cancelled".into());
        }
        let mut len = [0u8; 4];
        if !read_exact(stream, &mut len).await? {
            break;
        }
        let len = u32::from_le_bytes(len) as usize;
        if !files::entry_header_ok(len) {
            return Err("invalid entry".into());
        }
        let mut header = vec![0; len];
        if !read_exact(stream, &mut header).await? {
            return Err("stream ended early".into());
        }
        let entry = files::decode_entry(&header).ok_or("invalid entry")?;
        let path = files::safe_join(partial, &entry.path).ok_or("unsafe path")?;
        if links
            .iter()
            .any(|l| entry.path == *l || entry.path.starts_with(&format!("{l}/")))
        {
            return Err("unsafe path".into());
        }
        let root = entry.path.split('/').next().unwrap_or_default().to_string();
        if !roots.contains(&root) {
            roots.push(root);
        }
        match &entry.kind {
            Kind::Dir => tokio::fs::create_dir_all(&path).await.map_err(|e| e.to_string())?,
            Kind::Link(target) => {
                if !files::link_ok(&entry.path, target) {
                    return Err("unsafe link".into());
                }
                links.push(entry.path.clone());
                if let Some(parent) = path.parent() {
                    tokio::fs::create_dir_all(parent).await.map_err(|e| e.to_string())?;
                }
                // Windows makes links only for administrators: left out there
                #[cfg(unix)]
                tokio::fs::symlink(target, &path).await.map_err(|e| e.to_string())?;
            }
            Kind::File | Kind::Executable => {
                if let Some(parent) = path.parent() {
                    tokio::fs::create_dir_all(parent).await.map_err(|e| e.to_string())?;
                }
                let mut file = tokio::fs::File::create(&path).await.map_err(|e| e.to_string())?;
                let mut left = entry.size;
                while left > 0 {
                    if cancel.load(Ordering::SeqCst) {
                        return Err("cancelled".into());
                    }
                    let want = (left as usize).min(buf.len());
                    let n = stream.read(&mut buf[..want]).await.map_err(|e| e.to_string())?;
                    if n == 0 {
                        return Err("stream ended early".into());
                    }
                    file.write_all(&buf[..n]).await.map_err(|e| e.to_string())?;
                    left -= n as u64;
                    progress.add(n);
                }
                file.flush().await.map_err(|e| e.to_string())?;
                #[cfg(unix)]
                if entry.kind == Kind::Executable {
                    use std::os::unix::fs::PermissionsExt;
                    tokio::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755))
                        .await
                        .map_err(|e| e.to_string())?;
                }
            }
        }
    }
    Ok(roots)
}

/// Cancel switches of running transfers.
#[derive(Default)]
pub struct Cancels(std::collections::HashMap<u64, Arc<AtomicBool>>);

impl Cancels {
    pub fn add(&mut self, id: u64) -> Arc<AtomicBool> {
        let flag = Arc::new(AtomicBool::new(false));
        self.0.insert(id, flag.clone());
        flag
    }

    pub fn cancel(&mut self, id: u64) {
        if let Some(flag) = self.0.remove(&id) {
            flag.store(true, Ordering::SeqCst);
        }
    }

    pub fn finished(&mut self, id: u64) {
        self.0.remove(&id);
    }
}
