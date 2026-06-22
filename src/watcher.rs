use std::path::PathBuf;
use std::sync::mpsc::{self, Receiver, Sender};
use std::thread;
use std::time::Duration;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ForegroundInfo {
    pub exe_path: PathBuf,
    pub title: String,
}

pub fn spawn(poll_interval_ms: u64, egui_ctx: egui::Context) -> Receiver<ForegroundInfo> {
    let (tx, rx) = mpsc::channel();
    thread::spawn(move || {
        run(tx, poll_interval_ms, egui_ctx);
    });
    rx
}

#[cfg(windows)]
fn run(tx: Sender<ForegroundInfo>, poll_ms: u64, egui_ctx: egui::Context) {
    use windows::Win32::Foundation::{CloseHandle, HWND, MAX_PATH};
    use windows::Win32::System::Threading::{
        OpenProcess, QueryFullProcessImageNameW, PROCESS_NAME_FORMAT, PROCESS_QUERY_LIMITED_INFORMATION,
    };
    use windows::Win32::UI::WindowsAndMessaging::{
        GetForegroundWindow, GetWindowTextLengthW, GetWindowTextW, GetWindowThreadProcessId,
    };

    let mut last: Option<ForegroundInfo> = None;
    loop {
        let info = unsafe {
            let hwnd: HWND = GetForegroundWindow();
            if hwnd.0.is_null() {
                None
            } else {
                let mut pid: u32 = 0;
                GetWindowThreadProcessId(hwnd, Some(&mut pid));
                if pid == 0 {
                    None
                } else {
                    let exe = match OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) {
                        Ok(h) => {
                            let mut buf = [0u16; MAX_PATH as usize];
                            let mut size = buf.len() as u32;
                            let ok = QueryFullProcessImageNameW(
                                h,
                                PROCESS_NAME_FORMAT(0),
                                windows::core::PWSTR(buf.as_mut_ptr()),
                                &mut size,
                            )
                            .is_ok();
                            let _ = CloseHandle(h);
                            if ok {
                                Some(PathBuf::from(String::from_utf16_lossy(&buf[..size as usize])))
                            } else {
                                None
                            }
                        }
                        Err(_) => None,
                    };

                    let title = {
                        let len = GetWindowTextLengthW(hwnd);
                        if len > 0 {
                            let mut buf = vec![0u16; (len + 1) as usize];
                            let n = GetWindowTextW(hwnd, &mut buf);
                            String::from_utf16_lossy(&buf[..n as usize])
                        } else {
                            String::new()
                        }
                    };

                    exe.map(|exe_path| ForegroundInfo { exe_path, title })
                }
            }
        };

        if let Some(info) = info {
            // 忽略自己
            let is_self = info
                .exe_path
                .file_stem()
                .and_then(|s| s.to_str())
                .map(|s| s.eq_ignore_ascii_case("nxnote"))
                .unwrap_or(false);
            if !is_self && last.as_ref() != Some(&info) {
                last = Some(info.clone());
                let _ = tx.send(info);
                egui_ctx.request_repaint();
            }
        }

        thread::sleep(Duration::from_millis(poll_ms));
    }
}

#[cfg(not(windows))]
fn run(_tx: Sender<ForegroundInfo>, _poll_ms: u64, _egui_ctx: egui::Context) {
    // 其它平台暂未实现
}
