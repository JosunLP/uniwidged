//! M6's exit criterion, measured on a real macOS session.
//!
//! > As M5 — the widget stays below a maximised window, never steals focus,
//! > has no Alt-Tab entry, and shows the wallpaper through its glass — plus:
//! > the widget follows the user across Spaces and does not slide away under
//! > Mission Control.
//!
//! Read back from AppKit and the window server rather than from this crate's
//! own bookkeeping:
//!
//! * **below a window**: the window server's answer to "which window is
//!   frontmost at this point", with an ordinary titled window covering the
//!   widget. A level that was set but not honoured fails here.
//! * **never steals focus**: the panel refuses to become key, is a
//!   non-activating panel (so a click does not activate the application
//!   either), and the application is an accessory.
//! * **no switcher entry**: `.ignoresCycle`, and the accessory policy, which
//!   is what keeps it out of ⌘-Tab and the Dock.
//! * **across Spaces, through Mission Control**: `.canJoinAllSpaces` and
//!   `.stationary` on the panel as AppKit reports it back.
//! * **the wallpaper through its glass**: the window is not opaque, and a
//!   swapchain on its view is not `Opaque` either.
//!
//! Not run by libtest: AppKit has to be driven from the main thread, and
//! libtest runs every test on a thread of its own. So this is a plain `main`
//! (`harness = false` in `Cargo.toml`), which does nothing at all off macOS.
//! Without a window server it skips, unless `UNIWIDGED_REQUIRE_APPKIT` is set,
//! which is what CI does.

#[cfg(not(target_os = "macos"))]
fn main() {}

#[cfg(target_os = "macos")]
fn main() {
    use std::time::Duration;

    use objc2::{MainThreadMarker, MainThreadOnly};
    use objc2_app_kit::{
        NSApplication, NSApplicationActivationPolicy, NSBackingStoreType, NSWindow,
        NSWindowCollectionBehavior, NSWindowStyleMask,
    };
    use objc2_foundation::{NSPoint, NSRect, NSSize};
    use uniwidged_core::geom::PhysicalRect;
    use uniwidged_shell::platform::desktop::{self, DesktopWindow};

    let required = std::env::var_os("UNIWIDGED_REQUIRE_APPKIT").is_some();
    let mtm = MainThreadMarker::new().expect("a harness-less test runs on the main thread");

    let rect = PhysicalRect::new(80, 80, 240, 160);
    let mut windows = match DesktopWindow::open(rect, "uniwidged test widget") {
        Ok(window) => vec![window],
        Err(e) if !required => {
            eprintln!("skipping: no window server ({e})");
            return;
        }
        Err(e) => panic!("could not open a widget window: {e}"),
    };
    let number = windows[0].window_id() as isize;
    let app = NSApplication::sharedApplication(mtm);
    let panel = app
        .windowWithWindowNumber(number)
        .expect("AppKit knows the widget's window");

    assert_eq!(
        app.activationPolicy(),
        NSApplicationActivationPolicy::Accessory,
        "no Dock tile, no menu bar, no ⌘-Tab"
    );
    assert!(!panel.canBecomeKeyWindow(), "the widget must never be key");
    assert!(
        panel
            .styleMask()
            .contains(NSWindowStyleMask::NonactivatingPanel),
        "a click would activate the application"
    );
    assert!(!panel.hidesOnDeactivate(), "it would vanish at once");
    let behaviour = panel.collectionBehavior();
    for (flag, what) in [
        (NSWindowCollectionBehavior::CanJoinAllSpaces, "every Space"),
        (NSWindowCollectionBehavior::Stationary, "Mission Control"),
        (NSWindowCollectionBehavior::IgnoresCycle, "the window cycle"),
    ] {
        assert!(behaviour.contains(flag), "missing the behaviour for {what}");
    }
    assert!(
        panel.level() < 0,
        "level {} is not below ordinary windows (0)",
        panel.level()
    );
    assert!(!panel.isOpaque(), "an opaque window has no glass");

    // An ordinary window over the widget, as somebody working has.
    // SAFETY: `NSWindow`'s designated initialiser with an ordinary style.
    let ordinary = unsafe {
        NSWindow::initWithContentRect_styleMask_backing_defer(
            NSWindow::alloc(mtm),
            NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(1200.0, 900.0)),
            NSWindowStyleMask::Titled,
            NSBackingStoreType::Buffered,
            false,
        )
    };
    // SAFETY: owned by `ordinary`, which outlives this function's use of it.
    unsafe { ordinary.setReleasedWhenClosed(false) };
    ordinary.orderFrontRegardless();
    for _ in 0..10 {
        desktop::wait_any(&mut windows, Some(Duration::from_millis(30)), None).expect("a wait");
    }
    let frame = panel.frame();
    let middle = NSPoint::new(
        frame.origin.x + frame.size.width / 2.0,
        frame.origin.y + frame.size.height / 2.0,
    );
    let frontmost = NSWindow::windowNumberAtPoint_belowWindowWithWindowNumber(middle, 0, mtm);
    assert_ne!(
        frontmost, number,
        "the widget is the frontmost window at its own centre, over an ordinary window"
    );
    ordinary.close();

    match uniwidged_render::Renderer::new() {
        Ok(mut renderer) => {
            let (display, handle) = windows[0].raw_handles();
            // SAFETY: the window outlives the surface, which is removed first.
            let surface = unsafe { renderer.add_swapchain_surface(display, handle, 240, 160) }
                .expect("a swapchain on the widget's view");
            assert_ne!(
                renderer.alpha_mode(surface),
                Some(uniwidged_render::wgpu::CompositeAlphaMode::Opaque),
                "an opaque layer has no glass"
            );
            renderer.remove_surface(surface);
        }
        Err(e) if !required => eprintln!("no adapter, so the glass is not measured: {e}"),
        Err(e) => panic!("no Metal adapter: {e}"),
    }
    println!("appkit_desktop: ok");
}
