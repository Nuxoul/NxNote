//! 颜色配置三级界面。
//! 从设置 → "颜色配置…" 按钮打开。可分别编辑深色和浅色主题下的 MD 渲染色。

use egui::{Align, Color32, Layout, Margin, Rect, RichText, Sense, Stroke, Vec2};

use crate::chrome::{self, TitleBarConfig, TITLE_BAR_HEIGHT};
use crate::config::{Config, MdColors};
use crate::theme::{palette, ThemeMode};

#[derive(Clone, Copy, PartialEq, Eq)]
enum Tab {
    Dark,
    Light,
}

pub fn draw_color_editor(ctx: &egui::Context, cfg: &mut Config) {
    let p = palette(cfg.theme_mode);

    let frame = egui::Frame {
        fill: p.bg,
        stroke: Stroke::new(1.0, p.stroke),
        rounding: egui::Rounding::ZERO,
        inner_margin: Margin::ZERO,
        outer_margin: Margin::ZERO,
        ..Default::default()
    };

    egui::CentralPanel::default().frame(frame).show(ctx, |ui| {
        let full = ui.max_rect();

        // 标题栏
        let title_rect = Rect::from_min_max(
            full.min,
            egui::pos2(full.right(), full.top() + TITLE_BAR_HEIGHT),
        );
        let tb_out = chrome::draw_title_bar(
            ctx,
            ui,
            title_rect,
            TitleBarConfig {
                title: "颜色配置",
                show_min_max: false,
                mode: cfg.theme_mode,
                on_top: None,
            },
        );
        if tb_out.close_clicked {
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        }

        // tab 状态默认跟随当前主题
        let tab_id = egui::Id::new("nx_color_editor_tab");
        let default_tab = if matches!(cfg.theme_mode, ThemeMode::Light) {
            Tab::Light
        } else {
            Tab::Dark
        };
        let mut tab: Tab = ctx
            .memory(|m| m.data.get_temp::<u8>(tab_id))
            .map(|v| if v == 1 { Tab::Light } else { Tab::Dark })
            .unwrap_or(default_tab);

        // 内容区
        let body_rect = Rect::from_min_max(
            egui::pos2(full.left() + 16.0, title_rect.bottom() + 10.0),
            egui::pos2(full.right() - 16.0, full.bottom() - 10.0),
        );
        let mut body = ui.new_child(
            egui::UiBuilder::new()
                .max_rect(body_rect)
                .layout(Layout::top_down(Align::LEFT)),
        );

        // 顶部：tab + 「恢复默认」
        body.horizontal(|ui| {
            ui.spacing_mut().button_padding = Vec2::new(10.0, 4.0);
            for (t, label) in [(Tab::Dark, "深色"), (Tab::Light, "浅色")] {
                let resp = ui.add(egui::SelectableLabel::new(tab == t, label));
                if resp.clicked() {
                    tab = t;
                }
            }
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if ui.button("恢复默认").clicked() {
                    match tab {
                        Tab::Dark => cfg.md_dark = MdColors::default_dark(),
                        Tab::Light => cfg.md_light = MdColors::default_light(),
                    }
                }
            });
        });
        body.add_space(2.0);
        body.label(
            RichText::new("修改后自动保存。预览效果在主窗口实时生效。")
                .small()
                .color(p.text_weak),
        );
        body.add_space(8.0);

        let colors = match tab {
            Tab::Dark => &mut cfg.md_dark,
            Tab::Light => &mut cfg.md_light,
        };

        egui::ScrollArea::vertical()
            .id_salt("nx_color_scroll")
            .auto_shrink([false; 2])
            .show(&mut body, |ui| {
                color_row(ui, "正文", &mut colors.text);
                color_row(ui, "标题", &mut colors.heading);
                color_row(ui, "加粗", &mut colors.bold);
                color_row(ui, "斜体", &mut colors.italic);
                color_row(ui, "行内代码 · 文字", &mut colors.code_text);
                color_row(ui, "行内代码 · 背景", &mut colors.code_bg);
                color_row(ui, "链接", &mut colors.link);
                color_row(ui, "引用 · 文字", &mut colors.quote_text);
                color_row(ui, "引用 · 左边线", &mut colors.quote_bar);
                color_row(ui, "列表标记 (· 1.)", &mut colors.list_marker);
                color_row(ui, "语法符号 (# ** [])", &mut colors.syntax);
            });

        ctx.memory_mut(|m| {
            m.data
                .insert_temp(tab_id, if matches!(tab, Tab::Light) { 1u8 } else { 0u8 })
        });

        chrome::draw_resize_handles(ctx, ui);
    });
}

fn color_row(ui: &mut egui::Ui, label: &str, color: &mut [u8; 3]) {
    ui.horizontal(|ui| {
        ui.set_min_height(28.0);
        ui.label(RichText::new(label).size(12.5));
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            // hex 显示在右
            let hex = format!("#{:02X}{:02X}{:02X}", color[0], color[1], color[2]);
            ui.label(
                RichText::new(hex)
                    .monospace()
                    .size(11.0)
                    .color(ui.visuals().weak_text_color()),
            );
            ui.add_space(8.0);
            egui::color_picker::color_edit_button_srgb(ui, color);
        });
    });
    let (rect, _) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 1.0), Sense::hover());
    let stroke_color = Color32::from_white_alpha(10);
    ui.painter().line_segment(
        [rect.left_center(), rect.right_center()],
        Stroke::new(0.5, stroke_color),
    );
    ui.add_space(2.0);
}
