//! The overlay process: a click-through, always-on-top window whose content is a
//! DirectComposition swap chain drawn with Direct2D (no CPU copy per frame), the message
//! handlers for the hook's messages, and the final window snap.

use crate::geom::{cell_at, selection_rect, Cells, Point, Rect, RectF};
use crate::gfx::{wide, Renderer, Scene, SelectionScene};
use crate::hud::{hint_bar, hud_text, HudOptions, HudSubject, HudText};
use crate::logger::log;
use crate::names;
use crate::shm::Section;
use crate::snap::{self, Mode, SnapParams, SnapRect, Verify, VerifyStep, VERIFY_TICK_MS};
use crate::state::{Effect, Machine, TICK_MS};
use crate::theme::Palette;
use std::cell::RefCell;
use std::ffi::c_void;
use windows::core::{Result, PCWSTR};
use windows::Win32::Foundation::{COLORREF, HWND, LPARAM, LRESULT, POINT, RECT, WPARAM};
use windows::Win32::Graphics::Dwm::{DwmGetWindowAttribute, DWMWA_EXTENDED_FRAME_BOUNDS};
use windows::Win32::Graphics::DirectComposition::{
    DCompositionCreateDevice, IDCompositionDevice, IDCompositionTarget, IDCompositionVisual,
};
use windows::Win32::Graphics::Dxgi::Common::*;
use windows::Win32::Graphics::Dxgi::*;
use windows::Win32::Graphics::Gdi::{GetMonitorInfoW, MonitorFromPoint, MONITORINFO, MONITOR_DEFAULTTONEAREST};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::HiDpi::{
    GetDpiForMonitor, SetProcessDpiAwarenessContext, DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2, MDT_EFFECTIVE_DPI,
};
use windows::Win32::UI::Input::KeyboardAndMouse::{GetAsyncKeyState, VK_ESCAPE, VK_LBUTTON, VK_RBUTTON};
use windows::Win32::UI::WindowsAndMessaging::*;
use wg_core::ipc::{unpack_point, GridWindowMemory, SettingsMemory, Wire};
use wg_core::settings::WindowAnchor;

const TIMER_ID: usize = 1;
const VERIFY_TIMER_ID: usize = 2;

/// A renderer plus the swap chain it draws into and the composition tree showing it.
struct Gfx {
    renderer: Renderer,
    swap: IDXGISwapChain1,
    _dcomp: IDCompositionDevice,
    _target: IDCompositionTarget,
    _visual: IDCompositionVisual,
    size: (u32, u32),
}

impl Gfx {
    fn new(hwnd: HWND, width: u32, height: u32) -> Result<Gfx> {
        let renderer = Renderer::new()?;
        // SAFETY: valid devices/window; the descriptor is fully initialised.
        unsafe {
            let adapter = renderer.dev.dxgi.GetAdapter()?;
            let factory: IDXGIFactory2 = adapter.GetParent()?;
            let desc = DXGI_SWAP_CHAIN_DESC1 {
                Width: width,
                Height: height,
                Format: DXGI_FORMAT_B8G8R8A8_UNORM,
                Stereo: false.into(),
                SampleDesc: DXGI_SAMPLE_DESC { Count: 1, Quality: 0 },
                BufferUsage: DXGI_USAGE_RENDER_TARGET_OUTPUT,
                BufferCount: 2,
                Scaling: DXGI_SCALING_STRETCH,
                SwapEffect: DXGI_SWAP_EFFECT_FLIP_SEQUENTIAL,
                AlphaMode: DXGI_ALPHA_MODE_PREMULTIPLIED,
                Flags: 0,
            };
            let swap = factory.CreateSwapChainForComposition(&renderer.dev.d3d, &desc, None)?;
            let dcomp: IDCompositionDevice = DCompositionCreateDevice(&renderer.dev.dxgi)?;
            let target = dcomp.CreateTargetForHwnd(hwnd, true)?;
            let visual = dcomp.CreateVisual()?;
            visual.SetContent(&swap)?;
            target.SetRoot(&visual)?;
            dcomp.Commit()?;
            Ok(Gfx { renderer, swap, _dcomp: dcomp, _target: target, _visual: visual, size: (width, height) })
        }
    }

    fn resize(&mut self, width: u32, height: u32) -> Result<()> {
        if self.size == (width, height) {
            return Ok(());
        }
        self.renderer.clear_target();
        // SAFETY: no outstanding references to the buffers (target cleared above).
        unsafe { self.swap.ResizeBuffers(0, width, height, DXGI_FORMAT_UNKNOWN, DXGI_SWAP_CHAIN_FLAG(0))? };
        self.size = (width, height);
        Ok(())
    }

    fn present(&self, scene: &Scene) -> Result<()> {
        // SAFETY: the buffer is released again (target cleared) before presenting.
        unsafe {
            let surface: IDXGISurface = self.swap.GetBuffer(0)?;
            self.renderer.set_target_surface(&surface)?;
            let drawn = self.renderer.draw(scene);
            self.renderer.clear_target();
            drawn?;
            self.swap.Present(0, DXGI_PRESENT(0)).ok()
        }
    }
}

/// Everything fixed for the duration of one grid session.
struct Session {
    work: Rect,
    scale: f32,
    cols: i32,
    rows: i32,
    palette: Palette,
    memory: GridWindowMemory,
    hint: Option<crate::hud::HintBar>,
    selection: Option<(RectF, Option<HudText>)>,
}

pub struct App {
    hwnd: HWND,
    gfx: Option<Gfx>,
    machine: Machine,
    session: Option<Session>,
    grid_window_map: Section,
    msg_show: u32,
    msg_area: u32,
    msg_stop: u32,
    msg_target: u32,
    target: HWND,
    /// The window the hook says is being dragged; preferred over guessing from the pointer.
    hook_target: HWND,
    /// The snap being verified (target and what it should look like).
    verifying: Option<(HWND, Verify)>,
    /// Test-only override of the monitor work area (isolated builds).
    test_rect: Option<Rect>,
}

thread_local! {
    static APP: RefCell<Option<App>> = const { RefCell::new(None) };
}

fn hwnd_to_i32(h: HWND) -> i32 {
    h.0 as isize as i32
}

fn key_down(vk: i32) -> bool {
    // SAFETY: plain query.
    unsafe { GetAsyncKeyState(vk) as u16 & 0x8000 != 0 }
}

fn rect_of(r: RECT) -> Rect {
    Rect::new(r.left, r.top, r.right, r.bottom)
}

impl App {
    fn new(hwnd: HWND, test_rect: Option<Rect>) -> Result<App> {
        let grid_window_map = Section::open_or_create(names::MAP_GRID_WINDOW, GridWindowMemory::SIZE)?;
        let register = |name: &str| {
            let w = wide(name);
            // SAFETY: NUL-terminated name; filter change on our own window.
            unsafe {
                let id = RegisterWindowMessageW(PCWSTR(w.as_ptr()));
                let _ = ChangeWindowMessageFilterEx(hwnd, id, MSGFLT_ALLOW, None);
                id
            }
        };
        Ok(App {
            hwnd,
            gfx: None,
            machine: Machine::default(),
            session: None,
            grid_window_map,
            msg_show: register(names::MSG_SHOW_GRID),
            msg_area: register(names::MSG_AREA),
            msg_stop: register(names::MSG_STOP),
            msg_target: register(names::MSG_TARGET),
            target: HWND::default(),
            hook_target: HWND::default(),
            verifying: None,
            test_rect,
        })
    }

    fn handle(&mut self, msg: u32, wparam: WPARAM, lparam: LPARAM) -> Option<LRESULT> {
        if msg == self.msg_show {
            let center = unpack_point(lparam.0 as usize);
            let Some(mode) = Mode::from_wparam(wparam.0) else { return Some(LRESULT(0)) };
            let esc = mode == Mode::Idle && key_down(VK_ESCAPE.0.into());
            let effects = self.machine.on_show_grid(mode, Point::new(center.0, center.1), esc);
            self.run(effects);
            Some(LRESULT(0))
        } else if msg == self.msg_area {
            let (sx, sy) = unpack_point(wparam.0);
            let (ex, ey) = unpack_point(lparam.0 as usize);
            if let Some(e) = self.machine.on_area(Cells::from_corners(sx, sy, ex, ey)) {
                self.run(vec![e]);
            }
            Some(LRESULT(0))
        } else if msg == self.msg_target {
            // Same sign-extension the handle section uses for 32-bit values.
            self.hook_target = HWND(wparam.0 as u32 as i32 as isize as *mut c_void);
            Some(LRESULT(0))
        } else if msg == self.msg_stop {
            // SAFETY: destroying our own window.
            unsafe {
                let _ = DestroyWindow(self.hwnd);
            }
            Some(LRESULT(0))
        } else if msg == WM_TIMER && wparam.0 == TIMER_ID {
            // SAFETY: plain query.
            let primary = if unsafe { GetSystemMetrics(SM_SWAPBUTTON) } != 0 { VK_RBUTTON } else { VK_LBUTTON };
            // The button state alone is not enough to call a drag finished: touch and pen
            // drags don't set it, and even after the release the target may still be inside
            // its move loop. Snapping then races the loop's own final SetWindowPos.
            let dragging = key_down(primary.0.into()) || in_move_size(self.target);
            let effects = self.machine.on_tick(key_down(VK_ESCAPE.0.into()), dragging);
            self.run(effects);
            Some(LRESULT(0))
        } else if msg == WM_TIMER && wparam.0 == VERIFY_TIMER_ID {
            self.verify_tick();
            Some(LRESULT(0))
        } else {
            None
        }
    }

    fn run(&mut self, effects: Vec<Effect>) {
        for effect in effects {
            match effect {
                Effect::StartTimer => {
                    // Tests have no mouse button held, which the safety net would read as a
                    // finished drag; isolated builds can switch it off.
                    if !test_flag("WG_TEST_NO_TIMER") {
                        // SAFETY: timer on our own window.
                        unsafe { SetTimer(Some(self.hwnd), TIMER_ID, TICK_MS, None) };
                    }
                }
                Effect::StopTimer => {
                    // SAFETY: timer on our own window.
                    unsafe {
                        let _ = KillTimer(Some(self.hwnd), TIMER_ID);
                    }
                }
                Effect::ClearTarget => {
                    self.target = HWND::default();
                    self.hook_target = HWND::default();
                }
                Effect::Begin { mode, center } => {
                    if let Err(e) = self.begin(mode, center) {
                        log(&format!("Begin failed: {e}"));
                        self.gfx = None; // rebuild the GPU resources next time
                    }
                }
                Effect::Select(cells) => {
                    if let Err(e) = self.select(cells) {
                        log(&format!("Select failed: {e}"));
                        self.gfx = None;
                    }
                }
                Effect::Hide => {
                    // SAFETY: hiding our own window.
                    unsafe {
                        let _ = ShowWindow(self.hwnd, SW_HIDE);
                    }
                    if let Some(s) = &mut self.session {
                        s.selection = None;
                    }
                }
                Effect::Snap { previous, current, cells } => self.snap(previous, current, cells),
            }
        }
    }

    /// The window under the pointer (its top-level ancestor), else the foreground window.
    fn detect_target(&self) -> HWND {
        if let Some(h) = test_target() {
            return h;
        }
        // SAFETY: plain queries; our own window is click-through so it is never "under" the pointer.
        unsafe {
            let mut pt = POINT::default();
            if GetCursorPos(&mut pt).is_ok() {
                let w = WindowFromPoint(pt);
                if !w.0.is_null() {
                    let root = GetAncestor(w, GA_ROOT);
                    if !root.0.is_null() && root != self.hwnd {
                        return root;
                    }
                }
            }
            let fg = GetForegroundWindow();
            if !fg.0.is_null() && fg != self.hwnd {
                return fg;
            }
        }
        HWND::default()
    }

    /// Work area, DPI scale and monitor bounds around `center`.
    fn locate_monitor(&self, center: Point) -> (Rect, Rect, f32) {
        if let Some(r) = self.test_rect {
            return (r, r, 1.0);
        }
        // SAFETY: valid out pointers.
        unsafe {
            let hmon = MonitorFromPoint(POINT { x: center.x, y: center.y }, MONITOR_DEFAULTTONEAREST);
            let mut mi = MONITORINFO { cbSize: std::mem::size_of::<MONITORINFO>() as u32, ..Default::default() };
            if !GetMonitorInfoW(hmon, &mut mi).as_bool() {
                return (Rect::new(0, 0, 1920, 1080), Rect::new(0, 0, 1920, 1080), 1.0);
            }
            let (mut dx, mut dy) = (96u32, 96u32);
            let _ = GetDpiForMonitor(hmon, MDT_EFFECTIVE_DPI, &mut dx, &mut dy);
            (rect_of(mi.rcWork), rect_of(mi.rcMonitor), dx.max(96) as f32 / 96.0)
        }
    }

    fn begin(&mut self, mode: Mode, center: Point) -> Result<()> {
        // A new drag supersedes any verification still running for the previous snap.
        self.stop_verifying();
        let detected = if is_window(self.hook_target) { self.hook_target } else { self.detect_target() };
        if !detected.0.is_null() {
            self.target = detected;
        }
        let (work, bounds, scale) = self.locate_monitor(center);
        let memory: GridWindowMemory = self.grid_window_map.read();
        let palette = Palette::from_memory(&memory);

        let (gx, gy) = (if memory.grid_x > 0 { memory.grid_x } else { 12 }, if memory.grid_y > 0 { memory.grid_y } else { 6 });
        let (cols, rows) = if bounds.width() >= bounds.height() {
            (gx, gy)
        } else {
            (if memory.grid_y > 0 { memory.grid_y } else { 6 }, if memory.grid_x > 0 { memory.grid_x } else { 12 })
        };
        let hint = hint_bar(mode, memory.show_mode_indicator != 0, memory.show_interaction_hints != 0);
        self.session = Some(Session { work, scale, cols, rows, palette, memory, hint, selection: None });

        let (w, h) = (work.width().max(1) as u32, work.height().max(1) as u32);
        match &mut self.gfx {
            Some(g) => g.resize(w, h)?,
            None => self.gfx = Some(Gfx::new(self.hwnd, w, h)?),
        }
        // Draw the empty grid first, then show, so no stale frame ever appears.
        self.render()?;
        // SAFETY: positioning and showing our own window without activating it.
        unsafe {
            SetWindowPos(
                self.hwnd,
                Some(HWND_TOPMOST),
                work.left,
                work.top,
                work.width(),
                work.height(),
                SWP_NOACTIVATE | SWP_SHOWWINDOW,
            )?;
        }

        // Initial selection: the cell under the pointer.
        let rel = Point::new(center.x - work.left, center.y - work.top);
        let c = cell_at(rel, work.width(), work.height(), cols, rows);
        if let Some(Effect::Select(cells)) = self.machine.on_area(Cells::from_corners(c.x, c.y, c.x, c.y)) {
            self.select(cells)?;
        }
        Ok(())
    }

    fn select(&mut self, cells: Cells) -> Result<()> {
        let Some(s) = &mut self.session else { return Ok(()) };
        let (w, h) = (f64::from(s.work.width()), f64::from(s.work.height()));
        let rect = selection_rect(cells, s.cols, s.rows, w, h);
        let hud = (s.memory.show_selection_hud != 0).then(|| {
            hud_text(
                HudOptions {
                    show_dimensions: s.memory.show_dimensions != 0,
                    show_cell_span: s.memory.show_cell_span != 0,
                    show_coverage: s.memory.show_screen_coverage != 0,
                    show_aspect_ratio: s.memory.show_aspect_ratio != 0,
                },
                HudSubject {
                    px_w: rect.width.round() as i32,
                    px_h: rect.height.round() as i32,
                    cells,
                    cols: s.cols,
                    rows: s.rows,
                    screen_w: s.work.width(),
                    screen_h: s.work.height(),
                },
            )
        });
        s.selection = Some((rect, hud));
        self.render()
    }

    fn render(&mut self) -> Result<()> {
        let (Some(gfx), Some(s)) = (&self.gfx, &self.session) else { return Ok(()) };
        let selection = s.selection.as_ref().map(|(rect, hud)| SelectionScene { rect: *rect, hud: hud.as_ref() });
        let scene = Scene {
            width: s.work.width() as f32,
            height: s.work.height() as f32,
            scale: s.scale,
            cols: s.cols,
            rows: s.rows,
            palette: s.palette,
            selection,
            hint: s.hint,
        };
        gfx.present(&scene)
    }

    fn snap(&mut self, previous: Mode, current: Mode, cells: Option<Cells>) {
        let (Some(cells), Some(s)) = (cells, &self.session) else { return };
        let target = if !self.target.0.is_null() && self.target != self.hwnd { self.target } else { self.detect_target() };
        if target.0.is_null() || target == self.hwnd {
            return;
        }
        // SAFETY: valid out pointers for window queries.
        let (window, frame, cursor) = unsafe {
            let mut wr = RECT::default();
            let window = GetWindowRect(target, &mut wr).is_ok().then(|| rect_of(wr));
            let mut fr = RECT::default();
            let frame = DwmGetWindowAttribute(
                target,
                DWMWA_EXTENDED_FRAME_BOUNDS,
                (&mut fr as *mut RECT).cast::<c_void>(),
                std::mem::size_of::<RECT>() as u32,
            )
            .is_ok()
            .then(|| rect_of(fr));
            let mut pt = POINT::default();
            let cursor = GetCursorPos(&mut pt).is_ok().then(|| Point::new(pt.x, pt.y));
            (window, frame, cursor)
        };
        let m = &s.memory;
        let params = SnapParams {
            work: s.work,
            cols: s.cols,
            rows: s.rows,
            min: Point::new(cells.min_x, cells.min_y),
            max: Point::new(cells.max_x, cells.max_y),
            previous,
            current,
            window,
            frame,
            anchor: WindowAnchor::from_i32(m.anchor),
            compensate_margins: m.compensate_window_margins != 0,
            preserve_size_on_move: m.preserve_size_on_move != 0,
            cursor,
        };
        let Some(r) = snap::compute(&params) else { return };
        log(&format!(
            "Snap: Target=0x{:08X}, min=({},{}), max=({},{}), rect=({},{},{},{}), prev={previous:?}, cur={current:?}",
            target.0 as usize, cells.min_x, cells.min_y, cells.max_x, cells.max_y, r.x, r.y, r.w, r.h
        ));
        self.apply(target, r);
        self.verifying = Some((target, Verify::new(r)));
        // SAFETY: timer on our own window.
        unsafe { SetTimer(Some(self.hwnd), VERIFY_TIMER_ID, VERIFY_TICK_MS, None) };
    }

    /// Hands the rectangle to the target's own thread (asynchronously, so a hung window can
    /// never block the overlay). A maximized window is restored first.
    fn apply(&self, target: HWND, r: SnapRect) {
        // SAFETY: repositioning another window asynchronously, as the legacy overlay did.
        unsafe {
            if IsZoomed(target).as_bool() {
                let _ = ShowWindowAsync(target, SW_RESTORE);
            }
            let _ = SetWindowPos(
                target,
                None,
                r.x,
                r.y,
                r.w,
                r.h,
                SWP_NOZORDER | SWP_NOACTIVATE | SWP_FRAMECHANGED | SWP_ASYNCWINDOWPOS,
            );
        }
    }

    fn stop_verifying(&mut self) {
        if self.verifying.take().is_some() {
            // SAFETY: timer on our own window.
            unsafe {
                let _ = KillTimer(Some(self.hwnd), VERIFY_TIMER_ID);
            }
        }
    }

    /// Checks that the snapped window stayed where it was put, and puts it back if not.
    fn verify_tick(&mut self) {
        let Some((target, mut verify)) = self.verifying else {
            self.stop_verifying();
            return;
        };
        if !is_window(target) || self.machine.mode() != Mode::Idle {
            self.stop_verifying();
            return;
        }
        let mut wr = RECT::default();
        // SAFETY: valid out pointer.
        let ok = unsafe { GetWindowRect(target, &mut wr).is_ok() };
        // SAFETY: plain query.
        let zoomed = unsafe { IsZoomed(target).as_bool() };
        let step = if ok { verify.tick(rect_of(wr), zoomed, in_move_size(target)) } else { VerifyStep::GiveUp };
        match step {
            VerifyStep::Wait => {}
            VerifyStep::Done => {
                if verify.reapplied() > 0 {
                    log(&format!("Snap: Target=0x{:08X} needed {} correction(s)", target.0 as usize, verify.reapplied()));
                }
                self.stop_verifying();
                return;
            }
            VerifyStep::Reapply => self.apply(target, verify.want),
            VerifyStep::GiveUp => {
                let w = verify.want;
                log(&format!(
                    "Snap: Target=0x{:08X} did not settle at ({},{},{},{}): now ({},{},{},{}), zoomed={zoomed}",
                    target.0 as usize, w.x, w.y, w.w, w.h, wr.left, wr.top, wr.right - wr.left, wr.bottom - wr.top
                ));
                self.stop_verifying();
                return;
            }
        }
        self.verifying = Some((target, verify));
    }
}

fn is_window(h: HWND) -> bool {
    // SAFETY: plain query; any handle value is acceptable.
    !h.0.is_null() && unsafe { IsWindow(Some(h)).as_bool() }
}

/// True while the window's thread is inside the modal move/size loop.
fn in_move_size(h: HWND) -> bool {
    if h.0.is_null() {
        return false;
    }
    // SAFETY: valid out pointer with `cbSize` set.
    unsafe {
        let tid = GetWindowThreadProcessId(h, None);
        if tid == 0 {
            return false;
        }
        let mut info = GUITHREADINFO { cbSize: std::mem::size_of::<GUITHREADINFO>() as u32, ..Default::default() };
        GetGUIThreadInfo(tid, &mut info).is_ok() && info.flags.0 & GUI_INMOVESIZE.0 != 0
    }
}

unsafe extern "system" fn wnd_proc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    match msg {
        WM_NCHITTEST => return LRESULT(-1), // HTTRANSPARENT
        WM_DPICHANGED => return LRESULT(0), // never let the system resize us on a monitor change
        WM_DESTROY => {
            PostQuitMessage(0);
            return LRESULT(0);
        }
        _ => {}
    }
    let handled = APP.with(|cell| match cell.try_borrow_mut() {
        Ok(mut slot) => slot.as_mut().and_then(|app| app.handle(msg, wparam, lparam)),
        // Re-entered from a call we made ourselves (e.g. SetWindowPos): not ours to handle.
        Err(_) => None,
    });
    handled.unwrap_or_else(|| DefWindowProcW(hwnd, msg, wparam, lparam))
}

/// True when the (isolated-build-only) environment flag `name` is set to `1`.
fn test_flag(name: &str) -> bool {
    cfg!(feature = "isolated") && std::env::var(name).is_ok_and(|v| v == "1")
}

/// `WG_TEST_TARGET=<hwnd>` forces the snap target (isolated builds only), so tests never
/// have to guess which real window is under the pointer.
fn test_target() -> Option<HWND> {
    if !cfg!(feature = "isolated") {
        return None;
    }
    let v: usize = std::env::var("WG_TEST_TARGET").ok()?.trim().parse().ok()?;
    Some(HWND(v as *mut c_void))
}

/// Parses `WG_TEST_RECT=left,top,right,bottom` (isolated builds only).
fn test_rect() -> Option<Rect> {
    if !cfg!(feature = "isolated") {
        return None;
    }
    let v = std::env::var("WG_TEST_RECT").ok()?;
    let n: Vec<i32> = v.split(',').filter_map(|p| p.trim().parse().ok()).collect();
    (n.len() == 4).then(|| Rect::new(n[0], n[1], n[2], n[3]))
}

pub fn run() -> Result<()> {
    crate::logger::start(env!("CARGO_PKG_VERSION"));
    let Some(_instance) = wg_win::instance::acquire(names::OVERLAY_INSTANCE) else { return Ok(()) };
    // SAFETY: process-wide setup before any window exists.
    unsafe {
        let _ = SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
    }
    let class = wide("WindowGridOverlayWindow");
    // SAFETY: standard class registration and window creation with valid arguments.
    let hwnd = unsafe {
        let hinstance = GetModuleHandleW(None)?;
        let wc = WNDCLASSEXW {
            cbSize: std::mem::size_of::<WNDCLASSEXW>() as u32,
            lpfnWndProc: Some(wnd_proc),
            hInstance: hinstance.into(),
            lpszClassName: PCWSTR(class.as_ptr()),
            ..Default::default()
        };
        RegisterClassExW(&wc);
        let hwnd = CreateWindowExW(
            WS_EX_NOREDIRECTIONBITMAP | WS_EX_LAYERED | WS_EX_TRANSPARENT | WS_EX_TOPMOST | WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE,
            PCWSTR(class.as_ptr()),
            PCWSTR(wide("WindowGrid").as_ptr()),
            WS_POPUP,
            0,
            0,
            1,
            1,
            None,
            None,
            Some(hinstance.into()),
            None,
        )?;
        SetLayeredWindowAttributes(hwnd, COLORREF(0), 255, LWA_ALPHA)?;
        hwnd
    };

    let app = App::new(hwnd, test_rect())?;
    // Tell the hook DLLs and the app where to send messages.
    let handle_map = Section::open_or_create(names::MAP_GRID_WINDOW_HANDLE, 4)?;
    let hook_map = Section::open_or_create(names::MAP_HOOK, SettingsMemory::SIZE)?;
    let h = hwnd_to_i32(hwnd).to_le_bytes();
    handle_map.write_bytes(0, &h);
    hook_map.write_bytes(0, &h); // only `GridWindow`; the other fields belong to the app
    log(&format!("Overlay window HWND=0x{:08X} written to shared memory", hwnd_to_i32(hwnd)));
    APP.with(|cell| *cell.borrow_mut() = Some(app));

    let mut msg = MSG::default();
    // SAFETY: standard message loop on the thread that created the window.
    unsafe {
        while GetMessageW(&mut msg, None, 0, 0).as_bool() {
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
    }
    APP.with(|cell| *cell.borrow_mut() = None);
    log("Overlay exiting");
    drop((handle_map, hook_map));
    Ok(())
}
