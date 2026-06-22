#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod app;
mod chrome;
mod config;
mod fonts;
mod hotkey;
mod icons;
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
    let cfg = config::Config::load();

    let mut viewport = egui::ViewportBuilder::default()
        .with_title("NxNote")
        .with_inner_size([cfg.window_width, cfg.window_height])
        .with_min_inner_size([220.0, 200.0])
        .with_resizable(true)
        .with_decorations(false)
        .with_transparent(false);

    if let Some(icon) = load_icon() {
        viewport = viewport.with_icon(icon);
    }

    let viewport = if cfg.always_on_top {
        viewport.with_window_level(egui::WindowLevel::AlwaysOnTop)
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
            Ok(Box::new(NxNoteApp::new(cc, cfg)))
        }),
    )
}
