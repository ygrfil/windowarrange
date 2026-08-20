use std::{
    collections::{HashMap, HashSet},
    ffi::c_void,
    mem::size_of,
    sync::{
        Arc, OnceLock,
        atomic::{AtomicBool, Ordering},
    },
    thread,
    time::Duration,
};

use crossbeam_channel::Sender;
use windows::{
    Win32::{
        Foundation::{
            CloseHandle, ERROR_ACCESS_DENIED, ERROR_ALREADY_EXISTS, GetLastError, HANDLE, HWND,
            LPARAM, POINT, RECT, WPARAM,
        },
        Graphics::{
            Dwm::{DWMWA_CLOAKED, DwmGetWindowAttribute},
            Gdi::{
                BI_RGB, BITMAPINFO, CreateCompatibleDC, CreateDIBSection, DIB_RGB_COLORS, DeleteDC,
                DeleteObject, EnumDisplayMonitors, GetMonitorInfoW, HDC, HGDIOBJ, HMONITOR,
                MONITOR_DEFAULTTONEAREST, MONITORINFO, MONITORINFOEXW, MonitorFromPoint,
                SelectObject,
            },
        },
        System::{
            Diagnostics::ToolHelp::{
                CreateToolhelp32Snapshot, PROCESSENTRY32W, Process32FirstW, Process32NextW,
                TH32CS_SNAPPROCESS,
            },
            SystemServices::PROCESS_MITIGATION_EXTENSION_POINT_DISABLE_POLICY,
            Threading::{
                CreateMutexW, ProcessExtensionPointDisablePolicy, SetProcessMitigationPolicy,
            },
        },
        UI::{
            Accessibility::{HWINEVENTHOOK, SetWinEventHook},
            HiDpi::{DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2, SetProcessDpiAwarenessContext},
            WindowsAndMessaging::{
                BringWindowToTop, DI_NORMAL, DrawIconEx, EVENT_OBJECT_CREATE, EVENT_OBJECT_DESTROY,
                EVENT_OBJECT_HIDE, EVENT_OBJECT_LOCATIONCHANGE, EVENT_OBJECT_SHOW, EnumWindows,
                FLASHW_ALL, FLASHW_TIMERNOFG, FLASHWINFO, FindWindowW, FlashWindowEx, GA_ROOT,
                GCLP_HICON, GCLP_HICONSM, GWL_EXSTYLE, GWL_STYLE, GetAncestor, GetClassLongPtrW,
                GetClassNameW, GetCursorPos, GetForegroundWindow, GetMessageW, GetWindowLongPtrW,
                GetWindowRect, GetWindowTextLengthW, GetWindowTextW, GetWindowThreadProcessId,
                HICON, HWND_TOP, ICON_BIG, ICON_SMALL, ICON_SMALL2, IsIconic, IsWindowVisible,
                MINMAXINFO, MONITORINFOF_PRIMARY, MSG, OBJID_WINDOW, SET_WINDOW_POS_FLAGS,
                SMTO_ABORTIFHUNG, SW_RESTORE, SW_SHOWNOACTIVATE, SWP_ASYNCWINDOWPOS,
                SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOOWNERZORDER, SWP_NOSIZE, SWP_NOZORDER,
                SWP_SHOWWINDOW, SendMessageTimeoutW, SetForegroundWindow, SetWindowPos,
                ShowWindowAsync, WINEVENT_OUTOFCONTEXT, WINEVENT_SKIPOWNPROCESS, WM_GETICON,
                WM_GETMINMAXINFO, WS_CHILD, WS_DISABLED, WS_EX_TOOLWINDOW,
            },
        },
    },
    core::{BOOL, Error as WindowsError, HSTRING, PCWSTR, w},
};

use crate::{
    controller::ControllerCommand,
    identity::PANEL_TITLE,
    layout::normalized_ldplayer_aspect_ratio,
    model::{
        BackendError, MonitorInfo, PokerClientKind, Rect, Size, WindowBackend, WindowCandidate,
        WindowId, WindowSignature,
    },
};

static WINDOW_EVENT_SENDER: OnceLock<Sender<ControllerCommand>> = OnceLock::new();
static WINDOW_EVENT_PENDING: AtomicBool = AtomicBool::new(false);
const WINDOW_EVENT_STACK_BYTES: usize = 256 * 1024;
const APPLICATION_ICON_SIZE: usize = 64;

pub struct WindowIcon {
    pub size: usize,
    pub rgba: Vec<u8>,
}

/// Loads a window's application icon into owned RGBA pixels without transferring icon ownership.
#[must_use]
pub fn window_icon(id: WindowId) -> Option<WindowIcon> {
    let hwnd = hwnd_from_id(id);
    let icon = query_window_icon(hwnd)?;
    render_window_icon(icon)
}

fn query_window_icon(hwnd: HWND) -> Option<HICON> {
    for icon_kind in [ICON_BIG, ICON_SMALL2, ICON_SMALL] {
        let mut result = 0_usize;
        // SAFETY: hwnd comes from an enumerated top-level window. The call is bounded and writes
        // only to the valid result pointer supplied here.
        unsafe {
            let _ = SendMessageTimeoutW(
                hwnd,
                WM_GETICON,
                WPARAM(icon_kind as usize),
                LPARAM(0),
                SMTO_ABORTIFHUNG,
                100,
                Some(&raw mut result),
            );
        }
        if result != 0 {
            return Some(HICON(result as *mut c_void));
        }
    }

    for class_index in [GCLP_HICON, GCLP_HICONSM] {
        // SAFETY: reading the icon handle associated with a valid window class does not transfer
        // ownership and does not mutate the target window.
        let result = unsafe { GetClassLongPtrW(hwnd, class_index) };
        if result != 0 {
            return Some(HICON(result as *mut c_void));
        }
    }
    None
}

fn render_window_icon(icon: HICON) -> Option<WindowIcon> {
    let size = i32::try_from(APPLICATION_ICON_SIZE).ok()?;
    let mut bitmap_info = BITMAPINFO::default();
    bitmap_info.bmiHeader.biSize =
        u32::try_from(size_of::<windows::Win32::Graphics::Gdi::BITMAPINFOHEADER>()).ok()?;
    bitmap_info.bmiHeader.biWidth = size;
    bitmap_info.bmiHeader.biHeight = -size;
    bitmap_info.bmiHeader.biPlanes = 1;
    bitmap_info.bmiHeader.biBitCount = 32;
    bitmap_info.bmiHeader.biCompression = BI_RGB.0;

    // SAFETY: the compatible memory DC is released on every path below.
    let dc = unsafe { CreateCompatibleDC(None) };
    if dc.0.is_null() {
        return None;
    }
    let mut bits = std::ptr::null_mut::<c_void>();
    // SAFETY: bitmap_info is fully initialized for a top-down 32-bit DIB and bits is a valid
    // output pointer. The returned bitmap remains selected only while the DC is alive.
    let bitmap = unsafe {
        CreateDIBSection(
            Some(dc),
            &raw const bitmap_info,
            DIB_RGB_COLORS,
            &raw mut bits,
            None,
            0,
        )
    };
    let Ok(bitmap) = bitmap else {
        // SAFETY: dc was created successfully above and is not used afterward.
        unsafe {
            let _ = DeleteDC(dc);
        }
        return None;
    };
    // SAFETY: bitmap and dc are valid GDI handles owned by this function.
    let previous = unsafe { SelectObject(dc, HGDIOBJ(bitmap.0)) };
    // SAFETY: the DIB allocation contains exactly size*size*4 bytes.
    unsafe { std::ptr::write_bytes(bits, 0, APPLICATION_ICON_SIZE * APPLICATION_ICON_SIZE * 4) };
    // SAFETY: icon is a borrowed valid window/class icon; drawing does not transfer ownership.
    let drawn = unsafe { DrawIconEx(dc, 0, 0, icon, size, size, 0, None, DI_NORMAL) }.is_ok();

    let rgba = if drawn && !bits.is_null() {
        // SAFETY: CreateDIBSection returned this allocation for the exact byte count below and it
        // stays valid until bitmap is deleted after the copy.
        let bgra = unsafe {
            std::slice::from_raw_parts(
                bits.cast::<u8>(),
                APPLICATION_ICON_SIZE * APPLICATION_ICON_SIZE * 4,
            )
        };
        let has_alpha = bgra.chunks_exact(4).any(|pixel| pixel[3] != 0);
        let mut rgba = Vec::with_capacity(bgra.len());
        for pixel in bgra.chunks_exact(4) {
            let alpha = if has_alpha {
                pixel[3]
            } else if pixel[0] != 0 || pixel[1] != 0 || pixel[2] != 0 {
                u8::MAX
            } else {
                0
            };
            rgba.extend_from_slice(&[pixel[2], pixel[1], pixel[0], alpha]);
        }
        Some(rgba)
    } else {
        None
    };

    // SAFETY: restore the previous selection before deleting our bitmap and DC.
    unsafe {
        if !previous.0.is_null() {
            SelectObject(dc, previous);
        }
        let _ = DeleteObject(HGDIOBJ(bitmap.0));
        let _ = DeleteDC(dc);
    }
    rgba.map(|rgba| WindowIcon {
        size: APPLICATION_ICON_SIZE,
        rgba,
    })
}

pub fn apply_process_mitigations() -> Result<(), BackendError> {
    let mut policy = PROCESS_MITIGATION_EXTENSION_POINT_DISABLE_POLICY::default();
    policy.Anonymous.Flags = 1;
    unsafe {
        SetProcessMitigationPolicy(
            ProcessExtensionPointDisablePolicy,
            std::ptr::from_ref(&policy).cast::<c_void>(),
            size_of::<PROCESS_MITIGATION_EXTENSION_POINT_DISABLE_POLICY>(),
        )
        .map_err(map_windows_error)
    }
}

#[derive(Default)]
pub struct Win32Backend;

impl Win32Backend {
    #[must_use]
    pub fn new() -> Arc<Self> {
        // The manifest is authoritative. This call covers debug launches where resource
        // embedding has not happened yet; ERROR_ACCESS_DENIED simply means DPI was set already.
        unsafe {
            let _ = SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
        }
        Arc::new(Self)
    }
}

impl WindowBackend for Win32Backend {
    fn enumerate_candidates(&self) -> Result<Vec<WindowCandidate>, BackendError> {
        let processes = process_snapshot()?;
        let clubgg_ids = related_clubgg_processes(&processes);
        let mut handles = Vec::<HWND>::new();

        unsafe extern "system" fn collect(hwnd: HWND, lparam: LPARAM) -> BOOL {
            let handles = unsafe { &mut *(lparam.0 as *mut Vec<HWND>) };
            handles.push(hwnd);
            BOOL(1)
        }

        unsafe {
            EnumWindows(
                Some(collect),
                LPARAM(std::ptr::from_mut(&mut handles).cast::<c_void>() as isize),
            )
            .map_err(map_windows_error)?;
        }

        let mut candidates = Vec::new();
        for hwnd in handles {
            if let Some(candidate) = inspect_window(hwnd, &processes, &clubgg_ids) {
                candidates.push(candidate);
            }
        }
        candidates
            .sort_by_key(|candidate| (candidate.rect.top, candidate.rect.left, candidate.id.0));
        Ok(candidates)
    }

    fn monitors(&self) -> Result<Vec<MonitorInfo>, BackendError> {
        let mut monitors = Vec::<MonitorInfo>::new();
        unsafe extern "system" fn collect(
            monitor: HMONITOR,
            _dc: HDC,
            _rect: *mut RECT,
            data: LPARAM,
        ) -> BOOL {
            let output = unsafe { &mut *(data.0 as *mut Vec<MonitorInfo>) };
            let mut info = MONITORINFOEXW::default();
            info.monitorInfo.cbSize =
                u32::try_from(size_of::<MONITORINFOEXW>()).unwrap_or(u32::MAX);
            let ok = unsafe {
                GetMonitorInfoW(monitor, std::ptr::from_mut(&mut info).cast::<MONITORINFO>())
            };
            if !ok.as_bool() {
                return BOOL(1);
            }
            let work = info.monitorInfo.rcWork;
            let device = wide_buffer_to_string(&info.szDevice);
            let width = work.right.saturating_sub(work.left);
            let height = work.bottom.saturating_sub(work.top);
            output.push(MonitorInfo {
                id: device.clone(),
                label: format!("{device} ({width}×{height})"),
                work_area: Rect::new(work.left, work.top, width, height),
                primary: info.monitorInfo.dwFlags & MONITORINFOF_PRIMARY != 0,
            });
            BOOL(1)
        }

        unsafe {
            let ok = EnumDisplayMonitors(
                None,
                None,
                Some(collect),
                LPARAM(std::ptr::from_mut(&mut monitors).cast::<c_void>() as isize),
            );
            if !ok.as_bool() {
                return Err(map_last_error("monitor enumeration failed"));
            }
        }
        monitors.sort_by_key(|monitor| {
            (
                !monitor.primary,
                monitor.work_area.left,
                monitor.work_area.top,
            )
        });
        Ok(monitors)
    }

    fn move_resize(&self, id: WindowId, rect: Rect) -> Result<Rect, BackendError> {
        let hwnd = hwnd_from_id(id);
        unsafe {
            if IsIconic(hwnd).as_bool() {
                let _ = ShowWindowAsync(hwnd, SW_SHOWNOACTIVATE);
            }
            SetWindowPos(
                hwnd,
                None,
                rect.left,
                rect.top,
                rect.width.max(1),
                rect.height.max(1),
                SWP_ASYNCWINDOWPOS | SWP_NOACTIVATE | SWP_NOOWNERZORDER | SWP_NOZORDER,
            )
            .map_err(map_windows_error)?;
        }
        thread::sleep(Duration::from_millis(30));
        window_rect(hwnd)
    }

    fn minimum_size(&self, id: WindowId, aspect_ratio: f64) -> Result<Size, BackendError> {
        let hwnd = hwnd_from_id(id);
        let mut limits = MINMAXINFO::default();
        unsafe {
            let _ = SendMessageTimeoutW(
                hwnd,
                WM_GETMINMAXINFO,
                WPARAM(0),
                LPARAM(std::ptr::from_mut(&mut limits).cast::<c_void>() as isize),
                SMTO_ABORTIFHUNG,
                250,
                None,
            );
        }
        let width = limits.ptMinTrackSize.x;
        let height = limits.ptMinTrackSize.y;
        if width > 0 && height > 0 {
            return Ok(Size::new(width, height));
        }

        let ratio = if aspect_ratio.is_finite() && aspect_ratio > 0.0 {
            aspect_ratio
        } else {
            4.0 / 3.0
        };
        let height = 180;
        let width = (f64::from(height) * ratio).round() as i32;
        Ok(Size::new(width.max(200), height))
    }

    fn locate(&self, id: WindowId) -> Result<(), BackendError> {
        let hwnd = hwnd_from_id(id);
        explicit_locate(hwnd)
    }

    fn foreground_window(&self) -> Option<WindowId> {
        let hwnd = unsafe { GetForegroundWindow() };
        (!hwnd.0.is_null()).then(|| id_from_hwnd(hwnd))
    }
}

fn locate_raise_flags() -> SET_WINDOW_POS_FLAGS {
    // Do not use SWP_NOOWNERZORDER here. ClubGG lobbies are owned top-level surfaces,
    // so their owner chain must be allowed to rise with an explicit Locate request.
    SWP_NOMOVE | SWP_NOSIZE | SWP_SHOWWINDOW
}

fn explicit_locate(hwnd: HWND) -> Result<(), BackendError> {
    unsafe {
        let _ = ShowWindowAsync(hwnd, SW_RESTORE);
        SetWindowPos(hwnd, Some(HWND_TOP), 0, 0, 0, 0, locate_raise_flags())
            .map_err(map_windows_error)?;
        BringWindowToTop(hwnd).map_err(map_windows_error)?;
        if !SetForegroundWindow(hwnd).as_bool() {
            let info = FLASHWINFO {
                cbSize: u32::try_from(size_of::<FLASHWINFO>()).unwrap_or(u32::MAX),
                hwnd,
                dwFlags: FLASHW_ALL | FLASHW_TIMERNOFG,
                uCount: 3,
                dwTimeout: 0,
            };
            let _ = FlashWindowEx(&raw const info);
        }
    }
    Ok(())
}

#[derive(Debug)]
pub struct SingleInstanceGuard {
    handle: HANDLE,
}

impl Drop for SingleInstanceGuard {
    fn drop(&mut self) {
        unsafe {
            let _ = CloseHandle(self.handle);
        }
    }
}

pub fn acquire_single_instance() -> Result<Option<SingleInstanceGuard>, BackendError> {
    let handle = unsafe {
        CreateMutexW(
            None,
            false,
            w!("Local\\TableArrangerControl-48D97DB8-4672-4DD7-A8E9-43656AE6BBFA"),
        )
        .map_err(map_windows_error)?
    };
    if unsafe { GetLastError() } == ERROR_ALREADY_EXISTS {
        unsafe {
            let _ = CloseHandle(handle);
        }
        Ok(None)
    } else {
        Ok(Some(SingleInstanceGuard { handle }))
    }
}

pub fn activate_existing_panel() {
    let title = HSTRING::from(PANEL_TITLE);
    let hwnd = unsafe { FindWindowW(PCWSTR::null(), PCWSTR(title.as_ptr())) };
    if let Ok(hwnd) = hwnd {
        unsafe {
            let _ = ShowWindowAsync(hwnd, SW_SHOWNOACTIVATE);
            let _ = SetForegroundWindow(hwnd);
        }
    }
}

#[must_use]
pub fn panel_is_visible() -> bool {
    let title = HSTRING::from(PANEL_TITLE);
    let Ok(hwnd) = (unsafe { FindWindowW(PCWSTR::null(), PCWSTR(title.as_ptr())) }) else {
        return false;
    };
    unsafe { IsWindowVisible(hwnd).as_bool() && !IsIconic(hwnd).as_bool() }
}

pub fn move_panel_to_cursor() -> bool {
    let title = HSTRING::from(PANEL_TITLE);
    let Ok(hwnd) = (unsafe { FindWindowW(PCWSTR::null(), PCWSTR(title.as_ptr())) }) else {
        return false;
    };
    let mut cursor = POINT::default();
    let mut window = RECT::default();
    let mut monitor = MONITORINFO {
        cbSize: u32::try_from(size_of::<MONITORINFO>()).unwrap_or(u32::MAX),
        ..Default::default()
    };

    // SAFETY: all output pointers reference initialized stack values, and hwnd belongs to the
    // current process. The panel is moved without resizing, activating, or changing Z-order.
    unsafe {
        if GetCursorPos(&raw mut cursor).is_err() || GetWindowRect(hwnd, &raw mut window).is_err() {
            return false;
        }
        let display = MonitorFromPoint(cursor, MONITOR_DEFAULTTONEAREST);
        if !GetMonitorInfoW(display, &raw mut monitor).as_bool() {
            return false;
        }

        let width = window.right.saturating_sub(window.left).max(1);
        let height = window.bottom.saturating_sub(window.top).max(1);
        let work = monitor.rcWork;
        let left = cursor
            .x
            .saturating_sub(width / 2)
            .clamp(work.left, work.right.saturating_sub(width).max(work.left));
        let top = cursor
            .y
            .saturating_sub(height / 2)
            .clamp(work.top, work.bottom.saturating_sub(height).max(work.top));
        SetWindowPos(
            hwnd,
            None,
            left,
            top,
            0,
            0,
            SWP_ASYNCWINDOWPOS | SWP_NOACTIVATE | SWP_NOOWNERZORDER | SWP_NOSIZE | SWP_NOZORDER,
        )
        .is_ok()
    }
}

pub fn spawn_window_event_watcher(sender: Sender<ControllerCommand>) {
    let _ = WINDOW_EVENT_SENDER.set(sender);
    thread::Builder::new()
        .name("clubgg-win-events".to_owned())
        .stack_size(WINDOW_EVENT_STACK_BYTES)
        .spawn(move || unsafe {
            let lifecycle_hook = SetWinEventHook(
                EVENT_OBJECT_CREATE,
                EVENT_OBJECT_HIDE,
                None,
                Some(window_event_callback),
                0,
                0,
                WINEVENT_OUTOFCONTEXT | WINEVENT_SKIPOWNPROCESS,
            );
            let location_hook = SetWinEventHook(
                EVENT_OBJECT_LOCATIONCHANGE,
                EVENT_OBJECT_LOCATIONCHANGE,
                None,
                Some(window_event_callback),
                0,
                0,
                WINEVENT_OUTOFCONTEXT | WINEVENT_SKIPOWNPROCESS,
            );
            if lifecycle_hook.0.is_null() && location_hook.0.is_null() {
                return;
            }
            let mut message = MSG::default();
            while GetMessageW(&mut message, None, 0, 0).as_bool() {}
        })
        .expect("WinEvent watcher thread must start");
}

unsafe extern "system" fn window_event_callback(
    _hook: HWINEVENTHOOK,
    event: u32,
    hwnd: HWND,
    object_id: i32,
    _child_id: i32,
    _event_thread: u32,
    _event_time: u32,
) {
    if object_id != OBJID_WINDOW.0 {
        return;
    }
    if event == EVENT_OBJECT_LOCATIONCHANGE
        && (hwnd.0.is_null() || unsafe { GetAncestor(hwnd, GA_ROOT) } != hwnd)
    {
        return;
    }
    if matches!(
        event,
        EVENT_OBJECT_CREATE
            | EVENT_OBJECT_DESTROY
            | EVENT_OBJECT_SHOW
            | EVENT_OBJECT_HIDE
            | EVENT_OBJECT_LOCATIONCHANGE
    ) && let Some(sender) = WINDOW_EVENT_SENDER.get()
        && WINDOW_EVENT_PENDING
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .is_ok()
        && sender
            .try_send(ControllerCommand::NativeWindowEvent(&WINDOW_EVENT_PENDING))
            .is_err()
    {
        WINDOW_EVENT_PENDING.store(false, Ordering::Release);
    }
}

#[derive(Clone)]
struct ProcessEntry {
    parent_id: u32,
    executable: String,
}

fn process_snapshot() -> Result<HashMap<u32, ProcessEntry>, BackendError> {
    let snapshot =
        unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0).map_err(map_windows_error)? };
    let mut output = HashMap::new();
    let mut entry = PROCESSENTRY32W {
        dwSize: u32::try_from(size_of::<PROCESSENTRY32W>()).unwrap_or(u32::MAX),
        ..Default::default()
    };

    unsafe {
        if Process32FirstW(snapshot, &raw mut entry).is_ok() {
            loop {
                output.insert(
                    entry.th32ProcessID,
                    ProcessEntry {
                        parent_id: entry.th32ParentProcessID,
                        executable: wide_buffer_to_string(&entry.szExeFile),
                    },
                );
                entry.dwSize = u32::try_from(size_of::<PROCESSENTRY32W>()).unwrap_or(u32::MAX);
                if Process32NextW(snapshot, &raw mut entry).is_err() {
                    break;
                }
            }
        }
        let _ = CloseHandle(snapshot);
    }
    Ok(output)
}

fn related_clubgg_processes(processes: &HashMap<u32, ProcessEntry>) -> HashSet<u32> {
    let mut related: HashSet<u32> = processes
        .iter()
        .filter_map(|(id, entry)| {
            entry
                .executable
                .as_bytes()
                .windows(b"clubgg".len())
                .any(|window| window.eq_ignore_ascii_case(b"clubgg"))
                .then_some(*id)
        })
        .collect();
    loop {
        let before = related.len();
        for (id, entry) in processes {
            if related.contains(&entry.parent_id) {
                related.insert(*id);
            }
        }
        if related.len() == before {
            break;
        }
    }
    related
}

fn inspect_window(
    hwnd: HWND,
    processes: &HashMap<u32, ProcessEntry>,
    clubgg_ids: &HashSet<u32>,
) -> Option<WindowCandidate> {
    if !unsafe { IsWindowVisible(hwnd) }.as_bool() || is_cloaked(hwnd) {
        return None;
    }

    let mut process_id = 0_u32;
    unsafe {
        GetWindowThreadProcessId(hwnd, Some(&raw mut process_id));
    }
    if process_id == std::process::id() {
        return None;
    }

    let title = window_text(hwnd);
    let class_name = class_name(hwnd);
    let process_name = processes
        .get(&process_id)
        .map_or_else(|| "unknown".to_owned(), |entry| entry.executable.clone());
    let belongs_to_clubgg = clubgg_ids.contains(&process_id)
        || process_name.to_ascii_lowercase().contains("clubgg")
        || title.to_ascii_lowercase().contains("clubgg")
        || class_name.to_ascii_lowercase().contains("clubgg");
    let belongs_to_ldplayer = is_ldplayer_process(&process_name);
    let poker_client = if belongs_to_clubgg {
        Some(PokerClientKind::ClubGg)
    } else if belongs_to_ldplayer {
        Some(PokerClientKind::LdPlayer)
    } else {
        None
    };

    let rect = window_rect(hwnd).ok()?;
    if rect.width < 100 || rect.height < 100 {
        return None;
    }
    let style = unsafe { GetWindowLongPtrW(hwnd, GWL_STYLE) } as u32;
    let ex_style = unsafe { GetWindowLongPtrW(hwnd, GWL_EXSTYLE) } as u32;
    if style & WS_CHILD.0 != 0 || style & WS_DISABLED.0 != 0 {
        return None;
    }

    let lower_title = title.to_ascii_lowercase();
    let utility_words = [
        "lobby",
        "cashier",
        "login",
        "setting",
        "message",
        "history",
        "support",
        "tournament lobby",
    ];
    let clubgg_lobby = belongs_to_clubgg && is_clubgg_lobby_surface(&lower_title);
    let looks_utility = clubgg_lobby || utility_words.iter().any(|word| lower_title.contains(word));
    let ratio = rect.aspect_ratio().unwrap_or(0.0);
    let table_shape = (0.9..=2.1).contains(&ratio);
    let tool_window = ex_style & WS_EX_TOOLWINDOW.0 != 0;
    if poker_client.is_none()
        && (title.trim().is_empty() || tool_window || is_system_shell_window(&class_name))
    {
        return None;
    }
    if belongs_to_ldplayer && (tool_window || title.trim().is_empty()) {
        return None;
    }
    let likely_table = match poker_client {
        Some(PokerClientKind::ClubGg) => {
            !looks_utility && table_shape && (!title.is_empty() || !tool_window)
        }
        Some(PokerClientKind::LdPlayer) => !looks_utility,
        None => false,
    };
    let label = if clubgg_lobby {
        "ClubGG lobby".to_owned()
    } else if title.trim().is_empty() {
        format!("{process_name} window")
    } else {
        title
    };

    Some(WindowCandidate {
        id: id_from_hwnd(hwnd),
        label,
        process_name: process_name.clone(),
        class_name: class_name.clone(),
        signature: WindowSignature {
            process_name: process_name.to_ascii_lowercase(),
            class_name,
            title_pattern: normalize_title_pattern(&lower_title),
        },
        rect,
        poker_client,
        is_clubgg_lobby: clubgg_lobby,
        preferred_aspect_ratio: if poker_client == Some(PokerClientKind::LdPlayer) {
            normalized_ldplayer_aspect_ratio(ratio)
        } else {
            ratio
        },
        likely_table,
    })
}

fn is_ldplayer_process(process_name: &str) -> bool {
    matches!(
        process_name.to_ascii_lowercase().as_str(),
        "dnplayer" | "dnplayer.exe"
    )
}

fn is_generic_clubgg_surface(title: &str) -> bool {
    matches!(title.trim().to_ascii_lowercase().as_str(), "" | "clubgg")
}

fn is_clubgg_lobby_surface(title: &str) -> bool {
    is_generic_clubgg_surface(title) || title.to_ascii_lowercase().contains("lobby")
}

fn is_system_shell_window(class_name: &str) -> bool {
    matches!(
        class_name.to_ascii_lowercase().as_str(),
        "progman"
            | "workerw"
            | "shell_traywnd"
            | "shell_secondarytraywnd"
            | "notifyiconoverflowwindow"
    )
}

fn is_cloaked(hwnd: HWND) -> bool {
    let mut cloaked = 0_u32;
    unsafe {
        DwmGetWindowAttribute(
            hwnd,
            DWMWA_CLOAKED,
            std::ptr::from_mut(&mut cloaked).cast::<c_void>(),
            u32::try_from(size_of::<u32>()).unwrap_or(4),
        )
        .is_ok()
            && cloaked != 0
    }
}

fn window_rect(hwnd: HWND) -> Result<Rect, BackendError> {
    let mut rect = RECT::default();
    unsafe {
        GetWindowRect(hwnd, &raw mut rect).map_err(map_windows_error)?;
    }
    Ok(Rect::new(
        rect.left,
        rect.top,
        rect.right.saturating_sub(rect.left),
        rect.bottom.saturating_sub(rect.top),
    ))
}

fn window_text(hwnd: HWND) -> String {
    let length = unsafe { GetWindowTextLengthW(hwnd) }.max(0);
    let mut buffer = vec![0_u16; usize::try_from(length).unwrap_or(0).saturating_add(1)];
    let copied = unsafe { GetWindowTextW(hwnd, &mut buffer) }.max(0) as usize;
    String::from_utf16_lossy(&buffer[..copied])
}

fn class_name(hwnd: HWND) -> String {
    let mut buffer = [0_u16; 256];
    let copied = unsafe { GetClassNameW(hwnd, &mut buffer) }.max(0) as usize;
    String::from_utf16_lossy(&buffer[..copied])
}

fn wide_buffer_to_string(buffer: &[u16]) -> String {
    let length = buffer
        .iter()
        .position(|character| *character == 0)
        .unwrap_or(buffer.len());
    String::from_utf16_lossy(&buffer[..length])
}

fn normalize_title_pattern(title: &str) -> String {
    let mut output = String::with_capacity(title.len());
    let mut previous_was_number = false;
    let mut previous_was_space = false;
    for character in title.chars() {
        if character.is_ascii_digit() || matches!(character, '$' | '€' | '£' | '¥') {
            if !previous_was_number {
                output.push('#');
            }
            previous_was_number = true;
            previous_was_space = false;
        } else if character.is_whitespace() {
            if !previous_was_space {
                output.push(' ');
            }
            previous_was_number = false;
            previous_was_space = true;
        } else {
            output.push(character);
            previous_was_number = false;
            previous_was_space = false;
        }
    }
    output.trim().to_owned()
}

fn id_from_hwnd(hwnd: HWND) -> WindowId {
    WindowId(hwnd.0 as usize as u64)
}

fn hwnd_from_id(id: WindowId) -> HWND {
    HWND(id.0 as usize as *mut c_void)
}

fn map_windows_error(error: WindowsError) -> BackendError {
    if unsafe { GetLastError() } == ERROR_ACCESS_DENIED {
        BackendError::AccessDenied
    } else {
        BackendError::Other(error.message())
    }
}

fn map_last_error(context: &str) -> BackendError {
    let error = unsafe { GetLastError() };
    if error == ERROR_ACCESS_DENIED {
        BackendError::AccessDenied
    } else {
        BackendError::Other(format!("{context} (Windows error {})", error.0))
    }
}

#[cfg(test)]
mod tests {
    use super::{
        is_clubgg_lobby_surface, is_generic_clubgg_surface, is_ldplayer_process,
        is_system_shell_window, locate_raise_flags, normalize_title_pattern,
    };
    use windows::Win32::UI::WindowsAndMessaging::{
        SWP_NOACTIVATE, SWP_NOOWNERZORDER, SWP_SHOWWINDOW,
    };

    #[test]
    fn title_patterns_remove_variable_numbers() {
        assert_eq!(
            normalize_title_pattern("Club 123 - Table €5/€10"),
            "Club # - Table #/#"
        );
    }

    #[test]
    fn desktop_and_taskbar_windows_are_not_candidates() {
        assert!(is_system_shell_window("Progman"));
        assert!(is_system_shell_window("Shell_TrayWnd"));
        assert!(!is_system_shell_window("Chrome_WidgetWin_1"));
    }

    #[test]
    fn only_the_ldplayer_frontend_process_matches() {
        assert!(is_ldplayer_process("dnplayer.exe"));
        assert!(is_ldplayer_process("DNPLAYER"));
        assert!(!is_ldplayer_process("Ld9BoxHeadless.exe"));
        assert!(!is_ldplayer_process("Ld9BoxSVC.exe"));
    }

    #[test]
    fn generic_clubgg_titles_identify_lobbies_not_tables() {
        assert!(is_generic_clubgg_surface(""));
        assert!(is_generic_clubgg_surface("  ClubGG  "));
        assert!(!is_generic_clubgg_surface("PLO5 10-20 - Table 2"));
    }

    #[test]
    fn titled_clubgg_lobbies_are_identified_consistently() {
        assert!(is_clubgg_lobby_surface("ClubGG lobby"));
        assert!(is_clubgg_lobby_surface("Tournament Lobby - ClubGG"));
        assert!(!is_clubgg_lobby_surface("PLO5 10-20 - Table 2"));
    }

    #[test]
    fn locate_raise_allows_owned_lobby_z_order_to_change() {
        let flags = locate_raise_flags();
        assert_ne!(flags.0 & SWP_SHOWWINDOW.0, 0);
        assert_eq!(flags.0 & SWP_NOOWNERZORDER.0, 0);
        assert_eq!(flags.0 & SWP_NOACTIVATE.0, 0);
    }
}
