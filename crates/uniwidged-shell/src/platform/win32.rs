//! The widget's window on Windows.
//!
//! Adapted from Ephemeris `src/win/window.rs`.
//!
//! It behaves like a Vista gadget, through five specific flags:
//!
//! * `WS_EX_NOREDIRECTIONBITMAP` — the prerequisite for a DirectComposition
//!   surface with true per-pixel alpha. Must be set at `CreateWindowEx` time;
//!   it cannot be added afterwards, which is one of the reasons this crate
//!   does not use a windowing abstraction.
//! * `WS_EX_TOOLWINDOW` — no entry in the taskbar or in Alt-Tab.
//! * `WS_EX_NOACTIVATE` — clicking the widget never takes focus away from the
//!   application you are working in. This is exactly what separates a gadget
//!   from a window in the way. `WM_MOUSEACTIVATE` answers `MA_NOACTIVATE` as
//!   well, because the style alone still lets a click activate a window whose
//!   thread has none in the foreground.
//! * `HWND_BOTTOM`, forced in `WM_WINDOWPOSCHANGING` — below every ordinary
//!   window, above the desktop, and therefore still clickable. Forced on
//!   *every* change rather than set once, because the shell raises a window it
//!   has just been told about, and a click raises one too.
//! * `WS_POPUP` without `WS_THICKFRAME` — the system provides neither dragging
//!   nor resizing, so the widget provides its own.
//!
//! ## What is new here, relative to Ephemeris
//!
//! Ephemeris rendered with Direct2D into a DirectComposition surface it built
//! by hand. uniwidged lets wgpu's DX12 backend build the composition target
//! instead — `Dx12SwapchainKind::DxgiFromVisual`, set in `uniwidged-render` —
//! so the window only has to exist with the right flags, and Vello renders into
//! a composition swapchain that DWM blends per-pixel against the wallpaper.
//! That is the whole reason this file hands wgpu a plain `Win32WindowHandle`:
//! the visual is wgpu's, made for exactly this window.
//!
//! A backdrop is nearly free here: `DwmSetWindowAttribute` with
//! `DWMWA_SYSTEMBACKDROP_TYPE` gives the window an acrylic backdrop for one
//! call. It is **opt-in** (`UNIWIDGED_BACKDROP=system`) because it blurs the
//! window's *rectangle*, rounded by DWM to its own radius rather than the
//! theme's, and a panel whose corners disagree with its blur is worse than one
//! without. Where it is asked for and taken, [`Capabilities::backdrop_blur`]
//! says so.
//!
//! DPI is per-monitor v2 (`SetProcessDpiAwarenessContext`), so every
//! coordinate this file sees or gives out is in physical pixels, which is what
//! the rest of the project measures in.
//!
//! ## One thread, one queue
//!
//! Unlike X11 and Wayland, where every window carries its own connection,
//! Win32 delivers every window's messages to the thread that created it. So
//! there is one queue here, filled by the window procedure, and
//! [`DesktopWindow::drain_events`] on *any* window empties all of it. The
//! desktop loop drains every window and concatenates the results, so the
//! events arrive exactly once whichever window is asked first.
//!
//! ## What is not handled
//!
//! *Show desktop* (Win+D) puts the desktop above every window, this one
//! included, until something is restored — the same thing it does to every
//! gadget that has ever existed, and something a widget cannot fix without
//! parenting itself into the shell's `WorkerW`, which is undocumented and
//! breaks between builds. Said here so that nobody files it as a bug in
//! z-ordering: the z-ordering is what keeps the widget *below* windows.

use std::cell::RefCell;
use std::num::NonZeroIsize;
use std::sync::OnceLock;
use std::time::Duration;

use raw_window_handle::{
    RawDisplayHandle, RawWindowHandle, Win32WindowHandle, WindowsDisplayHandle,
};
use uniwidged_core::geom::PhysicalRect;
use windows_sys::Win32::Foundation::{
    GetLastError, HANDLE, HINSTANCE, HWND, LPARAM, LRESULT, POINT, RECT, WAIT_FAILED, WPARAM,
};
use windows_sys::Win32::Graphics::Dwm::{DwmExtendFrameIntoClientArea, DwmSetWindowAttribute};
use windows_sys::Win32::Graphics::Gdi::{
    ClientToScreen, EnumDisplayMonitors, GetMonitorInfoW, HDC, HMONITOR, MONITORINFO,
    MONITORINFOEXW, ValidateRect,
};
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::UI::Controls::MARGINS;
use windows_sys::Win32::UI::HiDpi::{
    DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2, GetDpiForWindow, SetProcessDpiAwarenessContext,
};
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{ReleaseCapture, SetCapture};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    CS_HREDRAW, CS_VREDRAW, CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW,
    GetSystemMetrics, HWND_BOTTOM, IDC_ARROW, LoadCursorW, MA_NOACTIVATE, MSG, MWMO_INPUTAVAILABLE,
    MsgWaitForMultipleObjectsEx, PM_REMOVE, PeekMessageW, QS_ALLINPUT, RegisterClassExW,
    SM_CXVIRTUALSCREEN, SM_CYVIRTUALSCREEN, SM_XVIRTUALSCREEN, SM_YVIRTUALSCREEN,
    SW_SHOWNOACTIVATE, SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE, SWP_NOZORDER, SetWindowPos,
    ShowWindow, TranslateMessage, WINDOWPOS, WM_CLOSE, WM_DISPLAYCHANGE, WM_DPICHANGED,
    WM_ERASEBKGND, WM_LBUTTONDOWN, WM_LBUTTONUP, WM_MOUSEACTIVATE, WM_MOUSEMOVE, WM_PAINT,
    WM_WINDOWPOSCHANGING, WNDCLASSEXW, WS_EX_NOACTIVATE, WS_EX_NOREDIRECTIONBITMAP,
    WS_EX_TOOLWINDOW, WS_POPUP,
};

use crate::{Capabilities, Input, Output, WakePipe};

/// What went wrong, in the words Windows used.
#[derive(Debug)]
pub struct DesktopError(String);

impl DesktopError {
    fn last(what: &str) -> Self {
        // SAFETY: reads the calling thread's last-error value.
        let code = unsafe { GetLastError() };
        DesktopError(format!(
            "{what} failed: {}",
            std::io::Error::from_raw_os_error(code as i32)
        ))
    }
}

impl std::fmt::Display for DesktopError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for DesktopError {}

/// `MK_LBUTTON` in a mouse message's `wParam`.
const MK_LBUTTON: usize = 0x0001;
/// `DWMWA_WINDOW_CORNER_PREFERENCE` and `DWMWCP_ROUND`.
const DWMWA_WINDOW_CORNER_PREFERENCE: u32 = 33;
const DWMWCP_ROUND: i32 = 2;
/// `DWMWA_SYSTEMBACKDROP_TYPE` and `DWMSBT_TRANSIENTWINDOW` (acrylic).
const DWMWA_SYSTEMBACKDROP_TYPE: u32 = 38;
const DWMSBT_TRANSIENTWINDOW: i32 = 3;

thread_local! {
    /// Everything the window procedure has seen and the loop has not.
    static QUEUE: RefCell<Vec<Input>> = const { RefCell::new(Vec::new()) };
}

fn push(input: Input) {
    QUEUE.with(|queue| queue.borrow_mut().push(input));
}

/// The low 32 bits of a window handle, which is all a handle uses: Windows
/// guarantees `HWND`s fit in 32 bits so that 32- and 64-bit processes can share
/// them. That makes it a stable `u32` for [`Input`] to carry.
fn id_of(hwnd: HWND) -> u32 {
    hwnd as usize as u32
}

/// The signed coordinates packed into a mouse message's `lParam`.
fn point_of(lparam: LPARAM) -> (i32, i32) {
    let x = (lparam & 0xffff) as u16 as i16 as i32;
    let y = ((lparam >> 16) & 0xffff) as u16 as i16 as i32;
    (x, y)
}

fn to_screen(hwnd: HWND, local: (i32, i32)) -> (i32, i32) {
    let mut point = POINT {
        x: local.0,
        y: local.1,
    };
    // SAFETY: a live window and a local point.
    unsafe { ClientToScreen(hwnd, &mut point) };
    (point.x, point.y)
}

unsafe extern "system" fn window_proc(
    hwnd: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    let window = id_of(hwnd);
    match message {
        WM_WINDOWPOSCHANGING => {
            // Whatever asked for this change — the shell raising a window it
            // was just told about, a click, another program's `SetWindowPos`
            // — the answer about z-order is the same one.
            // SAFETY: for this message `lParam` is a `WINDOWPOS` the system
            // owns for the duration of the call.
            let pos = unsafe { &mut *(lparam as *mut WINDOWPOS) };
            pos.hwndInsertAfter = HWND_BOTTOM;
            pos.flags &= !SWP_NOZORDER;
            0
        }
        WM_MOUSEACTIVATE => MA_NOACTIVATE as LRESULT,
        WM_LBUTTONDOWN => {
            let local = point_of(lparam);
            push(Input::Pressed {
                window,
                local,
                root: to_screen(hwnd, local),
            });
            0
        }
        WM_MOUSEMOVE if wparam & MK_LBUTTON != 0 => {
            push(Input::Motion {
                window,
                root: to_screen(hwnd, point_of(lparam)),
            });
            0
        }
        WM_LBUTTONUP => {
            push(Input::Released {
                window,
                root: to_screen(hwnd, point_of(lparam)),
            });
            0
        }
        WM_PAINT => {
            // The pixels are wgpu's, so there is nothing to paint here — only
            // the region to validate, or this message arrives forever.
            // SAFETY: a live window; a null rectangle means all of it.
            unsafe { ValidateRect(hwnd, std::ptr::null()) };
            push(Input::Exposed { window });
            0
        }
        // The background is the swapchain's. Erasing it would flash.
        WM_ERASEBKGND => 1,
        // A monitor arrived, left, moved or changed resolution — or this
        // window crossed onto one with a different scale. Both mean "ask
        // again", which is what this event means everywhere.
        WM_DISPLAYCHANGE | WM_DPICHANGED => {
            push(Input::OutputsChanged);
            0
        }
        WM_CLOSE => {
            push(Input::Closed { window });
            0
        }
        // SAFETY: the system's default for everything else.
        _ => unsafe { DefWindowProcW(hwnd, message, wparam, lparam) },
    }
}

fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain(std::iter::once(0)).collect()
}

fn instance() -> HINSTANCE {
    // SAFETY: a null name means this executable's own module.
    unsafe { GetModuleHandleW(std::ptr::null()) }
}

/// Makes the process per-monitor aware, once.
///
/// Before the first window *and* before the first question about monitors:
/// the desktop asks for the monitors before it opens anything, and a process
/// that is not yet aware is answered in virtualised logical pixels — a
/// 2560×1440 screen at 150 % reads as 1707×960, and every placement resolved
/// and saved against that is wrong.
fn dpi_aware() {
    static ONCE: OnceLock<()> = OnceLock::new();
    ONCE.get_or_init(|| {
        // SAFETY: a process-wide setting. It fails only if something already
        // set it — a manifest, typically — which is the same answer.
        unsafe { SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2) };
    });
}

/// Registers the class once, after making the process DPI-aware.
fn class() -> Result<*const u16, DesktopError> {
    static CLASS: OnceLock<Result<Vec<u16>, String>> = OnceLock::new();
    let registered = CLASS.get_or_init(|| {
        dpi_aware();
        let name = wide("uniwidged.widget");
        let class = WNDCLASSEXW {
            cbSize: std::mem::size_of::<WNDCLASSEXW>() as u32,
            style: CS_HREDRAW | CS_VREDRAW,
            lpfnWndProc: Some(window_proc),
            cbClsExtra: 0,
            cbWndExtra: 0,
            hInstance: instance(),
            hIcon: std::ptr::null_mut(),
            // SAFETY: a system cursor; no module.
            hCursor: unsafe { LoadCursorW(std::ptr::null_mut(), IDC_ARROW) },
            // No brush: the background is the swapchain's.
            hbrBackground: std::ptr::null_mut(),
            lpszMenuName: std::ptr::null(),
            lpszClassName: name.as_ptr(),
            hIconSm: std::ptr::null_mut(),
        };
        // SAFETY: a fully initialised class whose name outlives the process
        // (it is kept in this `OnceLock`).
        if unsafe { RegisterClassExW(&class) } == 0 {
            return Err(DesktopError::last("RegisterClassExW").0);
        }
        Ok(name)
    });
    match registered {
        Ok(name) => Ok(name.as_ptr()),
        Err(e) => Err(DesktopError(e.clone())),
    }
}

/// One widget's window.
pub struct DesktopWindow {
    hwnd: HWND,
    rect: PhysicalRect,
    backdrop: bool,
}

impl DesktopWindow {
    /// Creates the window, below every other, without taking focus.
    pub fn open(rect: PhysicalRect, title: &str) -> Result<Self, DesktopError> {
        let class = class()?;
        let title = wide(title);
        // SAFETY: a registered class, a title that outlives the call, and no
        // parent, menu or creation parameter.
        let hwnd = unsafe {
            CreateWindowExW(
                WS_EX_NOREDIRECTIONBITMAP | WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE,
                class,
                title.as_ptr(),
                WS_POPUP,
                rect.x,
                rect.y,
                rect.width.max(1) as i32,
                rect.height.max(1) as i32,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                instance(),
                std::ptr::null(),
            )
        };
        if hwnd.is_null() {
            return Err(DesktopError::last("CreateWindowExW"));
        }
        let backdrop = std::env::var("UNIWIDGED_BACKDROP").is_ok_and(|v| v == "system")
            && apply_backdrop(hwnd);
        // SAFETY: the window was just created.
        unsafe {
            ShowWindow(hwnd, SW_SHOWNOACTIVATE);
            SetWindowPos(
                hwnd,
                HWND_BOTTOM,
                0,
                0,
                0,
                0,
                SWP_NOACTIVATE | SWP_NOMOVE | SWP_NOSIZE,
            );
        }
        Ok(DesktopWindow {
            hwnd,
            rect,
            backdrop,
        })
    }

    /// One line for the start-up log.
    pub fn describe(&self) -> String {
        let what = if self.backdrop {
            "Windows (DirectComposition, HWND_BOTTOM, acrylic backdrop)"
        } else {
            "Windows (DirectComposition, HWND_BOTTOM)"
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

    /// DWM cannot be turned off from Windows 8 on, so there always is one.
    pub fn has_compositor(&self) -> bool {
        true
    }

    pub fn window_id(&self) -> u32 {
        id_of(self.hwnd)
    }

    /// Device pixels per logical pixel on the monitor this window is on.
    pub fn scale(&self) -> f64 {
        // SAFETY: a live window.
        let dpi = unsafe { GetDpiForWindow(self.hwnd) };
        if dpi == 0 { 1.0 } else { dpi as f64 / 96.0 }
    }

    pub fn raw_handles(&self) -> (RawDisplayHandle, RawWindowHandle) {
        let mut handle = Win32WindowHandle::new(
            NonZeroIsize::new(self.hwnd as isize).expect("a window handle is never null"),
        );
        handle.hinstance = NonZeroIsize::new(instance() as isize);
        (
            RawDisplayHandle::Windows(WindowsDisplayHandle::new()),
            RawWindowHandle::Win32(handle),
        )
    }

    pub fn geometry(&self) -> PhysicalRect {
        self.rect
    }

    pub fn set_geometry(&mut self, rect: PhysicalRect) -> Result<(), DesktopError> {
        // SAFETY: a live window. Z-order is left alone here and forced by the
        // window procedure anyway.
        let moved = unsafe {
            SetWindowPos(
                self.hwnd,
                HWND_BOTTOM,
                rect.x,
                rect.y,
                rect.width.max(1) as i32,
                rect.height.max(1) as i32,
                SWP_NOACTIVATE,
            )
        };
        if moved == 0 {
            return Err(DesktopError::last("SetWindowPos"));
        }
        self.rect = rect;
        Ok(())
    }

    /// The virtual screen: the bounding box of every monitor.
    pub fn desktop_bounds(&self) -> PhysicalRect {
        // SAFETY: plain metrics.
        unsafe {
            PhysicalRect::new(
                GetSystemMetrics(SM_XVIRTUALSCREEN),
                GetSystemMetrics(SM_YVIRTUALSCREEN),
                GetSystemMetrics(SM_CXVIRTUALSCREEN).max(1) as u32,
                GetSystemMetrics(SM_CYVIRTUALSCREEN).max(1) as u32,
            )
        }
    }

    pub fn outputs(&self) -> Vec<Output> {
        outputs()
    }

    /// Keeps the pointer while a drag leaves the window.
    pub fn grab_pointer(&self) -> Result<(), DesktopError> {
        // SAFETY: a live window on this thread.
        unsafe { SetCapture(self.hwnd) };
        Ok(())
    }

    pub fn release_pointer(&self) -> Result<(), DesktopError> {
        // SAFETY: releases whatever this thread captured.
        unsafe { ReleaseCapture() };
        Ok(())
    }

    /// Everything every window on this thread has reported. See the module
    /// notes for why *every*.
    pub fn drain_events(&mut self) -> Result<Vec<Input>, DesktopError> {
        Ok(QUEUE.with(|queue| std::mem::take(&mut *queue.borrow_mut())))
    }

    /// The window is the size it was asked to be, always.
    pub fn take_resize(&mut self) -> Option<(u32, u32)> {
        None
    }
}

impl Drop for DesktopWindow {
    fn drop(&mut self) {
        // SAFETY: a window this struct created and nothing else destroys.
        unsafe { DestroyWindow(self.hwnd) };
    }
}

/// Asks DWM for an acrylic backdrop and rounded corners. Answers whether it
/// was given one: before Windows 11 22H2 the attribute does not exist, and the
/// call fails rather than doing something else.
fn apply_backdrop(hwnd: HWND) -> bool {
    let margins = MARGINS {
        cxLeftWidth: -1,
        cxRightWidth: -1,
        cyTopHeight: -1,
        cyBottomHeight: -1,
    };
    let corners = DWMWCP_ROUND;
    let kind = DWMSBT_TRANSIENTWINDOW;
    // SAFETY: a live window, and attribute values of the size given.
    unsafe {
        DwmExtendFrameIntoClientArea(hwnd, &margins);
        DwmSetWindowAttribute(
            hwnd,
            DWMWA_WINDOW_CORNER_PREFERENCE,
            (&corners as *const i32).cast(),
            4,
        );
        DwmSetWindowAttribute(
            hwnd,
            DWMWA_SYSTEMBACKDROP_TYPE,
            (&kind as *const i32).cast(),
            4,
        ) >= 0
    }
}

/// Every monitor, by the name Windows gives its adapter output.
///
/// `\\.\DISPLAY1` and so on: stable across a reboot and across the order the
/// monitors were plugged in, which is what a placement needs from a name.
pub fn outputs() -> Vec<Output> {
    unsafe extern "system" fn each(
        monitor: HMONITOR,
        _dc: HDC,
        _rect: *mut RECT,
        data: LPARAM,
    ) -> windows_sys::core::BOOL {
        // SAFETY: `data` is the `Vec` passed below, alive for the whole
        // enumeration.
        let list = unsafe { &mut *(data as *mut Vec<Output>) };
        let mut info = MONITORINFOEXW::default();
        info.monitorInfo.cbSize = std::mem::size_of::<MONITORINFOEXW>() as u32;
        // SAFETY: a monitor the system just handed over, and a struct whose
        // size is declared.
        if unsafe {
            GetMonitorInfoW(
                monitor,
                (&mut info as *mut MONITORINFOEXW).cast::<MONITORINFO>(),
            )
        } != 0
        {
            let r = info.monitorInfo.rcMonitor;
            let len = info
                .szDevice
                .iter()
                .position(|c| *c == 0)
                .unwrap_or(info.szDevice.len());
            list.push(Output {
                name: String::from_utf16_lossy(&info.szDevice[..len]),
                rect: PhysicalRect::new(
                    r.left,
                    r.top,
                    (r.right - r.left).max(0) as u32,
                    (r.bottom - r.top).max(0) as u32,
                ),
                // `MONITORINFOF_PRIMARY`.
                primary: info.monitorInfo.dwFlags & 1 != 0,
            });
        }
        1
    }
    dpi_aware();
    let mut list: Vec<Output> = Vec::new();
    // SAFETY: every monitor of the whole desktop, with a callback that only
    // writes into `list`.
    unsafe {
        EnumDisplayMonitors(
            std::ptr::null_mut(),
            std::ptr::null(),
            Some(each),
            &mut list as *mut Vec<Output> as LPARAM,
        )
    };
    list
}

/// Blocks until a message arrives for any window on this thread, a background
/// task finishes, or the timeout elapses — then dispatches everything waiting.
///
/// The order is the Linux one, for the same reason: anything already queued is
/// returned at once, because no wait will report it. Then one
/// `MsgWaitForMultipleObjectsEx` over the thread's message queue *and* the
/// wake event — the Win32 spelling of polling a socket and a pipe together.
/// `MWMO_INPUTAVAILABLE` so that a message which arrived before the wait, and
/// was seen but not removed by somebody's `PeekMessage`, still ends it.
pub fn wait_any(
    _windows: &mut [DesktopWindow],
    timeout: Option<Duration>,
    wake: Option<&WakePipe>,
) -> Result<(), DesktopError> {
    if QUEUE.with(|queue| !queue.borrow().is_empty()) {
        return Ok(());
    }
    let millis = match timeout {
        Some(d) => d.as_millis().min(u32::MAX as u128 - 1) as u32,
        None => windows_sys::Win32::System::Threading::INFINITE,
    };
    let handles: Vec<HANDLE> = wake.map(|w| w.as_handle()).into_iter().collect();
    // SAFETY: zero or one live handle, and this thread's queue.
    let woke = unsafe {
        MsgWaitForMultipleObjectsEx(
            handles.len() as u32,
            handles.as_ptr(),
            millis,
            QS_ALLINPUT,
            MWMO_INPUTAVAILABLE,
        )
    };
    if woke == WAIT_FAILED {
        return Err(DesktopError::last("MsgWaitForMultipleObjectsEx"));
    }
    if let Some(wake) = wake {
        wake.drain();
    }
    pump();
    Ok(())
}

/// Dispatches every message waiting on this thread, without blocking.
fn pump() {
    // SAFETY: a zeroed `MSG` is a valid out-parameter.
    let mut message: MSG = unsafe { std::mem::zeroed() };
    // SAFETY: this thread's queue, every window, removing what is read.
    while unsafe { PeekMessageW(&mut message, std::ptr::null_mut(), 0, 0, PM_REMOVE) } != 0 {
        // SAFETY: a message just read from the queue.
        unsafe {
            TranslateMessage(&message);
            DispatchMessageW(&message);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mouse_coordinates_are_signed() {
        // A monitor to the left of the primary has negative coordinates, and
        // so does a drag that leaves a window on the left; `LOWORD` read as
        // unsigned would put the pointer 65 000 pixels to the right.
        let lparam = (((-5i16) as u16 as isize) << 16) | ((-20i16) as u16 as isize);
        assert_eq!(point_of(lparam), (-20, -5));
    }
}
