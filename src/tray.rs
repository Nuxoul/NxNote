use std::sync::mpsc::{self, Receiver};
use std::thread;

use tray_icon::menu::{Menu, MenuEvent, MenuItem, PredefinedMenuItem};
use tray_icon::{Icon, TrayIcon, TrayIconBuilder, TrayIconEvent};

const APP_ICON_PNG: &[u8] = include_bytes!("../assets/icon.png");

#[derive(Debug, Clone, Copy)]
pub enum TrayAction {
    Toggle, // 左键单击 / 双击 / "显示/隐藏" 菜单
    Quit,
}

/// 保留 TrayIcon 句柄，否则托盘图标会立刻消失
pub struct TrayHandle {
    _tray: TrayIcon,
}

pub fn install(egui_ctx: egui::Context) -> Option<(TrayHandle, Receiver<TrayAction>)> {
    let icon = load_icon()?;

    let menu = Menu::new();
    let show_item = MenuItem::with_id("nx.toggle", "显示 / 隐藏", true, None);
    let sep = PredefinedMenuItem::separator();
    let quit_item = MenuItem::with_id("nx.quit", "退出", true, None);
    menu.append(&show_item).ok()?;
    menu.append(&sep).ok()?;
    menu.append(&quit_item).ok()?;

    let tray = TrayIconBuilder::new()
        .with_menu(Box::new(menu))
        .with_tooltip("NxNote")
        .with_icon(icon)
        .build()
        .ok()?;

    let (tx, rx) = mpsc::channel::<TrayAction>();

    // 菜单事件
    let tx_menu = tx.clone();
    let ctx_menu = egui_ctx.clone();
    thread::spawn(move || {
        let recv = MenuEvent::receiver();
        loop {
            match recv.recv() {
                Ok(ev) => {
                    let action = match ev.id.as_ref() {
                        "nx.toggle" => Some(TrayAction::Toggle),
                        "nx.quit" => Some(TrayAction::Quit),
                        _ => None,
                    };
                    if let Some(a) = action {
                        if tx_menu.send(a).is_err() {
                            break;
                        }
                        ctx_menu.request_repaint();
                    }
                }
                Err(_) => break,
            }
        }
    });

    // 托盘图标事件（左键单击切换显示/隐藏）
    let tx_tray = tx;
    let ctx_tray = egui_ctx.clone();
    thread::spawn(move || {
        let recv = TrayIconEvent::receiver();
        loop {
            match recv.recv() {
                Ok(ev) => {
                    let toggle = matches!(
                        ev,
                        TrayIconEvent::Click {
                            button: tray_icon::MouseButton::Left,
                            button_state: tray_icon::MouseButtonState::Up,
                            ..
                        } | TrayIconEvent::DoubleClick {
                            button: tray_icon::MouseButton::Left,
                            ..
                        }
                    );
                    if toggle {
                        if tx_tray.send(TrayAction::Toggle).is_err() {
                            break;
                        }
                        ctx_tray.request_repaint();
                    }
                }
                Err(_) => break,
            }
        }
    });

    Some((TrayHandle { _tray: tray }, rx))
}

fn load_icon() -> Option<Icon> {
    let img = image::load_from_memory(APP_ICON_PNG).ok()?;
    let rgba = img.to_rgba8();
    let (w, h) = rgba.dimensions();
    Icon::from_rgba(rgba.into_raw(), w, h).ok()
}
