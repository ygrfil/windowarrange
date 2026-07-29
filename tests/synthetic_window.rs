#![cfg(target_os = "windows")]

use clubgg_table_arranger::{
    model::{Rect, WindowBackend, WindowId},
    win32::Win32Backend,
};
use windows::{
    Win32::{
        Foundation::{HWND, LPARAM, LRESULT, WPARAM},
        System::LibraryLoader::GetModuleHandleW,
        UI::WindowsAndMessaging::{
            CS_HREDRAW, CS_VREDRAW, CreateWindowExW, DefWindowProcW, DestroyWindow, RegisterClassW,
            UnregisterClassW, WINDOW_EX_STYLE, WNDCLASSW, WS_OVERLAPPEDWINDOW,
        },
    },
    core::w,
};

/// This is ignored by default because it creates a real native window. It is isolated to a
/// handle created by this test process and never invokes ClubGG discovery.
#[test]
#[ignore = "explicit native harness; run with --ignored when no live-table automation is desired"]
fn moves_only_the_synthetic_window() {
    unsafe extern "system" fn window_proc(
        hwnd: HWND,
        message: u32,
        wparam: WPARAM,
        lparam: LPARAM,
    ) -> LRESULT {
        unsafe { DefWindowProcW(hwnd, message, wparam, lparam) }
    }

    let instance = unsafe { GetModuleHandleW(None).expect("module handle") };
    let class_name = w!("ClubGGTableArrangerSyntheticTest");
    let class = WNDCLASSW {
        style: CS_HREDRAW | CS_VREDRAW,
        lpfnWndProc: Some(window_proc),
        hInstance: instance.into(),
        lpszClassName: class_name,
        ..Default::default()
    };
    let atom = unsafe { RegisterClassW(&raw const class) };
    assert_ne!(atom, 0);

    let hwnd = unsafe {
        CreateWindowExW(
            WINDOW_EX_STYLE::default(),
            class_name,
            w!("Synthetic test table"),
            WS_OVERLAPPEDWINDOW,
            0,
            0,
            400,
            300,
            None,
            None,
            Some(instance.into()),
            None,
        )
        .expect("create synthetic window")
    };

    let backend = Win32Backend::new();
    let id = WindowId(hwnd.0 as usize as u64);
    let actual = backend
        .move_resize(id, Rect::new(40, 50, 640, 480))
        .expect("move synthetic window");
    assert_eq!(actual, Rect::new(40, 50, 640, 480));

    unsafe {
        let _ = DestroyWindow(hwnd);
        let _ = UnregisterClassW(class_name, Some(instance.into()));
    }
}
