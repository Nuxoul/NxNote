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

/// GPUI 0.3.9's CF_HDROP fallback uses a locked pointer rather than the HDROP handle.
/// Read image file paths with the native handle; no clipboard contents are modified.
#[cfg(windows)]
pub fn clipboard_files() -> Vec<std::path::PathBuf> {
    use windows::Win32::System::DataExchange::{CloseClipboard, GetClipboardData, OpenClipboard};
    use windows::Win32::UI::Shell::{DragQueryFileW, HDROP};
    unsafe {
        if OpenClipboard(None).is_err() {
            return Vec::new();
        }
        let result = (|| {
            let Ok(handle) = GetClipboardData(15) else {
                return Vec::new();
            };
            let drop = HDROP(handle.0);
            let count = DragQueryFileW(drop, u32::MAX, None);
            let mut paths = Vec::new();
            for index in 0..count {
                let length = DragQueryFileW(drop, index, None);
                if length == 0 {
                    continue;
                }
                let mut buffer = vec![0u16; length as usize + 1];
                let read = DragQueryFileW(drop, index, Some(&mut buffer));
                if read > 0 {
                    paths.push(std::path::PathBuf::from(String::from_utf16_lossy(
                        &buffer[..read as usize],
                    )));
                }
            }
            paths
        })();
        let _ = CloseClipboard();
        result
    }
}
#[cfg(not(windows))]
pub fn clipboard_files() -> Vec<std::path::PathBuf> {
    Vec::new()
}
