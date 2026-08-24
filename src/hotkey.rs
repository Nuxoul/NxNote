use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::thread;
use std::time::Duration;

use global_hotkey::{
    hotkey::{Code, HotKey, Modifiers},
    GlobalHotKeyEvent, GlobalHotKeyManager, HotKeyState,
};

/// 当前注册的 toggle / capture 热键 id（0 = 未注册）。
/// 监听线程只启动一次，重绑热键时改这里即可 —— 避免多个线程
/// 同时 recv 全局事件通道互相抢事件。
static TOGGLE_ID: AtomicU32 = AtomicU32::new(0);
static CAPTURE_ID: AtomicU32 = AtomicU32::new(0);

pub struct HotkeyHandle {
    _manager: GlobalHotKeyManager,
}

/// 常驻监听线程（整个进程只 spawn 一次）：
/// - toggle 热键 → 直接 Win32 切换窗口可见性（不等主线程）
/// - capture 热键 → 读剪贴板文本发给主线程追加进速记本
pub fn spawn_listener(egui_ctx: egui::Context, capture_tx: Sender<String>) -> Receiver<()> {
    let (tx, rx) = mpsc::channel();
    thread::spawn(move || {
        let receiver = GlobalHotKeyEvent::receiver();
        while let Ok(ev) = receiver.recv() {
            if ev.state != HotKeyState::Pressed {
                continue;
            }
            let toggle_id = TOGGLE_ID.load(Ordering::Relaxed);
            let capture_id = CAPTURE_ID.load(Ordering::Relaxed);
            if ev.id == toggle_id {
                // 直接 Win32 切换 —— 不等主线程响应
                crate::app::force_toggle();
                if tx.send(()).is_err() {
                    break;
                }
                egui_ctx.request_repaint();
            } else if capture_id != 0 && ev.id == capture_id {
                let text = read_clipboard();
                if !text.trim().is_empty() {
                    let _ = capture_tx.send(text);
                }
                egui_ctx.request_repaint();
            }
        }
    });
    rx
}

pub fn parse(spec: &str) -> Option<(Modifiers, Code)> {
    let mut mods = Modifiers::empty();
    let mut code: Option<Code> = None;
    for part in spec.split('+') {
        let p = part.trim();
        match p.to_ascii_lowercase().as_str() {
            "ctrl" | "control" => mods |= Modifiers::CONTROL,
            "alt" => mods |= Modifiers::ALT,
            "shift" => mods |= Modifiers::SHIFT,
            "win" | "super" | "meta" => mods |= Modifiers::META,
            other => code = key_code(other),
        }
    }
    code.map(|c| (mods, c))
}

fn key_code(s: &str) -> Option<Code> {
    let s = s.to_ascii_uppercase();
    Some(match s.as_str() {
        "A" => Code::KeyA, "B" => Code::KeyB, "C" => Code::KeyC, "D" => Code::KeyD,
        "E" => Code::KeyE, "F" => Code::KeyF, "G" => Code::KeyG, "H" => Code::KeyH,
        "I" => Code::KeyI, "J" => Code::KeyJ, "K" => Code::KeyK, "L" => Code::KeyL,
        "M" => Code::KeyM, "N" => Code::KeyN, "O" => Code::KeyO, "P" => Code::KeyP,
        "Q" => Code::KeyQ, "R" => Code::KeyR, "S" => Code::KeyS, "T" => Code::KeyT,
        "U" => Code::KeyU, "V" => Code::KeyV, "W" => Code::KeyW, "X" => Code::KeyX,
        "Y" => Code::KeyY, "Z" => Code::KeyZ,
        "0" => Code::Digit0, "1" => Code::Digit1, "2" => Code::Digit2, "3" => Code::Digit3,
        "4" => Code::Digit4, "5" => Code::Digit5, "6" => Code::Digit6, "7" => Code::Digit7,
        "8" => Code::Digit8, "9" => Code::Digit9,
        "F1" => Code::F1, "F2" => Code::F2, "F3" => Code::F3, "F4" => Code::F4,
        "F5" => Code::F5, "F6" => Code::F6, "F7" => Code::F7, "F8" => Code::F8,
        "F9" => Code::F9, "F10" => Code::F10, "F11" => Code::F11, "F12" => Code::F12,
        "SPACE" => Code::Space,
        "UP" => Code::ArrowUp, "DOWN" => Code::ArrowDown,
        "LEFT" => Code::ArrowLeft, "RIGHT" => Code::ArrowRight,
        "HOME" => Code::Home, "END" => Code::End,
        "PAGEUP" => Code::PageUp, "PAGEDOWN" => Code::PageDown,
        _ => return None,
    })
}

/// 注册显示/隐藏热键（必须成功才返回 Some）；
/// 捕获热键失败 / 与主热键冲突时静默降级为无此功能。
pub fn install(toggle_spec: &str, capture_spec: &str) -> Option<HotkeyHandle> {
    let (mods, code) = parse(toggle_spec)?;
    let manager = GlobalHotKeyManager::new().ok()?;
    let toggle = HotKey::new(Some(mods), code);
    manager.register(toggle).ok()?;
    TOGGLE_ID.store(toggle.id(), Ordering::Release);

    let mut capture_id = 0u32;
    if let Some((cm, cc)) = parse(capture_spec) {
        let hk = HotKey::new(Some(cm), cc);
        if hk.id() != toggle.id()
            && manager.register(hk).is_ok()
        {
            capture_id = hk.id();
        }
    }
    CAPTURE_ID.store(capture_id, Ordering::Release);

    Some(HotkeyHandle { _manager: manager })
}

#[cfg(windows)]
fn read_clipboard() -> String {
    use windows::Win32::Foundation::HANDLE;
    use windows::Win32::Foundation::HGLOBAL;
    use windows::Win32::System::DataExchange::{
        CloseClipboard, GetClipboardData, IsClipboardFormatAvailable, OpenClipboard,
    };
    use windows::Win32::System::Memory::{GlobalLock, GlobalUnlock};
    const CF_UNICODETEXT: u32 = 13;

    unsafe {
        for _ in 0..3 {
            if IsClipboardFormatAvailable(CF_UNICODETEXT).is_ok()
                && OpenClipboard(None).is_ok()
            {
                let text = (|| -> Option<String> {
                    let h: HANDLE = GetClipboardData(CF_UNICODETEXT).ok()?;
                    let hg = HGLOBAL(h.0);
                    let p = GlobalLock(hg);
                    if p.is_null() {
                        return None;
                    }
                    let wp = p as *const u16;
                    let mut len = 0usize;
                    while *wp.add(len) != 0 {
                        len += 1;
                    }
                    let s = std::slice::from_raw_parts(wp, len);
                    let out = String::from_utf16_lossy(s);
                    let _ = GlobalUnlock(hg);
                    Some(out)
                })();
                let _ = CloseClipboard();
                return text.unwrap_or_default();
            }
            thread::sleep(Duration::from_millis(15));
        }
        String::new()
    }
}

#[cfg(not(windows))]
fn read_clipboard() -> String {
    String::new()
}
