use std::ffi::c_void;
use std::fs;
use std::mem::zeroed;
use std::path::PathBuf;
use std::ptr::{null, null_mut};
use std::sync::{
    Mutex, MutexGuard, OnceLock,
    atomic::{AtomicBool, AtomicI32, AtomicIsize, AtomicU32, AtomicU64, Ordering},
};
use std::thread::{self, JoinHandle};

use eframe::egui;

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
            unsafe {
                PostMessageW(hwnd as Hwnd, WM_REPOSITION, 0, 0);
            }
        }
    }

    pub fn stop(&self) {
        ACTIVE.store(false, Ordering::Release);
        let hwnd = WINDOW_HANDLE.load(Ordering::Acquire);
        if hwnd != 0 {
            unsafe {
                PostMessageW(hwnd as Hwnd, WM_CLOSE, 0, 0);
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

type Hwnd = *mut c_void;
type Hinstance = *mut c_void;
type Hdc = *mut c_void;
type Hbrush = *mut c_void;
type Hfont = *mut c_void;
type Hobject = *mut c_void;
type Hcursor = *mut c_void;
type Hmenu = *mut c_void;
type Lparam = isize;
type Wparam = usize;
type Lresult = isize;

const WM_DESTROY: u32 = 0x0002;
const WM_CLOSE: u32 = 0x0010;
const WM_REPOSITION: u32 = 0x8001;
const WM_PAINT: u32 = 0x000F;
const WM_TIMER: u32 = 0x0113;
const WM_LBUTTONDOWN: u32 = 0x0201;
const WM_RBUTTONDOWN: u32 = 0x0204;
const WM_MOUSEWHEEL: u32 = 0x020A;
const CS_HREDRAW: u32 = 0x0002;
const CS_VREDRAW: u32 = 0x0001;
const WS_POPUP: u32 = 0x80000000;
const WS_VISIBLE: u32 = 0x10000000;
const WS_EX_TOPMOST: u32 = 0x00000008;
const WS_EX_TOOLWINDOW: u32 = 0x00000080;
const WS_EX_LAYERED: u32 = 0x00080000;
const LWA_COLORKEY: u32 = 0x00000001;
const SW_SHOW: i32 = 5;
const SWP_NOACTIVATE: u32 = 0x0010;
const SWP_SHOWWINDOW: u32 = 0x0040;
const MF_STRING: u32 = 0x0000;
const MF_CHECKED: u32 = 0x0008;
const MF_POPUP: u32 = 0x0010;
const MF_SEPARATOR: u32 = 0x0800;
const TPM_RIGHTBUTTON: u32 = 0x0002;
const TPM_NONOTIFY: u32 = 0x0080;
const TPM_RETURNCMD: u32 = 0x0100;
const DT_CENTER: u32 = 0x00000001;
const DT_VCENTER: u32 = 0x00000004;
const DT_SINGLELINE: u32 = 0x00000020;
const TRANSPARENT: i32 = 1;
const FW_BOLD: i32 = 700;
const DEFAULT_CHARSET: u32 = 1;
const OUT_DEFAULT_PRECIS: u32 = 0;
const CLIP_DEFAULT_PRECIS: u32 = 0;
// Color-key transparency needs solid glyph pixels; antialiasing would blend
// edge pixels with the invisible key color and create a faint halo.
const NONANTIALIASED_QUALITY: u32 = 3;
const DEFAULT_PITCH: u32 = 0;
const IDC_ARROW: *const u16 = 32512usize as *const u16;

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

#[repr(C)]
struct Point {
    x: i32,
    y: i32,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct Rect {
    left: i32,
    top: i32,
    right: i32,
    bottom: i32,
}

#[repr(C)]
struct PaintStruct {
    hdc: Hdc,
    erase: i32,
    paint: Rect,
    restore: i32,
    inc_update: i32,
    reserved: [u8; 32],
}

#[repr(C)]
struct Msg {
    hwnd: Hwnd,
    message: u32,
    w_param: Wparam,
    l_param: Lparam,
    time: u32,
    point: Point,
    private: u32,
}

#[repr(C)]
struct WndClassW {
    style: u32,
    wnd_proc: Option<unsafe extern "system" fn(Hwnd, u32, Wparam, Lparam) -> Lresult>,
    cls_extra: i32,
    wnd_extra: i32,
    instance: Hinstance,
    icon: *mut c_void,
    cursor: Hcursor,
    background: Hbrush,
    menu_name: *const u16,
    class_name: *const u16,
}

#[link(name = "user32")]
unsafe extern "system" {
    fn RegisterClassW(class: *const WndClassW) -> u16;
    fn CreateWindowExW(
        ex_style: u32,
        class_name: *const u16,
        window_name: *const u16,
        style: u32,
        x: i32,
        y: i32,
        width: i32,
        height: i32,
        parent: Hwnd,
        menu: *mut c_void,
        instance: Hinstance,
        param: *mut c_void,
    ) -> Hwnd;
    fn DefWindowProcW(hwnd: Hwnd, msg: u32, w_param: Wparam, l_param: Lparam) -> Lresult;
    fn ShowWindow(hwnd: Hwnd, command: i32) -> i32;
    fn UpdateWindow(hwnd: Hwnd) -> i32;
    fn GetMessageW(msg: *mut Msg, hwnd: Hwnd, min: u32, max: u32) -> i32;
    fn TranslateMessage(msg: *const Msg) -> i32;
    fn DispatchMessageW(msg: *const Msg) -> Lresult;
    fn PostQuitMessage(exit_code: i32);
    fn BeginPaint(hwnd: Hwnd, paint: *mut PaintStruct) -> Hdc;
    fn EndPaint(hwnd: Hwnd, paint: *const PaintStruct) -> i32;
    fn GetClientRect(hwnd: Hwnd, rect: *mut Rect) -> i32;
    fn FillRect(hdc: Hdc, rect: *const Rect, brush: Hbrush) -> i32;
    fn DrawTextW(hdc: Hdc, text: *const u16, count: i32, rect: *mut Rect, format: u32) -> i32;
    fn SetTimer(hwnd: Hwnd, id: usize, delay_ms: u32, callback: *mut c_void) -> usize;
    fn InvalidateRect(hwnd: Hwnd, rect: *const Rect, erase: i32) -> i32;
    fn LoadCursorW(instance: Hinstance, cursor_name: *const u16) -> Hcursor;
    fn SetLayeredWindowAttributes(hwnd: Hwnd, color_key: u32, alpha: u8, flags: u32) -> i32;
    fn DestroyWindow(hwnd: Hwnd) -> i32;
    fn CreatePopupMenu() -> Hmenu;
    fn AppendMenuW(menu: Hmenu, flags: u32, item: usize, text: *const u16) -> i32;
    fn TrackPopupMenu(
        menu: Hmenu,
        flags: u32,
        x: i32,
        y: i32,
        reserved: i32,
        hwnd: Hwnd,
        rect: *const Rect,
    ) -> u32;
    fn DestroyMenu(menu: Hmenu) -> i32;
    fn GetCursorPos(point: *mut Point) -> i32;
    fn SetForegroundWindow(hwnd: Hwnd) -> i32;
    fn SetWindowPos(
        hwnd: Hwnd,
        insert_after: Hwnd,
        x: i32,
        y: i32,
        width: i32,
        height: i32,
        flags: u32,
    ) -> i32;
    fn PostMessageW(hwnd: Hwnd, message: u32, w_param: Wparam, l_param: Lparam) -> i32;
}

#[link(name = "gdi32")]
unsafe extern "system" {
    fn CreateSolidBrush(color: u32) -> Hbrush;
    fn DeleteObject(object: Hobject) -> i32;
    fn SetTextColor(hdc: Hdc, color: u32) -> u32;
    fn SetBkMode(hdc: Hdc, mode: i32) -> i32;
    fn CreateFontW(
        height: i32,
        width: i32,
        escapement: i32,
        orientation: i32,
        weight: i32,
        italic: u32,
        underline: u32,
        strike_out: u32,
        charset: u32,
        out_precision: u32,
        clip_precision: u32,
        quality: u32,
        pitch_and_family: u32,
        face: *const u16,
    ) -> Hfont;
    fn SelectObject(hdc: Hdc, object: Hobject) -> Hobject;
}

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
    if let Some(parent) = path.parent() {
        let _ = fs::create_dir_all(parent);
    }
    let contents = format!(
        "interval={}\ncolor={}\nsize={}\ncorner={}\n",
        INTERVAL_SECONDS.load(Ordering::Relaxed),
        NUMBER_COLOR.load(Ordering::Relaxed),
        FONT_SIZE.load(Ordering::Relaxed),
        CORNER.load(Ordering::Relaxed),
    );
    let _ = fs::write(path, contents);
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

fn reroll(hwnd: Hwnd) {
    NUMBER.store(random_number(), Ordering::Relaxed);
    SECONDS_LEFT.store(INTERVAL_SECONDS.load(Ordering::Relaxed), Ordering::Relaxed);
    unsafe {
        InvalidateRect(hwnd, null(), 0);
    }
}

fn menu_flags(active: bool) -> u32 {
    MF_STRING | if active { MF_CHECKED } else { 0 }
}

unsafe fn add_menu_item(menu: Hmenu, id: usize, label: &str, active: bool) {
    let label = wide(label);
    unsafe {
        AppendMenuW(menu, menu_flags(active), id, label.as_ptr());
    }
}

unsafe fn add_submenu(parent: Hmenu, submenu: Hmenu, label: &str) {
    let label = wide(label);
    unsafe {
        AppendMenuW(parent, MF_POPUP, submenu as usize, label.as_ptr());
    }
}

fn apply_layout(hwnd: Hwnd) {
    let font_size = FONT_SIZE.load(Ordering::Relaxed);
    // Leave enough invisible space for three wide digits plus the font's full
    // ascent/descent. A tight box clips small glyphs at their baseline.
    let width = (font_size * 5 / 2 + 32).max(140);
    let height = (font_size * 3 / 2 + 32).max(100);
    let margin = 24;
    let work_area = Rect {
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
    let topmost = -1isize as Hwnd;
    unsafe {
        SetWindowPos(
            hwnd,
            topmost,
            x,
            y,
            width,
            height,
            SWP_NOACTIVATE | SWP_SHOWWINDOW,
        );
    }
    unsafe {
        InvalidateRect(hwnd, null(), 0);
    }
}

fn adjust_size(hwnd: Hwnd, change: i32) {
    let current = FONT_SIZE.load(Ordering::Relaxed);
    FONT_SIZE.store((current + change).clamp(40, 400), Ordering::Relaxed);
    save_settings();
    apply_layout(hwnd);
}

unsafe fn show_settings(hwnd: Hwnd) {
    let root = unsafe { CreatePopupMenu() };
    let interval = unsafe { CreatePopupMenu() };
    let colors = unsafe { CreatePopupMenu() };
    let sizes = unsafe { CreatePopupMenu() };
    let corners = unsafe { CreatePopupMenu() };

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
        AppendMenuW(root, MF_SEPARATOR, 0, null());
    }
    unsafe {
        add_menu_item(root, 900, "Turn RnG off", false);
    }

    let mut cursor = Point { x: 0, y: 0 };
    unsafe {
        GetCursorPos(&mut cursor);
    }
    unsafe {
        SetForegroundWindow(hwnd);
    }
    let command = unsafe {
        TrackPopupMenu(
            root,
            TPM_RIGHTBUTTON | TPM_NONOTIFY | TPM_RETURNCMD,
            cursor.x,
            cursor.y,
            0,
            hwnd,
            null(),
        )
    };

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
            DestroyWindow(hwnd);
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
            InvalidateRect(hwnd, null(), 0);
        }
    }
    unsafe {
        DestroyMenu(root);
    }
}

unsafe extern "system" fn window_proc(
    hwnd: Hwnd,
    msg: u32,
    w_param: Wparam,
    l_param: Lparam,
) -> Lresult {
    match msg {
        WM_PAINT => {
            unsafe {
                paint(hwnd);
            }
            0
        }
        WM_TIMER => {
            let remaining = SECONDS_LEFT.fetch_sub(1, Ordering::Relaxed) - 1;
            if remaining <= 0 {
                reroll(hwnd);
            } else {
                unsafe {
                    InvalidateRect(hwnd, null(), 0);
                }
            }
            0
        }
        WM_LBUTTONDOWN => {
            // The number is the control: click anywhere to generate immediately.
            reroll(hwnd);
            0
        }
        WM_RBUTTONDOWN => {
            unsafe {
                show_settings(hwnd);
            }
            0
        }
        WM_MOUSEWHEEL => {
            let wheel_delta = ((w_param >> 16) & 0xFFFF) as u16 as i16;
            if wheel_delta > 0 {
                adjust_size(hwnd, 5);
            } else if wheel_delta < 0 {
                adjust_size(hwnd, -5);
            }
            0
        }
        WM_REPOSITION => {
            apply_layout(hwnd);
            0
        }
        WM_DESTROY => {
            WINDOW_HANDLE.store(0, Ordering::Release);
            ACTIVE.store(false, Ordering::Release);
            request_root_repaint();
            unsafe {
                PostQuitMessage(0);
            }
            0
        }
        _ => unsafe { DefWindowProcW(hwnd, msg, w_param, l_param) },
    }
}

unsafe fn paint(hwnd: Hwnd) {
    let mut paint: PaintStruct = unsafe { zeroed() };
    let hdc = unsafe { BeginPaint(hwnd, &mut paint) };
    let mut client: Rect = unsafe { zeroed() };
    unsafe {
        GetClientRect(hwnd, &mut client);
    }

    // This exact color is removed by the layered-window color key, leaving
    // only the painted yellow number visible on the desktop.
    let background = unsafe { CreateSolidBrush(rgb(1, 2, 3)) };
    unsafe {
        FillRect(hdc, &client, background);
    }
    unsafe {
        DeleteObject(background);
    }
    unsafe {
        SetBkMode(hdc, TRANSPARENT);
    }

    let face = wide("Segoe UI");
    let number_font = unsafe {
        CreateFontW(
            -FONT_SIZE.load(Ordering::Relaxed),
            0,
            0,
            0,
            FW_BOLD,
            0,
            0,
            0,
            DEFAULT_CHARSET,
            OUT_DEFAULT_PRECIS,
            CLIP_DEFAULT_PRECIS,
            NONANTIALIASED_QUALITY,
            DEFAULT_PITCH,
            face.as_ptr(),
        )
    };
    let old = unsafe { SelectObject(hdc, number_font) };
    unsafe {
        SetTextColor(hdc, NUMBER_COLOR.load(Ordering::Relaxed));
    }
    let number_text = wide(&NUMBER.load(Ordering::Relaxed).to_string());
    // Draw into nearly the whole transparent window. The window dimensions
    // already provide scalable padding, so this rect never becomes shorter
    // than the selected font.
    let mut number_rect = Rect {
        left: 8,
        top: 8,
        right: client.right - 8,
        bottom: client.bottom - 8,
    };
    unsafe {
        DrawTextW(
            hdc,
            number_text.as_ptr(),
            -1,
            &mut number_rect,
            DT_CENTER | DT_VCENTER | DT_SINGLELINE,
        );
    }

    unsafe {
        SelectObject(hdc, old);
    }
    unsafe {
        DeleteObject(number_font);
    }
    unsafe {
        EndPaint(hwnd, &paint);
    }
}

fn run_native_overlay() {
    load_settings();
    NUMBER.store(random_number(), Ordering::Relaxed);
    unsafe {
        let instance = null_mut();
        let class_name = wide("TableArrangerRngOverlayWindow");
        let class = WndClassW {
            style: CS_HREDRAW | CS_VREDRAW,
            wnd_proc: Some(window_proc),
            cls_extra: 0,
            wnd_extra: 0,
            instance,
            icon: null_mut(),
            cursor: LoadCursorW(null_mut(), IDC_ARROW),
            background: null_mut(),
            menu_name: null(),
            class_name: class_name.as_ptr(),
        };
        let _ = RegisterClassW(&class);

        let title = wide("Table Arranger Control — RnG");
        let width = 280;
        let height = 180;
        let hwnd = CreateWindowExW(
            WS_EX_LAYERED | WS_EX_TOPMOST | WS_EX_TOOLWINDOW,
            class_name.as_ptr(),
            title.as_ptr(),
            WS_POPUP | WS_VISIBLE,
            0,
            0,
            width,
            height,
            null_mut(),
            null_mut(),
            instance,
            null_mut(),
        );
        if hwnd.is_null() {
            ACTIVE.store(false, Ordering::Release);
            request_root_repaint();
            return;
        }
        WINDOW_HANDLE.store(hwnd as isize, Ordering::Release);
        if !ACTIVE.load(Ordering::Acquire) {
            DestroyWindow(hwnd);
            return;
        }
        SetLayeredWindowAttributes(hwnd, rgb(1, 2, 3), 0, LWA_COLORKEY);
        apply_layout(hwnd);
        SetTimer(hwnd, 1, 1000, null_mut());
        ShowWindow(hwnd, SW_SHOW);
        UpdateWindow(hwnd);

        let mut msg: Msg = zeroed();
        while GetMessageW(&mut msg, null_mut(), 0, 0) > 0 {
            TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
        WINDOW_HANDLE.store(0, Ordering::Release);
        ACTIVE.store(false, Ordering::Release);
        request_root_repaint();
    }
}
