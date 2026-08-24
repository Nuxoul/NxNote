use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::Arc;
use std::thread;
use std::time::Duration;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ForegroundInfo {
    pub exe_path: PathBuf,
    pub title: String,
}

/// poll_interval_ms 用 Arc<AtomicU64> 传递，设置里改了立刻生效，无需重启。
pub fn spawn(poll_ms: Arc<AtomicU64>, egui_ctx: egui::Context) -> Receiver<ForegroundInfo> {
    let (tx, rx) = mpsc::channel();
    thread::spawn(move || {
        run(tx, poll_ms, egui_ctx);
    });
    rx
}

#[cfg(windows)]
fn run(tx: Sender<ForegroundInfo>, poll_ms: Arc<AtomicU64>, egui_ctx: egui::Context) {
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
                        // GetWindowTextLengthW 与 GetWindowTextW 之间标题可能变长，
                        // 读到截断就把缓冲区翻倍重试
                        let mut want = GetWindowTextLengthW(hwnd);
                        let mut title = String::new();
                        for _ in 0..3 {
                            if want <= 0 {
                                break;
                            }
                            let mut buf = vec![0u16; (want + 2) as usize];
                            let n = GetWindowTextW(hwnd, &mut buf);
                            if (n as usize) + 1 < buf.len() {
                                title = String::from_utf16_lossy(&buf[..n as usize]);
                                break;
                            }
                            want = (want * 2).min(8192);
                        }
                        title
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

        let poll = poll_ms.load(Ordering::Relaxed).max(20);
        thread::sleep(Duration::from_millis(poll));
    }
}

#[cfg(not(windows))]
fn run(_tx: Sender<ForegroundInfo>, _poll_ms: Arc<AtomicU64>, _egui_ctx: egui::Context) {
    // 其它平台暂未实现
}
