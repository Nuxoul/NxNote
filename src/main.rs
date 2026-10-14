#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
// GPUI's derives resolve through this alias while all application imports use the Kit facade.
extern crate gpui_kit as gpui;
mod app;
mod attachments;
mod autostart;
mod config;
mod document_layout;
mod editor_support;
mod fonts;
mod fsutil;
mod hotkey;
mod markdown_edit;
mod markdown_format;
mod markdown_extensions;
mod markdown_renderers;
mod markdown_projection;
mod note_editor;
mod note_links;
mod platform;
mod session;
mod settings_ui;
mod storage;
mod theme;
mod tray;
mod watcher;

use gpui_kit::{point, px, size, AppContext, Bounds, TitlebarOptions, WindowBounds, WindowOptions};

fn main() {
    let mut args = std::env::args().skip(1);
    let mut hidden = false;
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--data-dir" => {
                let Some(dir) = args.next() else {
                    eprintln!("--data-dir 需要目录参数");
                    return;
                };
                config::set_data_dir_override(dir.into());
            }
            "--hidden" => hidden = true,
            _ => {}
        }
    }
    let cfg = config::Config::load();
    let session = match session::Session::new(cfg.clone()) {
        Ok(session) => session,
        Err(error) => {
            eprintln!("无法加载笔记，未修改原文件: {error:#}");
            return;
        }
    };
    gpui_kit::application()
        .with_assets(gpui_kit::assets::Assets)
        .with_http_client(std::sync::Arc::new(
            gpui_reqwest_client::ReqwestClient::new(),
        ))
        .run(move |cx| {
            gpui_kit::init(cx);
            app::bind_keys(cx);
            if let Err(error) = fonts::install(&cfg, cx) {
                eprintln!("字体设置无效: {error:#}");
            }
            theme::apply(&cfg, None, cx);
            let origin = cfg
                .window_pos
                .filter(|p| p.iter().all(|v| v.is_finite()))
                .map(|[x, y]| point(px(x), px(y)))
                .unwrap_or(point(px(120.), px(120.)));
            let options = WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(Bounds::new(
                    origin,
                    size(
                        px(cfg.window_width.max(220.)),
                        px(cfg.window_height.max(200.)),
                    ),
                ))),
                titlebar: Some(TitlebarOptions {
                    title: Some("NxNote".into()),
                    appears_transparent: true,
                    ..Default::default()
                }),
                window_min_size: Some(size(px(220.), px(200.))),
                show: !hidden,
                focus: !hidden,
                app_id: Some("NxNote".into()),
                ..Default::default()
            };
            match gpui_kit::open_window(options, cx, |window, cx| {
                cx.new(|cx| app::NxNoteApp::new(session, hidden, window, cx))
            }) {
                Ok((_, view)) => {
                    cx.on_app_quit(move |cx| {
                        view.update(cx, |app, _| {
                            if let Err(error) = app.session.persist() {
                                eprintln!("退出保存失败: {error:#}");
                            }
                        });
                        async {}
                    })
                    .detach();
                }
                Err(error) => {
                    eprintln!("创建 NxNote 窗口失败: {error:#}");
                    cx.quit();
                }
            }
        });
}
