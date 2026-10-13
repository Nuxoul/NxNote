use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::Duration;

use async_channel::{Sender, TrySendError};

use crate::platform::PlatformEvent;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ForegroundInfo {
    pub exe_path: PathBuf,
    pub title: String,
}

/// Starts the Windows foreground-window monitor. Poll changes take effect
/// immediately through the shared atomic interval.
pub fn spawn(poll_ms: Arc<AtomicU64>, sender: Sender<PlatformEvent>) {
    #[cfg(windows)]
    thread::spawn(move || run(sender, poll_ms));

    #[cfg(not(windows))]
    {
        let _ = (poll_ms, sender);
    }
}

#[cfg(windows)]
fn run(sender: Sender<PlatformEvent>, poll_ms: Arc<AtomicU64>) {
    use windows::Win32::Foundation::{CloseHandle, HWND, MAX_PATH};
    use windows::Win32::System::Threading::{
        OpenProcess, QueryFullProcessImageNameW, PROCESS_NAME_FORMAT,
        PROCESS_QUERY_LIMITED_INFORMATION,
    };
    use windows::Win32::UI::WindowsAndMessaging::{
        GetForegroundWindow, GetWindowTextLengthW, GetWindowTextW, GetWindowThreadProcessId,
    };

    let mut last = None;
    loop {
        if sender.is_closed() {
            break;
        }

        let info = unsafe {
            let hwnd: HWND = GetForegroundWindow();
            if hwnd.0.is_null() {
                None
            } else {
                let mut pid = 0;
                GetWindowThreadProcessId(hwnd, Some(&mut pid));
                if pid == 0 {
                    None
                } else {
                    let exe_path = match OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid)
                    {
                        Ok(process) => {
                            let mut buffer = [0u16; MAX_PATH as usize];
                            let mut size = buffer.len() as u32;
                            let queried = QueryFullProcessImageNameW(
                                process,
                                PROCESS_NAME_FORMAT(0),
                                windows::core::PWSTR(buffer.as_mut_ptr()),
                                &mut size,
                            )
                            .is_ok();
                            let _ = CloseHandle(process);
                            queried.then(|| {
                                PathBuf::from(String::from_utf16_lossy(&buffer[..size as usize]))
                            })
                        }
                        Err(_) => None,
                    };

                    let title = {
                        // The title can grow between these calls; retry with a
                        // larger buffer when the first read may be truncated.
                        let mut wanted = GetWindowTextLengthW(hwnd);
                        let mut title = String::new();
                        for _ in 0..3 {
                            if wanted <= 0 {
                                break;
                            }
                            let mut buffer = vec![0u16; (wanted + 2) as usize];
                            let len = GetWindowTextW(hwnd, &mut buffer);
                            if (len as usize) + 1 < buffer.len() {
                                title = String::from_utf16_lossy(&buffer[..len as usize]);
                                break;
                            }
                            wanted = (wanted * 2).min(8192);
                        }
                        title
                    };

                    exe_path.map(|exe_path| ForegroundInfo { exe_path, title })
                }
            }
        };

        if let Some(info) = info {
            let is_self = info
                .exe_path
                .file_stem()
                .and_then(|stem| stem.to_str())
                .map(|stem| stem.eq_ignore_ascii_case("nxnote"))
                .unwrap_or(false);

            if is_self {
                last = None;
            }
            if !is_self && last.as_ref() != Some(&info) {
                last = Some(info.clone());
                match sender.try_send(PlatformEvent::Foreground(info)) {
                    Ok(()) | Err(TrySendError::Full(_)) => {}
                    Err(TrySendError::Closed(_)) => break,
                }
            }
        } else {
            last = None;
        }

        let poll = poll_ms.load(Ordering::Relaxed).max(20);
        thread::sleep(Duration::from_millis(poll));
    }
}
