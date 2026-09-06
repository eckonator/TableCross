//! macOS: the general pasteboard (`NSPasteboard`).

use objc2::rc::autoreleasepool;
use objc2::runtime::AnyObject;
use std::path::PathBuf;

use objc2::runtime::ProtocolObject;
use objc2_app_kit::{
    NSBitmapImageFileType, NSBitmapImageRep, NSPasteboard, NSPasteboardItem, NSPasteboardTypeFileURL,
    NSPasteboardTypeHTML, NSPasteboardTypePNG, NSPasteboardTypeRTF, NSPasteboardTypeString, NSPasteboardTypeTIFF,
    NSPasteboardWriting,
};
use objc2_foundation::{NSArray, NSData, NSDictionary, NSString, NSURL};

use crate::{ClipContent, ClipError, Read, SystemClipboard};

/// Markers of password managers and of clipboard tools (nspasteboard.org): never sent anywhere.
const PRIVATE_TYPES: [&str; 2] = ["org.nspasteboard.ConcealedType", "org.nspasteboard.TransientType"];

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
            if types.iter().any(|t| t == "public.file-url") {
                let files: Vec<PathBuf> = board
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
                    .unwrap_or_default();
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
                    ok &= board.setString_forType(&NSString::from_str(text), NSPasteboardTypeString);
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
}

impl MacClipboard {
    fn write_files_inner(&self, files: &[PathBuf]) -> Result<u64, ClipError> {
        autoreleasepool(|_| {
            let board = self.board();
            board.clearContents();
            let items: Vec<objc2::rc::Retained<ProtocolObject<dyn NSPasteboardWriting>>> = files
                .iter()
                .map(|path| {
                    let url = NSURL::fileURLWithPath(&NSString::from_str(&path.to_string_lossy()));
                    let item = NSPasteboardItem::new();
                    if let Some(text) = url.absoluteString() {
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
