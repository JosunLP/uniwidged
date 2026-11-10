//! M5's exit criterion, measured on a real Windows desktop.
//!
//! > The widget stays below a maximised window, never steals focus, has no
//! > Alt-Tab entry, and shows the wallpaper through its glass.
//!
//! Each clause is one assertion here, and none of them is taken from this
//! crate's own idea of what it did — they are read back from the window
//! manager:
//!
//! * **below a maximised window**: the z-order, walked with `GW_HWNDNEXT` from
//!   an ordinary window that was maximised *after* the widget, and then again
//!   after the widget was clicked — a click is what raises a window, so the
//!   second walk is the one that matters.
//! * **never steals focus**: the foreground window, before and after a real
//!   (synthesised) click on the widget. The click must also arrive, or a
//!   window that ignored the mouse entirely would pass.
//! * **no Alt-Tab entry**: `WS_EX_TOOLWINDOW`, which is what the shell's
//!   switcher and taskbar read, and the absence of `WS_EX_APPWINDOW`.
//! * **the wallpaper through its glass**: a swapchain on this window whose
//!   alpha mode is `PreMultiplied`. An HWND swapchain only ever offers
//!   `Opaque`; this is the proof that the DirectComposition path was taken.
//!
//! It needs an interactive desktop — a GitHub Windows runner has one — and a
//! DX12 adapter, which WARP provides everywhere. Without either it skips,
//! unless `UNIWIDGED_REQUIRE_WIN32` is set, which is what CI does.

#![cfg(windows)]

use std::time::{Duration, Instant};

use uniwidged_core::geom::PhysicalRect;
use uniwidged_shell::Input;
use uniwidged_shell::platform::desktop::{self, DesktopWindow};
use windows_sys::Win32::Foundation::{HWND, LPARAM, POINT, RECT};
use windows_sys::Win32::System::Threading::GetCurrentProcessId;
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
    INPUT, INPUT_0, INPUT_MOUSE, MOUSEEVENTF_LEFTDOWN, MOUSEEVENTF_LEFTUP, MOUSEINPUT, SendInput,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DestroyWindow, EnumWindows, GA_ROOT, GW_HWNDNEXT, GWL_EXSTYLE, GetAncestor,
    GetClassNameW, GetDesktopWindow, GetForegroundWindow, GetShellWindow, GetWindow,
    GetWindowLongPtrW, GetWindowRect, GetWindowTextW, GetWindowThreadProcessId, IsIconic,
    IsWindowVisible, SW_MAXIMIZE, SW_MINIMIZE, SW_SHOWMINNOACTIVE, SW_SHOWNOACTIVATE, SetCursorPos,
    SetForegroundWindow, ShowWindow, WS_EX_APPWINDOW, WS_EX_NOACTIVATE, WS_EX_NOREDIRECTIONBITMAP,
    WS_EX_TOOLWINDOW, WS_EX_TOPMOST, WS_OVERLAPPEDWINDOW, WS_VISIBLE, WindowFromPoint,
};

fn required() -> bool {
    std::env::var_os("UNIWIDGED_REQUIRE_WIN32").is_some()
}

fn hwnd_of(window: &DesktopWindow) -> HWND {
    match window.raw_handles().1 {
        raw_window_handle::RawWindowHandle::Win32(handle) => handle.hwnd.get() as HWND,
        other => panic!("a Win32 window gave a {other:?} handle"),
    }
}

/// Pumps messages for a while, collecting what the widget reported.
fn pump(windows: &mut [DesktopWindow], for_: Duration) -> Vec<Input> {
    let until = Instant::now() + for_;
    let mut seen = Vec::new();
    while Instant::now() < until {
        desktop::wait_any(windows, Some(Duration::from_millis(20)), None).expect("a wait");
        for window in windows.iter_mut() {
            seen.extend(window.drain_events().expect("events"));
        }
    }
    seen
}

/// A window's class and title, for a failure message.
fn describe(hwnd: HWND) -> String {
    if hwnd.is_null() {
        return "none".into();
    }
    let mut class = [0u16; 256];
    let mut title = [0u16; 256];
    // SAFETY: buffers of the length given, on a window handle that may have
    // died since — both calls then return 0.
    let (c, t) = unsafe {
        (
            GetClassNameW(hwnd, class.as_mut_ptr(), class.len() as i32),
            GetWindowTextW(hwnd, title.as_mut_ptr(), title.len() as i32),
        )
    };
    format!(
        "{hwnd:?} class {:?} title {:?}",
        String::from_utf16_lossy(&class[..c.max(0) as usize]),
        String::from_utf16_lossy(&title[..t.max(0) as usize])
    )
}

/// Is nothing but the desktop itself at this point?
fn bare(x: i32, y: i32) -> bool {
    // SAFETY: plain queries; a null window is answered as such.
    unsafe {
        let at = WindowFromPoint(POINT { x, y });
        if at.is_null() {
            return true;
        }
        let root = GetAncestor(at, GA_ROOT);
        root == GetShellWindow() || root == GetDesktopWindow() || is_desktop(root)
    }
}

/// Other programs' windows this test minimised, put back when it ends —
/// including when it fails.
struct Cleared(Vec<HWND>);

impl Drop for Cleared {
    fn drop(&mut self) {
        for &hwnd in &self.0 {
            // SAFETY: a window that may have gone since; the call then fails.
            unsafe { ShowWindow(hwnd, SW_SHOWNOACTIVATE) };
        }
    }
}

/// Minimises, without activating anything, every other program's ordinary
/// window over `rect` — what "show the desktop" does, for one rectangle.
fn clear(rect: PhysicalRect) -> Cleared {
    struct Search {
        rect: RECT,
        found: Vec<HWND>,
    }
    unsafe extern "system" fn each(hwnd: HWND, data: LPARAM) -> windows_sys::core::BOOL {
        // SAFETY: `data` is the `Search` below, alive for the enumeration.
        let search = unsafe { &mut *(data as *mut Search) };
        // SAFETY: plain queries of a window the enumeration handed over.
        unsafe {
            let mut pid = 0;
            GetWindowThreadProcessId(hwnd, &mut pid);
            let style = GetWindowLongPtrW(hwnd, GWL_EXSTYLE) as u32;
            let mut at = std::mem::zeroed::<RECT>();
            let overlaps = GetWindowRect(hwnd, &mut at) != 0
                && at.left < search.rect.right
                && at.right > search.rect.left
                && at.top < search.rect.bottom
                && at.bottom > search.rect.top;
            if overlaps
                && pid != GetCurrentProcessId()
                && IsWindowVisible(hwnd) != 0
                && IsIconic(hwnd) == 0
                && style & WS_EX_TOPMOST == 0
                && hwnd != GetShellWindow()
                && !is_desktop(hwnd)
            {
                search.found.push(hwnd);
            }
        }
        1
    }
    let mut search = Search {
        rect: RECT {
            left: rect.x,
            top: rect.y,
            right: rect.x + rect.width as i32,
            bottom: rect.y + rect.height as i32,
        },
        found: Vec::new(),
    };
    // SAFETY: the callback only reads windows and writes `search`.
    unsafe { EnumWindows(Some(each), &mut search as *mut Search as LPARAM) };
    for &hwnd in &search.found {
        // SAFETY: a live top-level window; minimising activates nothing.
        unsafe { ShowWindow(hwnd, SW_SHOWMINNOACTIVE) };
    }
    Cleared(search.found)
}

/// The shell's own desktop windows, which are the wallpaper rather than
/// anything in front of it.
fn is_desktop(hwnd: HWND) -> bool {
    let mut class = [0u16; 64];
    // SAFETY: a buffer of the length given.
    let n = unsafe { GetClassNameW(hwnd, class.as_mut_ptr(), class.len() as i32) };
    let class = String::from_utf16_lossy(&class[..n.max(0) as usize]);
    class == "Progman" || class == "WorkerW"
}

/// Somewhere on the primary screen that no other window covers.
///
/// The widget sits below every ordinary window, so a click on it only lands
/// where nothing else is — and a CI runner has windows of its own open (the
/// hosted agent's terminal fills the top of the screen). If nowhere is bare,
/// the windows over the first candidate are minimised for the test's length.
/// Clear of where the test's own application window will be moved to, too.
fn uncovered(width: u32, height: u32) -> (PhysicalRect, Cleared) {
    let screen = desktop::outputs()
        .into_iter()
        .find(|o| o.primary)
        .map(|o| o.rect)
        .unwrap_or(PhysicalRect::new(0, 0, 1024, 768));
    let (w, h) = (width as i32, height as i32);
    let app = (APP.0, APP.1, APP.0 + APP.2, APP.1 + APP.3);
    let bare_rect = |x: i32, y: i32| {
        [
            (x + 4, y + 4),
            (x + w - 4, y + 4),
            (x + 4, y + h - 4),
            (x + w - 4, y + h - 4),
            (x + w / 2, y + h / 2),
        ]
        .iter()
        .all(|&(x, y)| bare(x, y))
    };
    for y in (screen.y + 40..screen.y + screen.height as i32 - h - 40).step_by(40) {
        for x in (screen.x + 40..screen.x + screen.width as i32 - w - 40).step_by(40) {
            let overlaps_app = x < app.2 && x + w > app.0 && y < app.3 && y + h > app.1;
            if !overlaps_app && bare_rect(x, y) {
                return (PhysicalRect::new(x, y, width, height), Cleared(Vec::new()));
            }
        }
    }
    let rect = PhysicalRect::new(screen.x + 40, screen.y + 40, width, height);
    let cleared = clear(rect);
    // Minimising animates; give it a moment before anything is asked of it.
    std::thread::sleep(Duration::from_millis(300));
    (rect, cleared)
}

/// Where the test's ordinary application window is moved to before the click.
const APP: (i32, i32, i32, i32) = (600, 400, 400, 300);

/// Is `lower` somewhere below `upper` in the z-order?
fn is_below(lower: HWND, upper: HWND) -> bool {
    let mut at = upper;
    for _ in 0..10_000 {
        // SAFETY: walking the z-order from a live window.
        at = unsafe { GetWindow(at, GW_HWNDNEXT) };
        if at.is_null() {
            return false;
        }
        if at == lower {
            return true;
        }
    }
    false
}

fn click(x: i32, y: i32) {
    let button = |flags| INPUT {
        r#type: INPUT_MOUSE,
        Anonymous: INPUT_0 {
            mi: MOUSEINPUT {
                dx: 0,
                dy: 0,
                mouseData: 0,
                dwFlags: flags,
                time: 0,
                dwExtraInfo: 0,
            },
        },
    };
    let inputs = [button(MOUSEEVENTF_LEFTDOWN), button(MOUSEEVENTF_LEFTUP)];
    // SAFETY: two well-formed inputs of the size given.
    unsafe {
        SetCursorPos(x, y);
        SendInput(2, inputs.as_ptr(), std::mem::size_of::<INPUT>() as i32);
    }
}

#[test]
fn a_widget_is_a_gadget_rather_than_a_window() {
    let (rect, _cleared) = uncovered(240, 160);
    let mut windows = match DesktopWindow::open(rect, "uniwidged test widget") {
        Ok(window) => vec![window],
        Err(e) if !required() => {
            eprintln!("skipping: no desktop to open a window on ({e})");
            return;
        }
        Err(e) => panic!("could not open a widget window: {e}"),
    };
    let widget = hwnd_of(&windows[0]);

    // SAFETY: plain style reads of a live window.
    let style = unsafe { GetWindowLongPtrW(widget, GWL_EXSTYLE) } as u32;
    assert_ne!(style & WS_EX_TOOLWINDOW, 0, "no taskbar or Alt-Tab entry");
    assert_eq!(
        style & WS_EX_APPWINDOW,
        0,
        "WS_EX_APPWINDOW would put it back"
    );
    assert_ne!(style & WS_EX_NOACTIVATE, 0, "a click must not activate it");
    assert_ne!(style & WS_EX_NOREDIRECTIONBITMAP, 0, "per-pixel alpha");

    // An ordinary application window, maximised over the widget and made
    // foreground — what somebody working looks like.
    let class: Vec<u16> = "STATIC\0".encode_utf16().collect();
    let title: Vec<u16> = "an application\0".encode_utf16().collect();
    // SAFETY: a system class, and strings that outlive the call.
    let app = unsafe {
        CreateWindowExW(
            0,
            class.as_ptr(),
            title.as_ptr(),
            WS_OVERLAPPEDWINDOW | WS_VISIBLE,
            0,
            0,
            800,
            600,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            std::ptr::null(),
        )
    };
    assert!(!app.is_null(), "an ordinary window");
    // SAFETY: a live window this test owns.
    unsafe {
        ShowWindow(app, SW_MAXIMIZE);
        SetForegroundWindow(app);
    }
    pump(&mut windows, Duration::from_millis(300));
    assert!(
        is_below(widget, app),
        "the widget is above a maximised window"
    );

    // Now somebody clicks where the widget would be, were it visible — and on
    // a desktop whose maximised window is minimised, it is. Raise nothing, and
    // see whether the click arrives and the foreground stays put.
    // SAFETY: as above.
    unsafe {
        ShowWindow(app, SW_MINIMIZE);
        ShowWindow(app, SW_SHOWNOACTIVATE);
    }
    // SAFETY: a plain query.
    let foreground = unsafe { GetForegroundWindow() };
    // The widget is below the restored window, so click a point the
    // restored-not-maximised application does not cover: it was created at
    // 800x600 from the origin, where the widget may well be, so move the
    // application to `APP`, which `uncovered` kept the widget clear of.
    // SAFETY: as above.
    unsafe {
        windows_sys::Win32::UI::WindowsAndMessaging::SetWindowPos(
            app,
            std::ptr::null_mut(),
            APP.0,
            APP.1,
            APP.2,
            APP.3,
            windows_sys::Win32::UI::WindowsAndMessaging::SWP_NOZORDER
                | windows_sys::Win32::UI::WindowsAndMessaging::SWP_NOACTIVATE,
        );
    }
    pump(&mut windows, Duration::from_millis(200));
    let (x, y) = (rect.x + 20, rect.y + 20);
    // The widget sits below every ordinary window, so whatever else the
    // machine has open over that point takes the click instead. Say what.
    // SAFETY: a plain query.
    let under = describe(unsafe { WindowFromPoint(POINT { x, y }) });
    click(x, y);
    let seen = pump(&mut windows, Duration::from_millis(400));
    assert!(
        seen.iter().any(|i| matches!(i, Input::Pressed { .. })),
        "the click never reached the widget: {seen:?}; the window under ({x}, {y}) was {under}"
    );
    // SAFETY: a plain query.
    assert_eq!(
        unsafe { GetForegroundWindow() },
        foreground,
        "clicking the widget took the foreground"
    );
    assert!(
        is_below(widget, app),
        "a click raised the widget above an application"
    );

    // The glass: a real swapchain on this window, and what it can blend.
    match uniwidged_render::Renderer::new() {
        Ok(mut renderer) => {
            let (display, handle) = windows[0].raw_handles();
            // SAFETY: the window outlives the surface — it is removed below,
            // before the window is dropped.
            let surface = unsafe { renderer.add_swapchain_surface(display, handle, 240, 160) }
                .expect("a swapchain on the widget window");
            assert_eq!(
                renderer.alpha_mode(surface),
                Some(uniwidged_render::wgpu::CompositeAlphaMode::PreMultiplied),
                "without a premultiplied composition swapchain there is no glass"
            );
            renderer.remove_surface(surface);
        }
        Err(e) if !required() => eprintln!("no adapter, so the glass is not measured: {e}"),
        Err(e) => panic!("no DX12 adapter: {e}"),
    }

    // SAFETY: the window this test created.
    unsafe { DestroyWindow(app) };
}
