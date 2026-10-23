//! macOS: the general pasteboard (`NSPasteboard`).

use objc2::rc::{Retained, autoreleasepool};
use objc2::runtime::AnyObject;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use objc2::runtime::{NSObject, NSObjectProtocol, ProtocolObject};
use objc2::{AllocAnyThread, DefinedClass, define_class, msg_send};
use objc2_app_kit::{
    NSBitmapImageFileType, NSBitmapImageRep, NSPasteboard, NSPasteboardContentsOptions, NSPasteboardItem,
    NSPasteboardItemDataProvider, NSPasteboardNameDrag, NSPasteboardType, NSPasteboardTypeFileURL,
    NSPasteboardTypeHTML, NSPasteboardTypePNG, NSPasteboardTypeRTF, NSPasteboardTypeString, NSPasteboardTypeTIFF,
    NSPasteboardWriting,
};
use objc2_foundation::{NSArray, NSData, NSDictionary, NSString, NSURL};

use crate::{ClipContent, ClipError, Fulfil, Read, SystemClipboard};

/// Markers of password managers and of clipboard tools (nspasteboard.org): never sent anywhere.
const PRIVATE_TYPES: [&str; 2] = ["org.nspasteboard.ConcealedType", "org.nspasteboard.TransientType"];

/// Marks content that Apple's Universal Clipboard brought from another device.
const UNIVERSAL_CLIPBOARD: &str = "com.apple.is-remote-clipboard";

/// The general pasteboard, or a named one (tests, so the user's clipboard stays untouched).
#[derive(Default)]
pub struct MacClipboard {
    name: Option<String>,
}

impl MacClipboard {
    fn board(&self) -> objc2::rc::Retained<NSPasteboard> {
        match &self.name {
            Some(name) => NSPasteboard::pasteboardWithName(&NSString::from_str(name)),
            None => NSPasteboard::generalPasteboard(),
        }
    }
}

/// The files on a pasteboard.
fn files_of(board: &NSPasteboard) -> Vec<PathBuf> {
    board
        .pasteboardItems()
        .map(|items| {
            items
                .iter()
                .filter_map(|item| unsafe { item.stringForType(NSPasteboardTypeFileURL) })
                .filter_map(|url| NSURL::URLWithString(&url))
                .filter_map(|url| url.path())
                .map(|path| PathBuf::from(path.to_string()))
                .collect()
        })
        .unwrap_or_default()
}

fn drag_board() -> objc2::rc::Retained<NSPasteboard> {
    NSPasteboard::pasteboardWithName(unsafe { NSPasteboardNameDrag })
}

/// Changes whenever a drag starts somewhere.
pub fn drag_change_count() -> u64 {
    autoreleasepool(|_| drag_board().changeCount() as u64)
}

/// The files of the drag going on (or of the last one).
pub fn drag_files() -> Vec<PathBuf> {
    autoreleasepool(|_| files_of(&drag_board()))
}

fn file_url(path: &Path) -> Option<Retained<NSString>> {
    NSURL::fileURLWithPath(&NSString::from_str(&path.to_string_lossy())).absoluteString()
}

struct PromiseIvars {
    /// Which of the promised files this item stands for.
    index: usize,
    fulfil: Fulfil,
}

define_class!(
    /// Hands out the file URL of one promised file when it is pasted, after fetching the files.
    #[unsafe(super(NSObject))]
    #[name = "TableCrossFilePromise"]
    #[ivars = PromiseIvars]
    struct FilePromise;

    unsafe impl NSObjectProtocol for FilePromise {}

    unsafe impl NSPasteboardItemDataProvider for FilePromise {
        #[unsafe(method(pasteboard:item:provideDataForType:))]
        fn provide(&self, _board: Option<&NSPasteboard>, item: &NSPasteboardItem, _kind: &NSPasteboardType) {
            let ivars = self.ivars();
            let Some(paths) = (ivars.fulfil)() else {
                tracing::warn!("promised files could not be fetched");
                return;
            };
            if let Some(url) = paths.get(ivars.index).and_then(|p| file_url(p)) {
                unsafe { item.setString_forType(&url, NSPasteboardTypeFileURL) };
            }
        }
    }
);

impl FilePromise {
    fn new(index: usize, fulfil: Fulfil) -> Retained<Self> {
        let this = Self::alloc().set_ivars(PromiseIvars { index, fulfil });
        unsafe { msg_send![super(this), init] }
    }
}

/// The providers of the promise on the pasteboard, kept alive until the next one.
struct Providers(#[allow(dead_code)] Vec<Retained<FilePromise>>);
// Only kept and dropped here; AppKit calls them on its own thread
unsafe impl Send for Providers {}
static PROVIDERS: Mutex<Option<Providers>> = Mutex::new(None);

/// TIFF (what many Mac programs put there for images) to PNG.
fn tiff_to_png(tiff: &NSData) -> Option<Vec<u8>> {
    let rep = NSBitmapImageRep::imageRepWithData(tiff)?;
    let properties = NSDictionary::<NSString, AnyObject>::new();
    let png = unsafe { rep.representationUsingType_properties(NSBitmapImageFileType::PNG, &properties) }?;
    Some(png.to_vec())
}

impl SystemClipboard for MacClipboard {
    fn change_count(&self) -> u64 {
        autoreleasepool(|_| self.board().changeCount() as u64)
    }

    fn read(&self) -> Result<Read, ClipError> {
        autoreleasepool(|_| {
            let board = self.board();
            let types: Vec<String> = board
                .types()
                .map(|t| t.iter().map(|s| s.to_string()).collect())
                .unwrap_or_default();
            if types.iter().any(|t| PRIVATE_TYPES.contains(&t.as_str())) {
                return Ok(Read::Concealed);
            }
            if types.iter().any(|t| t == UNIVERSAL_CLIPBOARD) {
                // Apple's own sync brought it from another device; passing it on as well makes the
                // two syncs overwrite each other with older content
                return Ok(Read::Empty);
            }
            if types.iter().any(|t| t == "public.file-url") {
                let files = files_of(&board);
                if !files.is_empty() {
                    return Ok(Read::Files(files));
                }
            }
            let data = |kind: &NSString| board.dataForType(kind).map(|d| d.to_vec());
            let png = unsafe {
                data(NSPasteboardTypePNG)
                    .or_else(|| board.dataForType(NSPasteboardTypeTIFF).and_then(|t| tiff_to_png(&t)))
            };
            let content = unsafe {
                ClipContent {
                    text: board.stringForType(NSPasteboardTypeString).map(|s| s.to_string()),
                    html: data(NSPasteboardTypeHTML).map(|b| String::from_utf8_lossy(&b).into_owned()),
                    rtf: data(NSPasteboardTypeRTF),
                    png,
                }
            };
            Ok(if content.is_empty() {
                Read::Empty
            } else {
                Read::Content(content)
            })
        })
    }

    fn write(&self, content: &ClipContent) -> Result<u64, ClipError> {
        autoreleasepool(|_| {
            let board = self.board();
            board.clearContents();
            let mut ok = true;
            unsafe {
                if let Some(text) = &content.text {
                    ok &= board
                        .setString_forType(&NSString::from_str(&crate::native_lines(text)), NSPasteboardTypeString);
                }
                if let Some(html) = &content.html {
                    ok &= board.setData_forType(Some(&NSData::with_bytes(html.as_bytes())), NSPasteboardTypeHTML);
                }
                if let Some(rtf) = &content.rtf {
                    ok &= board.setData_forType(Some(&NSData::with_bytes(rtf)), NSPasteboardTypeRTF);
                }
                if let Some(png) = &content.png {
                    ok &= board.setData_forType(Some(&NSData::with_bytes(png)), NSPasteboardTypePNG);
                }
            }
            if !ok {
                return Err(ClipError::System("the pasteboard refused the content".into()));
            }
            Ok(board.changeCount() as u64)
        })
    }

    fn write_files(&self, files: &[PathBuf]) -> Result<u64, ClipError> {
        self.write_files_inner(files)
    }

    fn can_promise(&self) -> bool {
        true
    }

    fn write_file_promise(&self, count: usize, fulfil: Fulfil) -> Option<Result<u64, ClipError>> {
        Some(autoreleasepool(|_| {
            let board = self.board();
            // Not for Apple's own sync to other devices: it asks for the data right away, which would
            // fetch all files over the network although nobody pasted them
            board.prepareForNewContentsWithOptions(NSPasteboardContentsOptions::CurrentHostOnly);
            let types = NSArray::from_slice(&[unsafe { NSPasteboardTypeFileURL }]);
            let providers: Vec<Retained<FilePromise>> = (0..count)
                .map(|index| FilePromise::new(index, fulfil.clone()))
                .collect();
            let items: Vec<Retained<ProtocolObject<dyn NSPasteboardWriting>>> = providers
                .iter()
                .map(|provider| {
                    let item = NSPasteboardItem::new();
                    item.setDataProvider_forTypes(ProtocolObject::from_ref(&**provider), &types);
                    ProtocolObject::from_retained(item)
                })
                .collect();
            *PROVIDERS.lock().expect("providers lock") = Some(Providers(providers));
            if !board.writeObjects(&NSArray::from_retained_slice(&items)) {
                return Err(ClipError::System("the pasteboard refused the promised files".into()));
            }
            Ok(board.changeCount() as u64)
        }))
    }
}

impl MacClipboard {
    fn write_files_inner(&self, files: &[PathBuf]) -> Result<u64, ClipError> {
        autoreleasepool(|_| {
            let board = self.board();
            board.clearContents();
            let items: Vec<objc2::rc::Retained<ProtocolObject<dyn NSPasteboardWriting>>> = files
                .iter()
                .map(|path| {
                    let item = NSPasteboardItem::new();
                    if let Some(text) = file_url(path) {
                        unsafe { item.setString_forType(&text, NSPasteboardTypeFileURL) };
                    }
                    ProtocolObject::from_retained(item)
                })
                .collect();
            if !board.writeObjects(&NSArray::from_retained_slice(&items)) {
                return Err(ClipError::System("the pasteboard refused the files".into()));
            }
            Ok(board.changeCount() as u64)
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn write_and_read_back_on_a_private_pasteboard() {
        let clip = MacClipboard {
            name: Some(format!("de.eckonator.tablecross.test.{}", std::process::id())),
        };
        let before = clip.change_count();
        let png = crate::dib::dib_to_png(&crate::dib::png_to_dib(&tiny_png()).unwrap()).unwrap();
        let content = ClipContent {
            text: Some("Grüße aus TableCross".into()),
            html: Some("<b>Grüße</b>".into()),
            rtf: Some(b"{\\rtf1 Hallo}".to_vec()),
            png: Some(png),
        };
        let after = clip.write(&content).unwrap();
        assert!(after > before);
        assert_eq!(clip.read().unwrap(), Read::Content(content));
        clip.board().clearContents();
        assert_eq!(clip.read().unwrap(), Read::Empty);

        let files = vec![PathBuf::from("/tmp/notes.txt"), PathBuf::from("/tmp/a folder")];
        clip.write_files(&files).unwrap();
        assert_eq!(clip.read().unwrap(), Read::Files(files));
    }

    #[test]
    fn promised_files_are_fetched_when_pasted() {
        let clip = MacClipboard {
            name: Some(format!("de.eckonator.tablecross.promise.{}", std::process::id())),
        };
        let calls = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let counted = calls.clone();
        let fulfil: Fulfil = std::sync::Arc::new(move || {
            counted.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            Some(vec![PathBuf::from("/tmp/one.txt"), PathBuf::from("/tmp/two")])
        });
        clip.write_file_promise(2, fulfil).unwrap().unwrap();
        assert_eq!(
            calls.load(std::sync::atomic::Ordering::SeqCst),
            0,
            "nothing fetched before pasting"
        );
        assert_eq!(
            clip.read().unwrap(),
            Read::Files(vec![PathBuf::from("/tmp/one.txt"), PathBuf::from("/tmp/two")])
        );
        assert!(calls.load(std::sync::atomic::Ordering::SeqCst) >= 1);
        clip.board().clearContents();
    }

    #[test]
    fn files_being_dragged_are_found() {
        let before = drag_change_count();
        assert!(crate::drag::files_since(before).is_empty());
        let file = std::env::temp_dir().join(format!("tablecross-drag-{}.txt", std::process::id()));
        std::fs::write(&file, "x").unwrap();
        let drag = MacClipboard {
            name: Some(unsafe { NSPasteboardNameDrag }.to_string()),
        };
        drag.write_files(std::slice::from_ref(&file)).unwrap();
        let found = crate::drag::files_since(before);
        std::fs::remove_file(&file).unwrap();
        assert_eq!(found.len(), 1);
        assert!(found[0].ends_with(file.file_name().unwrap()));
    }

    #[test]
    fn several_lines_travel_whole() {
        let text = "rm -rf /Applications/Deskflow.app ~/Library/Deskflow\ndefaults delete org.deskflow.deskflow\n\
                    rm -f ~/Library/Application\\ Support/CrashReporter/deskflow-*\n";
        let pid = std::process::id();
        let from = MacClipboard {
            name: Some(format!("tablecross-test-from-{pid}")),
        };
        let to = MacClipboard {
            name: Some(format!("tablecross-test-to-{pid}")),
        };
        from.write(&ClipContent {
            text: Some(text.into()),
            ..ClipContent::default()
        })
        .unwrap();
        let Read::Content(read) = from.read().unwrap() else {
            panic!("no content");
        };
        let travelled = ClipContent::decode(&read.encode()).unwrap();
        to.write(&travelled).unwrap();
        let Read::Content(arrived) = to.read().unwrap() else {
            panic!("no content");
        };
        assert_eq!(arrived.text.as_deref(), Some(text));
    }

    fn tiny_png() -> Vec<u8> {
        let mut out = Vec::new();
        let mut encoder = png::Encoder::new(&mut out, 2, 1);
        encoder.set_color(png::ColorType::Rgba);
        let mut writer = encoder.write_header().unwrap();
        writer.write_image_data(&[255, 0, 0, 255, 0, 0, 255, 255]).unwrap();
        drop(writer);
        out
    }
}
