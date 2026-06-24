use egui::{Align, Color32, FontId, Layout, Margin, Rect, RichText, Sense, Stroke, Vec2};

use crate::chrome::{self, TitleBarConfig, TITLE_BAR_HEIGHT};
use crate::config::Config;
use crate::fonts;
use crate::icons;
use crate::theme::{palette, ThemeMode};

pub fn draw_settings_window(ctx: &egui::Context, cfg: &mut Config, current_fg: Option<String>) {
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

        // 自绘标题栏
        let title_rect = Rect::from_min_max(
            full.min,
            egui::pos2(full.right(), full.top() + TITLE_BAR_HEIGHT),
        );
        let tb_out = chrome::draw_title_bar(
            ctx,
            ui,
            title_rect,
            TitleBarConfig {
                title: "设置",
                show_min_max: false,
                mode: cfg.theme_mode,
                on_top: None,
            },
        );
        if tb_out.close_clicked {
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        }

        // 内容
        let content_rect = Rect::from_min_max(
            egui::pos2(full.left() + 16.0, title_rect.bottom() + 8.0),
            egui::pos2(full.right() - 16.0, full.bottom() - 30.0),
        );
        let mut content_ui = ui.new_child(
            egui::UiBuilder::new()
                .max_rect(content_rect)
                .layout(Layout::top_down(Align::LEFT)),
        );
        draw_body(&mut content_ui, cfg, current_fg.as_deref());

        // Footer
        let footer_rect = Rect::from_min_max(
            egui::pos2(full.left() + 16.0, full.bottom() - 26.0),
            egui::pos2(full.right() - 16.0, full.bottom() - 6.0),
        );
        let mut footer_ui = ui.new_child(
            egui::UiBuilder::new()
                .max_rect(footer_rect)
                .layout(Layout::right_to_left(Align::Center)),
        );
        footer_ui.label(
            RichText::new("NxNote · 配置自动保存")
                .small()
                .color(p.text_weak),
        );

        chrome::draw_resize_handles(ctx, ui);
    });
}

fn draw_body(ui: &mut egui::Ui, cfg: &mut Config, current_fg: Option<&str>) {
    let p = palette(cfg.theme_mode);

    let total_w = ui.available_width();
    let col_gap = 24.0;
    let col_w = ((total_w - col_gap) / 2.0).max(220.0);
    let total_h = ui.available_height();

    let origin = ui.cursor().left_top();
    let left_rect = Rect::from_min_size(origin, Vec2::new(col_w, total_h));
    let right_rect = Rect::from_min_size(
        egui::pos2(origin.x + col_w + col_gap, origin.y),
        Vec2::new(col_w, total_h),
    );
    ui.allocate_rect(
        Rect::from_min_size(origin, Vec2::new(total_w, total_h)),
        Sense::hover(),
    );

    // 左列
    let mut left = ui.new_child(
        egui::UiBuilder::new()
            .max_rect(left_rect)
            .layout(Layout::top_down(Align::LEFT)),
    );
    egui::ScrollArea::vertical()
        .id_salt("settings_left")
        .auto_shrink([false; 2])
        .show(&mut left, |ui| {
            section(ui, "显示", &p, |ui| {
                row(ui, "主题模式", "外观主题", |ui| {
                    segmented(
                        ui,
                        &mut cfg.theme_mode,
                        &[
                            (ThemeMode::System, "系统"),
                            (ThemeMode::Light, "浅色"),
                            (ThemeMode::Dark, "深色"),
                        ],
                    );
                });
                row(ui, "字号", "正文字号", |ui| {
                    ui.add(
                        egui::DragValue::new(&mut cfg.font_size)
                            .range(10.0..=24.0)
                            .speed(0.1)
                            .suffix(" px"),
                    );
                });
                ui.add_space(2.0);
                ui.label(RichText::new("界面字体").color(p.text).size(12.5));
                font_list_picker(ui, "ui_fonts", &mut cfg.ui_fonts);
                ui.add_space(4.0);
                ui.label(RichText::new("编辑字体").color(p.text).size(12.5));
                font_list_picker(ui, "editor_fonts", &mut cfg.editor_fonts);
                ui.add_space(6.0);
                // 颜色配置入口 —— 用 LayoutJob 把图标和文字拼到同一行
                let mut job = egui::text::LayoutJob::default();
                job.append(
                    icons::COLOR_LENS,
                    0.0,
                    egui::TextFormat {
                        font_id: icons::font(14.0),
                        color: ui.visuals().text_color(),
                        valign: Align::Center,
                        ..Default::default()
                    },
                );
                job.append(
                    "   颜色配置…",
                    0.0,
                    egui::TextFormat {
                        font_id: FontId::proportional(12.5),
                        color: ui.visuals().text_color(),
                        valign: Align::Center,
                        ..Default::default()
                    },
                );
                if ui
                    .add_sized(Vec2::new(170.0, 26.0), egui::Button::new(job))
                    .on_hover_text("分别配置深色 / 浅色主题下的 MD 渲染色")
                    .clicked()
                {
                    ui.ctx().memory_mut(|m| {
                        m.data
                            .insert_temp(egui::Id::new("nx_open_color_editor"), true)
                    });
                }
            });

            section(ui, "窗口", &p, |ui| {
                row(ui, "自动隐藏标题栏", "鼠标移开时收起窗口顶部", |ui| {
                    toggle(ui, &mut cfg.autohide_title_bar);
                });
                row(ui, "默认宽度", "启动时的初始宽度", |ui| {
                    ui.add(
                        egui::DragValue::new(&mut cfg.window_width)
                            .range(200.0..=1600.0)
                            .speed(1.0)
                            .suffix(" px"),
                    );
                });
                row(ui, "默认高度", "启动时的初始高度", |ui| {
                    ui.add(
                        egui::DragValue::new(&mut cfg.window_height)
                            .range(160.0..=1600.0)
                            .speed(1.0)
                            .suffix(" px"),
                    );
                });
            });

            section(ui, "系统", &p, |ui| {
                row(ui, "开机自启", "登录 Windows 后自动以隐藏态启动", |ui| {
                    toggle(ui, &mut cfg.autostart);
                });
                row(
                    ui,
                    "关闭最小化到托盘",
                    "点关闭 X 或按 Alt+F4 时隐藏到托盘而不是退出（托盘菜单「退出」始终生效）",
                    |ui| {
                        toggle(ui, &mut cfg.close_to_tray);
                    },
                );
            });
        });

    // 右列
    let mut right = ui.new_child(
        egui::UiBuilder::new()
            .max_rect(right_rect)
            .layout(Layout::top_down(Align::LEFT)),
    );
    egui::ScrollArea::vertical()
        .id_salt("settings_right")
        .auto_shrink([false; 2])
        .show(&mut right, |ui| {
            section(ui, "热键", &p, |ui| {
                row(ui, "全局热键", "点击按钮后按下组合键", |ui| {
                    hotkey_recorder(ui, &mut cfg.hotkey);
                });
                ui.add_space(2.0);
                ui.label(
                    RichText::new("按 Esc 取消录制；按修饰键 + 普通键完成。")
                        .small()
                        .color(p.text_weak),
                );
            });

            section(ui, "自动行为", &p, |ui| {
                row(
                    ui,
                    "随前台切换笔记",
                    "打开后 NxNote 自动跟随当前前台应用；关闭则由你自己在菜单里选择",
                    |ui| {
                        toggle(ui, &mut cfg.auto_follow_foreground);
                    },
                );
                row(ui, "保存延迟", "停止输入后写盘等待 (ms)", |ui| {
                    ui.add(
                        egui::DragValue::new(&mut cfg.autosave_delay_ms)
                            .range(200..=10000)
                            .speed(10),
                    );
                });
                row(ui, "前台轮询", "检测前台应用间隔 (ms)", |ui| {
                    ui.add(
                        egui::DragValue::new(&mut cfg.poll_interval_ms)
                            .range(100..=5000)
                            .speed(10),
                    );
                });
            });

            section(ui, "应用黑名单", &p, |ui| {
                ui.label(
                    RichText::new("命中后视为未绑定，写入速记本。匹配 exe 全路径或文件名。")
                        .small()
                        .color(p.text_weak),
                );
                ui.add_space(4.0);
                blocked_apps_picker(ui, &mut cfg.blocked_apps, current_fg, &p);
            });
        });
}

fn blocked_apps_picker(
    ui: &mut egui::Ui,
    value: &mut Vec<String>,
    current_fg: Option<&str>,
    p: &crate::theme::Palette,
) {
    // 当前前台 debug + 一键加黑
    if let Some(fg) = current_fg {
        ui.add_space(2.0);
        ui.label(
            RichText::new("当前前台 exe（NxNote 实际看到的路径）：")
                .small()
                .color(p.text_weak),
        );
        ui.horizontal(|ui| {
            ui.label(RichText::new(fg).small().color(p.text));
        });
        if ui
            .button("把当前前台加入黑名单")
            .on_hover_text("用 NxNote 实际看到的路径，避免手输出入")
            .clicked()
        {
            let s = fg.to_string();
            if !value.contains(&s) {
                value.push(s);
            }
        }
        ui.add_space(6.0);
    }

    let mut to_remove: Option<usize> = None;
    for (i, item) in value.iter().enumerate() {
        ui.horizontal(|ui| {
            let display = if item.len() > 36 {
                let s: String = item.chars().take(34).collect();
                format!("{}…", s)
            } else {
                item.clone()
            };
            ui.label(RichText::new(display).size(12.0))
                .on_hover_text(item);
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                ui.scope(|ui| {
                    ui.spacing_mut().button_padding = Vec2::new(4.0, 1.0);
                    if ui.button(RichText::new("×").size(11.0)).clicked() {
                        to_remove = Some(i);
                    }
                });
            });
        });
    }
    if let Some(i) = to_remove {
        value.remove(i);
    }
    ui.add_space(4.0);
    ui.horizontal(|ui| {
        let id = ui.id().with("blocked_input");
        let mut buf: String = ui
            .ctx()
            .memory_mut(|m| m.data.get_temp::<String>(id).unwrap_or_default());
        let resp = ui.add(
            egui::TextEdit::singleline(&mut buf)
                .desired_width(180.0)
                .hint_text("exe 文件名或完整路径"),
        );
        if resp.changed() {
            ui.ctx().memory_mut(|m| m.data.insert_temp(id, buf.clone()));
        }
        if ui.button("添加").clicked() {
            let trimmed = buf.trim().to_string();
            if !trimmed.is_empty() && !value.contains(&trimmed) {
                value.push(trimmed);
            }
            ui.ctx().memory_mut(|m| m.data.insert_temp(id, String::new()));
        }
    });
}

fn section<R>(
    ui: &mut egui::Ui,
    title: &str,
    p: &crate::theme::Palette,
    add_contents: impl FnOnce(&mut egui::Ui) -> R,
) -> R {
    ui.add_space(6.0);
    ui.label(RichText::new(title).color(p.text_strong).size(13.0));
    ui.add_space(2.0);
    let r = add_contents(ui);
    ui.add_space(8.0);
    r
}

fn row<R>(
    ui: &mut egui::Ui,
    label: &str,
    hint: &str,
    add_control: impl FnOnce(&mut egui::Ui) -> R,
) -> R {
    let mut result: Option<R> = None;
    ui.horizontal(|ui| {
        ui.set_min_height(26.0);
        ui.label(label).on_hover_text(hint);
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            result = Some(add_control(ui));
        });
    });
    result.unwrap()
}

fn segmented<T: PartialEq + Clone>(
    ui: &mut egui::Ui,
    value: &mut T,
    options: &[(T, &str)],
) {
    ui.scope(|ui| {
        ui.style_mut().spacing.item_spacing = Vec2::new(2.0, 0.0);
        ui.style_mut().spacing.button_padding = Vec2::new(10.0, 4.0);
        for (v, label) in options {
            let selected = value == v;
            let resp = ui.add(egui::SelectableLabel::new(selected, *label));
            if resp.clicked() {
                *value = v.clone();
            }
        }
    });
}

fn toggle(ui: &mut egui::Ui, value: &mut bool) {
    let desired = Vec2::new(36.0, 18.0);
    let (rect, resp) = ui.allocate_exact_size(desired, Sense::click());
    if resp.clicked() {
        *value = !*value;
    }
    let p = ui.painter();
    let bg = if *value {
        ui.visuals().selection.bg_fill
    } else {
        ui.visuals().widgets.inactive.bg_fill
    };
    p.rect_filled(rect, egui::Rounding::same(9.0), bg);
    let knob_r = 6.5;
    let pad = 2.0;
    let knob_cx = if *value {
        rect.right() - knob_r - pad
    } else {
        rect.left() + knob_r + pad
    };
    let knob_color = if *value { Color32::WHITE } else { ui.visuals().text_color() };
    p.circle_filled(egui::pos2(knob_cx, rect.center().y), knob_r, knob_color);
}

fn hotkey_recorder(ui: &mut egui::Ui, value: &mut String) {
    let id = ui.id().with("hotkey_recorder");
    let mut recording: bool = ui
        .ctx()
        .memory(|m| m.data.get_temp(id).unwrap_or(false));

    let label_txt = if recording {
        "按下任意组合键…".to_string()
    } else if value.is_empty() {
        "<未绑定>".to_string()
    } else {
        value.clone()
    };

    // 用 LayoutJob 组合图标 + 标签
    let color = if recording {
        ui.visuals().selection.bg_fill
    } else {
        ui.visuals().text_color()
    };
    let mut job = egui::text::LayoutJob::default();
    job.append(
        icons::KEYBOARD,
        0.0,
        egui::TextFormat {
            font_id: icons::font(14.0),
            color,
            valign: Align::Center,
            ..Default::default()
        },
    );
    job.append(
        &format!("   {}", label_txt),
        0.0,
        egui::TextFormat {
            font_id: FontId::proportional(12.5),
            color,
            valign: Align::Center,
            ..Default::default()
        },
    );

    let btn_size = Vec2::new(170.0, 26.0);
    let resp = ui.add_sized(btn_size, egui::Button::new(job));
    if resp.clicked() {
        recording = !recording;
    }

    if recording {
        if let Some(captured) = capture_hotkey(ui.ctx()) {
            if captured == "__cancel__" {
                recording = false;
            } else {
                *value = captured;
                recording = false;
            }
        }
    }

    ui.ctx().memory_mut(|m| m.data.insert_temp(id, recording));
}

fn capture_hotkey(ctx: &egui::Context) -> Option<String> {
    ctx.input(|i| {
        for ev in &i.events {
            if let egui::Event::Key {
                key,
                modifiers,
                pressed: true,
                repeat: false,
                ..
            } = ev
            {
                if *key == egui::Key::Escape {
                    return Some("__cancel__".to_string());
                }
                if let Some(name) = key_name(*key) {
                    return Some(format_hotkey(*modifiers, name));
                }
            }
        }
        None
    })
}

fn format_hotkey(mods: egui::Modifiers, key_name: &str) -> String {
    let mut parts: Vec<&str> = Vec::new();
    if mods.ctrl {
        parts.push("Ctrl");
    }
    if mods.alt {
        parts.push("Alt");
    }
    if mods.shift {
        parts.push("Shift");
    }
    if mods.mac_cmd || (mods.command && !mods.ctrl) {
        parts.push("Win");
    }
    parts.push(key_name);
    parts.join("+")
}

fn font_list_picker(ui: &mut egui::Ui, id: &str, value: &mut Vec<String>) {
    ui.add_space(2.0);
    let mut to_remove: Option<usize> = None;
    let mut to_move: Option<(usize, isize)> = None;

    let len = value.len();
    for (i, path) in value.iter().enumerate() {
        ui.horizontal(|ui| {
            let name = fonts::font_display_name(std::path::Path::new(path));
            ui.scope(|ui| {
                ui.spacing_mut().button_padding = Vec2::new(4.0, 1.0);
                let up = ui.add_enabled(i > 0, egui::Button::new(
                    egui::RichText::new("▲").size(10.0),
                ));
                if up.clicked() {
                    to_move = Some((i, -1));
                }
                let dn = ui.add_enabled(i + 1 < len, egui::Button::new(
                    egui::RichText::new("▼").size(10.0),
                ));
                if dn.clicked() {
                    to_move = Some((i, 1));
                }
            });
            ui.label(RichText::new(format!("{}. ", i + 1)).color(ui.visuals().weak_text_color()).small());
            ui.label(RichText::new(name).size(12.0));
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                ui.scope(|ui| {
                    ui.spacing_mut().button_padding = Vec2::new(4.0, 1.0);
                    if ui.button(RichText::new("×").size(11.0)).clicked() {
                        to_remove = Some(i);
                    }
                });
            });
        });
    }

    if let Some(i) = to_remove {
        value.remove(i);
    }
    if let Some((i, dir)) = to_move {
        let j = ((i as isize) + dir) as usize;
        if j < value.len() {
            value.swap(i, j);
        }
    }

    ui.add_space(2.0);
    egui::ComboBox::from_id_salt(format!("{}_add", id))
        .selected_text(RichText::new("＋ 添加字体").size(12.0))
        .width(160.0)
        .show_ui(ui, |ui| {
            ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Truncate);
            egui::ScrollArea::vertical().max_height(280.0).show(ui, |ui| {
                for path in fonts::list_system_fonts() {
                    let path_str = path.to_string_lossy().to_string();
                    if value.contains(&path_str) {
                        continue;
                    }
                    let name = fonts::font_display_name(path);
                    if ui.selectable_label(false, name).clicked() {
                        value.push(path_str);
                    }
                }
            });
        });
}

fn key_name(k: egui::Key) -> Option<&'static str> {
    use egui::Key::*;
    Some(match k {
        A => "A", B => "B", C => "C", D => "D", E => "E", F => "F",
        G => "G", H => "H", I => "I", J => "J", K => "K", L => "L",
        M => "M", N => "N", O => "O", P => "P", Q => "Q", R => "R",
        S => "S", T => "T", U => "U", V => "V", W => "W", X => "X",
        Y => "Y", Z => "Z",
        Num0 => "0", Num1 => "1", Num2 => "2", Num3 => "3", Num4 => "4",
        Num5 => "5", Num6 => "6", Num7 => "7", Num8 => "8", Num9 => "9",
        F1 => "F1", F2 => "F2", F3 => "F3", F4 => "F4", F5 => "F5", F6 => "F6",
        F7 => "F7", F8 => "F8", F9 => "F9", F10 => "F10", F11 => "F11", F12 => "F12",
        Space => "Space",
        ArrowUp => "Up", ArrowDown => "Down", ArrowLeft => "Left", ArrowRight => "Right",
        Home => "Home", End => "End", PageUp => "PageUp", PageDown => "PageDown",
        _ => return None,
    })
}
