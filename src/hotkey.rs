use std::sync::mpsc::{self, Receiver};
use std::thread;

use global_hotkey::{
    hotkey::{Code, HotKey, Modifiers},
    GlobalHotKeyEvent, GlobalHotKeyManager, HotKeyState,
};

pub struct HotkeyHandle {
    _manager: GlobalHotKeyManager,
    _hotkey: HotKey,
}

/// 启动一个常驻线程，监听全局热键事件。
/// 即使主窗口被隐藏（Visible(false)）也能通过 ctx.request_repaint() 唤醒 eframe。
pub fn spawn_listener(egui_ctx: egui::Context) -> Receiver<()> {
    let (tx, rx) = mpsc::channel();
    thread::spawn(move || {
        let receiver = GlobalHotKeyEvent::receiver();
        loop {
            match receiver.recv() {
                Ok(ev) => {
                    if ev.state == HotKeyState::Pressed {
                        // 直接 Win32 切换 —— 不等主线程响应
                        crate::app::force_toggle();
                        if tx.send(()).is_err() {
                            break;
                        }
                        egui_ctx.request_repaint();
                    }
                }
                Err(_) => break,
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

pub fn install(spec: &str) -> Option<HotkeyHandle> {
    let (mods, code) = parse(spec)?;
    let manager = GlobalHotKeyManager::new().ok()?;
    let hk = HotKey::new(Some(mods), code);
    manager.register(hk).ok()?;
    Some(HotkeyHandle {
        _manager: manager,
        _hotkey: hk,
    })
}
