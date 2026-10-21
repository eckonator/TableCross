//! macOS: files dragged over the screen edge from another computer. The drag continues here:
//! TableCross presses the mouse button in a nearly invisible window under the pointer and starts a
//! drag of promised files. Where they are dropped, the Finder asks for them, and they are fetched
//! and put right there.

use std::cell::RefCell;
use std::path::Path;
use std::time::Duration;

use block2::DynBlock;
use dispatch2::DispatchQueue;
use objc2::rc::Retained;
use objc2::runtime::{AnyObject, NSObject, NSObjectProtocol, ProtocolObject};
use objc2::{AllocAnyThread, DefinedClass, MainThreadMarker, MainThreadOnly, define_class, msg_send};
use objc2_app_kit::{
    NSBackingStoreType, NSColor, NSDragOperation, NSDraggingContext, NSDraggingItem, NSDraggingSession,
    NSDraggingSource, NSEvent, NSFilePromiseProvider, NSFilePromiseProviderDelegate, NSImage,
    NSImageNameMultipleDocuments, NSScreenSaverWindowLevel, NSView, NSWindow, NSWindowStyleMask,
};
use objc2_foundation::{NSArray, NSError, NSOperationQueue, NSPoint, NSRect, NSSize, NSString, NSURL};

use crate::Fulfil;

/// Size of the window that catches the press (points): the pointer moves on meanwhile.
const CATCHER: f64 = 160.0;
/// Time for the window to appear before the button is pressed in it.
const SETTLE: Duration = Duration::from_millis(80);

struct FileIvars {
    /// Which of the fetched files this one is.
    index: usize,
    name: String,
    fulfil: Fulfil,
}

define_class!(
    /// One dragged file or folder: named for the Finder, fetched and placed when dropped.
    #[unsafe(super(NSObject))]
    #[name = "TableCrossDraggedFile"]
    #[ivars = FileIvars]
    struct DraggedFile;

    unsafe impl NSObjectProtocol for DraggedFile {}

    unsafe impl NSFilePromiseProviderDelegate for DraggedFile {
        #[unsafe(method_id(filePromiseProvider:fileNameForType:))]
        fn file_name(&self, _provider: &NSFilePromiseProvider, _kind: &NSString) -> Retained<NSString> {
            NSString::from_str(&self.ivars().name)
        }

        #[unsafe(method(filePromiseProvider:writePromiseToURL:completionHandler:))]
        fn write(&self, _provider: &NSFilePromiseProvider, url: &NSURL, done: &DynBlock<dyn Fn(*mut NSError)>) {
            let placed = url.path().is_some_and(|to| self.place(Path::new(&to.to_string())));
            if placed {
                done.call((std::ptr::null_mut(),));
            } else {
                let error =
                    unsafe { NSError::errorWithDomain_code_userInfo(&NSString::from_str("TableCross"), 1, None) };
                done.call((Retained::as_ptr(&error) as *mut NSError,));
            }
        }

        #[unsafe(method_id(operationQueueForFilePromiseProvider:))]
        fn queue(&self, _provider: &NSFilePromiseProvider) -> Retained<NSOperationQueue> {
            // Fetching waits for the transfer: not on the main thread
            NSOperationQueue::new()
        }
    }
);

impl DraggedFile {
    fn new(index: usize, name: String, fulfil: Fulfil) -> Retained<Self> {
        let this = Self::alloc().set_ivars(FileIvars { index, name, fulfil });
        unsafe { msg_send![super(this), init] }
    }

    /// Fetches the files (once for all of them) and moves this one to where it was dropped.
    fn place(&self, to: &Path) -> bool {
        let ivars = self.ivars();
        let Some(fetched) = (ivars.fulfil)() else {
            tracing::warn!("dropped files could not be fetched");
            return false;
        };
        let Some(from) = fetched.get(ivars.index) else {
            return false;
        };
        match move_to(from, to) {
            Ok(()) => {
                tracing::info!("dropped file placed at {}", to.display());
                true
            }
            Err(e) => {
                tracing::warn!("cannot place the dropped file at {}: {e}", to.display());
                false
            }
        }
    }
}

/// Moves a file or folder, copying where it crosses to another volume.
fn move_to(from: &Path, to: &Path) -> std::io::Result<()> {
    if std::fs::rename(from, to).is_ok() {
        return Ok(());
    }
    copy_all(from, to)?;
    let _ = if from.is_dir() {
        std::fs::remove_dir_all(from)
    } else {
        std::fs::remove_file(from)
    };
    Ok(())
}

fn copy_all(from: &Path, to: &Path) -> std::io::Result<()> {
    if from.is_dir() {
        std::fs::create_dir_all(to)?;
        for entry in std::fs::read_dir(from)? {
            let entry = entry?;
            copy_all(&entry.path(), &to.join(entry.file_name()))?;
        }
        Ok(())
    } else {
        std::fs::copy(from, to).map(|_| ())
    }
}

struct ViewIvars {
    providers: Vec<Retained<NSFilePromiseProvider>>,
    /// The providers hold their delegates weakly.
    _files: Vec<Retained<DraggedFile>>,
    started: std::cell::Cell<bool>,
}

define_class!(
    /// Catches the press and starts the drag from there.
    #[unsafe(super(NSView, objc2_app_kit::NSResponder, NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "TableCrossDragView"]
    #[ivars = ViewIvars]
    struct DragView;

    impl DragView {
        #[unsafe(method(acceptsFirstMouse:))]
        fn accepts_first_mouse(&self, _event: Option<&NSEvent>) -> bool {
            true
        }

        #[unsafe(method(mouseDown:))]
        fn mouse_down(&self, event: &NSEvent) {
            self.begin(event);
        }

        #[unsafe(method(mouseDragged:))]
        fn mouse_dragged(&self, event: &NSEvent) {
            self.begin(event);
        }
    }

    unsafe impl NSObjectProtocol for DragView {}

    unsafe impl NSDraggingSource for DragView {
        #[unsafe(method(draggingSession:sourceOperationMaskForDraggingContext:))]
        fn operations(&self, _session: &NSDraggingSession, _context: NSDraggingContext) -> NSDragOperation {
            NSDragOperation::Copy
        }

        #[unsafe(method(draggingSession:endedAtPoint:operation:))]
        fn ended(&self, _session: &NSDraggingSession, _at: NSPoint, operation: NSDragOperation) {
            tracing::info!("drag of files from another computer ended (operation {:?})", operation.0);
            finish();
        }
    }
);

impl DragView {
    fn new(mtm: MainThreadMarker, size: NSSize, files: Vec<Retained<DraggedFile>>) -> Retained<Self> {
        let kind = NSString::from_str("public.data");
        let providers = files
            .iter()
            .map(|file| {
                NSFilePromiseProvider::initWithFileType_delegate(
                    NSFilePromiseProvider::alloc(),
                    &kind,
                    ProtocolObject::from_ref(&**file),
                )
            })
            .collect();
        let this = mtm.alloc::<Self>().set_ivars(ViewIvars {
            providers,
            _files: files,
            started: std::cell::Cell::new(false),
        });
        let frame = NSRect::new(NSPoint::new(0.0, 0.0), size);
        unsafe { msg_send![super(this), initWithFrame: frame] }
    }

    fn begin(&self, event: &NSEvent) {
        if self.ivars().started.replace(true) {
            return;
        }
        let at = self.convertPoint_fromView(event.locationInWindow(), None);
        let icon = unsafe { NSImage::imageNamed(NSImageNameMultipleDocuments) };
        let items: Vec<Retained<NSDraggingItem>> = self
            .ivars()
            .providers
            .iter()
            .enumerate()
            .map(|(i, provider)| {
                let item = NSDraggingItem::initWithPasteboardWriter(
                    NSDraggingItem::alloc(),
                    ProtocolObject::from_ref(&**provider),
                );
                let offset = i.min(4) as f64 * 6.0;
                let frame = NSRect::new(
                    NSPoint::new(at.x - 24.0 + offset, at.y - 24.0 - offset),
                    NSSize::new(48.0, 48.0),
                );
                let contents: Option<&AnyObject> = icon.as_deref().map(|i| i as &AnyObject);
                unsafe { item.setDraggingFrame_contents(frame, contents) };
                item
            })
            .collect();
        tracing::info!("drag of {} file(s) from another computer starts", items.len());
        self.beginDraggingSessionWithItems_event_source(
            &NSArray::from_retained_slice(&items),
            event,
            ProtocolObject::from_ref(self),
        );
        // Out of the way, so the places below take the drop
        if let Some(window) = self.window() {
            window.setIgnoresMouseEvents(true);
            window.setAlphaValue(0.0);
        }
    }
}

thread_local! {
    /// The window of the drag going on (main thread only).
    static WINDOW: RefCell<Option<Retained<NSWindow>>> = const { RefCell::new(None) };
}

fn finish() {
    if let Some(window) = WINDOW.with(|w| w.borrow_mut().take()) {
        window.orderOut(None);
    }
}

/// Continues a drag of `names` that came over the edge: shows the catching window under the
/// pointer, then calls `press` (which presses the left button there). `fulfil` fetches the files
/// when they are dropped; `gave_up` is called when the press did not start the drag.
pub fn start(names: Vec<String>, fulfil: Fulfil, press: Box<dyn FnOnce() + Send>, gave_up: Box<dyn FnOnce() + Send>) {
    DispatchQueue::main().exec_async(move || {
        let Some(mtm) = MainThreadMarker::new() else { return };
        finish();
        let at = NSEvent::mouseLocation();
        let size = NSSize::new(CATCHER, CATCHER);
        let frame = NSRect::new(NSPoint::new(at.x - CATCHER / 2.0, at.y - CATCHER / 2.0), size);
        let files = names
            .into_iter()
            .enumerate()
            .map(|(index, name)| DraggedFile::new(index, name, fulfil.clone()))
            .collect();
        let window = unsafe {
            NSWindow::initWithContentRect_styleMask_backing_defer(
                mtm.alloc(),
                frame,
                NSWindowStyleMask::Borderless,
                NSBackingStoreType::Buffered,
                false,
            )
        };
        unsafe { window.setReleasedWhenClosed(false) };
        window.setOpaque(false);
        // Not fully clear: clicks go through fully transparent windows
        window.setBackgroundColor(Some(&NSColor::colorWithWhite_alpha(1.0, 0.02)));
        window.setHasShadow(false);
        window.setLevel(NSScreenSaverWindowLevel);
        let view = DragView::new(mtm, size, files);
        window.setContentView(Some(&view));
        window.orderFrontRegardless();
        WINDOW.with(|w| *w.borrow_mut() = Some(window));
        std::thread::spawn(move || {
            std::thread::sleep(SETTLE);
            // Under the pointer where it is now: a quick drag has moved on since the window came
            DispatchQueue::main().exec_sync(|| {
                if let Some(window) = WINDOW.with(|w| w.borrow().clone()) {
                    let at = NSEvent::mouseLocation();
                    window.setFrameOrigin(NSPoint::new(at.x - CATCHER / 2.0, at.y - CATCHER / 2.0));
                }
            });
            press();
            // The press did not reach the window (moved away meanwhile): give up
            std::thread::sleep(Duration::from_secs(3));
            DispatchQueue::main().exec_async(|| {
                let unused = WINDOW.with(|w| {
                    w.borrow()
                        .as_ref()
                        .and_then(|w| w.contentView())
                        .and_then(|v| v.downcast::<DragView>().ok())
                        .is_some_and(|v| !v.ivars().started.get())
                });
                if unused {
                    tracing::warn!("the drag from another computer did not start here");
                    finish();
                    gave_up();
                }
            });
        });
    });
}
