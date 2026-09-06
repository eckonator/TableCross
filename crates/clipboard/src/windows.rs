//! Windows: the clipboard with text, HTML, RTF and images (PNG, and a DIB for older programs).

use std::path::PathBuf;
use std::time::Duration;

use windows_sys::Win32::Foundation::{GlobalFree, HANDLE, HGLOBAL};
use windows_sys::Win32::System::DataExchange::{
    CloseClipboard, EmptyClipboard, GetClipboardData, GetClipboardSequenceNumber, IsClipboardFormatAvailable,
    OpenClipboard, RegisterClipboardFormatW, SetClipboardData,
};
use windows_sys::Win32::System::Memory::{GMEM_MOVEABLE, GlobalAlloc, GlobalLock, GlobalSize, GlobalUnlock};

use windows_sys::Win32::UI::Shell::{DragQueryFileW, HDROP};

use crate::{ClipContent, ClipError, Read, SystemClipboard, cf_html, dib};

const CF_UNICODETEXT: u32 = 13;
const CF_DIB: u32 = 8;
const CF_HDROP: u32 = 15;
/// Size of DROPFILES: offset to the file list, point, "non-client" flag, "wide" flag.
const DROPFILES_SIZE: u32 = 20;
const DROPEFFECT_COPY: u32 = 1;

/// Files on the clipboard (from Explorer).
fn files() -> Option<Vec<PathBuf>> {
    unsafe {
        if IsClipboardFormatAvailable(CF_HDROP) == 0 {
            return None;
        }
        let drop = GetClipboardData(CF_HDROP) as HDROP;
        if drop.is_null() {
            return None;
        }
        let count = DragQueryFileW(drop, u32::MAX, std::ptr::null_mut(), 0);
        let mut out = Vec::new();
        for i in 0..count {
            let len = DragQueryFileW(drop, i, std::ptr::null_mut(), 0) as usize;
            let mut buf = vec![0u16; len + 1];
            DragQueryFileW(drop, i, buf.as_mut_ptr(), buf.len() as u32);
            out.push(PathBuf::from(String::from_utf16_lossy(&buf[..len])));
        }
        Some(out)
    }
}

/// DROPFILES followed by the paths, each ending with a zero, and one more zero.
fn drop_list(files: &[PathBuf]) -> Vec<u8> {
    let mut out = Vec::new();
    out.extend_from_slice(&DROPFILES_SIZE.to_le_bytes());
    out.extend_from_slice(&[0; 8]); // point
    out.extend_from_slice(&0u32.to_le_bytes()); // fNC
    out.extend_from_slice(&1u32.to_le_bytes()); // fWide
    for file in files {
        for unit in file.to_string_lossy().encode_utf16().chain(Some(0)) {
            out.extend_from_slice(&unit.to_le_bytes());
        }
    }
    out.extend_from_slice(&[0, 0]);
    out
}

pub struct WinClipboard;

fn format(name: &str) -> u32 {
    let wide: Vec<u16> = name.encode_utf16().chain(Some(0)).collect();
    unsafe { RegisterClipboardFormatW(wide.as_ptr()) }
}

/// Holds the clipboard open; closes it when dropped. Retries a few times, other programs often
/// hold it for a moment.
struct Open;

impl Open {
    fn new() -> Result<Self, ClipError> {
        for _ in 0..10 {
            if unsafe { OpenClipboard(std::ptr::null_mut()) } != 0 {
                return Ok(Open);
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        Err(ClipError::Busy)
    }
}

impl Drop for Open {
    fn drop(&mut self) {
        unsafe { CloseClipboard() };
    }
}

fn get(format: u32) -> Option<Vec<u8>> {
    unsafe {
        if IsClipboardFormatAvailable(format) == 0 {
            return None;
        }
        let handle = GetClipboardData(format);
        if handle.is_null() {
            return None;
        }
        let size = GlobalSize(handle as HGLOBAL);
        let ptr = GlobalLock(handle as HGLOBAL) as *const u8;
        if ptr.is_null() {
            return None;
        }
        let bytes = std::slice::from_raw_parts(ptr, size).to_vec();
        GlobalUnlock(handle as HGLOBAL);
        Some(bytes)
    }
}

fn set(format: u32, bytes: &[u8]) -> bool {
    unsafe {
        let memory = GlobalAlloc(GMEM_MOVEABLE, bytes.len().max(1));
        if memory.is_null() {
            return false;
        }
        let ptr = GlobalLock(memory) as *mut u8;
        if ptr.is_null() {
            GlobalFree(memory);
            return false;
        }
        std::ptr::copy_nonoverlapping(bytes.as_ptr(), ptr, bytes.len());
        GlobalUnlock(memory);
        // On success the system owns the memory
        if SetClipboardData(format, memory as HANDLE).is_null() {
            GlobalFree(memory);
            return false;
        }
        true
    }
}

fn utf16_text(bytes: &[u8]) -> String {
    let units: Vec<u16> = bytes
        .as_chunks::<2>()
        .0
        .iter()
        .map(|c| u16::from_le_bytes(*c))
        .take_while(|u| *u != 0)
        .collect();
    String::from_utf16_lossy(&units)
}

/// Password managers mark their entries so that clipboard tools leave them alone.
fn is_private() -> bool {
    if get(format("ExcludeClipboardContentFromMonitorProcessing")).is_some()
        || get(format("Clipboard Viewer Ignore")).is_some()
    {
        return true;
    }
    get(format("CanIncludeInClipboardHistory")).is_some_and(|v| v.get(..4) == Some(&[0, 0, 0, 0]))
}

impl SystemClipboard for WinClipboard {
    fn change_count(&self) -> u64 {
        u64::from(unsafe { GetClipboardSequenceNumber() })
    }

    fn read(&self) -> Result<Read, ClipError> {
        let _open = Open::new()?;
        if is_private() {
            return Ok(Read::Concealed);
        }
        if let Some(files) = files().filter(|f| !f.is_empty()) {
            return Ok(Read::Files(files));
        }
        let trim_nul = |mut b: Vec<u8>| {
            while b.last() == Some(&0) {
                b.pop();
            }
            b
        };
        let content = ClipContent {
            text: get(CF_UNICODETEXT).map(|b| utf16_text(&b)),
            html: get(format("HTML Format")).and_then(|b| cf_html::decode(&b)),
            rtf: get(format("Rich Text Format")).map(trim_nul),
            png: get(format("PNG")).or_else(|| get(CF_DIB).and_then(|d| dib::dib_to_png(&d))),
        };
        Ok(if content.is_empty() {
            Read::Empty
        } else {
            Read::Content(content)
        })
    }

    fn write(&self, content: &ClipContent) -> Result<u64, ClipError> {
        {
            let _open = Open::new()?;
            unsafe { EmptyClipboard() };
            if let Some(text) = &content.text {
                let wide: Vec<u8> = text.encode_utf16().chain(Some(0)).flat_map(u16::to_le_bytes).collect();
                set(CF_UNICODETEXT, &wide);
            }
            if let Some(html) = &content.html {
                let mut bytes = cf_html::encode(html);
                bytes.push(0);
                set(format("HTML Format"), &bytes);
            }
            if let Some(rtf) = &content.rtf {
                let mut bytes = rtf.clone();
                bytes.push(0);
                set(format("Rich Text Format"), &bytes);
            }
            if let Some(png) = &content.png {
                set(format("PNG"), png);
                if let Some(dib) = dib::png_to_dib(png) {
                    set(CF_DIB, &dib);
                }
            }
        }
        Ok(self.change_count())
    }

    fn write_files(&self, files: &[PathBuf]) -> Result<u64, ClipError> {
        {
            let _open = Open::new()?;
            unsafe { EmptyClipboard() };
            set(CF_HDROP, &drop_list(files));
            // Pasting copies (and does not move) the files
            set(format("Preferred DropEffect"), &DROPEFFECT_COPY.to_le_bytes());
        }
        Ok(self.change_count())
    }
}
