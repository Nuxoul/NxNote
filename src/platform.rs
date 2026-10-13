#[derive(Debug)]
pub enum PlatformEvent {
    Toggle,
    Show,
    Quit,
    Capture(String),
    Foreground(crate::watcher::ForegroundInfo),
}

#[cfg(windows)]
pub fn set_visible(hwnd: isize, visible: bool) {
    use windows::Win32::Foundation::HWND;
    use windows::Win32::UI::WindowsAndMessaging::{
        IsWindow, SetForegroundWindow, ShowWindow, SW_HIDE, SW_SHOW,
    };

    if hwnd == 0 {
        return;
    }
    let hwnd = HWND(hwnd as *mut std::ffi::c_void);
    unsafe {
        if !IsWindow(hwnd).as_bool() {
            return;
        }
        let _ = ShowWindow(hwnd, if visible { SW_SHOW } else { SW_HIDE });
        if visible {
            let _ = SetForegroundWindow(hwnd);
        }
    }
}

#[cfg(not(windows))]
pub fn set_visible(_hwnd: isize, _visible: bool) {}

#[cfg(windows)]
pub fn set_always_on_top(hwnd: isize, enabled: bool) {
    use windows::Win32::Foundation::HWND;
    use windows::Win32::UI::WindowsAndMessaging::{
        IsWindow, SetWindowPos, HWND_NOTOPMOST, HWND_TOPMOST, SWP_NOMOVE, SWP_NOSIZE,
    };

    if hwnd == 0 {
        return;
    }
    let hwnd = HWND(hwnd as *mut std::ffi::c_void);
    unsafe {
        if IsWindow(hwnd).as_bool() {
            let insert_after = if enabled {
                HWND_TOPMOST
            } else {
                HWND_NOTOPMOST
            };
            let _ = SetWindowPos(hwnd, insert_after, 0, 0, 0, 0, SWP_NOMOVE | SWP_NOSIZE);
        }
    }
}

#[cfg(not(windows))]
pub fn set_always_on_top(_hwnd: isize, _enabled: bool) {}
