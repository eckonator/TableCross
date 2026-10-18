//! Linux: X11 selections, or the Wayland data-control protocol where the compositor offers it
//! (KDE Plasma, wlroots); both through arboard, which keeps serving our content in the background.
//!
//! There is no change counter: changes are noticed by comparing the content (images less often,
//! they are expensive to fetch). Password managers mark what they copy with an extra format
//! (`x-kde-passwordManagerHint`); arboard does not list formats, so they are asked for over X11
//! directly (XWayland on Wayland desktops).

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::path::PathBuf;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use arboard::{Clipboard, ImageData};

use x11rb::connection::Connection;
use x11rb::protocol::Event;
use x11rb::protocol::xproto::{AtomEnum, ConnectionExt as _, CreateWindowAux, WindowClass};
use x11rb::rust_connection::RustConnection;

use crate::{ClipContent, ClipError, Read, SystemClipboard};

/// Formats a password manager puts next to a secret it copies.
const SECRET_MARKS: [&str; 2] = ["x-kde-passwordManagerHint", "application/x-nspasteboard-concealed-type"];
/// How long to wait for the owner of the clipboard to list its formats.
const TARGETS_WAIT: Duration = Duration::from_millis(150);

/// Asks the X11 clipboard for its formats (a window of our own receives the answer).
struct Targets {
    conn: RustConnection,
    window: u32,
    clipboard: u32,
    targets: u32,
    property: u32,
}

impl Targets {
    fn new() -> Option<Self> {
        let (conn, screen) = x11rb::connect(None).ok()?;
        let root = conn.setup().roots.get(screen)?.root;
        let window = conn.generate_id().ok()?;
        conn.create_window(
            0,
            window,
            root,
            0,
            0,
            1,
            1,
            0,
            WindowClass::INPUT_ONLY,
            0,
            &CreateWindowAux::new(),
        )
        .ok()?;
        let atom = |name: &str| {
            conn.intern_atom(false, name.as_bytes())
                .ok()?
                .reply()
                .ok()
                .map(|r| r.atom)
        };
        let (clipboard, targets, property) = (atom("CLIPBOARD")?, atom("TARGETS")?, atom("TABLECROSS_TARGETS")?);
        conn.flush().ok()?;
        Some(Self {
            conn,
            window,
            clipboard,
            targets,
            property,
        })
    }

    /// The names of the formats on offer (empty if the owner does not answer in time).
    fn list(&self) -> Vec<String> {
        let c = &self.conn;
        if c.convert_selection(self.window, self.clipboard, self.targets, self.property, 0u32)
            .is_err()
            || c.flush().is_err()
        {
            return Vec::new();
        }
        let until = Instant::now() + TARGETS_WAIT;
        let mut answered = false;
        while Instant::now() < until {
            match c.poll_for_event() {
                Ok(Some(Event::SelectionNotify(e))) if e.requestor == self.window => {
                    answered = e.property != 0;
                    break;
                }
                Ok(Some(_)) => {}
                Ok(None) => std::thread::sleep(Duration::from_millis(5)),
                Err(_) => return Vec::new(),
            }
        }
        if !answered {
            return Vec::new();
        }
        let Some(reply) = c
            .get_property(true, self.window, self.property, AtomEnum::ATOM, 0, 1024)
            .ok()
            .and_then(|r| r.reply().ok())
        else {
            return Vec::new();
        };
        let atoms: Vec<u32> = reply.value32().map(|v| v.collect()).unwrap_or_default();
        atoms
            .into_iter()
            .filter_map(|a| c.get_atom_name(a).ok()?.reply().ok())
            .map(|r| String::from_utf8_lossy(&r.name).into_owned())
            .collect()
    }

    fn secret(&self) -> bool {
        self.list().iter().any(|t| SECRET_MARKS.contains(&t.as_str()))
    }
}

const IMAGE_EVERY: Duration = Duration::from_millis(1500);

struct State {
    clipboard: Clipboard,
    /// `None` without an X server (then password markers are not seen).
    targets: Option<Targets>,
    signature: u64,
    image_signature: u64,
    image_checked: Option<Instant>,
    counter: u64,
}

pub struct LinuxClipboard {
    state: Mutex<State>,
}

impl LinuxClipboard {
    pub fn new() -> Option<Self> {
        let clipboard = Clipboard::new().map_err(|e| tracing::warn!("no clipboard: {e}")).ok()?;
        Some(Self {
            state: Mutex::new(State {
                clipboard,
                targets: Targets::new(),
                signature: 0,
                image_signature: 0,
                image_checked: None,
                counter: 0,
            }),
        })
    }
}

fn hash<T: Hash>(value: &T) -> u64 {
    let mut h = DefaultHasher::new();
    value.hash(&mut h);
    h.finish()
}

fn png_of(image: &ImageData<'_>) -> Option<Vec<u8>> {
    let mut out = Vec::new();
    let mut encoder = png::Encoder::new(&mut out, image.width as u32, image.height as u32);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    let mut writer = encoder.write_header().ok()?;
    writer.write_image_data(&image.bytes).ok()?;
    drop(writer);
    Some(out)
}

fn image_of(png_bytes: &[u8]) -> Option<ImageData<'static>> {
    let mut decoder = png::Decoder::new(std::io::Cursor::new(png_bytes));
    decoder.set_transformations(png::Transformations::EXPAND | png::Transformations::STRIP_16);
    let mut reader = decoder.read_info().ok()?;
    let mut buf = vec![0; reader.output_buffer_size()?];
    let frame = reader.next_frame(&mut buf).ok()?;
    let data = &buf[..frame.buffer_size()];
    let (w, h) = (frame.width as usize, frame.height as usize);
    let rgba: Vec<u8> = match frame.color_type {
        png::ColorType::Rgba => data.to_vec(),
        png::ColorType::Rgb => data.chunks(3).flat_map(|p| [p[0], p[1], p[2], 0xFF]).collect(),
        png::ColorType::GrayscaleAlpha => data.chunks(2).flat_map(|p| [p[0], p[0], p[0], p[1]]).collect(),
        png::ColorType::Grayscale => data.iter().flat_map(|g| [*g, *g, *g, 0xFF]).collect(),
        png::ColorType::Indexed => return None,
    };
    Some(ImageData {
        width: w,
        height: h,
        bytes: rgba.into(),
    })
}

impl State {
    fn texts(&mut self) -> (Option<String>, Option<String>, Option<Vec<PathBuf>>) {
        let text = self.clipboard.get_text().ok();
        let html = self.clipboard.get().html().ok();
        let files = self.clipboard.get().file_list().ok().filter(|f| !f.is_empty());
        (text, html, files)
    }
}

impl SystemClipboard for LinuxClipboard {
    fn change_count(&self) -> u64 {
        let mut state = self.state.lock().expect("clipboard lock");
        let signature = hash(&state.texts());
        let check_image = state.image_checked.is_none_or(|t| t.elapsed() >= IMAGE_EVERY);
        if check_image {
            state.image_checked = Some(Instant::now());
            let image = state.clipboard.get_image().ok().map(|i| hash(&i.bytes));
            let image_signature = hash(&image);
            if image_signature != state.image_signature {
                state.image_signature = image_signature;
                state.counter += 1;
            }
        }
        if signature != state.signature {
            state.signature = signature;
            state.counter += 1;
        }
        state.counter
    }

    fn read(&self) -> Result<Read, ClipError> {
        let mut state = self.state.lock().expect("clipboard lock");
        if state.targets.as_ref().is_some_and(Targets::secret) {
            return Ok(Read::Concealed);
        }
        let (text, html, files) = state.texts();
        if let Some(files) = files {
            return Ok(Read::Files(files));
        }
        let png = state.clipboard.get_image().ok().and_then(|i| png_of(&i));
        let content = ClipContent {
            text,
            html,
            rtf: None,
            png,
        };
        Ok(if content.is_empty() {
            Read::Empty
        } else {
            Read::Content(content)
        })
    }

    fn write(&self, content: &ClipContent) -> Result<u64, ClipError> {
        let mut state = self.state.lock().expect("clipboard lock");
        // One kind at a time here: text wins over an image that came with it (table cells, for
        // example, come as text and as a picture of them)
        let has_text = content.text.as_ref().is_some_and(|t| !t.is_empty());
        let result = if let Some(html) = content.html.as_ref().filter(|_| has_text || content.png.is_none()) {
            state.clipboard.set_html(html, content.text.as_ref())
        } else if let Some(text) = content.text.as_ref().filter(|_| has_text) {
            state.clipboard.set_text(text)
        } else if let Some(image) = content.png.as_deref().and_then(image_of) {
            state.clipboard.set_image(image)
        } else {
            Ok(())
        };
        result.map_err(|e| ClipError::System(e.to_string()))?;
        drop(state);
        // Take the new content as seen, so it is not announced as a change of our own
        let count = self.change_count();
        let mut state = self.state.lock().expect("clipboard lock");
        state.image_checked = None;
        drop(state);
        Ok(self.change_count().max(count))
    }

    fn write_files(&self, files: &[PathBuf]) -> Result<u64, ClipError> {
        self.state
            .lock()
            .expect("clipboard lock")
            .clipboard
            .set()
            .file_list(files)
            .map_err(|e| ClipError::System(e.to_string()))?;
        Ok(self.change_count())
    }
}
