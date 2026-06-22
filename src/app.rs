use std::sync::mpsc::Receiver;
use std::time::Instant;

use crate::chrome::{self, TitleBarConfig, TITLE_BAR_HEIGHT};
use crate::config::Config;
use crate::fonts;
use crate::hotkey::{self, HotkeyHandle};
use crate::icons;
use crate::md_highlight;
use crate::settings_ui;
use crate::storage::{
    self, AppEntry, AppIndex, TitleRule, DEFAULT_NOTE, GLOBAL_FOLDER, SCRATCH_NOTE,
};
use crate::theme::{self, palette};
use crate::watcher::{self, ForegroundInfo};

#[derive(Default, PartialEq, Eq, Clone)]
enum Modal {
    #[default]
    None,
    NotesList,
    TitleLearn { title: String },
    NewNote { input: String },
    Rename { input: String, old: String },
}

pub struct NxNoteApp {
    pub cfg: Config,
    cfg_dirty: bool,
    last_applied_theme: crate::theme::ThemeMode,
    last_applied_font: f32,
    last_applied_ui_fonts: Vec<String>,
    last_applied_editor_fonts: Vec<String>,
    last_bound_hotkey: String,
    index: AppIndex,

    fg_rx: Receiver<ForegroundInfo>,
    fg: Option<ForegroundInfo>,
    _hotkey: Option<HotkeyHandle>,
    hotkey_rx: Receiver<()>,
    _tray: Option<crate::tray::TrayHandle>,
    tray_rx: Option<Receiver<crate::tray::TrayAction>>,

    hwnd_raw: Option<isize>,

    folder_key: String,
    display_name: String,
    note_name: String,
    editor_text: String,
    dirty: bool,
    last_edit: Option<Instant>,

    pinned: bool,
    modal: Modal,
    hidden_pos: Option<egui::Pos2>,
    title_visible: bool,
    title_first_frame: bool,
    title_pending_target: Option<bool>,
    title_pending_since: Option<Instant>,
    editor_cursor_line: Option<usize>,

    settings_open: bool,
    settings_fonts_done: bool,
    settings_theme_done: bool,
    settings_pos_applied: bool,
}

impl NxNoteApp {
    pub fn new(cc: &eframe::CreationContext<'_>, cfg: Config) -> Self {
        let fg_rx = watcher::spawn(cfg.poll_interval_ms, cc.egui_ctx.clone());
        let hotkey = hotkey::install(&cfg.hotkey);
        let hotkey_rx = hotkey::spawn_listener(cc.egui_ctx.clone());
        let (tray_handle, tray_rx) = match crate::tray::install(cc.egui_ctx.clone()) {
            Some((h, rx)) => (Some(h), Some(rx)),
            None => (None, None),
        };

        let index = AppIndex::load();
        let folder_key = GLOBAL_FOLDER.to_string();
        let display_name = "速记".to_string();
        let note_name = SCRATCH_NOTE.to_string();
        let editor_text = storage::load_note(&folder_key, &note_name);

        let last_applied_theme = cfg.theme_mode;
        let last_applied_font = cfg.font_size;
        let last_applied_ui_fonts = cfg.ui_fonts.clone();
        let last_applied_editor_fonts = cfg.editor_fonts.clone();
        let last_bound_hotkey = cfg.hotkey.clone();

        Self {
            cfg,
            cfg_dirty: false,
            last_applied_theme,
            last_applied_font,
            last_applied_ui_fonts,
            last_applied_editor_fonts,
            last_bound_hotkey,
            index,
            fg_rx,
            fg: None,
            _hotkey: hotkey,
            hotkey_rx,
            _tray: tray_handle,
            tray_rx,
            hwnd_raw: None,
            folder_key,
            display_name,
            note_name,
            editor_text,
            dirty: false,
            last_edit: None,
            pinned: false,
            modal: Modal::None,
            hidden_pos: None,
            title_visible: true,
            title_first_frame: true,
            title_pending_target: None,
            title_pending_since: None,
            editor_cursor_line: None,
            settings_open: false,
            settings_fonts_done: false,
            settings_theme_done: false,
            settings_pos_applied: false,
        }
    }

    fn save_current(&mut self) {
        if !self.dirty {
            return;
        }
        let _ = storage::save_note(&self.folder_key, &self.note_name, &self.editor_text);
        self.dirty = false;
    }

    fn switch_to(&mut self, folder_key: String, display_name: String, note_name: String) {
        if folder_key == self.folder_key && note_name == self.note_name {
            return;
        }
        self.save_current();
        self.folder_key = folder_key;
        self.display_name = display_name;
        self.note_name = note_name;
        self.editor_text = storage::load_note(&self.folder_key, &self.note_name);
        self.last_edit = None;
    }

    fn handle_foreground_change(&mut self, info: ForegroundInfo) {
        self.fg = Some(info.clone());
        if self.pinned {
            return;
        }
        // 黑名单：命中就落回速记本
        if app_blocked(&self.cfg.blocked_apps, &info.exe_path) {
            if self.folder_key != GLOBAL_FOLDER {
                self.switch_to(
                    GLOBAL_FOLDER.to_string(),
                    "速记".to_string(),
                    SCRATCH_NOTE.to_string(),
                );
            }
            return;
        }
        let folder = storage::folder_key_for(&info.exe_path);
        let display = info
            .exe_path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("应用")
            .to_string();

        let entry = self.index.apps.entry(folder.clone()).or_insert_with(|| AppEntry {
            exe_path: info.exe_path.to_string_lossy().to_string(),
            display_name: display.clone(),
            title_rule: None,
            notes: vec![DEFAULT_NOTE.to_string()],
        });
        if entry.notes.is_empty() {
            entry.notes.push(DEFAULT_NOTE.to_string());
        }

        let target_note = match entry.title_rule.as_ref().and_then(|r| r.extract(&info.title)) {
            Some(sub) => {
                let sub = storage::sanitize_note_name(&sub);
                if !entry.notes.contains(&sub) {
                    entry.notes.push(sub.clone());
                }
                sub
            }
            None => entry
                .notes
                .first()
                .cloned()
                .unwrap_or_else(|| DEFAULT_NOTE.to_string()),
        };

        let display = entry.display_name.clone();
        let _ = self.index.save();
        self.switch_to(folder, display, target_note);
    }

    fn drain_foreground(&mut self) {
        while let Ok(info) = self.fg_rx.try_recv() {
            self.handle_foreground_change(info);
        }
    }

    fn drain_hotkey(&mut self, ctx: &egui::Context) {
        let mut presses = 0;
        while let Ok(()) = self.hotkey_rx.try_recv() {
            presses += 1;
        }
        if presses == 0 {
            return;
        }
        // 每次奇数次按下翻转一次（多次累计在一帧内的话相互抵消）
        for _ in 0..presses {
            self.toggle_hidden(ctx);
        }
    }

    fn drain_tray(&mut self, ctx: &egui::Context) {
        let Some(rx) = self.tray_rx.as_ref() else {
            return;
        };
        let mut actions = Vec::new();
        while let Ok(a) = rx.try_recv() {
            actions.push(a);
        }
        for a in actions {
            match a {
                crate::tray::TrayAction::Toggle => self.toggle_hidden(ctx),
                crate::tray::TrayAction::Quit => {
                    self.save_current();
                    let _ = self.cfg.save();
                    let _ = self.index.save();
                    ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                }
            }
        }
    }

    fn toggle_hidden(&mut self, ctx: &egui::Context) {
        if let Some(pos) = self.hidden_pos.take() {
            // 显示：移回原位 + 取得焦点
            ctx.send_viewport_cmd(egui::ViewportCommand::OuterPosition(pos));
            ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
            ctx.send_viewport_cmd(egui::ViewportCommand::WindowLevel(
                if self.cfg.always_on_top {
                    egui::WindowLevel::AlwaysOnTop
                } else {
                    egui::WindowLevel::Normal
                },
            ));
        } else {
            // 隐藏：记录当前位置 → 移到屏幕外。窗口对系统仍是可见的，
            // eframe 持续收到事件，下次按热键能可靠地把它移回来。
            let cur = ctx.input(|i| {
                i.viewport().outer_rect.map(|r| r.min)
            });
            if let Some(p) = cur {
                self.hidden_pos = Some(p);
            }
            ctx.send_viewport_cmd(egui::ViewportCommand::OuterPosition(egui::pos2(
                -32000.0, -32000.0,
            )));
        }
    }

    fn autosave_tick(&mut self) {
        if !self.dirty {
            return;
        }
        if let Some(t) = self.last_edit {
            if t.elapsed().as_millis() as u64 >= self.cfg.autosave_delay_ms {
                self.save_current();
            }
        }
    }

    fn maybe_reapply_theme(&mut self, ctx: &egui::Context) {
        if self.cfg.theme_mode != self.last_applied_theme
            || (self.cfg.font_size - self.last_applied_font).abs() > 0.01
        {
            theme::apply(ctx, self.cfg.theme_mode, self.cfg.font_size);
            self.last_applied_theme = self.cfg.theme_mode;
            self.last_applied_font = self.cfg.font_size;
            self.settings_theme_done = false;
        }
        if self.cfg.ui_fonts != self.last_applied_ui_fonts
            || self.cfg.editor_fonts != self.last_applied_editor_fonts
        {
            fonts::install_with_lists(ctx, &self.cfg.ui_fonts, &self.cfg.editor_fonts);
            self.last_applied_ui_fonts = self.cfg.ui_fonts.clone();
            self.last_applied_editor_fonts = self.cfg.editor_fonts.clone();
            self.settings_fonts_done = false;
        }
    }

    fn capture_hwnd(&mut self, frame: &eframe::Frame) {
        if self.hwnd_raw.is_some() {
            return;
        }
        use raw_window_handle::{HasWindowHandle, RawWindowHandle};
        if let Ok(handle) = frame.window_handle() {
            if let RawWindowHandle::Win32(w) = handle.as_raw() {
                self.hwnd_raw = Some(w.hwnd.get());
            }
        }
    }


    fn delete_current_note(&mut self) {
        if self.folder_key == GLOBAL_FOLDER && self.note_name == SCRATCH_NOTE {
            return;
        }
        let p = storage::note_path(&self.folder_key, &self.note_name);
        let _ = std::fs::remove_file(p);
        if let Some(entry) = self.index.apps.get_mut(&self.folder_key) {
            entry.notes.retain(|n| n != &self.note_name);
            if entry.notes.is_empty() {
                entry.notes.push(DEFAULT_NOTE.to_string());
            }
        }
        let _ = self.index.save();
        let next = self
            .index
            .apps
            .get(&self.folder_key)
            .and_then(|e| e.notes.first().cloned())
            .unwrap_or_else(|| DEFAULT_NOTE.to_string());
        let folder = self.folder_key.clone();
        let display = self.display_name.clone();
        self.dirty = false;
        self.switch_to(folder, display, next);
    }

    fn update_title_state(&mut self, ctx: &egui::Context) {
        if self.title_first_frame {
            self.title_first_frame = false;
            return;
        }

        let maxed = ctx.input(|i| i.viewport().maximized.unwrap_or(false));
        let pointer_held = ctx.input(|i| i.pointer.any_down());
        let mouse_in = ctx.input(|i| i.pointer.hover_pos().is_some());
        // 编辑器（或任何控件）正在被聚焦 → 保留标题栏，避免输入中段途切换
        let any_focus = ctx.memory(|m| m.focused().is_some());

        let want = if !self.cfg.autohide_title_bar || maxed || pointer_held || any_focus {
            true
        } else {
            mouse_in
        };

        // 滞后：show 立即；hide 等 220ms，避免鼠标短暂出界就缩窗口闪烁
        let now = Instant::now();
        if want != self.title_visible {
            if self.title_pending_target != Some(want) {
                self.title_pending_target = Some(want);
                self.title_pending_since = Some(now);
            }
            let elapsed = self
                .title_pending_since
                .map(|t| now.duration_since(t).as_millis())
                .unwrap_or(0);
            let needed = if want { 0 } else { 220 };
            if elapsed < needed {
                ctx.request_repaint_after(std::time::Duration::from_millis(
                    (needed - elapsed) as u64,
                ));
                return;
            }
        } else {
            self.title_pending_target = None;
            self.title_pending_since = None;
            return;
        }

        let Some(outer) = ctx.input(|i| i.viewport().outer_rect) else {
            return;
        };
        let delta = if want { TITLE_BAR_HEIGHT } else { -TITLE_BAR_HEIGHT };
        let new_y = outer.min.y - delta;
        let new_h = outer.size().y + delta;
        ctx.send_viewport_cmd(egui::ViewportCommand::InnerSize(egui::vec2(
            outer.width(),
            new_h,
        )));
        ctx.send_viewport_cmd(egui::ViewportCommand::OuterPosition(egui::pos2(
            outer.min.x, new_y,
        )));
        self.title_visible = want;
        self.title_pending_target = None;
        self.title_pending_since = None;
    }

    fn handle_keys(&mut self, ctx: &egui::Context) {
        let save_now = ctx.input(|i| i.modifiers.ctrl && i.key_pressed(egui::Key::S));
        if save_now {
            self.save_current();
        }
    }

    fn draw_main_frame(&mut self, ctx: &egui::Context) {
        let p = palette(self.cfg.theme_mode);
        let frame = egui::Frame {
            fill: p.bg,
            stroke: egui::Stroke::new(1.0, p.stroke),
            rounding: egui::Rounding::ZERO,
            inner_margin: egui::Margin::ZERO,
            outer_margin: egui::Margin::ZERO,
            ..Default::default()
        };

        egui::CentralPanel::default().frame(frame).show(ctx, |ui| {
            let full = ui.max_rect();

            // 标题栏（条件显示）：包含 ✕ □ — 与拖拽区
            let title_h = if self.title_visible { TITLE_BAR_HEIGHT } else { 0.0 };
            if self.title_visible {
                let title_rect = egui::Rect::from_min_max(
                    full.min,
                    egui::pos2(full.right(), full.top() + TITLE_BAR_HEIGHT),
                );
                chrome::draw_title_bar(
                    ctx,
                    ui,
                    title_rect,
                    TitleBarConfig {
                        title: "NxNote",
                        show_min_max: true,
                        mode: self.cfg.theme_mode,
                    },
                );
            }

            // 工具栏（始终在 title 下方；title 隐藏时即位于窗口顶部）
            let tool_h = 30.0;
            let tool_rect = egui::Rect::from_min_max(
                egui::pos2(full.left(), full.top() + title_h),
                egui::pos2(full.right(), full.top() + title_h + tool_h),
            );
            let mut tool_ui = ui.new_child(
                egui::UiBuilder::new()
                    .max_rect(tool_rect.shrink2(egui::vec2(6.0, 4.0)))
                    .layout(egui::Layout::left_to_right(egui::Align::Center)),
            );
            self.draw_tool_bar(&mut tool_ui);

            // 状态栏（painter 直绘，避免 horizontal 嵌套换行）
            let status_h = 22.0;
            let status_rect = egui::Rect::from_min_max(
                egui::pos2(full.left(), full.bottom() - status_h),
                full.max,
            );
            self.draw_status_bar_at(ui, status_rect);

            // 中部
            let content_rect = egui::Rect::from_min_max(
                egui::pos2(full.left(), tool_rect.bottom()),
                egui::pos2(full.right(), status_rect.top()),
            )
            .shrink2(egui::vec2(6.0, 4.0));
            let mut content_ui = ui.new_child(
                egui::UiBuilder::new()
                    .max_rect(content_rect)
                    .layout(egui::Layout::top_down(egui::Align::LEFT)),
            );
            self.draw_central(&mut content_ui);

            self.draw_modals(ctx);
            chrome::draw_resize_handles(ctx, ui);
        });
    }

    fn draw_tool_bar(&mut self, ui: &mut egui::Ui) {
        // 钉住
        let pin_glyph = if self.pinned { icons::PIN } else { icons::PIN_OFF };
        if icon_btn(ui, pin_glyph, "钉住当前笔记", self.pinned).clicked() {
            self.pinned = !self.pinned;
        }

        // 应用名 + 笔记下拉（按可用宽度截断，真实测量）
        let label_text = format!("{} / {}", self.display_name, self.note_name);
        let right_reserved = 40.0; // 仅 ⚙
        let avail = (ui.available_width() - right_reserved).max(48.0);
        let truncated = truncate_to_fit(
            ui,
            &label_text,
            avail - 12.0,
            egui::FontId::proportional(13.0),
        );
        let label = egui::RichText::new(truncated).size(13.0);
        ui.menu_button(label, |ui| {
            ui.set_min_width(160.0);
            ui.set_max_width(190.0);
            ui.spacing_mut().item_spacing.y = 2.0;

            // 所有菜单项放进同一 ScrollArea，永不溢出屏幕
            egui::ScrollArea::vertical()
                .id_salt("nx_menu")
                .max_height(280.0)
                .show(ui, |ui| {
                    ui.label(egui::RichText::new("当前应用的笔记").weak().small());
                    let notes = self
                        .index
                        .apps
                        .get(&self.folder_key)
                        .map(|e| e.notes.clone())
                        .unwrap_or_else(|| vec![self.note_name.clone()]);
                    let note_font = egui::FontId::proportional(12.5);
                    let max_item_w = 150.0;
                    for n in &notes {
                        let display = truncate_to_fit(ui, n, max_item_w, note_font.clone());
                        if menu_item(ui, icons::DESCRIPTION, &display, n == &self.note_name)
                            .clicked()
                        {
                            let folder = self.folder_key.clone();
                            let display_name = self.display_name.clone();
                            self.switch_to(folder, display_name, n.clone());
                            ui.close_menu();
                        }
                    }
                    ui.separator();
                    if menu_item(ui, icons::ADD, "新建笔记", false).clicked() {
                        self.modal = Modal::NewNote { input: String::new() };
                        ui.close_menu();
                    }
                    if menu_item(ui, icons::EDIT, "重命名", false).clicked() {
                        self.modal = Modal::Rename {
                            input: self.note_name.clone(),
                            old: self.note_name.clone(),
                        };
                        ui.close_menu();
                    }
                    if menu_item(ui, icons::DELETE, "删除当前笔记", false).clicked() {
                        self.delete_current_note();
                        ui.close_menu();
                    }
                    ui.separator();
                    if let Some(fg) = &self.fg {
                        if menu_item(ui, icons::TARGET, "学习标题规则…", false)
                            .on_hover_text("从窗口标题提取项目名")
                            .clicked()
                        {
                            self.modal = Modal::TitleLearn { title: fg.title.clone() };
                            ui.close_menu();
                        }
                        let fg_name = fg
                            .exe_path
                            .file_name()
                            .and_then(|s| s.to_str())
                            .unwrap_or("?")
                            .to_string();
                        let fg_path = fg.exe_path.to_string_lossy().to_string();
                        let already = self.cfg.blocked_apps.iter().any(|b| {
                            let l = b.to_lowercase();
                            l == fg_name.to_lowercase() || l == fg_path.to_lowercase()
                        });
                        if !already {
                            // 名字也要按宽度截断
                            let pretty = truncate_to_fit(
                                ui,
                                &format!("拉黑「{fg_name}」"),
                                150.0,
                                egui::FontId::proportional(12.5),
                            );
                            if menu_item(ui, icons::DELETE, &pretty, false)
                                .on_hover_text("加入应用黑名单（命中后落回速记本）")
                                .clicked()
                            {
                                self.cfg.blocked_apps.push(fg_name.clone());
                                let _ = self.cfg.save();
                                ui.close_menu();
                            }
                        }
                    }
                    if menu_item(ui, icons::FOLDER, "所有应用…", false).clicked() {
                        self.modal = Modal::NotesList;
                        ui.close_menu();
                    }
                });
        });

        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if icon_btn(ui, icons::SETTINGS, "设置", false).clicked() {
                self.settings_open = true;
                self.settings_fonts_done = false;
                self.settings_theme_done = false;
                self.settings_pos_applied = false;
            }
        });
    }

    fn draw_status_bar_at(&self, ui: &egui::Ui, rect: egui::Rect) {
        let p = palette(self.cfg.theme_mode);
        let font = egui::FontId::proportional(11.0);
        let painter = ui.painter_at(rect);
        let pad = 8.0;
        let total_w = (rect.width() - pad * 2.0).max(0.0);
        if total_w < 20.0 {
            return;
        }
        // 左侧：脏标 + 笔记名（最多占 55%）
        let dot = if self.dirty { "●" } else { "○" };
        let left_text = format!("{} {}", dot, self.note_name);
        let left_max = (total_w * 0.55 - 4.0).max(20.0);
        let left = truncate_to_fit(ui, &left_text, left_max, font.clone());
        let left_size = ui.fonts(|f| {
            f.layout_no_wrap(left.clone(), font.clone(), egui::Color32::PLACEHOLDER)
                .size()
        });
        painter.text(
            egui::pos2(rect.left() + pad, rect.center().y),
            egui::Align2::LEFT_CENTER,
            left,
            font.clone(),
            p.text_weak,
        );

        // 右侧：前台标题（剩余宽度，留 8px 间隔）
        let used = left_size.x + 8.0;
        let right_max = total_w - used - 4.0;
        if right_max < 24.0 {
            return;
        }
        if let Some(fg) = &self.fg {
            let right = truncate_to_fit(ui, &fg.title, right_max, font.clone());
            painter.text(
                egui::pos2(rect.right() - pad, rect.center().y),
                egui::Align2::RIGHT_CENTER,
                right,
                font,
                p.text_weak,
            );
        }
    }

    fn draw_central(&mut self, ui: &mut egui::Ui) {
        self.draw_editor(ui);
    }

    fn draw_editor(&mut self, ui: &mut egui::Ui) {
        let p = palette(self.cfg.theme_mode);
        let mut caret_target: Option<(f32, f32)> = None;

        egui::ScrollArea::vertical().show(ui, |ui| {
            let avail = ui.available_size_before_wrap();
            let line_count = self.editor_text.lines().count().max(1)
                + if self.editor_text.ends_with('\n') { 1 } else { 0 };
            let gutter_chars = line_count.to_string().len().max(2);
            let gutter_width = (gutter_chars as f32) * 8.0 + 10.0;

            ui.horizontal_top(|ui| {
                ui.spacing_mut().item_spacing.x = 4.0;

                // 行号列
                let mut numbers = String::new();
                for i in 1..=line_count {
                    numbers.push_str(&format!("{:>width$}\n", i, width = gutter_chars));
                }
                let gutter = egui::RichText::new(numbers)
                    .monospace()
                    .color(p.text_weak)
                    .size(self.cfg.font_size);
                ui.add_sized(
                    egui::vec2(gutter_width, 0.0),
                    egui::Label::new(gutter).wrap_mode(egui::TextWrapMode::Extend),
                );

                // 分隔线
                let sep_rect = egui::Rect::from_min_size(
                    ui.cursor().left_top(),
                    egui::vec2(1.0, ui.available_height().max(avail.y)),
                );
                ui.painter().rect_filled(sep_rect, 0.0, p.stroke);
                ui.add_space(6.0);

                // 编辑器主体（带 inline markdown 高亮）
                let editor_w = (avail.x - gutter_width - 12.0).max(40.0);
                let editor_h = avail.y.max(60.0);
                let theme_mode = self.cfg.theme_mode;
                let base_size = self.cfg.font_size;
                let cursor_line = self.editor_cursor_line;
                let mut layouter = move |ui: &egui::Ui, text: &str, wrap_width: f32| -> std::sync::Arc<egui::Galley> {
                    let styles = md_highlight::Styles {
                        p: palette(theme_mode),
                        base: base_size,
                        cursor_line,
                    };
                    let mut job = md_highlight::build(text, styles);
                    job.wrap.max_width = wrap_width;
                    ui.fonts(|f| f.layout_job(job))
                };
                let edit_output = egui::TextEdit::multiline(&mut self.editor_text)
                    .desired_width(editor_w)
                    .min_size(egui::vec2(editor_w, editor_h))
                    .frame(false)
                    .layouter(&mut layouter)
                    .show(ui);

                let resp = edit_output.response;
                if resp.changed() {
                    self.dirty = true;
                    self.last_edit = Some(Instant::now());
                }

                if resp.has_focus() {
                    if let Some(range) = edit_output.cursor_range {
                        // 给 layouter 下一帧用：当前光标所在段（=行）
                        self.editor_cursor_line = Some(range.primary.pcursor.paragraph);
                        let crect = edit_output.galley.pos_from_cursor(&range.primary);
                        let x = edit_output.galley_pos.x + crect.left();
                        let y_top = edit_output.galley_pos.y + crect.top();
                        let y_bottom = edit_output.galley_pos.y + crect.bottom();
                        caret_target = Some((x, y_top));
                        let line_h = (y_bottom - y_top).max(self.cfg.font_size * 1.4);
                        // 关键：覆盖 egui::TextEdit 默认输出的 IMEOutput。
                        // egui-winit 会把 rect 当作 IME 区域传给 winit::set_ime_cursor_area，
                        // 该 API 用 CFS_EXCLUDE 让候选窗"避开"这块区域。
                        // 默认 rect = 整个编辑器 → 候选窗被推到屏幕底。
                        // 我们改成 1×line_h 的小矩形紧贴光标 → 候选窗就紧贴光标下方。
                        let cursor_small = egui::Rect::from_min_size(
                            egui::pos2(x, y_top),
                            egui::vec2(1.0, line_h),
                        );
                        ui.ctx().output_mut(|o| {
                            o.ime = Some(egui::output::IMEOutput {
                                rect: cursor_small,
                                cursor_rect: cursor_small,
                            });
                        });
                    }
                }
                let _ = caret_target;
            });
        });
    }

    fn draw_modals(&mut self, ctx: &egui::Context) {
        let mut close = false;
        match self.modal.clone() {
            Modal::None => {}
            Modal::TitleLearn { title } => {
                egui::Window::new("学习标题规则")
                    .collapsible(false)
                    .resizable(false)
                    .anchor(egui::Align2::CENTER_CENTER, egui::vec2(0.0, 0.0))
                    .show(ctx, |ui| {
                        ui.label("点击属于「项目名」的那一段：");
                        ui.label(egui::RichText::new(&title).italics().small());
                        ui.separator();
                        let parts: Vec<&str> = title.split(" - ").collect();
                        let mut clicked_idx: Option<usize> = None;
                        ui.horizontal_wrapped(|ui| {
                            for (i, p) in parts.iter().enumerate() {
                                if ui.button(*p).clicked() {
                                    clicked_idx = Some(i);
                                }
                            }
                        });
                        if let Some(i) = clicked_idx {
                            let sub = storage::sanitize_note_name(parts[i]);
                            if let Some(entry) = self.index.apps.get_mut(&self.folder_key) {
                                entry.title_rule = Some(TitleRule::SplitIndex {
                                    sep: " - ".to_string(),
                                    index: i,
                                });
                                if !entry.notes.contains(&sub) {
                                    entry.notes.push(sub.clone());
                                }
                            }
                            let _ = self.index.save();
                            let folder = self.folder_key.clone();
                            let display = self.display_name.clone();
                            self.switch_to(folder, display, sub);
                            close = true;
                        }
                        ui.separator();
                        ui.horizontal(|ui| {
                            if ui.button("清除规则").clicked() {
                                if let Some(entry) = self.index.apps.get_mut(&self.folder_key) {
                                    entry.title_rule = None;
                                }
                                let _ = self.index.save();
                                close = true;
                            }
                            if ui.button("取消").clicked() {
                                close = true;
                            }
                        });
                    });
            }
            Modal::NewNote { mut input } => {
                egui::Window::new("新建笔记")
                    .collapsible(false)
                    .resizable(false)
                    .anchor(egui::Align2::CENTER_CENTER, egui::vec2(0.0, 0.0))
                    .show(ctx, |ui| {
                        ui.label("笔记名");
                        let resp = ui.text_edit_singleline(&mut input);
                        resp.request_focus();
                        ui.horizontal(|ui| {
                            let confirm = ui.button("创建").clicked()
                                || ui.input(|i| i.key_pressed(egui::Key::Enter));
                            if confirm {
                                let name = storage::sanitize_note_name(&input);
                                if let Some(entry) = self.index.apps.get_mut(&self.folder_key) {
                                    if !entry.notes.contains(&name) {
                                        entry.notes.push(name.clone());
                                    }
                                } else {
                                    self.index.apps.insert(
                                        self.folder_key.clone(),
                                        AppEntry {
                                            exe_path: String::new(),
                                            display_name: self.display_name.clone(),
                                            title_rule: None,
                                            notes: vec![name.clone()],
                                        },
                                    );
                                }
                                let _ = self.index.save();
                                let folder = self.folder_key.clone();
                                let display = self.display_name.clone();
                                self.switch_to(folder, display, name);
                                close = true;
                            }
                            if ui.button("取消").clicked() {
                                close = true;
                            }
                        });
                    });
                if !close {
                    self.modal = Modal::NewNote { input };
                }
            }
            Modal::Rename { mut input, old } => {
                egui::Window::new("重命名笔记")
                    .collapsible(false)
                    .resizable(false)
                    .anchor(egui::Align2::CENTER_CENTER, egui::vec2(0.0, 0.0))
                    .show(ctx, |ui| {
                        ui.label("新名称");
                        ui.text_edit_singleline(&mut input);
                        ui.horizontal(|ui| {
                            let confirm = ui.button("确认").clicked()
                                || ui.input(|i| i.key_pressed(egui::Key::Enter));
                            if confirm {
                                let new_name = storage::sanitize_note_name(&input);
                                if new_name != old {
                                    self.save_current();
                                    let from = storage::note_path(&self.folder_key, &old);
                                    let to = storage::note_path(&self.folder_key, &new_name);
                                    let _ = std::fs::rename(&from, &to);
                                    if let Some(entry) = self.index.apps.get_mut(&self.folder_key) {
                                        for n in entry.notes.iter_mut() {
                                            if n == &old {
                                                *n = new_name.clone();
                                            }
                                        }
                                    }
                                    let _ = self.index.save();
                                    let folder = self.folder_key.clone();
                                    let display = self.display_name.clone();
                                    self.note_name = new_name.clone();
                                    self.switch_to(folder, display, new_name);
                                }
                                close = true;
                            }
                            if ui.button("取消").clicked() {
                                close = true;
                            }
                        });
                    });
                if !close {
                    self.modal = Modal::Rename { input, old };
                }
            }
            Modal::NotesList => {
                egui::Window::new("所有应用与笔记")
                    .collapsible(false)
                    .resizable(true)
                    .default_size([320.0, 360.0])
                    .anchor(egui::Align2::CENTER_CENTER, egui::vec2(0.0, 0.0))
                    .show(ctx, |ui| {
                        let mut jump: Option<(String, String, String)> = None;
                        egui::ScrollArea::vertical().show(ui, |ui| {
                            ui.collapsing("📝 速记 (未绑定)", |ui| {
                                if ui.button("scratch").clicked() {
                                    jump = Some((
                                        GLOBAL_FOLDER.to_string(),
                                        "速记".to_string(),
                                        SCRATCH_NOTE.to_string(),
                                    ));
                                }
                            });
                            let apps: Vec<(String, AppEntry)> = self
                                .index
                                .apps
                                .iter()
                                .map(|(k, v)| (k.clone(), v.clone()))
                                .collect();
                            for (folder, entry) in apps {
                                ui.collapsing(format!("🪟 {}", entry.display_name), |ui| {
                                    ui.label(
                                        egui::RichText::new(&entry.exe_path).weak().small(),
                                    );
                                    for n in &entry.notes {
                                        if ui.button(n).clicked() {
                                            jump = Some((
                                                folder.clone(),
                                                entry.display_name.clone(),
                                                n.clone(),
                                            ));
                                        }
                                    }
                                });
                            }
                        });
                        if let Some((f, d, n)) = jump {
                            self.pinned = true;
                            self.switch_to(f, d, n);
                            close = true;
                        }
                        if ui.button("关闭").clicked() {
                            close = true;
                        }
                    });
            }
        }
        if close {
            self.modal = Modal::None;
        }
    }

    fn draw_settings_viewport(&mut self, ctx: &egui::Context) {
        if !self.settings_open {
            return;
        }
        let cfg = &mut self.cfg;
        let fonts_done = &mut self.settings_fonts_done;
        let theme_done = &mut self.settings_theme_done;
        let cfg_dirty = &mut self.cfg_dirty;
        let mut should_close = false;

        let size = egui::vec2(640.0, 460.0);
        let mut builder = egui::ViewportBuilder::default()
            .with_title("NxNote 设置")
            .with_inner_size(size)
            .with_min_inner_size([560.0, 380.0])
            .with_decorations(false)
            .with_resizable(true);

        if !self.settings_pos_applied {
            let monitor = ctx
                .input(|i| i.viewport().monitor_size)
                .unwrap_or(egui::vec2(1920.0, 1080.0));
            let pos = egui::pos2(
                ((monitor.x - size.x) * 0.5).max(0.0),
                ((monitor.y - size.y) * 0.5).max(0.0),
            );
            builder = builder.with_position(pos);
            self.settings_pos_applied = true;
        }

        ctx.show_viewport_immediate(
            egui::ViewportId::from_hash_of("nx_settings"),
            builder,
            |sctx, _| {
                if !*fonts_done {
                    crate::fonts::install_with_lists(sctx, &cfg.ui_fonts, &cfg.editor_fonts);
                    *fonts_done = true;
                }
                if !*theme_done {
                    crate::theme::apply(sctx, cfg.theme_mode, cfg.font_size);
                    *theme_done = true;
                }
                if sctx.input(|i| i.viewport().close_requested()) {
                    should_close = true;
                }
                let before = serde_json::to_string(cfg).unwrap_or_default();
                settings_ui::draw_settings_window(sctx, cfg);
                let after = serde_json::to_string(cfg).unwrap_or_default();
                if before != after {
                    *cfg_dirty = true;
                    *theme_done = false;
                }
            },
        );

        if should_close {
            self.settings_open = false;
            self.settings_fonts_done = false;
            self.settings_theme_done = false;
            self.settings_pos_applied = false;
            if self.cfg_dirty {
                let _ = self.cfg.save();
                self.cfg_dirty = false;
                // 应用置顶切换
                ctx.send_viewport_cmd(egui::ViewportCommand::WindowLevel(
                    if self.cfg.always_on_top {
                        egui::WindowLevel::AlwaysOnTop
                    } else {
                        egui::WindowLevel::Normal
                    },
                ));
                // 热键变化则重新绑定
                if self.cfg.hotkey != self.last_bound_hotkey {
                    self._hotkey = None;
                    self._hotkey = hotkey::install(&self.cfg.hotkey);
                    self.last_bound_hotkey = self.cfg.hotkey.clone();
                }
            }
        }
    }
}

impl eframe::App for NxNoteApp {
    fn update(&mut self, ctx: &egui::Context, frame: &mut eframe::Frame) {
        self.capture_hwnd(frame);
        self.drain_foreground();
        self.drain_hotkey(ctx);
        self.drain_tray(ctx);
        self.autosave_tick();
        self.handle_keys(ctx);
        self.maybe_reapply_theme(ctx);
        self.update_title_state(ctx);

        self.draw_main_frame(ctx);
        self.draw_settings_viewport(ctx);

        // 编辑器聚焦时连续重绘，最小化输入延迟
        let focused = ctx.memory(|m| m.focused().is_some());
        if focused {
            ctx.request_repaint();
        } else if self.dirty {
            ctx.request_repaint_after(std::time::Duration::from_millis(300));
        }
    }

    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        self.save_current();
        let _ = self.index.save();
        let _ = self.cfg.save();
    }
}

/// 用 egui 真实字体度量做二分截断；对中英文混排都准确。
/// 黑名单匹配：宽松匹配，支持完整路径 / 带扩展名文件 / 裸文件名 / 路径子串。
/// 全部不区分大小写。
fn app_blocked(blocked: &[String], exe: &std::path::Path) -> bool {
    if blocked.is_empty() {
        return false;
    }
    let full = exe.to_string_lossy().replace('/', "\\").to_lowercase();
    let file_name = exe
        .file_name()
        .and_then(|s| s.to_str())
        .map(|s| s.to_lowercase())
        .unwrap_or_default();
    let stem = exe
        .file_stem()
        .and_then(|s| s.to_str())
        .map(|s| s.to_lowercase())
        .unwrap_or_default();
    blocked.iter().any(|raw| {
        let b = raw.trim().to_lowercase().replace('/', "\\");
        if b.is_empty() {
            return false;
        }
        // 1) 用户输入的完整或末段路径
        if full == b || full.ends_with(&format!("\\{b}")) {
            return true;
        }
        // 2) 文件名（带扩展名）匹配
        if file_name == b {
            return true;
        }
        // 3) 裸名（不带扩展名）匹配
        if stem == b {
            return true;
        }
        // 4) 用户写了 xxx.exe 但实际 stem 是 xxx
        if let Some(b_stem) = b.strip_suffix(".exe") {
            if stem == b_stem {
                return true;
            }
        }
        // 5) 退化为子串匹配，但只对 >=3 字符的非通用关键词
        if b.len() >= 3 && full.contains(&b) {
            return true;
        }
        false
    })
}

fn truncate_to_fit(ui: &egui::Ui, text: &str, max_w: f32, font_id: egui::FontId) -> String {
    if text.is_empty() || max_w <= 0.0 {
        return String::new();
    }
    let measure = |s: &str| -> f32 {
        ui.fonts(|f| {
            f.layout_no_wrap(s.to_string(), font_id.clone(), egui::Color32::PLACEHOLDER)
                .size()
                .x
        })
    };
    if measure(text) <= max_w {
        return text.to_string();
    }
    let ellipsis_w = measure("…");
    let target = (max_w - ellipsis_w).max(0.0);
    if target <= 0.0 {
        return "…".to_string();
    }
    let chars: Vec<char> = text.chars().collect();
    // 二分：找到最大的 n 使 chars[..n] 宽度 <= target
    let mut lo = 0usize;
    let mut hi = chars.len();
    while lo < hi {
        let mid = (lo + hi + 1) / 2;
        let s: String = chars[..mid].iter().collect();
        if measure(&s) <= target {
            lo = mid;
        } else {
            hi = mid - 1;
        }
    }
    let mut out: String = chars[..lo].iter().collect();
    out.push('…');
    out
}

fn icon_btn(ui: &mut egui::Ui, glyph: &'static str, hint: &str, selected: bool) -> egui::Response {
    let txt = icons::rich(glyph, 16.0);
    let resp = ui.add(egui::SelectableLabel::new(selected, txt));
    resp.on_hover_text(hint)
}

fn menu_item(
    ui: &mut egui::Ui,
    glyph: &'static str,
    label: &str,
    selected: bool,
) -> egui::Response {
    use egui::text::LayoutJob;
    let color = ui.visuals().text_color();
    let mut job = LayoutJob::default();
    job.append(
        glyph,
        0.0,
        egui::TextFormat {
            font_id: icons::font(13.0),
            color,
            valign: egui::Align::Center,
            ..Default::default()
        },
    );
    job.append(
        &format!("  {}", label),
        0.0,
        egui::TextFormat {
            font_id: egui::FontId::proportional(12.5),
            color,
            valign: egui::Align::Center,
            ..Default::default()
        },
    );
    let full = ui.available_width();
    ui.add_sized(
        egui::vec2(full, 20.0),
        egui::SelectableLabel::new(selected, job),
    )
}
