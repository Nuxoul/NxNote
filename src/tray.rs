use std::sync::mpsc::{self, Receiver};
use std::thread;

use tray_icon::menu::{Menu, MenuEvent, MenuItem, PredefinedMenuItem};
use tray_icon::{Icon, TrayIcon, TrayIconBuilder, TrayIconEvent};

const APP_ICON_PNG: &[u8] = include_bytes!("../assets/icon.png");

#[derive(Debug, Clone, Copy)]
pub enum TrayAction {
    /// 后台线程已经 Win32 改了窗口可见性，main 只需 drain + 同步 self.hidden / 跑一帧
    Sync,
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
        // 左键单击只触发 TrayIconEvent::Click（用于 toggle 显示/隐藏），不弹菜单。
        // 菜单由右键单击触发。
        .with_menu_on_left_click(false)
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
                    match ev.id.as_ref() {
                        "nx.toggle" => {
                            // 菜单里点"显示/隐藏" —— 直接 Win32 切换；main 只同步状态
                            crate::app::force_toggle();
                            let _ = tx_menu.send(TrayAction::Sync);
                            ctx_menu.request_repaint();
                        }
                        "nx.quit" => {
                            // 隐藏状态下 Windows 不派发 WM_PAINT，eframe 的
                            // update() 不会跑，drain_tray 也就消费不到 Quit。
                            // 先把窗口拉出来一帧，主循环必跑 → 退出生效
                            crate::app::force_show();
                            let _ = tx_menu.send(TrayAction::Quit);
                            ctx_menu.request_repaint();
                        }
                        _ => {}
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
                    let left_click = matches!(
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
                    if left_click {
                        // 左键托盘 = 只显示，不会把已显示的窗口再藏起来
                        crate::app::force_show();
                        let _ = tx_tray.send(TrayAction::Sync);
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
