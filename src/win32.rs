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
            LPARAM, RECT, WPARAM,
        },
        Graphics::{
            Dwm::{DWMWA_CLOAKED, DwmGetWindowAttribute},
            Gdi::{
                EnumDisplayMonitors, GetMonitorInfoW, HDC, HMONITOR, MONITORINFO, MONITORINFOEXW,
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
                EVENT_OBJECT_CREATE, EVENT_OBJECT_DESTROY, EVENT_OBJECT_HIDE,
                EVENT_OBJECT_LOCATIONCHANGE, EVENT_OBJECT_SHOW, EnumWindows, FLASHW_ALL,
                FLASHW_TIMERNOFG, FLASHWINFO, FindWindowW, FlashWindowEx, GA_ROOT, GWL_EXSTYLE,
                GWL_STYLE, GetAncestor, GetClassNameW, GetForegroundWindow, GetMessageW,
                GetWindowLongPtrW, GetWindowRect, GetWindowTextLengthW, GetWindowTextW,
                GetWindowThreadProcessId, IsIconic, IsWindowVisible, MINMAXINFO,
                MONITORINFOF_PRIMARY, MSG, OBJID_WINDOW, SMTO_ABORTIFHUNG, SW_SHOWNOACTIVATE,
                SWP_ASYNCWINDOWPOS, SWP_NOACTIVATE, SWP_NOOWNERZORDER, SWP_NOZORDER,
                SendMessageTimeoutW, SetForegroundWindow, SetWindowPos, ShowWindowAsync,
                WINEVENT_OUTOFCONTEXT, WINEVENT_SKIPOWNPROCESS, WM_GETMINMAXINFO, WS_CHILD,
                WS_DISABLED, WS_EX_TOOLWINDOW,
            },
        },
    },
    core::{BOOL, Error as WindowsError, HSTRING, PCWSTR, w},
};

use crate::{
    controller::ControllerCommand,
    identity::PANEL_TITLE,
    model::{
        BackendError, MonitorInfo, Rect, Size, WindowBackend, WindowCandidate, WindowId,
        WindowSignature,
    },
};

static WINDOW_EVENT_SENDER: OnceLock<Sender<ControllerCommand>> = OnceLock::new();
static WINDOW_EVENT_PENDING: AtomicBool = AtomicBool::new(false);
const WINDOW_EVENT_STACK_BYTES: usize = 256 * 1024;

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

    fn highlight(&self, id: WindowId) -> Result<(), BackendError> {
        let info = FLASHWINFO {
            cbSize: u32::try_from(size_of::<FLASHWINFO>()).unwrap_or(u32::MAX),
            hwnd: hwnd_from_id(id),
            dwFlags: FLASHW_ALL | FLASHW_TIMERNOFG,
            uCount: 3,
            dwTimeout: 0,
        };
        unsafe {
            let _ = FlashWindowEx(&raw const info);
        }
        Ok(())
    }

    fn foreground_window(&self) -> Option<WindowId> {
        let hwnd = unsafe { GetForegroundWindow() };
        (!hwnd.0.is_null()).then(|| id_from_hwnd(hwnd))
    }
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
    let looks_utility = utility_words.iter().any(|word| lower_title.contains(word));
    let ratio = rect.aspect_ratio().unwrap_or(0.0);
    let table_shape = (0.9..=2.1).contains(&ratio);
    let tool_window = ex_style & WS_EX_TOOLWINDOW.0 != 0;
    if !belongs_to_clubgg
        && (title.trim().is_empty() || tool_window || is_system_shell_window(&class_name))
    {
        return None;
    }
    let likely_table = !looks_utility && table_shape && (!title.is_empty() || !tool_window);
    let likely_table = belongs_to_clubgg && likely_table;
    let label = if title.trim().is_empty() && belongs_to_clubgg {
        format!("ClubGG window ({class_name})")
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
        is_clubgg: belongs_to_clubgg,
        likely_table,
    })
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
    use super::{is_system_shell_window, normalize_title_pattern};

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
}
