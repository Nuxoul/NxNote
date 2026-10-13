use std::sync::atomic::{AtomicU32, Ordering};
use std::thread;
use std::time::Duration;

use anyhow::{anyhow, Context};
use async_channel::{Sender, TrySendError};
use global_hotkey::{
    hotkey::{Code, HotKey, Modifiers},
    GlobalHotKeyEvent, GlobalHotKeyManager, HotKeyState,
};

use crate::platform::PlatformEvent;

/// Current toggle/capture hotkey ids (0 means unregistered). The event listener
/// is process-wide and is intentionally spawned only once by the caller.
static TOGGLE_ID: AtomicU32 = AtomicU32::new(0);
static CAPTURE_ID: AtomicU32 = AtomicU32::new(0);

/// Keeps the native registrations alive. Dropping the manager destroys its
/// native window and unregisters all hotkeys registered through it.
pub struct HotkeyHandle {
    manager: GlobalHotKeyManager,
    toggle_id: u32,
    capture_id: u32,
    pub capture_error: Option<String>,
}

impl Drop for HotkeyHandle {
    fn drop(&mut self) {
        let _ = TOGGLE_ID.compare_exchange(self.toggle_id, 0, Ordering::AcqRel, Ordering::Acquire);
        let _ =
            CAPTURE_ID.compare_exchange(self.capture_id, 0, Ordering::AcqRel, Ordering::Acquire);
        // `manager` drops after this method and releases its native registrations.
        let _ = &self.manager;
    }
}

/// Starts the process-wide native hotkey event listener.
pub fn spawn_listener(sender: Sender<PlatformEvent>) {
    thread::spawn(move || {
        let receiver = GlobalHotKeyEvent::receiver();
        while let Ok(event) = receiver.recv() {
            if event.state != HotKeyState::Pressed {
                continue;
            }

            let toggle_id = TOGGLE_ID.load(Ordering::Acquire);
            let capture_id = CAPTURE_ID.load(Ordering::Acquire);
            let platform_event = if toggle_id != 0 && event.id == toggle_id {
                Some(PlatformEvent::Toggle)
            } else if capture_id != 0 && event.id == capture_id {
                let text = read_clipboard();
                (!text.trim().is_empty()).then_some(PlatformEvent::Capture(text))
            } else {
                None
            };

            if let Some(platform_event) = platform_event {
                match sender.try_send(platform_event) {
                    Ok(()) | Err(TrySendError::Full(_)) => {}
                    Err(TrySendError::Closed(_)) => break,
                }
            }
        }
    });
}

pub fn parse(spec: &str) -> Option<(Modifiers, Code)> {
    let mut mods = Modifiers::empty();
    let mut code = None;

    for part in spec.split('+') {
        let part = part.trim();
        if part.is_empty() {
            return None;
        }
        match part.to_ascii_lowercase().as_str() {
            "ctrl" | "control" => mods |= Modifiers::CONTROL,
            "alt" => mods |= Modifiers::ALT,
            "shift" => mods |= Modifiers::SHIFT,
            "win" | "super" | "meta" => mods |= Modifiers::META,
            other => {
                if code.is_some() {
                    return None;
                }
                code = Some(key_code(other)?);
            }
        }
    }

    code.map(|code| (mods, code))
}

fn key_code(s: &str) -> Option<Code> {
    let s = s.to_ascii_uppercase();
    Some(match s.as_str() {
        "A" => Code::KeyA,
        "B" => Code::KeyB,
        "C" => Code::KeyC,
        "D" => Code::KeyD,
        "E" => Code::KeyE,
        "F" => Code::KeyF,
        "G" => Code::KeyG,
        "H" => Code::KeyH,
        "I" => Code::KeyI,
        "J" => Code::KeyJ,
        "K" => Code::KeyK,
        "L" => Code::KeyL,
        "M" => Code::KeyM,
        "N" => Code::KeyN,
        "O" => Code::KeyO,
        "P" => Code::KeyP,
        "Q" => Code::KeyQ,
        "R" => Code::KeyR,
        "S" => Code::KeyS,
        "T" => Code::KeyT,
        "U" => Code::KeyU,
        "V" => Code::KeyV,
        "W" => Code::KeyW,
        "X" => Code::KeyX,
        "Y" => Code::KeyY,
        "Z" => Code::KeyZ,
        "0" => Code::Digit0,
        "1" => Code::Digit1,
        "2" => Code::Digit2,
        "3" => Code::Digit3,
        "4" => Code::Digit4,
        "5" => Code::Digit5,
        "6" => Code::Digit6,
        "7" => Code::Digit7,
        "8" => Code::Digit8,
        "9" => Code::Digit9,
        "F1" => Code::F1,
        "F2" => Code::F2,
        "F3" => Code::F3,
        "F4" => Code::F4,
        "F5" => Code::F5,
        "F6" => Code::F6,
        "F7" => Code::F7,
        "F8" => Code::F8,
        "F9" => Code::F9,
        "F10" => Code::F10,
        "F11" => Code::F11,
        "F12" => Code::F12,
        "SPACE" => Code::Space,
        "UP" => Code::ArrowUp,
        "DOWN" => Code::ArrowDown,
        "LEFT" => Code::ArrowLeft,
        "RIGHT" => Code::ArrowRight,
        "HOME" => Code::Home,
        "END" => Code::End,
        "PAGEUP" => Code::PageUp,
        "PAGEDOWN" => Code::PageDown,
        _ => return None,
    })
}

/// Registers both hotkeys. Any capture registration failure fails the entire
/// capture registration may fail independently; the toggle remains available and
/// `capture_error` describes the disabled capture shortcut.
pub fn install(toggle_spec: &str, capture_spec: &str) -> anyhow::Result<HotkeyHandle> {
    let (toggle_mods, toggle_code) = parse(toggle_spec)
        .ok_or_else(|| anyhow!("invalid toggle hotkey specification: {toggle_spec:?}"))?;
    let (capture_mods, capture_code) = parse(capture_spec)
        .ok_or_else(|| anyhow!("invalid capture hotkey specification: {capture_spec:?}"))?;

    let toggle = HotKey::new(Some(toggle_mods), toggle_code);
    let capture = HotKey::new(Some(capture_mods), capture_code);
    if toggle.id() == capture.id() {
        return Err(anyhow!(
            "capture hotkey {capture_spec:?} conflicts with toggle hotkey {toggle_spec:?}"
        ));
    }

    let manager = GlobalHotKeyManager::new().context("failed to create global hotkey manager")?;
    manager
        .register(toggle)
        .with_context(|| format!("failed to register toggle hotkey {toggle_spec:?}"))?;
    let capture_error = manager
        .register(capture)
        .err()
        .map(|e| format!("捕获热键 {capture_spec} 注册失败: {e}"));
    let capture_id = if capture_error.is_none() {
        capture.id()
    } else {
        0
    };
    TOGGLE_ID.store(toggle.id(), Ordering::Release);
    CAPTURE_ID.store(capture_id, Ordering::Release);

    Ok(HotkeyHandle {
        manager,
        toggle_id: toggle.id(),
        capture_id,
        capture_error,
    })
}

#[cfg(windows)]
fn read_clipboard() -> String {
    use windows::Win32::Foundation::{HANDLE, HGLOBAL};
    use windows::Win32::System::DataExchange::{
        CloseClipboard, GetClipboardData, IsClipboardFormatAvailable, OpenClipboard,
    };
    use windows::Win32::System::Memory::{GlobalLock, GlobalUnlock};

    const CF_UNICODETEXT: u32 = 13;

    unsafe {
        for _ in 0..3 {
            if IsClipboardFormatAvailable(CF_UNICODETEXT).is_ok() && OpenClipboard(None).is_ok() {
                let text = (|| -> Option<String> {
                    let handle: HANDLE = GetClipboardData(CF_UNICODETEXT).ok()?;
                    let global = HGLOBAL(handle.0);
                    let pointer = GlobalLock(global);
                    if pointer.is_null() {
                        return None;
                    }
                    let pointer = pointer as *const u16;
                    let mut len = 0usize;
                    while *pointer.add(len) != 0 {
                        len += 1;
                    }
                    let text = String::from_utf16_lossy(std::slice::from_raw_parts(pointer, len));
                    let _ = GlobalUnlock(global);
                    Some(text)
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_modifiers_and_key_case_insensitively() {
        let (mods, code) = parse("control + ALT + Shift + n").unwrap();
        assert!(mods.contains(Modifiers::CONTROL));
        assert!(mods.contains(Modifiers::ALT));
        assert!(mods.contains(Modifiers::SHIFT));
        assert_eq!(code, Code::KeyN);
    }

    #[test]
    fn parses_aliases_and_special_keys() {
        let (mods, code) = parse("super+PageDown").unwrap();
        assert!(mods.contains(Modifiers::META));
        assert_eq!(code, Code::PageDown);
    }

    #[test]
    fn rejects_missing_unknown_or_multiple_keys() {
        assert!(parse("Ctrl+Alt").is_none());
        assert!(parse("Ctrl+Bogus").is_none());
        assert!(parse("Ctrl+N+C").is_none());
        assert!(parse("Ctrl++N").is_none());
    }
}
