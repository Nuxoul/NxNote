#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod app;
mod autostart;
mod chrome;
mod color_ui;
mod config;
mod fonts;
mod fsutil;
mod hotkey;
mod icons;
mod images;
mod md_highlight;
mod settings_ui;
mod storage;
mod theme;
mod tray;
mod watcher;

use app::NxNoteApp;

const APP_ICON_PNG: &[u8] = include_bytes!("../assets/icon.png");

fn load_icon() -> Option<egui::IconData> {
    let img = image::load_from_memory(APP_ICON_PNG).ok()?;
    let rgba = img.to_rgba8();
    let (w, h) = rgba.dimensions();
    Some(egui::IconData {
        rgba: rgba.into_raw(),
        width: w,
        height: h,
    })
}

fn main() -> eframe::Result<()> {
    // --data-dir <path>：数据目录覆盖（便携模式 / 隔离测试），必须在加载配置前生效
    {
        let mut args = std::env::args().skip(1);
        while let Some(a) = args.next() {
            if a == "--data-dir" {
                if let Some(dir) = args.next() {
                    config::set_data_dir_override(std::path::PathBuf::from(dir));
                }
            }
        }
    }

    let cfg = config::Config::load();
    let start_hidden = std::env::args().any(|a| a == "--hidden");

    let mut viewport = egui::ViewportBuilder::default()
        .with_title("NxNote")
        .with_inner_size([cfg.window_width, cfg.window_height])
        .with_min_inner_size([220.0, 200.0])
        .with_resizable(true)
        .with_decorations(false)
        .with_transparent(false);

    // 恢复上次窗口位置
    if let Some([x, y]) = cfg.window_pos {
        if x.is_finite() && y.is_finite() {
            viewport = viewport.with_position(egui::pos2(x, y));
        }
    }

    if let Some(icon) = load_icon() {
        viewport = viewport.with_icon(icon);
    }

    let viewport = if cfg.always_on_top {
        viewport.with_window_level(egui::WindowLevel::AlwaysOnTop)
    } else {
        viewport
    };

    // --hidden：用 with_visible(false) 真隐藏，任务栏不留图标
    let viewport = if start_hidden {
        viewport.with_visible(false)
    } else {
        viewport
    };

    let options = eframe::NativeOptions {
        viewport,
        vsync: false,
        ..Default::default()
    };

    eframe::run_native(
        "NxNote",
        options,
        Box::new(move |cc| {
            fonts::install_with_lists(&cc.egui_ctx, &cfg.ui_fonts, &cfg.editor_fonts);
            theme::apply(&cc.egui_ctx, cfg.theme_mode, cfg.font_size);
            let mut app = NxNoteApp::new(cc, cfg);
            if start_hidden {
                // 推迟两帧执行 hide：等 viewport 真正拿到了 outer_rect，
                // 否则 toggle_hidden 拿不到 current pos，restore 时会丢
                app.start_hidden_pending = Some(2);
            }
            Ok(Box::new(app))
        }),
    )
}
