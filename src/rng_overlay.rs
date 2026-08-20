use std::ffi::c_void;
use std::fs;
use std::path::PathBuf;
use std::sync::{
    Mutex, MutexGuard, OnceLock,
    atomic::{AtomicBool, AtomicI32, AtomicIsize, AtomicU32, AtomicU64, Ordering},
};
use std::thread::{self, JoinHandle};

use eframe::egui;
use windows::{
    Win32::{
        Foundation::{COLORREF, HWND, LPARAM, LRESULT, POINT, RECT, WPARAM},
        Graphics::Gdi::{
            BeginPaint, CLIP_DEFAULT_PRECIS, CreateFontW, CreateSolidBrush, DEFAULT_CHARSET,
            DEFAULT_PITCH, DT_CENTER, DT_SINGLELINE, DT_VCENTER, DeleteObject, DrawTextW, EndPaint,
            FW_BOLD, FillRect, InvalidateRect, NONANTIALIASED_QUALITY, OUT_DEFAULT_PRECIS,
            PAINTSTRUCT, SelectObject, SetBkMode, SetTextColor, TRANSPARENT, UpdateWindow,
        },
        UI::WindowsAndMessaging::{
            AppendMenuW, CS_HREDRAW, CS_VREDRAW, CreatePopupMenu, CreateWindowExW, DefWindowProcW,
            DestroyMenu, DestroyWindow, DispatchMessageW, GetClientRect, GetCursorPos, GetMessageW,
            HMENU, HWND_TOPMOST, IDC_ARROW, LWA_COLORKEY, LoadCursorW, MENU_ITEM_FLAGS, MF_CHECKED,
            MF_POPUP, MF_SEPARATOR, MF_STRING, MSG, PostMessageW, PostQuitMessage, RegisterClassW,
            SW_SHOW, SWP_NOACTIVATE, SWP_SHOWWINDOW, SetForegroundWindow,
            SetLayeredWindowAttributes, SetTimer, SetWindowPos, ShowWindow, TPM_NONOTIFY,
            TPM_RETURNCMD, TPM_RIGHTBUTTON, TrackPopupMenu, TranslateMessage, WM_CLOSE, WM_DESTROY,
            WM_LBUTTONDOWN, WM_MOUSEWHEEL, WM_PAINT, WM_RBUTTONDOWN, WM_TIMER, WNDCLASSW,
            WS_EX_LAYERED, WS_EX_TOOLWINDOW, WS_EX_TOPMOST, WS_POPUP, WS_VISIBLE,
        },
    },
    core::{PCWSTR, w},
};

use crate::model::Rect as WorkRect;
use std::time::{SystemTime, UNIX_EPOCH};

// This module is the native boundary for the RnG overlay. Raw handles remain on
// its Win32 message-loop thread; the rest of the application only uses this
// safe lifecycle wrapper and plain work-area rectangles.

pub struct RngOverlay {
    worker: Mutex<Option<JoinHandle<()>>>,
}

impl RngOverlay {
    #[must_use]
    pub fn new() -> Self {
        Self {
            worker: Mutex::new(None),
        }
    }

    #[must_use]
    pub fn enabled(&self) -> bool {
        ACTIVE.load(Ordering::Acquire)
    }

    pub fn toggle(&self, context: &egui::Context, work_area: WorkRect) {
        self.sync_work_area(work_area);
        if self.enabled() {
            self.stop();
        } else {
            self.start(context);
        }
    }

    pub fn sync_work_area(&self, work_area: WorkRect) {
        let old_left = WORK_LEFT.swap(work_area.left, Ordering::AcqRel);
        let old_top = WORK_TOP.swap(work_area.top, Ordering::AcqRel);
        let old_right = WORK_RIGHT.swap(work_area.right(), Ordering::AcqRel);
        let old_bottom = WORK_BOTTOM.swap(work_area.bottom(), Ordering::AcqRel);
        let changed = old_left != work_area.left
            || old_top != work_area.top
            || old_right != work_area.right()
            || old_bottom != work_area.bottom();
        let hwnd = WINDOW_HANDLE.load(Ordering::Acquire);
        if changed && hwnd != 0 {
            let hwnd = HWND(hwnd as *mut c_void);
            if let Err(error) =
                unsafe { PostMessageW(Some(hwnd), WM_REPOSITION, WPARAM(0), LPARAM(0)) }
            {
                log::warn!("could not reposition RnG overlay: {error}");
            }
        }
    }

    pub fn stop(&self) {
        ACTIVE.store(false, Ordering::Release);
        let hwnd = WINDOW_HANDLE.load(Ordering::Acquire);
        if hwnd != 0 {
            let hwnd = HWND(hwnd as *mut c_void);
            if let Err(error) = unsafe { PostMessageW(Some(hwnd), WM_CLOSE, WPARAM(0), LPARAM(0)) }
            {
                log::warn!("could not stop RnG overlay: {error}");
            }
        }
    }

    fn start(&self, context: &egui::Context) {
        if let Some(previous) = lock(&self.worker).take() {
            let _ = previous.join();
        }
        *lock(REPAINT_CONTEXT.get_or_init(|| Mutex::new(None))) = Some(context.clone());
        ACTIVE.store(true, Ordering::Release);
        match thread::Builder::new()
            .name("rng-overlay".to_owned())
            .stack_size(256 * 1024)
            .spawn(run_native_overlay)
        {
            Ok(worker) => *lock(&self.worker) = Some(worker),
            Err(error) => {
                ACTIVE.store(false, Ordering::Release);
                log::error!("could not start RnG overlay: {error}");
            }
        }
    }
}

impl Default for RngOverlay {
    fn default() -> Self {
        Self::new()
    }
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

fn request_root_repaint() {
    if let Some(context) = REPAINT_CONTEXT
        .get()
        .and_then(|context| lock(context).clone())
    {
        context.request_repaint_of(egui::ViewportId::ROOT);
    }
}

const WM_REPOSITION: u32 = 0x8001;

static ACTIVE: AtomicBool = AtomicBool::new(false);
static WINDOW_HANDLE: AtomicIsize = AtomicIsize::new(0);
static WORK_LEFT: AtomicI32 = AtomicI32::new(0);
static WORK_TOP: AtomicI32 = AtomicI32::new(0);
static WORK_RIGHT: AtomicI32 = AtomicI32::new(1920);
static WORK_BOTTOM: AtomicI32 = AtomicI32::new(1080);
static REPAINT_CONTEXT: OnceLock<Mutex<Option<egui::Context>>> = OnceLock::new();

static NUMBER: AtomicU32 = AtomicU32::new(1);
static SECONDS_LEFT: AtomicI32 = AtomicI32::new(60);
static RNG_STATE: AtomicU64 = AtomicU64::new(0);
static INTERVAL_SECONDS: AtomicI32 = AtomicI32::new(60);
static NUMBER_COLOR: AtomicU32 = AtomicU32::new(0x00D5FF);
static FONT_SIZE: AtomicI32 = AtomicI32::new(150);
// 0 = top-left, 1 = top-right, 2 = bottom-left, 3 = bottom-right.
static CORNER: AtomicU32 = AtomicU32::new(1);

fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain(Some(0)).collect()
}

fn rgb(red: u8, green: u8, blue: u8) -> u32 {
    red as u32 | ((green as u32) << 8) | ((blue as u32) << 16)
}

fn random_number() -> u32 {
    let mut state = RNG_STATE.load(Ordering::Relaxed);
    if state == 0 {
        state = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos() as u64;
        state ^= 0x9E37_79B9_7F4A_7C15;
    }
    state ^= state << 13;
    state ^= state >> 7;
    state ^= state << 17;
    RNG_STATE.store(state, Ordering::Relaxed);
    (state % 100) as u32 + 1
}

fn settings_path() -> PathBuf {
    let base = std::env::var_os("APPDATA")
        .map(PathBuf::from)
        .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")));
    base.join("SunnyRandomiser").join("settings.txt")
}

fn save_settings() {
    let path = settings_path();
    if let Some(parent) = path.parent()
        && let Err(error) = fs::create_dir_all(parent)
    {
        log::warn!("could not create RnG settings directory: {error}");
        return;
    }
    let contents = format!(
        "interval={}\ncolor={}\nsize={}\ncorner={}\n",
        INTERVAL_SECONDS.load(Ordering::Relaxed),
        NUMBER_COLOR.load(Ordering::Relaxed),
        FONT_SIZE.load(Ordering::Relaxed),
        CORNER.load(Ordering::Relaxed),
    );
    if let Err(error) = fs::write(path, contents) {
        log::warn!("could not save RnG settings: {error}");
    }
}

fn load_settings() {
    let Ok(contents) = fs::read_to_string(settings_path()) else {
        return;
    };
    for line in contents.lines() {
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        match key {
            "interval" => {
                if let Ok(value) = value.parse::<i32>() {
                    INTERVAL_SECONDS.store(value.clamp(1, 86_400), Ordering::Relaxed);
                }
            }
            "color" => {
                if let Ok(value) = value.parse::<u32>() {
                    NUMBER_COLOR.store(value, Ordering::Relaxed);
                }
            }
            "size" => {
                if let Ok(value) = value.parse::<i32>() {
                    FONT_SIZE.store(value.clamp(40, 400), Ordering::Relaxed);
                }
            }
            "corner" => {
                if let Ok(value) = value.parse::<u32>() {
                    CORNER.store(value.min(3), Ordering::Relaxed);
                }
            }
            _ => {}
        }
    }
    SECONDS_LEFT.store(INTERVAL_SECONDS.load(Ordering::Relaxed), Ordering::Relaxed);
}

fn reroll(hwnd: HWND) {
    NUMBER.store(random_number(), Ordering::Relaxed);
    SECONDS_LEFT.store(INTERVAL_SECONDS.load(Ordering::Relaxed), Ordering::Relaxed);
    unsafe {
        let _ = InvalidateRect(Some(hwnd), None, false);
    }
}

fn menu_flags(active: bool) -> MENU_ITEM_FLAGS {
    MF_STRING
        | if active {
            MF_CHECKED
        } else {
            MENU_ITEM_FLAGS(0)
        }
}

unsafe fn add_menu_item(menu: HMENU, id: usize, label: &str, active: bool) {
    let label = wide(label);
    if let Err(error) = unsafe { AppendMenuW(menu, menu_flags(active), id, PCWSTR(label.as_ptr())) }
    {
        log::warn!("could not add an RnG settings menu item: {error}");
    }
}

unsafe fn add_submenu(parent: HMENU, submenu: HMENU, label: &str) {
    let label = wide(label);
    if let Err(error) =
        unsafe { AppendMenuW(parent, MF_POPUP, submenu.0 as usize, PCWSTR(label.as_ptr())) }
    {
        log::warn!("could not add an RnG settings submenu: {error}");
    }
}

fn apply_layout(hwnd: HWND) {
    let font_size = FONT_SIZE.load(Ordering::Relaxed);
    // Leave enough invisible space for three wide digits plus the font's full
    // ascent/descent. A tight box clips small glyphs at their baseline.
    let width = (font_size * 5 / 2 + 32).max(140);
    let height = (font_size * 3 / 2 + 32).max(100);
    let margin = 24;
    let work_area = RECT {
        left: WORK_LEFT.load(Ordering::Acquire),
        top: WORK_TOP.load(Ordering::Acquire),
        right: WORK_RIGHT.load(Ordering::Acquire),
        bottom: WORK_BOTTOM.load(Ordering::Acquire),
    };
    let corner = CORNER.load(Ordering::Relaxed);
    let x = if corner == 0 || corner == 2 {
        work_area.left + margin
    } else {
        work_area.right - width - margin
    };
    let y = if corner == 0 || corner == 1 {
        work_area.top + margin
    } else {
        work_area.bottom - height - margin
    };
    if let Err(error) = unsafe {
        SetWindowPos(
            hwnd,
            Some(HWND_TOPMOST),
            x,
            y,
            width,
            height,
            SWP_NOACTIVATE | SWP_SHOWWINDOW,
        )
    } {
        log::warn!("could not position RnG overlay: {error}");
    }
    unsafe {
        let _ = InvalidateRect(Some(hwnd), None, false);
    }
}

fn adjust_size(hwnd: HWND, change: i32) {
    let current = FONT_SIZE.load(Ordering::Relaxed);
    FONT_SIZE.store((current + change).clamp(40, 400), Ordering::Relaxed);
    save_settings();
    apply_layout(hwnd);
}

unsafe fn show_settings(hwnd: HWND) {
    let Ok(root) = (unsafe { CreatePopupMenu() }) else {
        log::warn!("could not create the RnG settings menu");
        return;
    };
    let Ok(interval) = (unsafe { CreatePopupMenu() }) else {
        let _ = unsafe { DestroyMenu(root) };
        log::warn!("could not create the RnG interval menu");
        return;
    };
    let Ok(colors) = (unsafe { CreatePopupMenu() }) else {
        let _ = unsafe { DestroyMenu(root) };
        let _ = unsafe { DestroyMenu(interval) };
        log::warn!("could not create the RnG color menu");
        return;
    };
    let Ok(sizes) = (unsafe { CreatePopupMenu() }) else {
        let _ = unsafe { DestroyMenu(root) };
        let _ = unsafe { DestroyMenu(interval) };
        let _ = unsafe { DestroyMenu(colors) };
        log::warn!("could not create the RnG size menu");
        return;
    };
    let Ok(corners) = (unsafe { CreatePopupMenu() }) else {
        let _ = unsafe { DestroyMenu(root) };
        let _ = unsafe { DestroyMenu(interval) };
        let _ = unsafe { DestroyMenu(colors) };
        let _ = unsafe { DestroyMenu(sizes) };
        log::warn!("could not create the RnG corner menu");
        return;
    };

    let current_interval = INTERVAL_SECONDS.load(Ordering::Relaxed);
    unsafe {
        add_menu_item(interval, 110, "10 seconds", current_interval == 10);
    }
    unsafe {
        add_menu_item(interval, 130, "30 seconds", current_interval == 30);
    }
    unsafe {
        add_menu_item(interval, 160, "1 minute", current_interval == 60);
    }
    unsafe {
        add_menu_item(interval, 300, "5 minutes", current_interval == 300);
    }
    unsafe {
        add_menu_item(interval, 600, "10 minutes", current_interval == 600);
    }

    let current_color = NUMBER_COLOR.load(Ordering::Relaxed);
    unsafe {
        add_menu_item(colors, 201, "Yellow", current_color == rgb(255, 213, 0));
    }
    unsafe {
        add_menu_item(colors, 202, "White", current_color == rgb(255, 255, 255));
    }
    unsafe {
        add_menu_item(colors, 203, "Orange", current_color == rgb(255, 128, 0));
    }
    unsafe {
        add_menu_item(colors, 204, "Green", current_color == rgb(76, 235, 115));
    }
    unsafe {
        add_menu_item(colors, 205, "Blue", current_color == rgb(74, 163, 255));
    }
    unsafe {
        add_menu_item(colors, 206, "Pink", current_color == rgb(255, 91, 174));
    }

    let current_size = FONT_SIZE.load(Ordering::Relaxed);
    unsafe {
        add_menu_item(sizes, 301, "Smaller (-10 px)", false);
    }
    unsafe {
        add_menu_item(sizes, 302, "Larger (+10 px)", false);
    }
    unsafe {
        add_menu_item(sizes, 303, "Reset to 150 px", current_size == 150);
    }

    let current_corner = CORNER.load(Ordering::Relaxed);
    unsafe {
        add_menu_item(corners, 401, "Top left", current_corner == 0);
    }
    unsafe {
        add_menu_item(corners, 402, "Top right", current_corner == 1);
    }
    unsafe {
        add_menu_item(corners, 403, "Bottom left", current_corner == 2);
    }
    unsafe {
        add_menu_item(corners, 404, "Bottom right", current_corner == 3);
    }

    unsafe {
        add_submenu(root, interval, "Interval");
    }
    unsafe {
        add_submenu(root, colors, "Color");
    }
    unsafe {
        add_submenu(root, sizes, &format!("Size ({current_size} px)"));
    }
    unsafe {
        add_submenu(root, corners, "Corner");
    }
    unsafe {
        if let Err(error) = AppendMenuW(root, MF_SEPARATOR, 0, PCWSTR::null()) {
            log::warn!("could not add the RnG menu separator: {error}");
        }
    }
    unsafe {
        add_menu_item(root, 900, "Turn RnG off", false);
    }

    let mut cursor = POINT::default();
    if let Err(error) = unsafe { GetCursorPos(&mut cursor) } {
        log::warn!("could not read the cursor position for the RnG menu: {error}");
    }
    unsafe {
        let _ = SetForegroundWindow(hwnd);
    }
    let command = unsafe {
        TrackPopupMenu(
            root,
            TPM_RIGHTBUTTON | TPM_NONOTIFY | TPM_RETURNCMD,
            cursor.x,
            cursor.y,
            None,
            hwnd,
            None,
        )
    }
    .0 as u32;

    match command {
        110 => INTERVAL_SECONDS.store(10, Ordering::Relaxed),
        130 => INTERVAL_SECONDS.store(30, Ordering::Relaxed),
        160 => INTERVAL_SECONDS.store(60, Ordering::Relaxed),
        300 => INTERVAL_SECONDS.store(300, Ordering::Relaxed),
        600 => INTERVAL_SECONDS.store(600, Ordering::Relaxed),
        201 => NUMBER_COLOR.store(rgb(255, 213, 0), Ordering::Relaxed),
        202 => NUMBER_COLOR.store(rgb(255, 255, 255), Ordering::Relaxed),
        203 => NUMBER_COLOR.store(rgb(255, 128, 0), Ordering::Relaxed),
        204 => NUMBER_COLOR.store(rgb(76, 235, 115), Ordering::Relaxed),
        205 => NUMBER_COLOR.store(rgb(74, 163, 255), Ordering::Relaxed),
        206 => NUMBER_COLOR.store(rgb(255, 91, 174), Ordering::Relaxed),
        301 => adjust_size(hwnd, -10),
        302 => adjust_size(hwnd, 10),
        303 => FONT_SIZE.store(150, Ordering::Relaxed),
        401 => CORNER.store(0, Ordering::Relaxed),
        402 => CORNER.store(1, Ordering::Relaxed),
        403 => CORNER.store(2, Ordering::Relaxed),
        404 => CORNER.store(3, Ordering::Relaxed),
        900 => unsafe {
            if let Err(error) = DestroyWindow(hwnd) {
                log::warn!("could not close the RnG overlay: {error}");
            }
        },
        _ => {}
    }

    if matches!(command, 110 | 130 | 160 | 300 | 600) {
        SECONDS_LEFT.store(INTERVAL_SECONDS.load(Ordering::Relaxed), Ordering::Relaxed);
    }
    if command != 0 && command != 900 {
        save_settings();
    }
    if matches!(command, 301..=303 | 401..=404) {
        apply_layout(hwnd);
    } else if command != 0 && command != 900 {
        unsafe {
            let _ = InvalidateRect(Some(hwnd), None, false);
        }
    }
    if let Err(error) = unsafe { DestroyMenu(root) } {
        log::warn!("could not destroy the RnG settings menu: {error}");
    }
}

unsafe extern "system" fn window_proc(
    hwnd: HWND,
    msg: u32,
    w_param: WPARAM,
    l_param: LPARAM,
) -> LRESULT {
    match msg {
        WM_PAINT => {
            unsafe {
                paint(hwnd);
            }
            LRESULT(0)
        }
        WM_TIMER => {
            let remaining = SECONDS_LEFT.fetch_sub(1, Ordering::Relaxed) - 1;
            if remaining <= 0 {
                reroll(hwnd);
            } else {
                unsafe {
                    let _ = InvalidateRect(Some(hwnd), None, false);
                }
            }
            LRESULT(0)
        }
        WM_LBUTTONDOWN => {
            // The number is the control: click anywhere to generate immediately.
            reroll(hwnd);
            LRESULT(0)
        }
        WM_RBUTTONDOWN => {
            unsafe {
                show_settings(hwnd);
            }
            LRESULT(0)
        }
        WM_MOUSEWHEEL => {
            let wheel_delta = ((w_param.0 >> 16) & 0xFFFF) as u16 as i16;
            if wheel_delta > 0 {
                adjust_size(hwnd, 5);
            } else if wheel_delta < 0 {
                adjust_size(hwnd, -5);
            }
            LRESULT(0)
        }
        WM_REPOSITION => {
            apply_layout(hwnd);
            LRESULT(0)
        }
        WM_DESTROY => {
            WINDOW_HANDLE.store(0, Ordering::Release);
            ACTIVE.store(false, Ordering::Release);
            request_root_repaint();
            unsafe {
                PostQuitMessage(0);
            }
            LRESULT(0)
        }
        _ => unsafe { DefWindowProcW(hwnd, msg, w_param, l_param) },
    }
}

unsafe fn paint(hwnd: HWND) {
    let mut paint = PAINTSTRUCT::default();
    let hdc = unsafe { BeginPaint(hwnd, &mut paint) };
    let mut client = RECT::default();
    if let Err(error) = unsafe { GetClientRect(hwnd, &mut client) } {
        log::warn!("could not read the RnG overlay bounds: {error}");
        unsafe {
            let _ = EndPaint(hwnd, &paint);
        }
        return;
    }

    // This exact color is removed by the layered-window color key, leaving
    // only the painted yellow number visible on the desktop.
    let background = unsafe { CreateSolidBrush(COLORREF(rgb(1, 2, 3))) };
    unsafe {
        FillRect(hdc, &client, background);
    }
    unsafe {
        let _ = DeleteObject(background.into());
    }
    unsafe {
        SetBkMode(hdc, TRANSPARENT);
    }

    let number_font = unsafe {
        CreateFontW(
            -FONT_SIZE.load(Ordering::Relaxed),
            0,
            0,
            0,
            FW_BOLD.0 as i32,
            0,
            0,
            0,
            DEFAULT_CHARSET,
            OUT_DEFAULT_PRECIS,
            CLIP_DEFAULT_PRECIS,
            NONANTIALIASED_QUALITY,
            u32::from(DEFAULT_PITCH.0),
            w!("Segoe UI"),
        )
    };
    let old = unsafe { SelectObject(hdc, number_font.into()) };
    unsafe {
        SetTextColor(hdc, COLORREF(NUMBER_COLOR.load(Ordering::Relaxed)));
    }
    let mut number_text: Vec<_> = NUMBER
        .load(Ordering::Relaxed)
        .to_string()
        .encode_utf16()
        .collect();
    // Draw into nearly the whole transparent window. The window dimensions
    // already provide scalable padding, so this rect never becomes shorter
    // than the selected font.
    let mut number_rect = RECT {
        left: 8,
        top: 8,
        right: client.right - 8,
        bottom: client.bottom - 8,
    };
    unsafe {
        DrawTextW(
            hdc,
            &mut number_text,
            &mut number_rect,
            DT_CENTER | DT_VCENTER | DT_SINGLELINE,
        );
    }

    unsafe {
        SelectObject(hdc, old);
    }
    unsafe {
        let _ = DeleteObject(number_font.into());
    }
    unsafe {
        let _ = EndPaint(hwnd, &paint);
    }
}

fn run_native_overlay() {
    load_settings();
    NUMBER.store(random_number(), Ordering::Relaxed);
    unsafe {
        let class_name = w!("TableArrangerRngOverlayWindow");
        let cursor = match LoadCursorW(None, IDC_ARROW) {
            Ok(cursor) => cursor,
            Err(error) => {
                ACTIVE.store(false, Ordering::Release);
                log::error!("could not load the RnG cursor: {error}");
                request_root_repaint();
                return;
            }
        };
        let class = WNDCLASSW {
            style: CS_HREDRAW | CS_VREDRAW,
            lpfnWndProc: Some(window_proc),
            hCursor: cursor,
            lpszClassName: class_name,
            ..Default::default()
        };
        if RegisterClassW(&class) == 0 {
            log::debug!("RnG window class was already registered or could not be registered");
        }

        let width = 280;
        let height = 180;
        let hwnd = match CreateWindowExW(
            WS_EX_LAYERED | WS_EX_TOPMOST | WS_EX_TOOLWINDOW,
            class_name,
            w!("Table Arranger Control — RnG"),
            WS_POPUP | WS_VISIBLE,
            0,
            0,
            width,
            height,
            None,
            None,
            None,
            None,
        ) {
            Ok(hwnd) => hwnd,
            Err(error) => {
                ACTIVE.store(false, Ordering::Release);
                log::error!("could not create the RnG overlay window: {error}");
                request_root_repaint();
                return;
            }
        };
        WINDOW_HANDLE.store(hwnd.0 as isize, Ordering::Release);
        if !ACTIVE.load(Ordering::Acquire) {
            let _ = DestroyWindow(hwnd);
            return;
        }
        if let Err(error) =
            SetLayeredWindowAttributes(hwnd, COLORREF(rgb(1, 2, 3)), 0, LWA_COLORKEY)
        {
            log::error!("could not enable RnG overlay transparency: {error}");
            let _ = DestroyWindow(hwnd);
            return;
        }
        apply_layout(hwnd);
        if SetTimer(Some(hwnd), 1, 1000, None) == 0 {
            log::warn!("could not start the RnG interval timer");
        }
        let _ = ShowWindow(hwnd, SW_SHOW);
        let _ = UpdateWindow(hwnd);

        let mut msg = MSG::default();
        loop {
            let result = GetMessageW(&mut msg, None, 0, 0).0;
            if result <= 0 {
                if result < 0 {
                    log::error!("RnG overlay message loop failed");
                }
                break;
            }
            let _ = TranslateMessage(&msg);
            let _ = DispatchMessageW(&msg);
        }
        WINDOW_HANDLE.store(0, Ordering::Release);
        ACTIVE.store(false, Ordering::Release);
        request_root_repaint();
    }
}
