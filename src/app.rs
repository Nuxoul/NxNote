use std::sync::atomic::{AtomicBool, AtomicIsize, Ordering};
use std::sync::mpsc::Receiver;
use std::time::Instant;

use crate::chrome::{self, TitleBarConfig, TITLE_BAR_HEIGHT};

/// 全局主窗口 HWND，后台线程能拿到它直接调 Win32。
pub static MAIN_HWND: AtomicIsize = AtomicIsize::new(0);
/// 是否处于"最小化到托盘"隐藏态。所有线程共享，是窗口可见性的 source of truth。
pub static MAIN_HIDDEN: AtomicBool = AtomicBool::new(false);

/// 给主窗口塞一条 WM_NULL，让 winit GetMessage 立刻返回 → eframe 跑一帧。
#[cfg(windows)]
pub fn wake_event_loop() {
    let hwnd = MAIN_HWND.load(Ordering::Acquire);
    if hwnd == 0 {
        return;
    }
    unsafe {
        use windows::Win32::Foundation::{HWND, LPARAM, WPARAM};
        use windows::Win32::UI::WindowsAndMessaging::{PostMessageW, WM_NULL};
        let h = HWND(hwnd as *mut std::ffi::c_void);
        let _ = PostMessageW(h, WM_NULL, WPARAM(0), LPARAM(0));
    }
}

#[cfg(not(windows))]
pub fn wake_event_loop() {}

/// 任意线程都可调：把主窗口拽出来 + 抢前台 + 通知 eframe 跑一帧同步 self.hidden。
#[cfg(windows)]
pub fn force_show() {
    let hwnd = MAIN_HWND.load(Ordering::Acquire);
    if hwnd == 0 {
        return;
    }
    unsafe {
        use windows::Win32::Foundation::HWND;
        use windows::Win32::UI::WindowsAndMessaging::{
            SetForegroundWindow, ShowWindow, SW_SHOW,
        };
        let h = HWND(hwnd as *mut std::ffi::c_void);
        let _ = ShowWindow(h, SW_SHOW);
        let _ = SetForegroundWindow(h);
    }
    MAIN_HIDDEN.store(false, Ordering::Release);
    wake_event_loop();
}

#[cfg(windows)]
pub fn force_hide() {
    let hwnd = MAIN_HWND.load(Ordering::Acquire);
    if hwnd == 0 {
        return;
    }
    unsafe {
        use windows::Win32::Foundation::HWND;
        use windows::Win32::UI::WindowsAndMessaging::{ShowWindow, SW_HIDE};
        let h = HWND(hwnd as *mut std::ffi::c_void);
        let _ = ShowWindow(h, SW_HIDE);
    }
    MAIN_HIDDEN.store(true, Ordering::Release);
    wake_event_loop();
}

#[cfg(windows)]
pub fn force_toggle() {
    if MAIN_HIDDEN.load(Ordering::Acquire) {
        force_show();
    } else {
        force_hide();
    }
}

#[cfg(not(windows))]
pub fn force_show() {}
#[cfg(not(windows))]
pub fn force_hide() {}
#[cfg(not(windows))]
pub fn force_toggle() {}
use crate::config::Config;
use crate::fonts;
use crate::hotkey::{self, HotkeyHandle};
use crate::icons;
use crate::md_highlight;
use crate::settings_ui;
use crate::storage::{
    self, AppEntry, AppIndex, TitleRule, DEFAULT_NOTE, GLOBAL_FOLDER, SCRATCH_NOTE,
};
use crate::theme::{self, palette, ThemeMode};
use crate::watcher::{self, ForegroundInfo};

enum PendingEditorAction {
    InsertText(String),
    Backspaces(usize),
}

#[derive(Copy, Clone)]
enum EditorShortcut {
    MoveLineUp,
    MoveLineDown,
    CopyLineUp,
    CopyLineDown,
    DeleteLine,
}

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
    /// 是否已经"最小化到托盘"（窗口对 Windows 不可见，taskbar 上也没图标）
    hidden: bool,
    title_visible: bool,
    title_first_frame: bool,
    title_pending_target: Option<bool>,
    title_pending_since: Option<Instant>,
    /// (line, col) 都是 0-based；显示时 +1
    editor_cursor_pos: Option<(usize, usize)>,
    last_editor_text_len: usize,
    /// 排队到下一帧由 egui 自己注入的事件（列表续行/退出）。
    /// 自己 mutate 文本 + state.cursor.store 在 0.29 不可靠（光标飘到符号前），
    /// 改用 Event::Text / Event::Key(Backspace) 让 TextEdit 自己处理。
    pending_editor_action: Option<PendingEditorAction>,

    settings_open: bool,
    settings_fonts_done: bool,
    settings_theme_done: bool,
    settings_pos_applied: bool,
    color_editor_open: bool,
    color_editor_pos_applied: bool,
    /// 用于 cfg 比对：autostart 变化时同步注册表
    last_applied_autostart: bool,
    /// 托盘菜单点了退出 → 接下来这次 close_requested 不要再被拦回托盘
    force_quit: bool,
    /// 启动参数带 --hidden：n 帧后调 toggle_hidden（等 viewport 拿到 outer_rect）
    pub start_hidden_pending: Option<u8>,
    /// IME 上屏期间需要吃掉的 Enter 帧数 —— 防止输入法回车上屏的同一/紧接
    /// 帧里，TextEdit 也把 Key::Enter 当换行处理
    ime_swallow_enter: u8,
    /// 行级快捷键产生的下一帧光标目标 char idx —— draw_editor 用它覆盖
    /// TextEditState 里的 cursor，从而让 TextEdit 渲染新位置
    pending_cursor_char: Option<usize>,
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
        let (folder_key, display_name, note_name) =
            match (
                index.last_folder_key.clone(),
                index.last_note_name.clone(),
                index.last_display_name.clone(),
            ) {
                (Some(f), Some(n), Some(d)) => {
                    // 保险：folder 必须真的存在，否则退回速记
                    if f == GLOBAL_FOLDER || index.apps.contains_key(&f) {
                        (f, d, n)
                    } else {
                        (
                            GLOBAL_FOLDER.to_string(),
                            "速记".to_string(),
                            SCRATCH_NOTE.to_string(),
                        )
                    }
                }
                _ => (
                    GLOBAL_FOLDER.to_string(),
                    "速记".to_string(),
                    SCRATCH_NOTE.to_string(),
                ),
            };
        let editor_text = storage::load_note(&folder_key, &note_name);

        let last_applied_theme = cfg.theme_mode;
        let last_applied_font = cfg.font_size;
        let last_applied_ui_fonts = cfg.ui_fonts.clone();
        let last_applied_editor_fonts = cfg.editor_fonts.clone();
        let last_bound_hotkey = cfg.hotkey.clone();

        let mut s = Self {
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
            hidden: false,
            title_visible: true,
            title_first_frame: true,
            title_pending_target: None,
            title_pending_since: None,
            editor_cursor_pos: None,
            last_editor_text_len: 0,
            pending_editor_action: None,
            settings_open: false,
            settings_fonts_done: false,
            settings_theme_done: false,
            settings_pos_applied: false,
            color_editor_open: false,
            color_editor_pos_applied: false,
            last_applied_autostart: false, // 在 new 里下面修复
            force_quit: false,
            start_hidden_pending: None,
            ime_swallow_enter: 0,
            pending_cursor_char: None,
        };
        s.last_applied_autostart = s.cfg.autostart;
        s
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
        // 持久化"上次界面"，方便启动时回到这里
        self.index.last_folder_key = Some(self.folder_key.clone());
        self.index.last_note_name = Some(self.note_name.clone());
        self.index.last_display_name = Some(self.display_name.clone());
        let _ = self.index.save();
    }

    fn handle_foreground_change(&mut self, info: ForegroundInfo) {
        self.fg = Some(info.clone());
        if self.pinned {
            return;
        }
        // 自动跟随关闭：只刷新 fg + 索引，不切换当前视图。
        // 仍维护索引让用户能从「所有应用…」里选到这个 app。
        let auto = self.cfg.auto_follow_foreground;

        // 黑名单：仅自动跟随模式下生效（关闭模式下用户自己选择，不需要兜底）
        if auto && app_blocked(&self.cfg.blocked_apps, &info.exe_path) {
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
        if auto {
            self.switch_to(folder, display, target_note);
        }
    }

    fn drain_foreground(&mut self) {
        while let Ok(info) = self.fg_rx.try_recv() {
            self.handle_foreground_change(info);
        }
    }

    fn drain_hotkey(&mut self, ctx: &egui::Context) {
        // 热键 toggle 由后台线程的 force_toggle 直接处理过；这里只
        // 同步 self.hidden 并在刚显示时下发 WindowLevel
        let mut got = false;
        while let Ok(()) = self.hotkey_rx.try_recv() {
            got = true;
        }
        if got && !MAIN_HIDDEN.load(Ordering::Acquire) {
            ctx.send_viewport_cmd(egui::ViewportCommand::WindowLevel(
                if self.cfg.always_on_top {
                    egui::WindowLevel::AlwaysOnTop
                } else {
                    egui::WindowLevel::Normal
                },
            ));
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
                crate::tray::TrayAction::Sync => {
                    // 后台线程已经 Win32 切换过，self.hidden 在 update() 顶部统一同步
                    if !MAIN_HIDDEN.load(Ordering::Acquire) {
                        ctx.send_viewport_cmd(egui::ViewportCommand::WindowLevel(
                            if self.cfg.always_on_top {
                                egui::WindowLevel::AlwaysOnTop
                            } else {
                                egui::WindowLevel::Normal
                            },
                        ));
                    }
                }
                crate::tray::TrayAction::Quit => {
                    self.save_current();
                    let _ = self.cfg.save();
                    let _ = self.index.save();
                    // 已经手动落盘了，直接结束进程 —— eframe Close 在某些
                    // 隐藏/无前台路径下不可靠，硬退出最稳
                    std::process::exit(0);
                }
            }
        }
    }

    fn toggle_hidden(&mut self, ctx: &egui::Context) {
        // 转发到全局函数 —— 它从任意线程都能安全调用，会直接走 Win32 + 同步 atomic
        force_toggle();
        self.hidden = MAIN_HIDDEN.load(Ordering::Acquire);
        if !self.hidden {
            // 恢复后再下发 WindowLevel（隐藏期间可能漂移）
            ctx.send_viewport_cmd(egui::ViewportCommand::WindowLevel(
                if self.cfg.always_on_top {
                    egui::WindowLevel::AlwaysOnTop
                } else {
                    egui::WindowLevel::Normal
                },
            ));
            ctx.request_repaint();
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

    /// 每帧重检：黑名单变更后不需要等下次前台切换就能生效。
    /// 仅自动跟随模式下生效。
    fn enforce_blocklist(&mut self) {
        if self.pinned || !self.cfg.auto_follow_foreground {
            return;
        }
        let Some(fg) = self.fg.clone() else {
            return;
        };
        if !app_blocked(&self.cfg.blocked_apps, &fg.exe_path) {
            return;
        }
        if self.folder_key != GLOBAL_FOLDER {
            self.switch_to(
                GLOBAL_FOLDER.to_string(),
                "速记".to_string(),
                SCRATCH_NOTE.to_string(),
            );
        }
    }

    fn capture_hwnd(&mut self, frame: &eframe::Frame) {
        if self.hwnd_raw.is_some() {
            return;
        }
        use raw_window_handle::{HasWindowHandle, RawWindowHandle};
        if let Ok(handle) = frame.window_handle() {
            if let RawWindowHandle::Win32(w) = handle.as_raw() {
                let hwnd = w.hwnd.get();
                self.hwnd_raw = Some(hwnd);
                MAIN_HWND.store(hwnd, std::sync::atomic::Ordering::Release);
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

        // 滞后：show 立即；hide 等 220ms，避免鼠标短暂出界就抖动
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
            // 不再 resize 窗口：只切内部状态，draw_main_frame 会用 title_h=0
            // 把上方 32px 让给工具栏，从而"标题栏向下位移到窗口后面"
            self.title_visible = want;
            ctx.request_repaint();
        }
        self.title_pending_target = None;
        self.title_pending_since = None;
    }

    fn handle_keys(&mut self, ctx: &egui::Context) {
        let save_now = ctx.input(|i| i.modifiers.ctrl && i.key_pressed(egui::Key::S));
        if save_now {
            self.save_current();
        }
    }

    /// 行级快捷键：Alt+↑/↓ 移动行，Alt+Shift+↑/↓ 复制行，Ctrl+Shift+K 删除当前行
    fn handle_editor_shortcuts(&mut self, ctx: &egui::Context) {
        // 模态打开时，焦点在弹窗的 text_edit，不能误伤主编辑器
        if self.modal != Modal::None {
            return;
        }
        let Some((line, col)) = self.editor_cursor_pos else {
            return;
        };

        let (action, consumed_key) = ctx.input(|i| {
            let m = i.modifiers;
            if m.alt && !m.ctrl {
                if i.key_pressed(egui::Key::ArrowUp) {
                    let a = if m.shift {
                        EditorShortcut::CopyLineUp
                    } else {
                        EditorShortcut::MoveLineUp
                    };
                    return (Some(a), Some(egui::Key::ArrowUp));
                }
                if i.key_pressed(egui::Key::ArrowDown) {
                    let a = if m.shift {
                        EditorShortcut::CopyLineDown
                    } else {
                        EditorShortcut::MoveLineDown
                    };
                    return (Some(a), Some(egui::Key::ArrowDown));
                }
            }
            if m.ctrl && m.shift && !m.alt && i.key_pressed(egui::Key::K) {
                return (Some(EditorShortcut::DeleteLine), Some(egui::Key::K));
            }
            (None, None)
        });
        let Some(action) = action else { return };

        let mut lines: Vec<String> =
            self.editor_text.split('\n').map(String::from).collect();
        let n = line.min(lines.len().saturating_sub(1));

        let new_pos: (usize, usize) = match action {
            EditorShortcut::MoveLineUp => {
                if n == 0 {
                    return;
                }
                lines.swap(n, n - 1);
                (n - 1, col.min(lines[n - 1].chars().count()))
            }
            EditorShortcut::MoveLineDown => {
                if n + 1 >= lines.len() {
                    return;
                }
                lines.swap(n, n + 1);
                (n + 1, col.min(lines[n + 1].chars().count()))
            }
            EditorShortcut::CopyLineUp => {
                let dup = lines[n].clone();
                lines.insert(n, dup);
                // 光标停在新插入的上一行（即旧 n 位置）
                (n, col.min(lines[n].chars().count()))
            }
            EditorShortcut::CopyLineDown => {
                let dup = lines[n].clone();
                lines.insert(n + 1, dup);
                (n + 1, col.min(lines[n + 1].chars().count()))
            }
            EditorShortcut::DeleteLine => {
                if lines.len() == 1 {
                    lines[0].clear();
                    (0, 0)
                } else {
                    lines.remove(n);
                    let nl = n.min(lines.len() - 1);
                    (nl, 0)
                }
            }
        };

        self.editor_text = lines.join("\n");
        self.dirty = true;
        self.last_edit = Some(std::time::Instant::now());
        self.editor_cursor_pos = Some(new_pos);
        self.last_editor_text_len = self.editor_text.len();
        self.pending_cursor_char =
            Some(char_idx_at_line_col(&self.editor_text, new_pos.0, new_pos.1));

        // 把刚消费的按键从本帧 events 里挖掉，避免 TextEdit 再当一次箭头/字符处理
        if let Some(k) = consumed_key {
            ctx.input_mut(|i| {
                i.events.retain(|e| {
                    !matches!(
                        e,
                        egui::Event::Key { key, pressed: true, .. } if *key == k
                    )
                });
            });
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

            // 标题栏（条件显示）：包含 ✕ □ — 置顶 与拖拽区
            let title_h = if self.title_visible { TITLE_BAR_HEIGHT } else { 0.0 };
            if self.title_visible {
                let title_rect = egui::Rect::from_min_max(
                    full.min,
                    egui::pos2(full.right(), full.top() + TITLE_BAR_HEIGHT),
                );
                let out = chrome::draw_title_bar(
                    ctx,
                    ui,
                    title_rect,
                    TitleBarConfig {
                        title: "NxNote",
                        show_min_max: true,
                        mode: self.cfg.theme_mode,
                        on_top: Some(self.cfg.always_on_top),
                    },
                );
                if out.on_top_toggled {
                    self.cfg.always_on_top = !self.cfg.always_on_top;
                    let _ = self.cfg.save();
                    ctx.send_viewport_cmd(egui::ViewportCommand::WindowLevel(
                        if self.cfg.always_on_top {
                            egui::WindowLevel::AlwaysOnTop
                        } else {
                            egui::WindowLevel::Normal
                        },
                    ));
                }
                if out.close_clicked {
                    if self.cfg.close_to_tray {
                        // 隐藏到托盘 —— 文件先存盘，避免下一次启动丢内容
                        self.save_current();
                        self.toggle_hidden(ctx);
                    } else {
                        self.force_quit = true;
                        ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                    }
                }
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
                                // 立刻切回速记本，不等下一次前台切换
                                self.switch_to(
                                    GLOBAL_FOLDER.to_string(),
                                    "速记".to_string(),
                                    SCRATCH_NOTE.to_string(),
                                );
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

        // 左下：光标位置（聚焦时）+ 脏标
        let cursor_part = self
            .editor_cursor_pos
            .map(|(l, c)| format!("行 {} 列 {}", l + 1, c + 1));
        let dirty_part = if self.dirty { Some("●".to_string()) } else { None };
        let left_text = match (cursor_part, dirty_part) {
            (Some(c), Some(d)) => format!("{c}  {d}"),
            (Some(c), None) => c,
            (None, Some(d)) => d,
            (None, None) => String::new(),
        };
        if !left_text.is_empty() {
            let left = truncate_to_fit(ui, &left_text, (total_w * 0.55).max(40.0), font.clone());
            painter.text(
                egui::pos2(rect.left() + pad, rect.center().y),
                egui::Align2::LEFT_CENTER,
                left,
                font.clone(),
                p.text_weak,
            );
        }

        // 右下：字数（按字符计，CJK 一个字 = 1）
        let count = self.editor_text.chars().count();
        let right_text = format!("{} 字", count);
        painter.text(
            egui::pos2(rect.right() - pad, rect.center().y),
            egui::Align2::RIGHT_CENTER,
            right_text,
            font,
            p.text_weak,
        );
    }

    fn draw_central(&mut self, ui: &mut egui::Ui) {
        self.draw_editor(ui);
    }

    fn draw_editor(&mut self, ui: &mut egui::Ui) {
        let p = palette(self.cfg.theme_mode);
        let mut caret_target: Option<(f32, f32)> = None;

        egui::ScrollArea::vertical().show(ui, |ui| {
            let avail = ui.available_size_before_wrap();
            // 逻辑行数 —— 仅用于估算 gutter 宽度。视觉行号由 galley.rows 决定。
            let logical_lines = self.editor_text.lines().count().max(1)
                + if self.editor_text.ends_with('\n') { 1 } else { 0 };
            let gutter_chars = logical_lines.to_string().len().max(2);
            let gutter_width = (gutter_chars as f32) * 8.0 + 10.0;

            ui.horizontal_top(|ui| {
                ui.spacing_mut().item_spacing.x = 4.0;

                // 行号列：先占位，editor 渲染完拿到 galley.rows 后用 painter 补
                let gutter_top_left = ui.cursor().left_top();
                let (_gutter_handle, _) = ui.allocate_exact_size(
                    egui::vec2(gutter_width, 0.0),
                    egui::Sense::hover(),
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
                let cursor_line = self.editor_cursor_pos.map(|(l, _)| l);
                let md_colors = match theme_mode {
                    ThemeMode::Light => self.cfg.md_light.clone(),
                    _ => self.cfg.md_dark.clone(),
                };
                let mut layouter = move |ui: &egui::Ui, text: &str, wrap_width: f32| -> std::sync::Arc<egui::Galley> {
                    let styles = md_highlight::Styles {
                        p: palette(theme_mode),
                        base: base_size,
                        cursor_line,
                        c: &md_colors,
                    };
                    let mut job = md_highlight::build(text, styles);
                    job.wrap.max_width = wrap_width;
                    ui.fonts(|f| f.layout_job(job))
                };
                let editor_id_salt = "nx_editor_main";
                // TextEdit 内部用的真实 id = ui.make_persistent_id(Id::new(salt))
                // —— 先 hash 成 Id 再 hash，跟直接 ui.make_persistent_id(salt) 不一样
                let editor_id =
                    ui.make_persistent_id(egui::Id::new(editor_id_salt));

                // 行级快捷键产生的目标 cursor：在 show 之前覆盖 state
                if let Some(target) = self.pending_cursor_char.take() {
                    use egui::text::{CCursor, CCursorRange};
                    if let Some(mut state) =
                        egui::TextEdit::load_state(ui.ctx(), editor_id)
                    {
                        state.cursor.set_char_range(Some(CCursorRange::one(
                            CCursor::new(target),
                        )));
                        egui::TextEdit::store_state(ui.ctx(), editor_id, state);
                    }
                }

                let edit_output = egui::TextEdit::multiline(&mut self.editor_text)
                    .id_salt(editor_id_salt)
                    .desired_width(editor_w)
                    .min_size(egui::vec2(editor_w, editor_h))
                    .frame(false)
                    .layouter(&mut layouter)
                    .show(ui);

                let resp = edit_output.response;
                let new_len = self.editor_text.len();
                let just_inserted_char = new_len == self.last_editor_text_len + 1;
                if resp.changed() {
                    self.dirty = true;
                    self.last_edit = Some(Instant::now());

                    // 自动续/退列表：当用户在列表行末尾按 Enter
                    // 改成给下一帧排队 Event::Text / Event::Key(Backspace)，
                    // 让 TextEdit 自己处理，光标位置由 egui 自动算 —— 不再走
                    // state.cursor.set_char_range（在 0.29 里它经常不生效）。
                    if just_inserted_char && self.pending_editor_action.is_none() {
                        if let Some(range) = edit_output.cursor_range {
                            let cursor_char = range.primary.ccursor.index;
                            let cursor_byte =
                                byte_offset_from_char(&self.editor_text, cursor_char);
                            if cursor_byte > 0
                                && cursor_byte <= self.editor_text.len()
                                && self.editor_text.as_bytes()[cursor_byte - 1] == b'\n'
                            {
                                let prev_line_end = cursor_byte - 1;
                                let prev_line_start = self.editor_text[..prev_line_end]
                                    .rfind('\n')
                                    .map(|p| p + 1)
                                    .unwrap_or(0);
                                let prev_line = self.editor_text
                                    [prev_line_start..prev_line_end]
                                    .to_string();
                                if let Some(cont) = continue_list_on_enter(&prev_line) {
                                    match cont {
                                        ListContinuation::Insert(prefix) => {
                                            self.pending_editor_action =
                                                Some(PendingEditorAction::InsertText(prefix));
                                        }
                                        ListContinuation::ExitList => {
                                            // 需要删除：[prev_line_start..cursor_byte) 这段
                                            // 包含空 marker + 刚刚 egui 插入的 \n
                                            let to_delete = self.editor_text
                                                [prev_line_start..cursor_byte]
                                                .chars()
                                                .count();
                                            self.pending_editor_action =
                                                Some(PendingEditorAction::Backspaces(to_delete));
                                        }
                                    }
                                    ui.ctx().request_repaint();
                                }
                            }
                        }
                    }
                }
                self.last_editor_text_len = self.editor_text.len();

                // 行号 gutter —— 用 galley.rows 精确对齐：处理软换行 + 标题行高
                // 不同的视觉行高问题。每个 source paragraph 的"第一行"画行号，
                // 软换行续行不画。
                let gutter_font =
                    egui::FontId::new(self.cfg.font_size, egui::FontFamily::Monospace);
                let gutter_x = gutter_top_left.x + gutter_width - 6.0;
                let gutter_painter = ui.painter();
                let mut para = 1usize;
                let mut paint_this = true;
                for row in &edit_output.galley.rows {
                    if paint_this {
                        let y = edit_output.galley_pos.y + row.rect.center().y;
                        gutter_painter.text(
                            egui::pos2(gutter_x, y),
                            egui::Align2::RIGHT_CENTER,
                            format!("{}", para),
                            gutter_font.clone(),
                            p.text_weak,
                        );
                    }
                    if row.ends_with_newline {
                        para += 1;
                        paint_this = true;
                    } else {
                        paint_this = false;
                    }
                }

                // 无序列表 - / * / + 渲染为 ·：md_highlight 把 marker 字符整段
                // 透明保宽，这里在原位画一个圆点 overlay 上去
                let list_marker_rgb = match theme_mode {
                    ThemeMode::Light => self.cfg.md_light.list_marker,
                    _ => self.cfg.md_dark.list_marker,
                };
                let bullet_color = egui::Color32::from_rgb(
                    list_marker_rgb[0],
                    list_marker_rgb[1],
                    list_marker_rgb[2],
                );
                let bullet_font = egui::FontId::new(
                    self.cfg.font_size * 1.4,
                    egui::FontFamily::Monospace,
                );
                let bullet_painter = ui.painter_at(edit_output.text_clip_rect);
                let galley = edit_output.galley.clone();
                let galley_pos = edit_output.galley_pos;
                let mut byte_pos = 0usize;
                for line in self.editor_text.split('\n') {
                    if let Some((indent_end, _marker_end)) =
                        md_highlight::unordered_list_marker(line)
                    {
                        let dash_byte = byte_pos + indent_end;
                        let dash_char =
                            self.editor_text[..dash_byte].chars().count();
                        let r0 = galley.pos_from_ccursor(egui::text::CCursor::new(dash_char));
                        let r1 = galley.pos_from_ccursor(egui::text::CCursor::new(dash_char + 1));
                        let cx = galley_pos.x + (r0.left() + r1.left()) / 2.0;
                        let cy = galley_pos.y + r0.center().y;
                        bullet_painter.text(
                            egui::pos2(cx, cy),
                            egui::Align2::CENTER_CENTER,
                            "·",
                            bullet_font.clone(),
                            bullet_color,
                        );
                    }
                    byte_pos += line.len() + 1;
                }

                if resp.has_focus() {
                    if let Some(range) = edit_output.cursor_range {
                        // 给 layouter 下一帧用：当前光标所在段（=行）
                        self.editor_cursor_pos = Some((
                            range.primary.pcursor.paragraph,
                            range.primary.pcursor.offset,
                        ));
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
        let current_fg = self
            .fg
            .as_ref()
            .map(|f| f.exe_path.to_string_lossy().to_string());

        let size = egui::vec2(640.0, 460.0);
        let mut builder = egui::ViewportBuilder::default()
            .with_title("NxNote 设置")
            .with_inner_size(size)
            .with_min_inner_size([560.0, 380.0])
            .with_decorations(false)
            .with_resizable(true)
            .with_window_level(egui::WindowLevel::AlwaysOnTop);

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
                settings_ui::draw_settings_window(sctx, cfg, current_fg.clone());
                let after = serde_json::to_string(cfg).unwrap_or_default();
                if before != after {
                    *cfg_dirty = true;
                    *theme_done = false;
                }
            },
        );

        // 设置里的「颜色配置」按钮通过 ctx memory 给我们传信号
        let open_color = ctx.memory_mut(|m| {
            m.data
                .remove_temp::<bool>(egui::Id::new("nx_open_color_editor"))
                .unwrap_or(false)
        });
        if open_color {
            self.color_editor_open = true;
            self.color_editor_pos_applied = false;
        }

        if should_close {
            self.settings_open = false;
            self.settings_fonts_done = false;
            self.settings_theme_done = false;
            self.settings_pos_applied = false;
            if self.cfg_dirty {
                let _ = self.cfg.save();
                self.cfg_dirty = false;
                // 热键变化则重新绑定
                if self.cfg.hotkey != self.last_bound_hotkey {
                    self._hotkey = None;
                    self._hotkey = hotkey::install(&self.cfg.hotkey);
                    self.last_bound_hotkey = self.cfg.hotkey.clone();
                }
                // autostart 变化则同步注册表
                if self.cfg.autostart != self.last_applied_autostart {
                    let _ = crate::autostart::set_enabled(self.cfg.autostart);
                    self.last_applied_autostart = self.cfg.autostart;
                }
            }
        }
    }

    fn draw_color_editor_viewport(&mut self, ctx: &egui::Context) {
        if !self.color_editor_open {
            return;
        }
        let cfg = &mut self.cfg;
        let cfg_dirty = &mut self.cfg_dirty;
        let mut should_close = false;

        let size = egui::vec2(520.0, 460.0);
        let mut builder = egui::ViewportBuilder::default()
            .with_title("NxNote 颜色配置")
            .with_inner_size(size)
            .with_min_inner_size([420.0, 340.0])
            .with_decorations(false)
            .with_resizable(true)
            .with_window_level(egui::WindowLevel::AlwaysOnTop);

        if !self.color_editor_pos_applied {
            let monitor = ctx
                .input(|i| i.viewport().monitor_size)
                .unwrap_or(egui::vec2(1920.0, 1080.0));
            let pos = egui::pos2(
                ((monitor.x - size.x) * 0.5).max(0.0),
                ((monitor.y - size.y) * 0.5).max(0.0),
            );
            builder = builder.with_position(pos);
            self.color_editor_pos_applied = true;
        }

        ctx.show_viewport_immediate(
            egui::ViewportId::from_hash_of("nx_color_editor"),
            builder,
            |sctx, _| {
                if sctx.input(|i| i.viewport().close_requested()) {
                    should_close = true;
                }
                let before = serde_json::to_string(cfg).unwrap_or_default();
                crate::color_ui::draw_color_editor(sctx, cfg);
                let after = serde_json::to_string(cfg).unwrap_or_default();
                if before != after {
                    *cfg_dirty = true;
                }
            },
        );

        if should_close {
            self.color_editor_open = false;
            self.color_editor_pos_applied = false;
            if self.cfg_dirty {
                let _ = self.cfg.save();
                self.cfg_dirty = false;
            }
        }
    }
}

impl eframe::App for NxNoteApp {
    fn update(&mut self, ctx: &egui::Context, frame: &mut eframe::Frame) {
        self.capture_hwnd(frame);

        // --hidden 启动：main.rs 已经用 with_visible(false) 创建窗口，
        // 把全局 atomic 也置位，托盘 / 热键才能 force_show 出来
        if self.start_hidden_pending.is_some() {
            self.hidden = true;
            MAIN_HIDDEN.store(true, Ordering::Release);
            self.start_hidden_pending = None;
        }

        // 后台线程可能直接改了窗口可见性（托盘左键 force_show 等），
        // 每帧把 self.hidden 拉回与 atomic 一致
        self.hidden = MAIN_HIDDEN.load(Ordering::Acquire);

        // 拦截 OS 关闭请求（Alt+F4 / 任务栏右键关闭等），按 close_to_tray 决定
        if ctx.input(|i| i.viewport().close_requested())
            && !self.force_quit
            && self.cfg.close_to_tray
        {
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            self.save_current();
            // 已隐藏就不重复发命令
            if !self.hidden {
                self.toggle_hidden(ctx);
            }
        }

        // 上一帧排队的列表续行注入到当前帧 events 里，让 TextEdit 自己消费
        if let Some(action) = self.pending_editor_action.take() {
            ctx.input_mut(|i| match action {
                PendingEditorAction::InsertText(s) => {
                    i.events.push(egui::Event::Text(s));
                }
                PendingEditorAction::Backspaces(n) => {
                    for _ in 0..n {
                        i.events.push(egui::Event::Key {
                            key: egui::Key::Backspace,
                            physical_key: None,
                            pressed: true,
                            repeat: false,
                            modifiers: egui::Modifiers::default(),
                        });
                    }
                }
            });
        }

        // IME 上屏吃 Enter：输入法回车上屏时，winit 同帧也会送一个
        // Key::Enter 进来，被 TextEdit 当成换行。看到 Ime 事件就把同帧
        // 以及紧接 1 帧里 pressed 的 Enter 过滤掉。
        let ime_event = ctx.input(|i| {
            i.events
                .iter()
                .any(|e| matches!(e, egui::Event::Ime(_)))
        });
        if ime_event {
            self.ime_swallow_enter = 2;
        }
        if self.ime_swallow_enter > 0 {
            ctx.input_mut(|i| {
                i.events.retain(|e| {
                    !matches!(
                        e,
                        egui::Event::Key {
                            key: egui::Key::Enter,
                            pressed: true,
                            ..
                        }
                    )
                });
            });
            self.ime_swallow_enter -= 1;
        }

        self.drain_foreground();
        self.drain_hotkey(ctx);
        self.drain_tray(ctx);
        self.enforce_blocklist();
        self.autosave_tick();
        self.handle_keys(ctx);
        self.handle_editor_shortcuts(ctx);
        self.maybe_reapply_theme(ctx);
        self.update_title_state(ctx);

        self.draw_main_frame(ctx);
        self.draw_settings_viewport(ctx);
        self.draw_color_editor_viewport(ctx);

        // 输入事件本身会唤醒 eframe；聚焦时每帧 request_repaint 会在关闭 vsync
        // 的情况下形成全速渲染循环。仅在等待自动保存的脏状态下安排低频重绘。
        // 隐藏到托盘时彻底 idle，等 tray/hotkey 主动 request_repaint。
        if !self.hidden && self.dirty {
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
enum ListContinuation {
    Insert(String),
    ExitList,
}

fn continue_list_on_enter(prev_line: &str) -> Option<ListContinuation> {
    let b = prev_line.as_bytes();
    let mut i = 0;
    while i < b.len() && (b[i] == b' ' || b[i] == b'\t') {
        i += 1;
    }
    let indent = &prev_line[..i];

    // 无序列表
    if i < b.len() && matches!(b[i], b'-' | b'*' | b'+') && b.get(i + 1) == Some(&b' ') {
        let marker_char = b[i] as char;
        let content_start = i + 2;
        let content = if content_start <= prev_line.len() {
            &prev_line[content_start..]
        } else {
            ""
        };
        if content.trim().is_empty() {
            return Some(ListContinuation::ExitList);
        }
        return Some(ListContinuation::Insert(format!(
            "{}{} ",
            indent, marker_char
        )));
    }

    // 有序列表
    let digit_start = i;
    while i < b.len() && b[i].is_ascii_digit() {
        i += 1;
    }
    if i > digit_start && b.get(i) == Some(&b'.') && b.get(i + 1) == Some(&b' ') {
        let num: usize = prev_line[digit_start..i].parse().unwrap_or(1);
        let content_start = i + 2;
        let content = if content_start <= prev_line.len() {
            &prev_line[content_start..]
        } else {
            ""
        };
        if content.trim().is_empty() {
            return Some(ListContinuation::ExitList);
        }
        return Some(ListContinuation::Insert(format!(
            "{}{}. ",
            indent,
            num + 1
        )));
    }
    None
}

fn char_idx_at_line_col(text: &str, line: usize, col: usize) -> usize {
    let mut total = 0usize;
    for (i, l) in text.split('\n').enumerate() {
        let len = l.chars().count();
        if i == line {
            return total + col.min(len);
        }
        total += len + 1; // +1 for '\n'
    }
    total
}

fn byte_offset_from_char(text: &str, char_idx: usize) -> usize {
    if char_idx == 0 {
        return 0;
    }
    text.char_indices()
        .nth(char_idx)
        .map(|(b, _)| b)
        .unwrap_or(text.len())
}

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
