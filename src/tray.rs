use std::thread;

use anyhow::Context;
use async_channel::{Sender, TrySendError};
use tray_icon::menu::{Menu, MenuEvent, MenuItem, PredefinedMenuItem};
use tray_icon::{Icon, TrayIcon, TrayIconBuilder, TrayIconEvent};

use crate::platform::PlatformEvent;

const APP_ICON_PNG: &[u8] = include_bytes!("../assets/icon.png");
const TOGGLE_MENU_ID: &str = "nx.toggle";
const QUIT_MENU_ID: &str = "nx.quit";

/// Keeps the tray icon alive.
pub struct TrayHandle {
    _tray: TrayIcon,
}

pub fn install(sender: Sender<PlatformEvent>) -> anyhow::Result<TrayHandle> {
    let icon = load_icon()?;

    let menu = Menu::new();
    let show_item = MenuItem::with_id(TOGGLE_MENU_ID, "显示 / 隐藏", true, None);
    let separator = PredefinedMenuItem::separator();
    let quit_item = MenuItem::with_id(QUIT_MENU_ID, "退出", true, None);
    menu.append(&show_item)
        .context("failed to append tray toggle menu item")?;
    menu.append(&separator)
        .context("failed to append tray menu separator")?;
    menu.append(&quit_item)
        .context("failed to append tray quit menu item")?;

    let tray = TrayIconBuilder::new()
        .with_menu(Box::new(menu))
        // Left click emits TrayIconEvent; right click opens the menu.
        .with_menu_on_left_click(false)
        .with_tooltip("NxNote")
        .with_icon(icon)
        .build()
        .context("failed to create tray icon")?;

    spawn_menu_listener(sender.clone());
    spawn_icon_listener(sender);

    Ok(TrayHandle { _tray: tray })
}

fn spawn_menu_listener(sender: Sender<PlatformEvent>) {
    thread::spawn(move || {
        let receiver = MenuEvent::receiver();
        while let Ok(event) = receiver.recv() {
            let platform_event = match event.id.as_ref() {
                TOGGLE_MENU_ID => Some(PlatformEvent::Toggle),
                QUIT_MENU_ID => Some(PlatformEvent::Quit),
                _ => None,
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

fn spawn_icon_listener(sender: Sender<PlatformEvent>) {
    thread::spawn(move || {
        let receiver = TrayIconEvent::receiver();
        while let Ok(event) = receiver.recv() {
            let left_click = matches!(
                event,
                TrayIconEvent::Click {
                    button: tray_icon::MouseButton::Left,
                    button_state: tray_icon::MouseButtonState::Up,
                    ..
                } | TrayIconEvent::DoubleClick {
                    button: tray_icon::MouseButton::Left,
                    ..
                }
            );
            if left_click {
                match sender.try_send(PlatformEvent::Show) {
                    Ok(()) | Err(TrySendError::Full(_)) => {}
                    Err(TrySendError::Closed(_)) => break,
                }
            }
        }
    });
}

fn load_icon() -> anyhow::Result<Icon> {
    let image = image::load_from_memory(APP_ICON_PNG).context("failed to decode tray icon")?;
    let rgba = image.to_rgba8();
    let (width, height) = rgba.dimensions();
    Icon::from_rgba(rgba.into_raw(), width, height).context("failed to create tray icon pixels")
}
