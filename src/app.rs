use std::sync::atomic::{AtomicBool, AtomicIsize, AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver};
use std::sync::Arc;
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
use crate::hotkey;
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

#[derive(Copy, Clone, PartialEq, Eq)]
enum EditorShortcut {
    MoveBlockUp,
    MoveBlockDown,
    CopyBlockUp,
    CopyBlockDown,
    DeleteLines,
}

#[derive(Default, PartialEq, Eq, Clone)]
enum Modal {
    #[default]
    None,
    NotesList,
    TitleLearn {
        title: String,
        sep_idx: usize,
        custom: String,
    },
    NewNote {
        input: String,
    },
    Rename {
        input: String,
        old: String,
        error: Option<String>,
    },
    /// 跨笔记快速切换 (Ctrl+P)
    QuickSwitch {
        query: String,
        sel: usize,
    },
}

pub struct NxNoteApp {
    pub cfg: Config,
    cfg_dirty: bool,
    last_applied_theme: crate::theme::ThemeMode,
    last_applied_font: f32,
    last_applied_ui_fonts: Vec<String>,
    last_applied_editor_fonts: Vec<String>,
    last_bound_hotkey: String,
    last_bound_capture: String,
    index: AppIndex,

    fg_rx: Receiver<ForegroundInfo>,
    fg: Option<ForegroundInfo>,
    /// 前台轮询间隔（watcher 线程每轮读取，设置改动实时生效）
    poll_ms: Arc<AtomicU64>,
    capture_rx: Receiver<String>,
    _hotkey: Option<hotkey::HotkeyHandle>,
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
    /// 上一帧的 hidden —— 用于检测"刚被隐藏"，立即落盘脏笔记
    prev_hidden: bool,
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
    /// 行级快捷键 / 搜索跳转产生的下一帧光标目标（anchor, primary），char idx。
    /// draw_editor 用它覆盖 TextEditState 里的 cursor，从而保留选区渲染新位置
    pending_cursor_range: Option<(usize, usize)>,
    /// 主编辑器 TextEdit 的真实持久 Id（首帧从 response.id 捕获），
    /// 快捷键在 draw 前读 TextEditState 需要它
    editor_text_id: Option<egui::Id>,
    /// 切换笔记后下一帧清空编辑器状态（旧光标/撤销栈）
    reset_editor_state: bool,
    /// 文内查找 (Ctrl+F) 状态
    search_open: bool,
    search_query: String,
    search_prev_query: String,
    search_hit: usize,
    search_matches: Vec<(usize, usize)>,
    /// 命中跳转后下一帧把当前项滚动到可视区
    scroll_to_match_once: bool,
    /// 主编辑器在屏幕上的矩形（点击外部关闭面板时判断是否点进正文）
    editor_rect: Option<egui::Rect>,
    /// 图片缓存（抓取/纹理），键为 URL
    img_cache: crate::images::ImageCache,
    /// 当前笔记里的图片引用（去重保序，供缩略图栏）
    /// 解析后的实际明暗（System 模式下会随系统变化，用于触发重新应用）
    last_applied_is_light: bool,
    /// 状态栏中央的临时提示 (文本, 出现时刻)
    status_msg: Option<(String, Instant)>,
    /// 缩放等零散 cfg 变更的延迟落盘时刻
    cfg_save_pending: Option<Instant>,
}

impl NxNoteApp {
    pub fn new(cc: &eframe::CreationContext<'_>, cfg: Config) -> Self {
        let poll_ms = Arc::new(AtomicU64::new(cfg.poll_interval_ms.max(20)));
        let fg_rx = watcher::spawn(poll_ms.clone(), cc.egui_ctx.clone());

        let (capture_tx, capture_rx) = mpsc::channel::<String>();
        let hotkey = hotkey::install(&cfg.hotkey, &cfg.hotkey_capture);
        let hotkey_rx =
            hotkey::spawn_listener(cc.egui_ctx.clone(), capture_tx);
        let (tray_handle, tray_rx) = match crate::tray::install(cc.egui_ctx.clone()) {
            Some((h, rx)) => (Some(h), Some(rx)),
            None => (None, None),
        };

        let index = AppIndex::load();
        let (folder_key, display_name, note_name) = storage::resolve_startup(&index);
        let editor_text = storage::load_note(&folder_key, &note_name);

        let last_applied_theme = cfg.theme_mode;
        let last_applied_font = cfg.font_size;
        let last_applied_is_light = theme::resolved_is_light(cfg.theme_mode);
        let last_applied_ui_fonts = cfg.ui_fonts.clone();
        let last_applied_editor_fonts = cfg.editor_fonts.clone();

        let last_bound_hotkey = cfg.hotkey.clone();
        let last_bound_capture = cfg.hotkey_capture.clone();

        let mut s = Self {
            cfg,
            cfg_dirty: false,
            last_applied_theme,
            last_applied_font,
            last_applied_is_light,
            last_applied_ui_fonts,
            last_applied_editor_fonts,
            last_bound_hotkey,
            last_bound_capture,
            index,
            fg_rx,
            fg: None,
            poll_ms,
            capture_rx,
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
            prev_hidden: false,
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
            pending_cursor_range: None,
            editor_text_id: None,
            reset_editor_state: false,
            search_open: false,
            search_query: String::new(),
            search_prev_query: String::new(),
            search_hit: 0,
            search_matches: Vec::new(),
            scroll_to_match_once: false,
            editor_rect: None,
            img_cache: crate::images::ImageCache::new(),
            status_msg: None,
            cfg_save_pending: None,
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
        // 切了笔记：清掉旧光标/撤销栈，避免越界或串味
        self.reset_editor_state = true;
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

        let mut mutated = false;
        let entry = self.index.apps.entry(folder.clone()).or_insert_with(|| {
            mutated = true;
            AppEntry {
                exe_path: info.exe_path.to_string_lossy().to_string(),
                display_name: display.clone(),
                title_rule: None,
                notes: vec![DEFAULT_NOTE.to_string()],
            }
        });
        if entry.notes.is_empty() {
            entry.notes.push(DEFAULT_NOTE.to_string());
            mutated = true;
        }

        let target_note = match entry.title_rule.as_ref().and_then(|r| r.extract(&info.title)) {
            Some(sub) => {
                let sub = storage::sanitize_note_name(&sub);
                if !entry.notes.contains(&sub) {
                    entry.notes.push(sub.clone());
                    mutated = true;
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
        // 只在索引真的变化时写盘，避免每次前台切换都 IO
        if mutated {
            let _ = self.index.save();
        }
        if auto {
            self.switch_to(folder, display, target_note);
        }
    }

    fn drain_foreground(&mut self) {
        while let Ok(info) = self.fg_rx.try_recv() {
            self.handle_foreground_change(info);
        }
    }

    /// 剪贴板快速捕获：当前视图在速记本 → 追加进编辑器（保留未保存内容）；
    /// 否则后台直写速记本文件。
    fn drain_capture(&mut self) {
        while let Ok(text) = self.capture_rx.try_recv() {
            let in_scratch =
                self.folder_key == GLOBAL_FOLDER && self.note_name == SCRATCH_NOTE;
            if in_scratch {
                if !self.editor_text.is_empty() && !self.editor_text.ends_with('\n') {
                    self.editor_text.push('\n');
                }
                self.editor_text.push_str(text.trim_end());
                self.editor_text.push('\n');
                self.dirty = true;
                self.last_edit = Some(Instant::now());
                self.last_editor_text_len = self.editor_text.len();
                // 光标跳到末尾，方便接着写
                let end = self.editor_text.chars().count();
                self.pending_cursor_range = Some((end, end));
            } else {
                match storage::append_scratch(text.trim_end()) {
                    Ok(()) => {}
                    Err(_) => {
                        self.set_status("捕获失败：无法写入速记本".to_string());
                        continue;
                    }
                }
            }
            let n = text.trim_end().chars().count();
            self.set_status(format!("已捕获 {n} 字到速记本"));
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
        // System 模式下系统明暗切换也要感知（每帧查缓存，开销可忽略）
        let now_is_light = theme::resolved_is_light(self.cfg.theme_mode);
        if self.cfg.theme_mode != self.last_applied_theme
            || (self.cfg.font_size - self.last_applied_font).abs() > 0.01
            || now_is_light != self.last_applied_is_light
        {
            theme::apply(ctx, self.cfg.theme_mode, self.cfg.font_size);
            self.last_applied_theme = self.cfg.theme_mode;
            self.last_applied_font = self.cfg.font_size;
            self.last_applied_is_light = now_is_light;
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
        let folder = self.folder_key.clone();
        let old_name = self.note_name.clone();
        // 进回收目录而不是直接删，误删可找回
        let _ = storage::trash_note(&folder, &old_name);
        if folder == GLOBAL_FOLDER {
            self.index.global_notes.retain(|n| n != &old_name);
        } else if let Some(entry) = self.index.apps.get_mut(&folder) {
            entry.notes.retain(|n| n != &old_name);
            if entry.notes.is_empty() {
                entry.notes.push(DEFAULT_NOTE.to_string());
            }
        }
        let _ = self.index.save();
        let fallback_note = if folder == GLOBAL_FOLDER {
            SCRATCH_NOTE
        } else {
            DEFAULT_NOTE
        };
        let next = self
            .index
            .notes_of(&folder)
            .into_iter()
            .find(|n| storage::note_path(&folder, n).exists())
            .unwrap_or_else(|| fallback_note.to_string());
        let display = self.display_name.clone();
        self.dirty = false;
        self.switch_to(folder, display, next);
    }

    /// 当前 folder 的笔记列表（工具栏菜单 / 快速切换共用）。
    /// 全局文件夹 = scratch + 用户自建的全局笔记。
    fn current_folder_notes(&self) -> Vec<String> {
        self.index.notes_of(&self.folder_key)
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
        let (save_now, ctrl_f, ctrl_p) = ctx.input(|i| {
            (
                i.modifiers.ctrl && i.key_pressed(egui::Key::S),
                i.modifiers.ctrl && i.key_pressed(egui::Key::F),
                i.modifiers.ctrl && !i.modifiers.shift && i.key_pressed(egui::Key::P),
            )
        });
        if save_now {
            self.save_current();
        }
        if ctrl_f {
            self.search_open = !self.search_open;
            if self.search_open {
                self.search_hit = 0;
                self.scroll_to_match_once = false;
            }
            consume_key(ctx, egui::Key::F);
        }
        if ctrl_p && self.modal == Modal::None {
            self.modal = Modal::QuickSwitch { query: String::new(), sel: 0 };
            consume_key(ctx, egui::Key::P);
        }
        if save_now {
            self.set_status("已保存".to_string());
        }

        // Ctrl+滚轮缩放字号
        let zoom = ctx.input(|i| i.zoom_delta());
        if zoom != 1.0 && !self.hidden {
            let new_size = (self.cfg.font_size * zoom).clamp(10.0, 24.0);
            if (new_size - self.cfg.font_size).abs() > f32::EPSILON {
                self.cfg.font_size = new_size;
                self.cfg_save_pending = Some(Instant::now());
            }
        }
    }

    fn set_status(&mut self, msg: String) {
        self.status_msg = Some((msg, Instant::now()));
    }

    /// 行级快捷键：Alt+↑/↓ 移动行（有选区时移动整块），Alt+Shift+↑/↓ 复制行，
    /// Ctrl+Shift+K 删除当前行。选区会被完整保留。
    fn handle_editor_shortcuts(&mut self, ctx: &egui::Context) {
        // 模态打开时，焦点在弹窗的 text_edit，不能误伤主编辑器
        if self.modal != Modal::None || self.editor_text_id.is_none() {
            return;
        }
        let (action, consumed_key) = ctx.input(|i| {
            let m = i.modifiers;
            if m.alt && !m.ctrl {
                if i.key_pressed(egui::Key::ArrowUp) {
                    let a = if m.shift {
                        EditorShortcut::CopyBlockUp
                    } else {
                        EditorShortcut::MoveBlockUp
                    };
                    return (Some(a), Some(egui::Key::ArrowUp));
                }
                if i.key_pressed(egui::Key::ArrowDown) {
                    let a = if m.shift {
                        EditorShortcut::CopyBlockDown
                    } else {
                        EditorShortcut::MoveBlockDown
                    };
                    return (Some(a), Some(egui::Key::ArrowDown));
                }
            }
            if m.ctrl && m.shift && !m.alt && i.key_pressed(egui::Key::K) {
                return (Some(EditorShortcut::DeleteLines), Some(egui::Key::K));
            }
            (None, None)
        });
        let Some(action) = action else { return };

        // 从上一帧存下的 TextEditState 里拿完整光标区（char idx），
        // 此时本帧的方向键还没被 TextEdit 处理 —— 正好是按键前的位置
        let editor_id = self.editor_text_id.unwrap();
        let sel: Option<(usize, usize)> = egui::TextEdit::load_state(ctx, editor_id).and_then(
            |st| st.cursor.char_range().map(|r| (r.secondary.index, r.primary.index)),
        );
        let (anchor_c, primary_c) = sel.unwrap_or((0, 0));

        let text = &self.editor_text;
        let (a_line, a_col) = char_to_line_col(text, anchor_c);
        let (p_line, p_col) = char_to_line_col(text, primary_c);
        let mut lo = a_line.min(p_line);
        let mut hi = a_line.max(p_line);
        if action == EditorShortcut::DeleteLines {
            // 删除行只看主光标所在行
            lo = p_line;
            hi = p_line;
        }

        let mut lines: Vec<String> = text.split('\n').map(String::from).collect();
        let n_lines = lines.len();
        let block = hi - lo + 1;

        // 把 (line,col) 映射到操作后的新位置
        let map_pos = |l: usize, c: usize| -> (usize, usize) {
            match action {
                EditorShortcut::MoveBlockUp => {
                    if lo == 0 {
                        (l, c)
                    } else if l == lo - 1 {
                        (hi, c)
                    } else if lo <= l && l <= hi {
                        (l - 1, c)
                    } else {
                        (l, c)
                    }
                }
                EditorShortcut::MoveBlockDown => {
                    if hi + 1 >= n_lines {
                        (l, c)
                    } else if l == hi + 1 {
                        (lo, c)
                    } else if lo <= l && l <= hi {
                        (l + 1, c)
                    } else {
                        (l, c)
                    }
                }
                EditorShortcut::CopyBlockUp => {
                    if l >= lo {
                        (l + block, c)
                    } else {
                        (l, c)
                    }
                }
                EditorShortcut::CopyBlockDown => {
                    if l > hi {
                        (l + block, c)
                    } else {
                        (l, c)
                    }
                }
                EditorShortcut::DeleteLines => {
                    if l < lo {
                        (l, c)
                    } else if l > hi {
                        (l - block, c)
                    } else {
                        // 被删区域内的字符 → 落到接缝行行首
                        (lo, 0)
                    }
                }
            }
        };

        match action {
            EditorShortcut::MoveBlockUp => {
                if lo == 0 {
                    return;
                }
                let moved_up = lines[lo - 1].clone();
                for i in lo..=hi {
                    lines[i - 1] = lines[i].clone();
                }
                lines[hi] = moved_up;
            }
            EditorShortcut::MoveBlockDown => {
                if hi + 1 >= n_lines {
                    return;
                }
                let moved_down = lines[hi + 1].clone();
                for i in (lo..=hi).rev() {
                    lines[i + 1] = lines[i].clone();
                }
                lines[lo] = moved_down;
            }
            EditorShortcut::CopyBlockUp => {
                let dups: Vec<String> = lines[lo..=hi].to_vec();
                lines.splice(lo..lo, dups);
            }
            EditorShortcut::CopyBlockDown => {
                let dups: Vec<String> = lines[lo..=hi].to_vec();
                lines.splice(hi + 1..hi + 1, dups);
            }
            EditorShortcut::DeleteLines => {
                if n_lines == block {
                    lines[0].clear();
                    lines.truncate(1);
                } else {
                    lines.drain(lo..=hi);
                }
            }
        }

        self.editor_text = lines.join("\n");
        self.dirty = true;
        self.last_edit = Some(std::time::Instant::now());
        // 选区两端各自映射，clamp 到新文本范围
        let clamp = |pos: (usize, usize)| -> (usize, usize) {
            let n_lines_now = self.editor_text.split('\n').count();
            let l = pos.0.min(n_lines_now.saturating_sub(1));
            let len = self
                .editor_text
                .split('\n')
                .nth(l)
                .map(|s| s.chars().count())
                .unwrap_or(0);
            (l, pos.1.min(len))
        };
        let new_primary = clamp(map_pos(p_line, p_col));
        let new_anchor = clamp(map_pos(a_line, a_col));
        self.editor_cursor_pos = Some(new_primary);
        self.last_editor_text_len = self.editor_text.len();
        self.pending_cursor_range = Some((
            char_idx_at_line_col(&self.editor_text, new_anchor.0, new_anchor.1),
            char_idx_at_line_col(&self.editor_text, new_primary.0, new_primary.1),
        ));

        // 把刚消费的按键从本帧 events 里挖掉，避免 TextEdit 再当一次箭头/字符处理
        if let Some(k) = consumed_key {
            consume_key(ctx, k);
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
                    let notes = self.current_folder_notes();
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
                            error: None,
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
                            self.modal = Modal::TitleLearn {
                                title: fg.title.clone(),
                                sep_idx: 0,
                                custom: String::new(),
                            };
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
            font.clone(),
            p.text_weak,
        );

        // 中央：临时提示（捕获完成、保存等）
        if let Some((msg, t)) = &self.status_msg {
            if t.elapsed().as_millis() <= 2500 {
                painter.text(
                    egui::pos2(rect.center().x, rect.center().y),
                    egui::Align2::CENTER_CENTER,
                    msg,
                    font,
                    p.accent,
                );
            }
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
                        // 图片行预留高度：随字号缩放，限制在合理范围
                        img_ph_size: (base_size * 9.0).clamp(90.0, 180.0),
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

                // 快捷键/搜索跳转产生的目标光标（anchor, primary）：在 show 之前覆盖 state，
                // 选区两端都保留
                if self.reset_editor_state {
                    self.reset_editor_state = false;
                    let mut st =
                        egui::TextEdit::load_state(ui.ctx(), editor_id).unwrap_or_default();
                    st.cursor.set_char_range(None);
                    st.clear_undoer();
                    egui::TextEdit::store_state(ui.ctx(), editor_id, st);
                }
                if let Some((anchor, primary)) = self.pending_cursor_range.take() {
                    use egui::text::{CCursor, CCursorRange};
                    if let Some(mut state) =
                        egui::TextEdit::load_state(ui.ctx(), editor_id)
                    {
                        state.cursor.set_char_range(Some(CCursorRange {
                            primary: CCursor::new(primary),
                            secondary: CCursor::new(anchor),
                        }));
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
                // 记录编辑器屏幕矩形（点击外部关闭面板时判断用）
                self.editor_rect = Some(resp.rect);
                // 捕获 TextEdit 真实持久 id，供 handle_editor_shortcuts 在
                // draw 之前读上一帧的完整光标区（含选区）
                self.editor_text_id = Some(resp.id);
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
                // 透明保宽，这里在原位画一个圆点 overlay 上去。
                // 任务列表 - [ ] / - [x] 则画成可点击的复选框。
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
                let box_size = (self.cfg.font_size * 1.05).max(12.0);
                let mut task_boxes: Vec<(egui::Rect, usize)> = Vec::new();
                let mut byte_pos = 0usize;
                for line in self.editor_text.split('\n') {
                    if let Some((indent_end, flag_byte, _marker_end, checked)) =
                        md_highlight::task_marker(line)
                    {
                        let char_of = |byte: usize| -> usize {
                            self.editor_text[..byte_pos + byte].chars().count()
                        };
                        let c0 = char_of(indent_end + 1); // '[' 左沿
                        let c1 = char_of(indent_end + 4); // ']' 右沿
                        let r0 = galley.pos_from_ccursor(egui::text::CCursor::new(c0));
                        let r1 = galley.pos_from_ccursor(egui::text::CCursor::new(c1));
                        let cx = galley_pos.x + (r0.left() + r1.left()) / 2.0;
                        let cy = galley_pos.y + r0.center().y;
                        let rect = egui::Rect::from_center_size(
                            egui::pos2(cx, cy),
                            egui::vec2(box_size, box_size),
                        );
                        task_boxes.push((rect, byte_pos + flag_byte));
                        bullet_painter.rect_stroke(
                            rect,
                            2.5,
                            egui::Stroke::new(1.2, bullet_color),
                        );
                        if checked {
                            bullet_painter.rect_filled(
                                rect.shrink(2.0),
                                2.0,
                                bullet_color.gamma_multiply(0.35),
                            );
                            bullet_painter.text(
                                rect.center(),
                                egui::Align2::CENTER_CENTER,
                                icons::CHECK,
                                icons::font(box_size * 0.85),
                                bullet_color,
                            );
                        }
                    } else if let Some((indent_end, _marker_end)) =
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

                // 整行图片 → 把真实纹理贴到被撑高的行内（Typora 式文内预览）。
                // 可点击打开原图，悬浮显示 URL；下载中/失败显示占位芯片。
                {
                    let img_painter = ui.painter_at(edit_output.text_clip_rect);
                    let img_font = egui::FontId::new(
                        self.cfg.font_size * 1.25,
                        egui::FontFamily::Name(crate::icons::ICON_FAMILY.into()),
                    );
                    let mut byte_pos_img = 0usize;
                    for line in self.editor_text.split('\n') {
                        if let Some((sp, _ep, url)) = md_highlight::image_line_span(line) {
                            let start_char =
                                self.editor_text[..byte_pos_img + sp].chars().count();
                            let r = galley.pos_from_ccursor(egui::text::CCursor::new(start_char));
                            // 行区域：从行号分隔线右侧到编辑器右缘
                            // （pos_from_ccursor 返回图库局部坐标，需加 galley_pos 转屏幕坐标）
                            let row_rect = egui::Rect::from_min_max(
                                egui::pos2(galley_pos.x + 2.0, galley_pos.y + r.top() + 4.0),
                                egui::pos2(
                                    galley_pos.x + editor_w - 6.0,
                                    galley_pos.y + r.bottom() - 4.0,
                                ),
                            );
                            if row_rect.height() > 12.0 && row_rect.width() > 24.0 {
                                let tex = self.img_cache.texture(&url).cloned();
                                let pending = self.img_cache.is_pending(&url)
                                    || (tex.is_none()
                                        && !self.img_cache.failed.contains(&url));
                                match tex {
                                    Some(t) => {
                                        let aspect = t.size()[0] as f32
                                            / t.size()[1].max(1) as f32;
                                        let mut w = row_rect.width();
                                        let mut h = w / aspect;
                                        if h > row_rect.height() {
                                            h = row_rect.height();
                                            w = h * aspect;
                                        }
                                        let img_rect = egui::Rect::from_center_size(
                                            row_rect.center(),
                                            egui::vec2(w.min(row_rect.width()), h),
                                        );
                                        img_painter.image(
                                            t.id(),
                                            img_rect,
                                            egui::Rect::from_min_max(
                                                egui::pos2(0.0, 0.0),
                                                egui::pos2(1.0, 1.0),
                                            ),
                                            egui::Color32::WHITE,
                                        );
                                        // 点击打开原图 + 悬浮提示
                                        let resp = ui.interact(
                                            img_rect,
                                            egui::Id::new(("nx_inline_img", byte_pos_img + sp)),
                                            egui::Sense::click(),
                                        );
                                        if resp.clicked() {
                                            crate::fsutil::open_url(&url);
                                        }
                                        resp.on_hover_text(format!("{url}\n点击打开原图"));
                                    }
                                    None if pending => {
                                        img_painter.rect_filled(row_rect, 6.0, p.bg_alt);
                                        img_painter.text(
                                            row_rect.center(),
                                            egui::Align2::CENTER_CENTER,
                                            format!("{} 加载中…", icons::IMAGE),
                                            img_font.clone(),
                                            p.text_weak,
                                        );
                                    }
                                    _ => {
                                        img_painter.rect_filled(row_rect, 6.0, p.bg_alt);
                                        img_painter.text(
                                            row_rect.center(),
                                            egui::Align2::CENTER_CENTER,
                                            format!("{} 图片加载失败（点击重试需重新输入链接）", icons::IMAGE),
                                            img_font.clone(),
                                            palette(theme_mode).danger,
                                        );
                                    }
                                }
                            }
                        }
                        byte_pos_img += line.len() + 1;
                    }
                }

                // 行内（非独占一行）的图片引用：语法隐藏，原位画小图标芯片
                {
                    let img_painter = ui.painter_at(edit_output.text_clip_rect);
                    let img_font = egui::FontId::new(
                        self.cfg.font_size * 1.25,
                        egui::FontFamily::Name(crate::icons::ICON_FAMILY.into()),
                    );
                    for (bs, _be, _url) in md_highlight::collect_images(&self.editor_text) {
                        if bs >= self.editor_text.len() {
                            continue;
                        }
                        // 跳过整行图片（上面已渲染）
                        let line_start = self.editor_text[..bs + 1]
                            .rfind('\n')
                            .map(|p| p + 1)
                            .unwrap_or(0);
                        let line_end = self.editor_text[bs..]
                            .find('\n')
                            .map(|p| bs + p)
                            .unwrap_or(self.editor_text.len());
                        let line = &self.editor_text[line_start..line_end];
                        if md_highlight::image_line_span(line).is_some() {
                            continue;
                        }
                        let start_char = self.editor_text[..bs].chars().count();
                        let r = galley.pos_from_ccursor(egui::text::CCursor::new(start_char));
                        let cx = galley_pos.x + r.left() + self.cfg.font_size * 0.7;
                        let cy = galley_pos.y + r.center().y;
                        img_painter.text(
                            egui::pos2(cx, cy),
                            egui::Align2::LEFT_CENTER,
                            icons::IMAGE,
                            img_font.clone(),
                            bullet_color,
                        );
                    }
                }

                // 点击复选框 → 翻转勾选（普通左键点击；点中框附近才算）
                if resp.clicked() && !task_boxes.is_empty() {
                    if let Some(pos) = resp.interact_pointer_pos() {
                        for (rect, flag_byte) in &task_boxes {
                            if rect.expand(3.0).contains(pos) {
                                let ch = self.editor_text.as_bytes()[*flag_byte];
                                let new_ch = if ch == b' ' { 'x' } else { ' ' };
                                self.editor_text.replace_range(
                                    *flag_byte..*flag_byte + 1,
                                    &new_ch.to_string(),
                                );
                                self.dirty = true;
                                self.last_edit = Some(Instant::now());
                                self.last_editor_text_len = self.editor_text.len();
                                break;
                            }
                        }
                    }
                }

                // Ctrl+点击 / 右键点击链接 → 打开（URL 取 ]( 和 ) 之间的部分）
                if open_link_requested(ui.ctx(), &resp) {
                    if let Some(pos) = resp.interact_pointer_pos() {
                        let local = pos - galley_pos;
                        let cursor = galley.cursor_from_pos(local);
                        let byte = byte_offset_from_char(&self.editor_text, cursor.ccursor.index);
                        for (bs, be) in md_highlight::collect_links(&self.editor_text) {
                            if byte >= bs && byte < be {
                                if let Some(url) =
                                    parse_markdown_link_url(&self.editor_text[bs..be])
                                {
                                    crate::fsutil::open_url(&url);
                                }
                                break;
                            }
                        }
                    }
                }

                // 悬停在链接上 → 手型光标
                if resp.hovered() {
                    if let Some(pos) = ui.ctx().input(|i| i.pointer.interact_pos()) {
                        let local = pos - galley_pos;
                        let cursor = galley.cursor_from_pos(local);
                        let byte = byte_offset_from_char(&self.editor_text, cursor.ccursor.index);
                        if md_highlight::collect_links(&self.editor_text)
                            .iter()
                            .any(|(bs, be)| byte >= *bs && byte < *be)
                        {
                            ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
                        }
                    }
                }

                // 搜索高亮：全部命中淡色，当前命中强色；跳转时滚动到当前项
                if self.search_open && !self.search_matches.is_empty() {
                    let hl_painter = ui.painter_at(edit_output.text_clip_rect);
                    let dim = p.accent.gamma_multiply(0.22);
                    let hot = p.accent.gamma_multiply(0.60);
                    for (i, (a, b)) in self.search_matches.iter().enumerate() {
                        let color = if i == self.search_hit { hot } else { dim };
                        for rect in galley_char_rects(
                            &edit_output.galley,
                            galley_pos,
                            *a,
                            *b,
                        ) {
                            hl_painter.rect_filled(rect, 2.0, color);
                        }
                    }
                }
                if self.scroll_to_match_once {
                    self.scroll_to_match_once = false;
                    if let Some((a, b)) = self.search_matches.get(self.search_hit) {
                        let all = galley_char_rects(&edit_output.galley, galley_pos, *a, *b);
                        if let (Some(first), Some(last)) = (all.first(), all.last()) {
                            let target = first.union(*last);
                            ui.scroll_to_rect(target, Some(egui::Align::Center));
                        }
                    }
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
        // Esc 关闭任何模态/查找面板（并吃掉本帧事件，避免泄漏给编辑器）
        if (self.modal != Modal::None || self.search_open)
            && ctx.input(|i| i.key_pressed(egui::Key::Escape))
        {
            consume_key(ctx, egui::Key::Escape);
            self.modal = Modal::None;
            self.search_open = false;
        }
        let mut close = false;
        match self.modal.clone() {
            Modal::None => {}
            Modal::TitleLearn { title, mut sep_idx, mut custom } => {
                egui::Window::new("学习标题规则")
                    .collapsible(false)
                    .resizable(false)
                    .anchor(egui::Align2::CENTER_CENTER, egui::vec2(0.0, 0.0))
                    .show(ctx, |ui| {
                        ui.label("选择标题里的分隔符，再点击属于「项目名」的那一段：");
                        ui.label(
                            egui::RichText::new(&title).italics().small(),
                        );
                        ui.add_space(4.0);

                        const SEP_CANDIDATES: &[&str] =
                            &[" - ", " | ", " — ", "｜", "|", "::", " · "];
                        let present: Vec<&str> = SEP_CANDIDATES
                            .iter()
                            .copied()
                            .filter(|s| title.contains(s))
                            .collect();
                        if !present.is_empty() {
                            ui.horizontal_wrapped(|ui| {
                                for (i, s) in present.iter().enumerate() {
                                    if ui
                                        .add(egui::Button::new(
                                            egui::RichText::new(format!("{s:?}")).size(11.5),
                                        ))
                                        .clicked()
                                    {
                                        sep_idx = i;
                                        custom.clear(); // 候选优先于自定义
                                    }
                                }
                            });
                        }
                        ui.add_space(2.0);
                        ui.horizontal(|ui| {
                            ui.label(weak_text("自定义分隔符"));
                            ui.text_edit_singleline(&mut custom);
                        });

                        // 自定义非空时优先生效；否则用候选里选中的那个
                        let sep: String = if !custom.trim().is_empty() {
                            custom.trim_start().to_string()
                        } else if !present.is_empty() {
                            present[sep_idx.min(present.len() - 1)].to_string()
                        } else {
                            String::new()
                        };

                        let parts: Vec<&str> =
                            if sep.is_empty() { vec![title.as_str()] } else { title.split(&sep).collect() };
                        if parts.len() < 2 && !sep.is_empty() {
                            ui.label(
                                weak_text("该分隔符没有把标题切成多段"),
                            );
                        }
                        ui.separator();
                        let mut clicked_idx: Option<usize> = None;
                        ui.horizontal_wrapped(|ui| {
                            for (i, p) in parts.iter().enumerate() {
                                if ui.button(*p).clicked() {
                                    clicked_idx = Some(i);
                                }
                            }
                        });
                        if let Some(i) = clicked_idx {
                            if !sep.is_empty() {
                                let sub = storage::sanitize_note_name(parts[i]);
                                if let Some(entry) = self.index.apps.get_mut(&self.folder_key) {
                                    entry.title_rule = Some(TitleRule::SplitIndex {
                                        sep: sep.clone(),
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
                            }
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
                if !close {
                    self.modal = Modal::TitleLearn { title, sep_idx, custom };
                }
            }
            Modal::NewNote { mut input } => {
                egui::Window::new("新建笔记")
                    .collapsible(false)
                    .resizable(false)
                    .anchor(egui::Align2::CENTER_CENTER, egui::vec2(0.0, 0.0))
                    .show(ctx, |ui| {
                        ui.label("笔记名");
                        let resp = ui.text_edit_singleline(&mut input);
                        focus_when_idle(ui, &resp);
                        let enter_here = resp.has_focus()
                            && ui.input(|i| i.key_pressed(egui::Key::Enter));
                        ui.horizontal(|ui| {
                            let confirm =
                                ui.button("创建").clicked() || enter_here;
                            if confirm {
                                let name = storage::sanitize_note_name(&input);
                                if self.folder_key == GLOBAL_FOLDER {
                                    if name != SCRATCH_NOTE
                                        && !self.index.global_notes.contains(&name)
                                    {
                                        self.index.global_notes.push(name.clone());
                                    }
                                } else if let Some(entry) =
                                    self.index.apps.get_mut(&self.folder_key)
                                {
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
            Modal::Rename { mut input, old, mut error } => {
                egui::Window::new("重命名笔记")
                    .collapsible(false)
                    .resizable(false)
                    .anchor(egui::Align2::CENTER_CENTER, egui::vec2(0.0, 0.0))
                    .show(ctx, |ui| {
                        ui.label("新名称");
                        let resp = ui.text_edit_singleline(&mut input);
                        focus_when_idle(ui, &resp);
                        let enter_here = resp.has_focus()
                            && ui.input(|i| i.key_pressed(egui::Key::Enter));
                        if let Some(e) = &error {
                            ui.label(
                                egui::RichText::new(e)
                                    .color(palette(self.cfg.theme_mode).danger)
                                    .small(),
                            );
                        }
                        ui.horizontal(|ui| {
                            let confirm =
                                ui.button("确认").clicked() || enter_here;
                            if confirm {
                                let new_name = storage::sanitize_note_name(&input);
                                if self.folder_key == GLOBAL_FOLDER && old == SCRATCH_NOTE {
                                    error = Some("内置速记不支持重命名".to_string());
                                } else if new_name != old {
                                    let dup_listed =
                                        self.current_folder_notes().contains(&new_name);
                                    let dup_disk = storage::note_path(
                                        &self.folder_key,
                                        &new_name,
                                    )
                                    .exists();
                                    if dup_listed || dup_disk {
                                        error =
                                            Some(format!("「{new_name}」已存在，换个名字"));
                                    } else {
                                        let folder = self.folder_key.clone();
                                        let display = self.display_name.clone();
                                        self.save_current();
                                        let from =
                                            storage::note_path(&folder, &old);
                                        let to =
                                            storage::note_path(&folder, &new_name);
                                        let _ = std::fs::rename(&from, &to);
                                        if folder == GLOBAL_FOLDER {
                                            for n in
                                                self.index.global_notes.iter_mut()
                                            {
                                                if *n == old {
                                                    *n = new_name.clone();
                                                }
                                            }
                                        } else if let Some(entry) =
                                            self.index.apps.get_mut(&folder)
                                        {
                                            for n in entry.notes.iter_mut() {
                                                if n == &old {
                                                    *n = new_name.clone();
                                                }
                                            }
                                        }
                                        let _ = self.index.save();
                                        // 注意：不要手动改 self.note_name，
                                        // 让 switch_to 完整走一遍（含 last_* 持久化）
                                        self.switch_to(folder, display, new_name);
                                        close = true;
                                    }
                                } else {
                                    close = true;
                                }
                            }
                            if ui.button("取消").clicked() {
                                close = true;
                            }
                        });
                    });
                if !close {
                    self.modal = Modal::Rename { input, old, error };
                }
            }
            Modal::QuickSwitch { mut query, mut sel } => {
                // 方向键/Enter 由列表消费，不进编辑器
                let (up, down, enter) = ctx.input(|i| {
                    (
                        i.key_pressed(egui::Key::ArrowUp),
                        i.key_pressed(egui::Key::ArrowDown),
                        i.key_pressed(egui::Key::Enter),
                    )
                });
                if up || down || enter {
                    consume_key(ctx, egui::Key::ArrowUp);
                    consume_key(ctx, egui::Key::ArrowDown);
                    consume_key(ctx, egui::Key::Enter);
                }

                // 候选：速记 + 全局笔记 + 各应用笔记
                let mut items: Vec<(String, String, String)> = vec![(
                    GLOBAL_FOLDER.to_string(),
                    "速记".to_string(),
                    SCRATCH_NOTE.to_string(),
                )];
                for n in &self.index.global_notes {
                    if n != &SCRATCH_NOTE.to_string() {
                        items.push((
                            GLOBAL_FOLDER.to_string(),
                            "速记".to_string(),
                            n.clone(),
                        ));
                    }
                }
                let apps: Vec<(String, AppEntry)> = self
                    .index
                    .apps
                    .iter()
                    .map(|(k, v)| (k.clone(), v.clone()))
                    .collect();
                for (folder, entry) in apps {
                    for n in entry.notes {
                        items.push((folder.clone(), entry.display_name.clone(), n));
                    }
                }

                let win = egui::Window::new("切换笔记")
                    .collapsible(false)
                    .resizable(false)
                    .default_width(320.0)
                    .anchor(egui::Align2::CENTER_CENTER, egui::vec2(0.0, -20.0))
                    .show(ctx, |ui| {
                        let q = query.trim().to_lowercase();
                        let filtered: Vec<(usize, &(String, String, String))> = items
                            .iter()
                            .enumerate()
                            .filter(|(_, (f, d, n))| {
                                q.is_empty() || {
                                    let hay = format!(
                                        "{}/{}",
                                        d,
                                        n
                                    ).to_lowercase();
                                    let hay = if f == GLOBAL_FOLDER {
                                        format!("速记/{n}").to_lowercase()
                                    } else {
                                        hay
                                    };
                                    fuzzy_match(&hay, &q)
                                }
                            })
                            .collect();
                        if sel >= filtered.len().max(1) {
                            sel = 0;
                        }
                        let resp = ui.add(
                            egui::TextEdit::singleline(&mut query)
                                .desired_width(280.0)
                                .hint_text("输入应用名 / 笔记名…"),
                        );
                        focus_when_idle(ui, &resp);
                        if up && sel > 0 {
                            sel -= 1;
                        }
                        if down && sel + 1 < filtered.len() {
                            sel += 1;
                        }
                        let mut jump: Option<(String, String, String)> = None;
                        egui::ScrollArea::vertical()
                            .max_height(280.0)
                            .auto_shrink([false, true])
                            .show(ui, |ui| {
                                if filtered.is_empty() {
                                    ui.label(weak_text("无匹配"));
                                }
                                for (list_i, (_orig_i, (f, d, n))) in
                                    filtered.iter().enumerate()
                                {
                                    let label = format!("{d} / {n}");
                                    if ui
                                        .selectable_label(sel == list_i, label)
                                        .clicked()
                                    {
                                        jump =
                                            Some((f.clone(), d.clone(), n.clone()));
                                    }
                                }
                            });
                        if enter && !filtered.is_empty() {
                            let (_, (f, d, n)) = filtered[sel];
                            jump = Some((f.clone(), d.clone(), n.clone()));
                        }
                        if let Some((f, d, n)) = jump {
                            // 快速切换不改变钉住状态，纯跳转
                            self.switch_to(f, d, n);
                            close = true;
                        }
                    });
                // 点击面板外部（含正文区）自动关闭
                let clicked = ctx.input(|i| i.pointer.any_click());
                let pos = ctx.input(|i| i.pointer.interact_pos());
                if let (Some(ir), Some(pos)) = (win, pos) {
                    if clicked && !ir.response.rect.contains(pos) {
                        close = true;
                    }
                }
                if !close {
                    self.modal = Modal::QuickSwitch { query, sel };
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
                                if ui.button(SCRATCH_NOTE).clicked() {
                                    jump = Some((
                                        GLOBAL_FOLDER.to_string(),
                                        "速记".to_string(),
                                        SCRATCH_NOTE.to_string(),
                                    ));
                                }
                                for n in self.index.global_notes.clone() {
                                    if n == SCRATCH_NOTE {
                                        continue;
                                    }
                                    if ui.button(n.clone()).clicked() {
                                        jump = Some((
                                            GLOBAL_FOLDER.to_string(),
                                            "速记".to_string(),
                                            n,
                                        ));
                                    }
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
        self.draw_search_panel(ctx);
    }

    /// 文内查找浮动条：圆角胶囊，图标 + 无边框输入 + 计数 + 上一个/下一个/关闭。
    /// 点击面板外部（非正文区）自动关闭。
    fn draw_search_panel(&mut self, ctx: &egui::Context) {
        if !self.search_open {
            return;
        }
        let p = palette(self.cfg.theme_mode);

        // 点击外部关闭：点在面板和编辑器之外 → 收起（先记录，面板绘制后判定）
        let clicked = ctx.input(|i| i.pointer.any_click());
        let click_pos = ctx.input(|i| i.pointer.interact_pos());
        let mut panel_rect: Option<egui::Rect> = None;

        let frame = egui::Frame::default()
            .fill(p.bg_alt)
            .stroke(egui::Stroke::new(1.0, p.stroke))
            .rounding(8.0)
            .inner_margin(egui::Margin::same(8.0));

        let win = egui::Window::new("")
            .title_bar(false)
            .resizable(false)
            .collapsible(false)
            .anchor(egui::Align2::RIGHT_TOP, egui::vec2(-10.0, 34.0))
            .frame(frame)
            .show(ctx, |ui| {
                ui.spacing_mut().item_spacing.x = 6.0;
                ui.horizontal(|ui| {
                    ui.label(icons::rich(icons::SEARCH, 15.0).color(p.text_weak));

                    let resp = egui::TextEdit::singleline(&mut self.search_query)
                        .desired_width(170.0)
                        .frame(false)
                        .hint_text("查找…")
                        .font(egui::TextStyle::Body)
                        .show(ui);
                    focus_when_idle(ui, &resp.response);

                    // 计数
                    let counter = if self.search_matches.is_empty() {
                        if self.search_query.trim().is_empty() {
                            "—".to_string()
                        } else {
                            "0".to_string()
                        }
                    } else {
                        format!("{} / {}", self.search_hit + 1, self.search_matches.len())
                    };
                    ui.label(
                        egui::RichText::new(counter)
                            .size(11.0)
                            .color(if self.search_matches.is_empty() {
                                p.text_weak
                            } else {
                                p.accent
                            }),
                    );

                    ui.separator();

                    let prev = small_icon_btn(ui, icons::KEYBOARD_ARROW_UP, "上一个 (Shift+Enter)");
                    let next = small_icon_btn(ui, icons::KEYBOARD_ARROW_DOWN, "下一个 (Enter)");
                    let close_x = small_icon_btn(ui, icons::CLOSE, "关闭 (Esc)");

                    let mut nav = |dir: isize| {
                        if !self.search_matches.is_empty() {
                            let n = self.search_matches.len() as isize;
                            self.search_hit =
                                ((self.search_hit as isize + dir).rem_euclid(n)) as usize;
                            let (a, b) = self.search_matches[self.search_hit];
                            self.pending_cursor_range = Some((a, b));
                            self.scroll_to_match_once = true;
                        }
                    };
                    if prev {
                        nav(-1);
                    }
                    if next {
                        nav(1);
                    }
                    if close_x {
                        self.search_open = false;
                    }
                    if ctx.input(|i| i.key_pressed(egui::Key::Enter)) && resp.response.has_focus()
                    {
                        let dir = if ctx.input(|i| i.modifiers.shift) { -1 } else { 1 };
                        nav(dir);
                    }
                });
            });
        if let Some(ir) = win {
            panel_rect = Some(ir.response.rect);
        }

        // 面板矩形拿到后再判断外部点击（点击正文不关，便于对照浏览）
        if clicked {
            if let (Some(cr), Some(pos)) = (panel_rect, click_pos) {
                let in_editor = self.editor_rect.map(|r| r.contains(pos)).unwrap_or(false);
                if !cr.contains(pos) && !in_editor {
                    self.search_open = false;
                }
            }
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
                // 热键变化则重新绑定（toggle + capture）
                if self.cfg.hotkey != self.last_bound_hotkey
                    || self.cfg.hotkey_capture != self.last_bound_capture
                {
                    self._hotkey = None;
                    self._hotkey = hotkey::install(&self.cfg.hotkey, &self.cfg.hotkey_capture);
                    self.last_bound_hotkey = self.cfg.hotkey.clone();
                    self.last_bound_capture = self.cfg.hotkey_capture.clone();
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

        // 刚被隐藏 → 立刻落盘脏笔记。隐藏后帧循环会停，autosave_tick
        // 不再运行，不在这里存的话热键隐藏后的编辑内容会一直悬在内存里
        if !self.prev_hidden && self.hidden && self.dirty {
            self.save_current();
            self.set_status("已保存".to_string());
        }
        self.prev_hidden = self.hidden;

        // 轮询间隔实时下发给 watcher 线程（设置改动即时生效）
        self.poll_ms
            .store(self.cfg.poll_interval_ms.max(20), Ordering::Relaxed);

        // 记录窗口位置/尺寸（退出、托盘退出、隐藏到托盘时随 cfg 落盘）
        let maximized = ctx.input(|i| i.viewport().maximized.unwrap_or(false));
        if let Some(rect) = ctx.input(|i| i.viewport().outer_rect) {
            if !maximized && rect.width() > 0.0 {
                self.cfg.window_width = rect.width();
                self.cfg.window_height = rect.height();
                self.cfg.window_pos = Some([rect.left(), rect.top()]);
            }
        }

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
        self.drain_capture();
        self.drain_hotkey(ctx);
        self.drain_tray(ctx);
        self.enforce_blocklist();
        self.autosave_tick();
        self.handle_keys(ctx);
        self.handle_editor_shortcuts(ctx);
        self.maybe_reapply_theme(ctx);
        self.update_title_state(ctx);

        // 文内查找：每帧重算匹配；查询变化时复位到第一个命中
        if self.search_open {
            if self.search_query != self.search_prev_query {
                self.search_prev_query = self.search_query.clone();
                self.search_hit = 0;
            }
            self.search_matches = if self.search_query.trim().is_empty() {
                Vec::new()
            } else {
                find_matches(&self.editor_text, &self.search_query)
            };
            if self.search_hit >= self.search_matches.len() {
                self.search_hit = 0;
            }
        } else {
            self.search_matches.clear();
        }

        // 图片引用：整行图片（文内预览）去重保序，未就绪的发起后台抓取
        {
            let mut seen: Vec<String> = Vec::new();
            for line in self.editor_text.split('\n') {
                if let Some((_s, _e, url)) = md_highlight::image_line_span(line) {
                    if !seen.contains(&url) {
                        seen.push(url.clone());
                        self.img_cache.request(&url);
                    }
                }
            }
        }
        self.img_cache.drain(ctx);

        // 主编辑器聚焦时粘贴图片 URL → 自动包成 ![图片](url)\n
        let editor_focused = ctx
            .memory(|m| m.focused())
            .zip(self.editor_text_id)
            .map(|(f, id)| f == id)
            .unwrap_or(false);
        if editor_focused {
            ctx.input_mut(|i| {
                for e in &mut i.events {
                    if let egui::Event::Paste(s) = e {
                        let t = s.trim();
                        if md_highlight::is_image_url(t) {
                            *s = format!("![图片]({t})\n");
                        }
                    }
                }
            });
        }

        self.draw_main_frame(ctx);
        self.draw_settings_viewport(ctx);
        self.draw_color_editor_viewport(ctx);

        // 状态栏提示 / 缩放延迟落盘到点后需要一帧收尾
        let status_expired =
            matches!(&self.status_msg, Some((_, t)) if t.elapsed().as_millis() > 2500);
        if status_expired {
            self.status_msg = None;
        }
        if let Some(t) = self.cfg_save_pending {
            if t.elapsed().as_millis() >= 800 {
                let _ = self.cfg.save();
                self.cfg_save_pending = None;
            }
        }

        // 输入事件本身会唤醒 eframe；聚焦时每帧 request_repaint 会在关闭 vsync
        // 的情况下形成全速渲染循环。仅在等待自动保存的脏状态下安排低频重绘。
        // 隐藏到托盘时彻底 idle，等 tray/hotkey 主动 request_repaint。
        if !self.hidden && self.dirty {
            ctx.request_repaint_after(std::time::Duration::from_millis(300));
        }
        if self.status_msg.is_some() || self.cfg_save_pending.is_some() {
            ctx.request_repaint_after(std::time::Duration::from_millis(250));
        }
        // 图片下载中：低频唤醒刷新缩略图
        if self.img_cache.pending_count() > 0 {
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

    // 任务列表：- [ ] / - [x] —— 有内容则续一个空勾选框，空则退出列表
    if let Some((ind, _flag, marker_end, _checked)) = crate::md_highlight::task_marker(prev_line) {
        let content = prev_line.get(marker_end..).unwrap_or("");
        if content.trim().is_empty() {
            return Some(ListContinuation::ExitList);
        }
        return Some(ListContinuation::Insert(format!(
            "{}- [ ] ",
            &prev_line[..ind]
        )));
    }

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

/// 全文 char idx → (line, col)，越界时 clamp 到末尾。
fn char_to_line_col(text: &str, char_idx: usize) -> (usize, usize) {
    let mut total = 0usize;
    for (i, l) in text.split('\n').enumerate() {
        let len = l.chars().count();
        if char_idx < total + len {
            return (i, char_idx - total);
        }
        // 正好落在本行末尾（含换行位置）也返回本行
        if char_idx == total + len {
            return (i, len);
        }
        total += len + 1;
    }
    let n = text.split('\n').count();
    (n.saturating_sub(1), 0)
}

/// 把本帧里 pressed 的某个键从事件队列挖掉，防止后续控件重复消费。
fn consume_key(ctx: &egui::Context, key: egui::Key) {
    ctx.input_mut(|i| {
        i.events.retain(|e| {
            !matches!(e, egui::Event::Key { key: k, pressed: true, .. } if *k == key)
        });
    });
}

/// 编辑器上打开链接的触发条件：Ctrl+左键 或 右键。
fn open_link_requested(ctx: &egui::Context, resp: &egui::Response) -> bool {
    let ctrl = ctx.input(|i| i.modifiers.ctrl);
    (ctrl && resp.clicked()) || resp.secondary_clicked()
}

/// 弱化小字。
fn weak_text(s: impl Into<String>) -> egui::RichText {
    egui::RichText::new(s.into()).weak().small()
}

/// 计算一段 char 区间在 galley 上占据的视觉矩形（处理软换行跨行）。
/// off 为 galley 原点的屏幕坐标。
fn galley_char_rects(
    galley: &egui::Galley,
    off: egui::Pos2,
    a: usize,
    b: usize,
) -> Vec<egui::Rect> {
    if a >= b {
        return Vec::new();
    }
    // pos_from_ccursor 返回该字符所在位置的 Rect（行高正确）
    let r0 = galley.pos_from_ccursor(egui::text::CCursor::new(a));
    let r1 = galley.pos_from_ccursor(egui::text::CCursor::new(b));

    // 同一视觉行：直接连成一段
    if r0.top() == r1.top() && r0.bottom() == r1.bottom() {
        return vec![egui::Rect::from_min_max(
            egui::pos2(off.x + r0.left(), off.y + r0.top()),
            egui::pos2(off.x + r1.left(), off.y + r1.bottom()),
        )];
    }

    // 跨行：用 rcursor 定位行号，逐行拼矩形
    let row_of = |r: egui::Rect| -> usize {
        galley.cursor_from_pos(egui::vec2(r.left() + 1.0, r.top())).rcursor.row
    };
    let row0 = row_of(r0);
    let row1 = row_of(r1);
    let mut out = Vec::new();
    // 起始行：从起点到行尾
    if let Some(row) = galley.rows.get(row0) {
        out.push(egui::Rect::from_min_max(
            egui::pos2(off.x + r0.left(), off.y + row.rect.top()),
            egui::pos2(off.x + row.rect.right(), off.y + row.rect.bottom()),
        ));
    }
    // 中间整行
    for r in (row0 + 1)..row1 {
        if let Some(row) = galley.rows.get(r) {
            out.push(egui::Rect::from_min_max(
                egui::pos2(off.x + row.rect.left(), off.y + row.rect.top()),
                egui::pos2(off.x + row.rect.right(), off.y + row.rect.bottom()),
            ));
        }
    }
    // 末行：从行首到终点
    if let Some(row) = galley.rows.get(row1) {
        out.push(egui::Rect::from_min_max(
            egui::pos2(off.x + row.rect.left(), off.y + row.rect.top()),
            egui::pos2(off.x + r1.left(), off.y + row.rect.bottom()),
        ));
    }
    out
}

/// 从 markdown 链接原文 `[text](url)` 里解析出 url 部分。
fn parse_markdown_link_url(raw: &str) -> Option<String> {
    let raw = raw.trim();
    let sep = raw.find("](")?;
    if !raw.ends_with(')') || sep == 0 {
        return None;
    }
    let url = raw[sep + 2..raw.len() - 1].trim();
    if url.is_empty() {
        None
    } else {
        Some(url.to_string())
    }
}

/// 仅当当前没有任何控件持有焦点时抢焦点 —— 模态输入框首帧聚焦，
/// 又不会在用户点按钮后把焦点抢回来。
fn focus_when_idle(ui: &egui::Ui, resp: &egui::Response) {
    if ui.memory(|m| m.focused().is_none()) {
        resp.request_focus();
    }
}

/// 文内查找：返回 char 区间列表（区分大小写仅当 needle 含大写）。
fn find_matches(haystack: &str, needle: &str) -> Vec<(usize, usize)> {
    let mut out = Vec::new();
    if needle.is_empty() {
        return out;
    }
    let case_insensitive = needle.chars().all(|c| c.to_lowercase().next() == Some(c));
    let hay: Vec<char> = haystack
        .chars()
        .map(|c| if case_insensitive { c.to_lowercase().next().unwrap_or(c) } else { c })
        .collect();
    let nd: Vec<char> = needle
        .chars()
        .map(|c| if case_insensitive { c.to_lowercase().next().unwrap_or(c) } else { c })
        .collect();
    if nd.is_empty() || hay.len() < nd.len() {
        return out;
    }
    for start in 0..=(hay.len() - nd.len()) {
        if hay[start..start + nd.len()] == nd[..] {
            out.push((start, start + nd.len()));
            if out.len() >= 2000 {
                break;
            }
        }
    }
    out
}

/// 模糊子序列匹配：needle 的字符按序出现在 hay 中即命中。
fn fuzzy_match(hay: &str, needle: &str) -> bool {
    let mut it = hay.chars();
    for n in needle.chars() {
        if !it.any(|h| h == n) {
            return false;
        }
    }
    true
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
        let mid = (lo + hi).div_ceil(2);
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

/// 紧凑图标按钮（搜索条用）。
fn small_icon_btn(ui: &mut egui::Ui, glyph: &'static str, hint: &str) -> bool {
    ui.scope(|ui| {
        ui.spacing_mut().button_padding = egui::vec2(4.0, 2.0);
        ui.button(icons::rich(glyph, 13.0)).on_hover_text(hint).clicked()
    })
    .inner
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
