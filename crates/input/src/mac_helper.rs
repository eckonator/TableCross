//! The app's side of the macOS system helper: connects (and reconnects) to its socket, asks it to
//! take the keyboards while the pointer is on another computer, and hands its keys to the capture
//! handler. Without the helper everything works as before, except while Secure Input is active.

use std::io::{BufReader, BufWriter, Write};
use std::os::unix::net::UnixStream;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use crate::helper_protocol::{
    ALIVE_INTERVAL, FromHelper, HELPER_VERSION, ToHelper, read_frame, socket_path, write_frame,
};

const RECONNECT: Duration = Duration::from_secs(2);

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct HelperStatus {
    pub connected: bool,
    /// The helper has the Input Monitoring permission.
    pub access: bool,
    pub seized: bool,
    pub devices: u16,
    pub failed: u16,
    /// The helper speaks another version (reinstall it).
    pub version_mismatch: Option<u16>,
}

pub struct HelperClient {
    writer: Mutex<Option<BufWriter<UnixStream>>>,
    want_seize: AtomicBool,
    status: Mutex<HelperStatus>,
}

impl HelperClient {
    /// Starts connecting in the background; `on_key` gets the helper's keys.
    pub fn start(on_key: impl Fn(u16, bool) + Send + Sync + 'static) -> Arc<Self> {
        let client = Arc::new(Self {
            writer: Mutex::new(None),
            want_seize: AtomicBool::new(false),
            status: Mutex::new(HelperStatus::default()),
        });
        let on_key = Arc::new(on_key);
        let weak = Arc::downgrade(&client);
        std::thread::Builder::new()
            .name("tablecross-helper-client".into())
            .spawn(move || {
                loop {
                    let Some(client) = weak.upgrade() else { return };
                    if let Ok(stream) = UnixStream::connect(socket_path()) {
                        client.serve(stream, on_key.clone());
                    }
                    drop(client);
                    std::thread::sleep(RECONNECT);
                }
            })
            .expect("thread");
        // Signs of life, so the helper keeps the keyboards only while we are running
        let weak = Arc::downgrade(&client);
        std::thread::Builder::new()
            .name("tablecross-helper-alive".into())
            .spawn(move || {
                loop {
                    std::thread::sleep(ALIVE_INTERVAL);
                    let Some(client) = weak.upgrade() else { return };
                    client.send(&ToHelper::Alive);
                }
            })
            .expect("thread");
        client
    }

    fn serve(&self, stream: UnixStream, on_key: Arc<impl Fn(u16, bool)>) {
        let Ok(reader) = stream.try_clone() else { return };
        let _ = stream.set_write_timeout(Some(Duration::from_millis(500)));
        *self.writer.lock().expect("helper writer") = Some(BufWriter::new(stream));
        self.send(&ToHelper::Hello {
            version: HELPER_VERSION,
        });
        let mut reader = BufReader::new(reader);
        while let Ok(message) = read_frame::<FromHelper>(&mut reader) {
            match message {
                FromHelper::Hello { version, access } => {
                    let mut status = self.status.lock().expect("helper status");
                    if version != HELPER_VERSION {
                        status.version_mismatch = Some(version);
                        drop(status);
                        break;
                    }
                    *status = HelperStatus {
                        connected: true,
                        access,
                        ..HelperStatus::default()
                    };
                    drop(status);
                    tracing::info!("system helper connected (input monitoring: {access})");
                    // Restore what we want after a reconnect
                    self.send(&ToHelper::Seize(self.want_seize.load(Ordering::SeqCst)));
                }
                FromHelper::Key { usage, down } => on_key(usage, down),
                FromHelper::Status {
                    seized,
                    devices,
                    failed,
                } => {
                    let mut status = self.status.lock().expect("helper status");
                    status.seized = seized;
                    status.devices = devices;
                    status.failed = failed;
                }
            }
        }
        *self.writer.lock().expect("helper writer") = None;
        let mut status = self.status.lock().expect("helper status");
        status.connected = false;
        status.seized = false;
    }

    fn send(&self, message: &ToHelper) {
        let mut writer = self.writer.lock().expect("helper writer");
        if let Some(stream) = writer.as_mut()
            && write_frame(stream, message).and_then(|()| stream.flush()).is_err()
        {
            *writer = None;
        }
    }

    pub fn set_seize(&self, seize: bool) {
        self.want_seize.store(seize, Ordering::SeqCst);
        self.send(&ToHelper::Seize(seize));
    }

    pub fn status(&self) -> HelperStatus {
        self.status.lock().expect("helper status").clone()
    }
}
