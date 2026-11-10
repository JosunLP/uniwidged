//! The widget's window on macOS.
//!
//! Adapted from Ephemeris `src/unix/mac/window.rs`.
//!
//! * **Level** `kCGDesktopIconWindowLevel + 1` — below every ordinary window,
//!   above the desktop and its icons, and still clickable. This is the one
//!   property the widget is defined by.
//! * **`canBecomeKeyWindow` returns `NO`** — clicking never takes focus from
//!   what you are working in. Needs an `NSWindow` subclass, which is the second
//!   reason there is no windowing abstraction in this crate. The subclass is of
//!   `NSPanel`, with `NSWindowStyleMaskNonactivatingPanel`: a window that will
//!   not become key still *activates its application* when clicked, and the
//!   menu bar would change to uniwidged's for every press on a widget. A
//!   non-activating panel is the one window kind AppKit lets receive a click
//!   without that. `hidesOnDeactivate` is turned off, because a panel's
//!   default is to vanish whenever its application is not frontmost — which,
//!   for an accessory application, is always.
//! * **Collection behaviour** `.canJoinAllSpaces | .stationary | .ignoresCycle`
//!   — follows you between Spaces, does not slide away under Mission Control,
//!   never appears in the switcher.
//! * **Activation policy** `.accessory` — no Dock tile, no menu bar. Set in
//!   code so a bare binary behaves the same as a bundled `.app`.
//! * **Borderless, `opaque = NO`, `hasShadow = NO`** — the renderer draws the
//!   glass, the corners and the shadow itself, so they match the other three
//!   platforms rather than matching AppKit.
//!
//! ## What is new here, relative to Ephemeris
//!
//! Ephemeris drew with Core Graphics into the view. uniwidged makes the
//! `NSView` layer-hosting with a `CAMetalLayer` (`isOpaque = false`) and hands
//! wgpu the view, whose Metal backend finds that layer and presents into it.
//! An `NSVisualEffectView` behind the Metal view gives a real backdrop blur;
//! it is **opt-in** (`UNIWIDGED_BACKDROP=system`) for the reason the Windows
//! one is — it blurs a rectangle the theme did not choose the corners of.
//!
//! ## Coordinates
//!
//! AppKit measures screens from the bottom left of the primary display, in
//! points; the rest of this project measures from the top left, in physical
//! pixels. The conversion happens here, at the functions that touch a screen
//! rectangle, so a saved placement means the same thing on every platform.
//!
//! **One scale for the whole desktop**: the primary display's. AppKit's
//! global coordinate space is in points, and a desktop whose monitors have
//! different scales has no single "physical pixel" to measure it in. Taking the
//! primary's factor keeps the space contiguous — a widget dragged across the
//! seam does not jump — at the cost of a widget on a secondary display of a
//! different scale being rendered at the primary's density there. That is the
//! same trade X11 makes, and it is said here rather than discovered.
//!
//! ## The event loop
//!
//! There is no `-[NSApplication run]`. The desktop loop is the host's, as on
//! every other platform, and this file pumps AppKit from inside it:
//! `nextEventMatchingMask:untilDate:inMode:dequeue:` with the host's timeout,
//! then everything else already queued. A background task ends the wait by
//! posting an application-defined event — see `crate::wake` — and so does a
//! change to the displays, which AppKit reports as a notification rather than
//! an event and which would otherwise not end a wait at all.

use std::cell::{Cell, RefCell};
use std::ptr::NonNull;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use objc2::rc::Retained;
use objc2::runtime::AnyObject;
use objc2::{MainThreadMarker, MainThreadOnly, class, define_class, msg_send};
use objc2_app_kit::{
    NSApplication, NSApplicationActivationPolicy,
    NSApplicationDidChangeScreenParametersNotification, NSBackingStoreType, NSColor, NSEvent,
    NSEventMask, NSEventModifierFlags, NSEventType, NSPanel, NSResponder, NSScreen, NSView,
    NSVisualEffectBlendingMode, NSVisualEffectState, NSVisualEffectView, NSWindow,
    NSWindowCollectionBehavior, NSWindowStyleMask,
};
use objc2_foundation::{
    NSDate, NSDefaultRunLoopMode, NSNotification, NSNotificationCenter, NSObjectProtocol, NSPoint,
    NSRect, NSSize,
};
use objc2_quartz_core::CAMetalLayer;
use raw_window_handle::{
    AppKitDisplayHandle, AppKitWindowHandle, RawDisplayHandle, RawWindowHandle,
};
use uniwidged_core::geom::PhysicalRect;

use crate::{Capabilities, Input, Output, WakePipe};

/// What went wrong.
#[derive(Debug)]
pub struct DesktopError(String);

impl std::fmt::Display for DesktopError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for DesktopError {}

#[link(name = "CoreGraphics", kind = "framework")]
unsafe extern "C" {
    fn CGWindowLevelForKey(key: i32) -> i32;
}

/// `kCGDesktopIconWindowLevelKey`.
const DESKTOP_ICON_LEVEL_KEY: i32 = 18;

/// Set once the application object exists, so that [`post_wake`] — which runs
/// on whatever thread finished a task — never creates it from the wrong one.
static APP_READY: AtomicBool = AtomicBool::new(false);

thread_local! {
    /// Everything the views have seen and the loop has not.
    static QUEUE: RefCell<Vec<Input>> = const { RefCell::new(Vec::new()) };
    /// The primary display's backing scale. See the module notes.
    static SCALE: Cell<f64> = const { Cell::new(1.0) };
    /// The primary display's height in points: where AppKit's y axis starts.
    static PRIMARY_HEIGHT: Cell<f64> = const { Cell::new(0.0) };
}

fn push(input: Input) {
    QUEUE.with(|queue| queue.borrow_mut().push(input));
}

define_class!(
    // SAFETY: `NSPanel` has no subclassing requirements beyond these two
    // overrides, and this class implements no `Drop`.
    #[unsafe(super(NSPanel, NSWindow, NSResponder, objc2_foundation::NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "UniwidgedPanel"]
    struct WidgetPanel;

    impl WidgetPanel {
        #[unsafe(method(canBecomeKeyWindow))]
        fn can_become_key_window(&self) -> bool {
            false
        }

        #[unsafe(method(canBecomeMainWindow))]
        fn can_become_main_window(&self) -> bool {
            false
        }
    }

    unsafe impl NSObjectProtocol for WidgetPanel {}
);

define_class!(
    // SAFETY: `NSView` may be subclassed freely; no `Drop`.
    #[unsafe(super(NSView, NSResponder, objc2_foundation::NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "UniwidgedView"]
    struct WidgetView;

    impl WidgetView {
        /// Top-left origin, like every other coordinate in this project.
        #[unsafe(method(isFlipped))]
        fn is_flipped(&self) -> bool {
            true
        }

        /// A click on a window that is not key is delivered rather than
        /// spent on making it key — which this window never becomes.
        #[unsafe(method(acceptsFirstMouse:))]
        fn accepts_first_mouse(&self, _event: Option<&NSEvent>) -> bool {
            true
        }

        #[unsafe(method(mouseDown:))]
        fn mouse_down(&self, event: &NSEvent) {
            let point = self.convertPoint_fromView(event.locationInWindow(), None);
            let scale = SCALE.with(Cell::get);
            push(Input::Pressed {
                window: self.window_number(),
                local: ((point.x * scale) as i32, (point.y * scale) as i32),
                root: pointer(),
            });
        }

        /// Delivered to this view until the button comes up, wherever the
        /// pointer goes — AppKit's implicit grab, which is why
        /// [`DesktopWindow::grab_pointer`] has nothing to do.
        #[unsafe(method(mouseDragged:))]
        fn mouse_dragged(&self, _event: &NSEvent) {
            push(Input::Motion {
                window: self.window_number(),
                root: pointer(),
            });
        }

        #[unsafe(method(mouseUp:))]
        fn mouse_up(&self, _event: &NSEvent) {
            push(Input::Released {
                window: self.window_number(),
                root: pointer(),
            });
        }
    }

    unsafe impl NSObjectProtocol for WidgetView {}
);

impl WidgetView {
    fn new(mtm: MainThreadMarker, frame: NSRect) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(());
        // SAFETY: `initWithFrame:` is `NSView`'s designated initialiser.
        unsafe { msg_send![super(this), initWithFrame: frame] }
    }

    fn window_number(&self) -> u32 {
        self.window().map(|w| w.windowNumber() as u32).unwrap_or(0)
    }
}

/// Where the pointer is, in the desktop's top-left physical coordinates.
fn pointer() -> (i32, i32) {
    let at = NSEvent::mouseLocation();
    let scale = SCALE.with(Cell::get);
    let height = PRIMARY_HEIGHT.with(Cell::get);
    ((at.x * scale) as i32, ((height - at.y) * scale) as i32)
}

/// A rectangle in this project's coordinates, as AppKit's.
fn to_appkit(rect: PhysicalRect) -> NSRect {
    let scale = SCALE.with(Cell::get);
    let height = PRIMARY_HEIGHT.with(Cell::get);
    let w = rect.width.max(1) as f64 / scale;
    let h = rect.height.max(1) as f64 / scale;
    NSRect::new(
        NSPoint::new(rect.x as f64 / scale, height - rect.y as f64 / scale - h),
        NSSize::new(w, h),
    )
}

/// An AppKit rectangle, in this project's coordinates.
fn from_appkit(frame: NSRect) -> PhysicalRect {
    let scale = SCALE.with(Cell::get);
    let height = PRIMARY_HEIGHT.with(Cell::get);
    PhysicalRect::new(
        (frame.origin.x * scale).round() as i32,
        ((height - frame.origin.y - frame.size.height) * scale).round() as i32,
        (frame.size.width * scale).round().max(0.0) as u32,
        (frame.size.height * scale).round().max(0.0) as u32,
    )
}

/// Reads the primary display's height and scale, which every conversion uses.
fn refresh_metrics(mtm: MainThreadMarker) {
    let screens = NSScreen::screens(mtm);
    if let Some(primary) = screens.firstObject() {
        SCALE.with(|s| s.set(primary.backingScaleFactor().max(1.0)));
        PRIMARY_HEIGHT.with(|h| h.set(primary.frame().size.height));
    }
}

/// Makes this process an accessory application, once, and starts listening
/// for the displays changing.
fn application(mtm: MainThreadMarker) -> Retained<NSApplication> {
    let app = NSApplication::sharedApplication(mtm);
    if !APP_READY.load(Ordering::Acquire) {
        app.setActivationPolicy(NSApplicationActivationPolicy::Accessory);
        app.finishLaunching();
        refresh_metrics(mtm);
        let block = block2::RcBlock::new(|_: NonNull<NSNotification>| {
            push(Input::OutputsChanged);
            // A notification is not an event, so it would not end the wait the
            // loop is sitting in. This does.
            post_wake();
        });
        // SAFETY: the main thread's default centre, with a block that only
        // touches this thread's queue. The observer lives for the process —
        // a desktop never stops caring whether a monitor was unplugged — so
        // its token is deliberately leaked.
        let token = unsafe {
            NSNotificationCenter::defaultCenter().addObserverForName_object_queue_usingBlock(
                Some(NSApplicationDidChangeScreenParametersNotification),
                None,
                None,
                &block,
            )
        };
        std::mem::forget(token);
        APP_READY.store(true, Ordering::Release);
    }
    app
}

/// Ends the loop's current wait, from any thread.
///
/// Posting an event is the one thing `NSApplication` documents as safe from a
/// thread other than the main one. The event is application-defined and
/// carries nothing: the loop knows to look at its tasks because it woke, not
/// because of what woke it.
pub fn post_wake() {
    if !APP_READY.load(Ordering::Acquire) {
        return;
    }
    // SAFETY: class methods on `NSEvent` are thread-safe, and
    // `-postEvent:atStart:` is documented as callable from subthreads; the
    // application object already exists (checked above), so asking for it
    // creates nothing.
    unsafe {
        let Some(event) = NSEvent::otherEventWithType_location_modifierFlags_timestamp_windowNumber_context_subtype_data1_data2(
            NSEventType::ApplicationDefined,
            NSPoint::new(0.0, 0.0),
            NSEventModifierFlags(0),
            0.0,
            0,
            None,
            0,
            0,
            0,
        ) else {
            return;
        };
        let app: *mut AnyObject = msg_send![class!(NSApplication), sharedApplication];
        let _: () = msg_send![app, postEvent: &*event, atStart: false];
    }
}

/// One widget's window.
pub struct DesktopWindow {
    panel: Retained<WidgetPanel>,
    view: Retained<WidgetView>,
    rect: PhysicalRect,
    backdrop: bool,
}

impl DesktopWindow {
    /// Creates the panel at desktop-icon level, without activating anything.
    pub fn open(rect: PhysicalRect, title: &str) -> Result<Self, DesktopError> {
        let mtm = MainThreadMarker::new().ok_or_else(|| {
            DesktopError("an AppKit window can only be made on the main thread".into())
        })?;
        let _app = application(mtm);
        let frame = to_appkit(rect);

        let panel = WidgetPanel::alloc(mtm).set_ivars(());
        // SAFETY: `NSPanel`'s designated initialiser, with a style this class
        // supports.
        let panel: Retained<WidgetPanel> = unsafe {
            msg_send![
                super(panel),
                initWithContentRect: frame,
                styleMask: NSWindowStyleMask::Borderless | NSWindowStyleMask::NonactivatingPanel,
                backing: NSBackingStoreType::Buffered,
                defer: false
            ]
        };
        // SAFETY: a plain query of a system constant.
        let level = unsafe { CGWindowLevelForKey(DESKTOP_ICON_LEVEL_KEY) } as isize + 1;
        panel.setLevel(level);
        panel.setCollectionBehavior(
            NSWindowCollectionBehavior::CanJoinAllSpaces
                | NSWindowCollectionBehavior::Stationary
                | NSWindowCollectionBehavior::IgnoresCycle,
        );
        panel.setOpaque(false);
        panel.setBackgroundColor(Some(&NSColor::clearColor()));
        panel.setHasShadow(false);
        panel.setHidesOnDeactivate(false);
        panel.setTitle(&objc2_foundation::NSString::from_str(title));
        // SAFETY: the panel is owned by this struct through `Retained`, so
        // AppKit must not release it a second time on close.
        unsafe { panel.setReleasedWhenClosed(false) };

        let bounds = NSRect::new(NSPoint::new(0.0, 0.0), frame.size);
        let view = WidgetView::new(mtm, bounds);
        let layer = CAMetalLayer::new();
        layer.setOpaque(false);
        layer.setContentsScale(SCALE.with(Cell::get));
        // Layer-*hosting*: the layer is set before `wantsLayer`, so AppKit
        // leaves it alone rather than replacing it with one of its own.
        view.setLayer(Some(&layer));
        view.setWantsLayer(true);

        let backdrop = std::env::var("UNIWIDGED_BACKDROP").is_ok_and(|v| v == "system");
        if backdrop {
            let effect = NSVisualEffectView::initWithFrame(NSVisualEffectView::alloc(mtm), bounds);
            effect.setBlendingMode(NSVisualEffectBlendingMode::BehindWindow);
            effect.setState(NSVisualEffectState::Active);
            effect.addSubview(&view);
            panel.setContentView(Some(&effect));
        } else {
            panel.setContentView(Some(&view));
        }
        panel.orderFrontRegardless();

        Ok(DesktopWindow {
            panel,
            view,
            rect,
            backdrop,
        })
    }

    /// One line for the start-up log.
    pub fn describe(&self) -> String {
        let what = if self.backdrop {
            "macOS (desktop-icon level, CAMetalLayer, vibrancy)"
        } else {
            "macOS (desktop-icon level, CAMetalLayer)"
        };
        match self.capabilities().degradation() {
            Some(lost) => format!("{what}; degraded: {lost}"),
            None => what.to_string(),
        }
    }

    pub fn capabilities(&self) -> Capabilities {
        Capabilities {
            backdrop_blur: self.backdrop,
            ..Capabilities::FULL
        }
    }

    /// The window server always composites.
    pub fn has_compositor(&self) -> bool {
        true
    }

    pub fn window_id(&self) -> u32 {
        self.panel.windowNumber() as u32
    }

    pub fn scale(&self) -> f64 {
        SCALE.with(Cell::get)
    }

    pub fn raw_handles(&self) -> (RawDisplayHandle, RawWindowHandle) {
        let view = NonNull::from(&*self.view).cast();
        (
            RawDisplayHandle::AppKit(AppKitDisplayHandle::new()),
            RawWindowHandle::AppKit(AppKitWindowHandle::new(view)),
        )
    }

    pub fn geometry(&self) -> PhysicalRect {
        self.rect
    }

    pub fn set_geometry(&mut self, rect: PhysicalRect) -> Result<(), DesktopError> {
        let frame = to_appkit(rect);
        self.panel.setFrame_display(frame, true);
        self.view
            .setFrameSize(NSSize::new(frame.size.width, frame.size.height));
        self.rect = rect;
        Ok(())
    }

    /// The bounding box of every display.
    pub fn desktop_bounds(&self) -> PhysicalRect {
        bounds_of(&outputs()).unwrap_or(self.rect)
    }

    pub fn outputs(&self) -> Vec<Output> {
        outputs()
    }

    /// AppKit grabs implicitly for the duration of a press.
    pub fn grab_pointer(&self) -> Result<(), DesktopError> {
        Ok(())
    }

    pub fn release_pointer(&self) -> Result<(), DesktopError> {
        Ok(())
    }

    /// Everything every view has reported: one thread, one queue, as on
    /// Windows.
    pub fn drain_events(&mut self) -> Result<Vec<Input>, DesktopError> {
        Ok(QUEUE.with(|queue| std::mem::take(&mut *queue.borrow_mut())))
    }

    pub fn take_resize(&mut self) -> Option<(u32, u32)> {
        None
    }
}

impl Drop for DesktopWindow {
    fn drop(&mut self) {
        self.panel.close();
    }
}

fn bounds_of(outputs: &[Output]) -> Option<PhysicalRect> {
    let first = outputs.first()?.rect;
    let (mut left, mut top) = (first.x, first.y);
    let (mut right, mut bottom) = (first.x + first.width as i32, first.y + first.height as i32);
    for output in outputs {
        let r = output.rect;
        left = left.min(r.x);
        top = top.min(r.y);
        right = right.max(r.x + r.width as i32);
        bottom = bottom.max(r.y + r.height as i32);
    }
    Some(PhysicalRect::new(
        left,
        top,
        (right - left).max(1) as u32,
        (bottom - top).max(1) as u32,
    ))
}

/// Every display, by its localised name.
///
/// Two identical monitors have the same name, and a placement stored against
/// a name that matches either is a placement that lands on the wrong one half
/// the time. So a repeated name gets a number — `LG UltraFine (2)` — in the
/// order AppKit lists them, which is the order of their positions and does not
/// change when nothing is plugged in or out.
pub fn outputs() -> Vec<Output> {
    let Some(mtm) = MainThreadMarker::new() else {
        return Vec::new();
    };
    refresh_metrics(mtm);
    let mut list: Vec<Output> = Vec::new();
    for (index, screen) in NSScreen::screens(mtm).iter().enumerate() {
        let base = screen.localizedName().to_string();
        let seen = list
            .iter()
            .filter(|o| o.name == base || o.name.starts_with(&format!("{base} (")))
            .count();
        let name = if seen == 0 {
            base
        } else {
            format!("{base} ({})", seen + 1)
        };
        list.push(Output {
            name,
            rect: from_appkit(screen.frame()),
            primary: index == 0,
        });
    }
    list
}

/// Blocks until AppKit has an event, a background task finishes, or the
/// timeout elapses — then dispatches everything waiting.
pub fn wait_any(
    _windows: &mut [DesktopWindow],
    timeout: Option<Duration>,
    wake: Option<&WakePipe>,
) -> Result<(), DesktopError> {
    let mtm = MainThreadMarker::new()
        .ok_or_else(|| DesktopError("the AppKit loop runs on the main thread".into()))?;
    if QUEUE.with(|queue| !queue.borrow().is_empty()) {
        return Ok(());
    }
    let app = application(mtm);
    let until = match timeout {
        Some(d) => NSDate::dateWithTimeIntervalSinceNow(d.as_secs_f64()),
        None => NSDate::distantFuture(),
    };
    // SAFETY: a constant AppKit exports for exactly this argument.
    let mode = unsafe { NSDefaultRunLoopMode };
    let mut next = app.nextEventMatchingMask_untilDate_inMode_dequeue(
        NSEventMask::Any,
        Some(&until),
        mode,
        true,
    );
    let past = NSDate::distantPast();
    while let Some(event) = next {
        // A wake-up is only a wake-up; there is nothing for a window to do
        // with it.
        if event.r#type() != NSEventType::ApplicationDefined {
            app.sendEvent(&event);
        }
        next = app.nextEventMatchingMask_untilDate_inMode_dequeue(
            NSEventMask::Any,
            Some(&past),
            mode,
            true,
        );
    }
    if let Some(wake) = wake {
        wake.drain();
    }
    Ok(())
}
