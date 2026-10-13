use crate::{
    config::{Config, OpenTab},
    platform::{self, PlatformEvent},
    session::Session,
    storage, theme,
};
use gpui_kit::component::{
    button::{Button, ButtonVariants},
    command::{Command, CommandItem, CommandState},
    dialog::{DialogAction, DialogClose, DialogFooter},
    input::{Editor, EditorState, Input, InputEvent, InputState},
    menu::DropdownMenu,
    notification::{Notification, NotificationType},
    status_bar::StatusBar,
    tab::{Tab, TabBar},
    toolbar::Toolbar,
    ActiveTheme, IconName, Selectable, Sizable, StyledExt, TitleBar, WindowExt,
};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::*;
use raw_window_handle::{HasWindowHandle, RawWindowHandle};
use std::sync::{
    atomic::{AtomicU64, Ordering},
    Arc,
};
use std::time::Duration;

actions!(
    nxnote,
    [
        Save,
        QuickSwitch,
        NewNote,
        RenameNote,
        DeleteNote,
        Settings,
        TogglePreview,
        NextTab,
        PreviousTab,
        CloseTab,
        Quit,
        LearnTitle,
        BlockForeground,
        ToggleTop,
        ZoomIn,
        ZoomOut,
        MoveLineUp,
        MoveLineDown,
        CopyLineUp,
        CopyLineDown,
        DeleteLines,
        Tab1,
        Tab2,
        Tab3,
        Tab4,
        Tab5,
        Tab6,
        Tab7,
        Tab8,
        Tab9
    ]
);

pub fn bind_keys(cx: &mut App) {
    cx.bind_keys([
        KeyBinding::new("ctrl-s", Save, Some("NxNote")),
        KeyBinding::new("ctrl-p", QuickSwitch, Some("NxNote")),
        KeyBinding::new("ctrl-n", NewNote, Some("NxNote")),
        KeyBinding::new("ctrl-shift-p", TogglePreview, Some("NxNote")),
        KeyBinding::new("ctrl-pagedown", NextTab, Some("NxNote")),
        KeyBinding::new("ctrl-pageup", PreviousTab, Some("NxNote")),
        KeyBinding::new("ctrl-tab", NextTab, Some("NxNote")),
        KeyBinding::new("ctrl-shift-tab", PreviousTab, Some("NxNote")),
        KeyBinding::new("ctrl-w", CloseTab, Some("NxNote")),
        KeyBinding::new("ctrl-q", Quit, Some("NxNote")),
        KeyBinding::new("ctrl-,", Settings, Some("NxNote")),
        KeyBinding::new("ctrl-=", ZoomIn, Some("NxNote")),
        KeyBinding::new("ctrl--", ZoomOut, Some("NxNote")),
        KeyBinding::new("ctrl-1", Tab1, Some("NxNote")),
        KeyBinding::new("ctrl-2", Tab2, Some("NxNote")),
        KeyBinding::new("ctrl-3", Tab3, Some("NxNote")),
        KeyBinding::new("ctrl-4", Tab4, Some("NxNote")),
        KeyBinding::new("ctrl-5", Tab5, Some("NxNote")),
        KeyBinding::new("ctrl-6", Tab6, Some("NxNote")),
        KeyBinding::new("ctrl-7", Tab7, Some("NxNote")),
        KeyBinding::new("ctrl-8", Tab8, Some("NxNote")),
        KeyBinding::new("ctrl-9", Tab9, Some("NxNote")),
        KeyBinding::new(
            "alt-up",
            MoveLineUp,
            Some("NxNote && Input && mode == editor"),
        ),
        KeyBinding::new(
            "alt-down",
            MoveLineDown,
            Some("NxNote && Input && mode == editor"),
        ),
        KeyBinding::new(
            "alt-shift-up",
            CopyLineUp,
            Some("NxNote && Input && mode == editor"),
        ),
        KeyBinding::new(
            "alt-shift-down",
            CopyLineDown,
            Some("NxNote && Input && mode == editor"),
        ),
        KeyBinding::new(
            "ctrl-shift-k",
            DeleteLines,
            Some("NxNote && Input && mode == editor"),
        ),
    ]);
}

pub struct NxNoteApp {
    pub session: Session,
    editor: Entity<EditorState>,
    editor_events: Subscription,
    editor_observer: Subscription,
    save_task: Option<Task<()>>,
    _events: Task<()>,
    _tray: Option<crate::tray::TrayHandle>,
    hotkey: Option<crate::hotkey::HotkeyHandle>,
    poll_ms: Arc<AtomicU64>,
    hwnd: isize,
    hidden: bool,
    preview: bool,
    title_hover: bool,
    status: String,
    focus: FocusHandle,
    replacing: bool,
}

impl NxNoteApp {
    pub fn new(
        session: Session,
        hidden: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let editor = Self::make_editor(&session.text, window, cx);
        let (editor_events, editor_observer) = Self::watch_editor(&editor, window, cx);
        let hwnd = match HasWindowHandle::window_handle(window) {
            Ok(handle) => match handle.as_raw() {
                RawWindowHandle::Win32(h) => h.hwnd.get(),
                _ => 0,
            },
            Err(_) => 0,
        };
        let (tx, rx) = async_channel::unbounded();
        crate::hotkey::spawn_listener(tx.clone());
        let poll_ms = Arc::new(AtomicU64::new(session.cfg.poll_interval_ms.max(100)));
        crate::watcher::spawn(poll_ms.clone(), tx.clone());
        let mut status = "就绪".to_string();
        let hotkey = match crate::hotkey::install(&session.cfg.hotkey, &session.cfg.hotkey_capture)
        {
            Ok(h) => {
                if let Some(error) = &h.capture_error {
                    status = error.clone();
                }
                Some(h)
            }
            Err(e) => {
                status = format!("热键注册失败: {e}");
                None
            }
        };
        let tray = match crate::tray::install(tx) {
            Ok(t) => Some(t),
            Err(e) => {
                status = format!("托盘初始化失败: {e}");
                None
            }
        };
        let events = cx.spawn_in(window, async move |view, cx| {
            while let Ok(event) = rx.recv().await {
                if cx
                    .update(|window, cx| {
                        if let Some(view) = view.upgrade() {
                            view.update(cx, |app, cx| app.platform_event(event, window, cx));
                        }
                    })
                    .is_err()
                {
                    break;
                }
            }
        });
        let appearance_view = cx.weak_entity();
        window
            .observe_window_appearance(move |window, cx| {
                if let Some(view) = appearance_view.upgrade() {
                    view.update(cx, |app, cx| {
                        if app.session.cfg.theme_mode == theme::ThemeMode::System {
                            theme::apply(&app.session.cfg, Some(window), cx);
                            cx.notify();
                        }
                    });
                }
            })
            .detach();
        if !hidden {
            editor.update(cx, |state, cx| state.focus(window, cx));
        }
        let weak = cx.weak_entity();
        window.on_window_should_close(cx, move |window, cx| {
            if let Some(view) = weak.upgrade() {
                view.update(cx, |app, cx| {
                    app.request_close(window, cx);
                });
            }
            false
        });
        platform::set_always_on_top(hwnd, session.cfg.always_on_top);
        let mut app = Self {
            session,
            editor,
            editor_events,
            editor_observer,
            save_task: None,
            _events: events,
            _tray: tray,
            hotkey,
            poll_ms,
            hwnd,
            hidden,
            preview: false,
            title_hover: false,
            status,
            focus: cx.focus_handle(),
            replacing: false,
        };
        // Never leave an unreachable hidden process when both recovery mechanisms failed.
        if hidden && app._tray.is_none() && app.hotkey.is_none() {
            platform::set_visible(hwnd, true);
            app.hidden = false;
        }
        app
    }

    fn make_editor(text: &str, window: &mut Window, cx: &mut Context<Self>) -> Entity<EditorState> {
        cx.new(|cx| {
            EditorState::new(window, cx)
                .language("markdown")
                .line_number(true)
                .soft_wrap(true)
                .auto_close(false)
                .smart_indent(false)
                .default_value(text.to_string())
        })
    }

    fn watch_editor(
        editor: &Entity<EditorState>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> (Subscription, Subscription) {
        let events = cx.subscribe_in(
            editor,
            window,
            |app, editor, event: &InputEvent, window, cx| {
                if matches!(
                    event,
                    InputEvent::PressEnter {
                        secondary: false,
                        shift: false
                    }
                ) {
                    let text = editor.read(cx).value().to_string();
                    let cursor = editor.read(cx).selected_range().end;
                    if let Some((range, replacement)) =
                        crate::editor_support::continuation_edit(&text, cursor)
                    {
                        editor.update(cx, |state, cx| {
                            state.set_selected_range(range, cx);
                            state.replace(replacement, window, cx);
                        });
                    }
                }
                if matches!(event, InputEvent::Change | InputEvent::PressEnter { .. })
                    && !app.replacing
                {
                    app.session.edit(editor.read(cx).value().to_string());
                    app.status = if app.session.dirty {
                        "未保存"
                    } else {
                        "已保存"
                    }
                    .into();
                    app.schedule_save(window, cx);
                    cx.notify();
                }
            },
        );
        let observer = cx.observe(editor, |_, _, cx| cx.notify());
        (events, observer)
    }

    fn reload_editor(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.save_task = None;
        self.editor = Self::make_editor(&self.session.text, window, cx);
        (self.editor_events, self.editor_observer) = Self::watch_editor(&self.editor, window, cx);
        if !self.hidden {
            if self.preview {
                self.focus.focus(window, cx);
            } else {
                self.editor.update(cx, |state, cx| state.focus(window, cx));
            }
        }
        cx.notify();
    }

    fn schedule_save(&mut self, window: &Window, cx: &mut Context<Self>) {
        let delay = Duration::from_millis(self.session.cfg.autosave_delay_ms.max(200));
        self.save_task = Some(cx.spawn_in(window, async move |view, cx| {
            cx.background_executor().timer(delay).await;
            let _ = cx.update(|window, cx| {
                if let Some(view) = view.upgrade() {
                    view.update(cx, |app, cx| {
                        app.save(window, cx);
                    });
                }
            });
        }));
    }

    fn error(
        &mut self,
        error: impl std::fmt::Display,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.editor.read(cx).value().as_ref() != self.session.text.as_str() {
            self.reload_editor(window, cx);
        }
        self.status = format!("{error}");
        window.push_notification(
            Notification::new()
                .message(self.status.clone())
                .with_type(NotificationType::Error)
                .autohide(false),
            cx,
        );
        if self.hidden {
            self.show(window, cx);
        }
        cx.notify();
    }

    fn save(&mut self, window: &mut Window, cx: &mut Context<Self>) -> bool {
        match self.session.save() {
            Ok(()) => {
                self.status = "已保存".into();
                cx.notify();
                true
            }
            Err(e) => {
                self.error(format!("保存失败: {e:#}"), window, cx);
                false
            }
        }
    }

    fn remember_window(&mut self, window: &Window) {
        if !self.hidden {
            if let WindowBounds::Windowed(bounds) = window.window_bounds() {
                self.session.cfg.window_pos =
                    Some([bounds.origin.x.into(), bounds.origin.y.into()]);
                self.session.cfg.window_width = bounds.size.width.into();
                self.session.cfg.window_height = bounds.size.height.into();
            }
        }
    }

    fn show(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        platform::set_visible(self.hwnd, true);
        self.hidden = false;
        window.activate_window();
        if self.preview {
            self.focus.focus(window, cx);
        } else {
            self.editor.update(cx, |state, cx| state.focus(window, cx));
        }
        cx.notify();
    }

    fn hide(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.remember_window(window);
        if let Err(e) = self.session.persist() {
            self.error(e, window, cx);
            return;
        }
        if self._tray.is_none() && self.hotkey.is_none() {
            self.error("托盘和热键均不可用，无法安全隐藏窗口", window, cx);
            return;
        }
        platform::set_visible(self.hwnd, false);
        self.hidden = true;
        cx.notify();
    }

    fn request_close(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.session.cfg.close_to_tray && self._tray.is_some() {
            self.hide(window, cx);
        } else {
            self.quit(window, cx);
        }
    }

    fn quit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.remember_window(window);
        match self.session.persist() {
            Ok(()) => cx.quit(),
            Err(e) => self.error(e, window, cx),
        }
    }

    fn platform_event(
        &mut self,
        event: PlatformEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match event {
            PlatformEvent::Toggle => {
                if self.hidden {
                    self.show(window, cx);
                } else {
                    self.hide(window, cx);
                }
            }
            PlatformEvent::Show => self.show(window, cx),
            PlatformEvent::Quit => self.quit(window, cx),
            PlatformEvent::Foreground(info) => {
                let previous = self.session.current.clone();
                let result = self.session.foreground(info);
                if previous != self.session.current {
                    self.reload_editor(window, cx);
                }
                if let Err(e) = result {
                    self.error(e, window, cx);
                }
            }
            PlatformEvent::Capture(text) => match self.session.capture(&text) {
                Ok(changed) => {
                    if changed {
                        self.replacing = true;
                        self.editor.update(cx, |state, cx| {
                            state.set_value(self.session.text.clone(), window, cx);
                            let end = self.session.text.len();
                            state.set_selected_range(end..end, cx);
                        });
                        self.replacing = false;
                    }
                    self.status = format!("已捕获 {} 字到速记本", text.trim_end().chars().count());
                    cx.notify();
                }
                Err(e) => {
                    if self.editor.read(cx).value().as_ref() != self.session.text.as_str() {
                        self.reload_editor(window, cx);
                    }
                    self.error(format!("捕获失败: {e:#}"), window, cx);
                }
            },
        }
    }

    fn switch(&mut self, tab: OpenTab, window: &mut Window, cx: &mut Context<Self>) {
        let previous = self.session.current.clone();
        let result = self.session.switch(tab);
        if previous != self.session.current {
            self.reload_editor(window, cx);
        }
        match result {
            Ok(true) => {
                self.status = "已保存".into();
                cx.notify();
            }
            Ok(false) => {}
            Err(e) => self.error(e, window, cx),
        }
    }

    fn activate_tab(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(tab) = self.session.tabs.get(index).cloned() {
            self.switch(tab, window, cx);
        }
    }

    fn cycle_tab(&mut self, delta: isize, window: &mut Window, cx: &mut Context<Self>) {
        let n = self.session.tabs.len() as isize;
        if n > 1 {
            self.activate_tab(
                (self.session.active_tab as isize + delta).rem_euclid(n) as usize,
                window,
                cx,
            );
        }
    }

    fn close_tab(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        match self.session.close_tab(index) {
            Ok(true) => self.reload_editor(window, cx),
            Ok(false) => cx.notify(),
            Err(e) => self.error(e, window, cx),
        }
    }

    fn quick_switch(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let notes = self.session.all_notes();
        let labels: Vec<String> = notes
            .iter()
            .map(|t| {
                format!(
                    "{} / {}",
                    self.session.display_name(&t.folder_key),
                    self.session.label(t)
                )
            })
            .collect();
        let state = cx.new(|cx| CommandState::new(window, cx));
        let weak = cx.weak_entity();
        window.open_dialog(cx, move |dialog, _, _| {
            let notes = notes.clone();
            let weak = weak.clone();
            dialog.title("切换笔记").w(px(440.)).child(
                Command::new(&state)
                    .placeholder("搜索应用或笔记…")
                    .bordered(false)
                    .items(labels.iter().map(|label| {
                        CommandItem::new()
                            .label(label.clone())
                            .icon(IconName::FileText)
                    }))
                    .on_confirm(move |index, window, cx| {
                        if let (Some(tab), Some(view)) =
                            (notes.get(index.row).cloned(), weak.upgrade())
                        {
                            window.close_dialog(cx);
                            view.update(cx, |app, cx| app.switch(tab, window, cx));
                        }
                    }),
            )
        });
    }

    fn note_dialog(&mut self, rename: bool, window: &mut Window, cx: &mut Context<Self>) {
        let value = if rename {
            self.session.current.note_name.clone()
        } else {
            String::new()
        };
        let input = cx.new(|cx| {
            InputState::new(window, cx)
                .default_value(value)
                .placeholder("笔记名称")
        });
        let initial_input = input.clone();
        window.on_next_frame(move |window, cx| {
            initial_input.read(cx).focus_handle(cx).focus(window, cx)
        });
        let weak = cx.weak_entity();
        window.open_dialog(cx, move |dialog, _, _| {
            let weak = weak.clone();
            let input_ok = input.clone();
            dialog
                .title(if rename {
                    "重命名笔记"
                } else {
                    "新建笔记"
                })
                .child(Input::new(&input))
                .footer(confirm_footer())
                .on_ok(move |_, window, cx: &mut App| {
                    let name = input_ok.read(cx).value().to_string();
                    weak.upgrade()
                        .map(|view| {
                            view.update(cx, |app, cx| {
                                let result = if rename {
                                    app.session.rename(&name)
                                } else {
                                    app.session.create(&name).map(|_| ())
                                };
                                match result {
                                    Ok(()) => {
                                        if !rename {
                                            app.reload_editor(window, cx);
                                        }
                                        cx.notify();
                                        true
                                    }
                                    Err(e) => {
                                        app.error(e, window, cx);
                                        false
                                    }
                                }
                            })
                        })
                        .unwrap_or(false)
                })
        });
    }

    fn delete_dialog(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let weak = cx.weak_entity();
        let label = self.session.label(&self.session.current);
        window.open_dialog(cx, move |dialog, _, _| {
            let weak = weak.clone();
            dialog
                .title("删除笔记")
                .child(format!("将「{label}」移入回收目录？"))
                .footer(confirm_footer())
                .on_ok(move |_, window, cx| {
                    weak.upgrade()
                        .map(|view| {
                            view.update(cx, |app, cx| match app.session.delete() {
                                Ok(()) => {
                                    app.reload_editor(window, cx);
                                    true
                                }
                                Err(e) => {
                                    app.error(e, window, cx);
                                    false
                                }
                            })
                        })
                        .unwrap_or(false)
                })
        });
    }

    fn learn_title(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(info) = self.session.foreground.clone() else {
            self.error("尚未检测到前台应用", window, cx);
            return;
        };
        let sep = cx.new(|cx| InputState::new(window, cx).default_value(" - "));
        let index = cx.new(|cx| InputState::new(window, cx).default_value("0"));
        let weak = cx.weak_entity();
        window.open_dialog(cx, move |dialog, _, _| {
            let sep_ok = sep.clone();
            let index_ok = index.clone();
            let info = info.clone();
            let weak = weak.clone();
            dialog
                .title("学习标题规则")
                .w(px(480.))
                .child(
                    div()
                        .v_flex()
                        .gap_3()
                        .child(info.title.clone())
                        .child("分隔符")
                        .child(Input::new(&sep))
                        .child("取第几段（从 0 开始）")
                        .child(Input::new(&index)),
                )
                .footer(confirm_footer())
                .on_ok(move |_, window, cx: &mut App| {
                    let sep = sep_ok.read(cx).value().to_string();
                    let number = index_ok.read(cx).value().parse::<usize>();
                    weak.upgrade()
                        .map(|view| {
                            view.update(cx, |app, cx| {
                                let Ok(index) = number else {
                                    app.error("段编号必须是非负整数", window, cx);
                                    return false;
                                };
                                let rule = storage::TitleRule::SplitIndex {
                                    sep: sep.clone(),
                                    index,
                                };
                                if sep.is_empty() || rule.extract(&info.title).is_none() {
                                    app.error("规则不能提取有效标题", window, cx);
                                    return false;
                                }
                                let folder = storage::folder_key_for(&info.exe_path);
                                let Some(entry) = app.session.index.apps.get_mut(&folder) else {
                                    app.error("应用索引尚未就绪", window, cx);
                                    return false;
                                };
                                entry.title_rule = Some(rule);
                                match app.session.index.save() {
                                    Ok(()) => true,
                                    Err(e) => {
                                        app.error(e, window, cx);
                                        false
                                    }
                                }
                            })
                        })
                        .unwrap_or(false)
                })
        });
    }

    fn block_foreground(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(info) = self.session.foreground.clone() else {
            return;
        };
        let path = info.exe_path.to_string_lossy().to_string();
        if !self.session.cfg.blocked_apps.contains(&path) {
            self.session.cfg.blocked_apps.push(path);
        }
        if let Err(e) = self.session.cfg.save() {
            self.error(e, window, cx);
        }
        if self.session.cfg.auto_follow_foreground {
            self.switch(
                OpenTab {
                    folder_key: storage::GLOBAL_FOLDER.into(),
                    note_name: storage::SCRATCH_NOTE.into(),
                },
                window,
                cx,
            );
        }
    }

    fn open_settings(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let cfg = self.session.cfg.clone();
        let weak = cx.weak_entity();
        let main = Window::window_handle(window);
        let result = crate::settings_ui::open(
            cfg,
            move |cfg, _, cx| {
                main.update(cx, |_, window, cx| {
                    weak.upgrade()
                        .map(|view| view.update(cx, |app, cx| app.apply_config(cfg, window, cx)))
                        .unwrap_or(false)
                })
                .unwrap_or(false)
            },
            cx,
        );
        match result {
            Ok(view) => {
                cx.observe_release_in(&view, window, |app, _, window, cx| {
                    if !app.hidden {
                        window.activate_window();
                        if app.preview {
                            app.focus.focus(window, cx);
                        } else {
                            app.editor.update(cx, |state, cx| state.focus(window, cx));
                        }
                    }
                })
                .detach();
            }
            Err(e) => self.error(e, window, cx),
        }
    }

    fn apply_config(
        &mut self,
        mut cfg: Config,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        let old = self.session.cfg.clone();
        if let Err(error) = crate::fonts::install(&cfg, cx) {
            self.error(error, window, cx);
            return false;
        }
        let changed_hotkey = old.hotkey != cfg.hotkey || old.hotkey_capture != cfg.hotkey_capture;
        if changed_hotkey {
            self.hotkey = None;
            match crate::hotkey::install(&cfg.hotkey, &cfg.hotkey_capture) {
                Ok(h) => {
                    if let Some(error) = h.capture_error.clone() {
                        drop(h);
                        self.hotkey = crate::hotkey::install(&old.hotkey, &old.hotkey_capture).ok();
                        self.error(error, window, cx);
                        return false;
                    }
                    self.hotkey = Some(h);
                }
                Err(e) => {
                    self.hotkey = crate::hotkey::install(&old.hotkey, &old.hotkey_capture).ok();
                    self.error(e, window, cx);
                    return false;
                }
            }
        }
        if old.autostart != cfg.autostart {
            if let Err(e) = crate::autostart::set_enabled(cfg.autostart) {
                if changed_hotkey {
                    self.hotkey = None;
                    self.hotkey = crate::hotkey::install(&old.hotkey, &old.hotkey_capture).ok();
                }
                self.error(e, window, cx);
                return false;
            }
        }
        cfg.open_tabs = self.session.tabs.clone();
        cfg.active_tab = self.session.active_tab;
        if let Err(e) = cfg.save() {
            if changed_hotkey {
                self.hotkey = None;
                self.hotkey = crate::hotkey::install(&old.hotkey, &old.hotkey_capture).ok();
            }
            if old.autostart != cfg.autostart {
                let _ = crate::autostart::set_enabled(old.autostart);
            }
            self.error(e, window, cx);
            return false;
        }
        self.poll_ms
            .store(cfg.poll_interval_ms.max(100), Ordering::Relaxed);
        if old.window_width != cfg.window_width || old.window_height != cfg.window_height {
            window.resize(size(
                px(cfg.window_width.max(220.)),
                px(cfg.window_height.max(200.)),
            ));
        }
        platform::set_always_on_top(self.hwnd, cfg.always_on_top);
        theme::apply(&cfg, Some(window), cx);
        self.session.cfg = cfg;
        if self.session.cfg.auto_follow_foreground {
            if let Some(info) = self.session.foreground.clone() {
                self.platform_event(PlatformEvent::Foreground(info), window, cx);
            }
        }
        if self.session.dirty {
            self.schedule_save(window, cx);
        }
        cx.notify();
        true
    }

    fn edit_lines(
        &mut self,
        action: crate::editor_support::LineAction,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.preview || !self.editor.focus_handle(cx).is_focused(window) {
            return;
        }
        let text = self.editor.read(cx).value().to_string();
        let selection = self.editor.read(cx).selected_range();
        if let Some((text, selection)) = crate::editor_support::edit_lines(&text, selection, action)
        {
            self.editor.update(cx, |state, cx| {
                state.replace_all(text, window, cx);
                state.set_selected_range(selection, cx);
            });
        }
    }

    fn toggle_preview(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.preview = !self.preview;
        if self.preview {
            self.focus.focus(window, cx);
        } else {
            self.editor.update(cx, |state, cx| state.focus(window, cx));
        }
        cx.notify();
    }

    fn zoom(&mut self, delta: f32, window: &mut Window, cx: &mut Context<Self>) {
        self.session.cfg.font_size = (self.session.cfg.font_size + delta).clamp(10., 24.);
        theme::apply(&self.session.cfg, Some(window), cx);
        if let Err(e) = self.session.cfg.save() {
            self.error(e, window, cx);
        }
        cx.notify();
    }
}

impl Render for NxNoteApp {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let label = format!(
            "{} / {}",
            self.session.display_name(&self.session.current.folder_key),
            self.session.label(&self.session.current)
        );
        let toolbar = Toolbar::new("note-toolbar")
            .xsmall()
            .child(
                Button::new("pin")
                    .icon(IconName::FileText)
                    .selected(self.session.pinned)
                    .tooltip("钉住当前笔记")
                    .on_click(cx.listener(|app, _, _, cx| {
                        app.session.pinned = !app.session.pinned;
                        cx.notify();
                    })),
            )
            .content(
                Button::new("note-menu")
                    .ghost()
                    .xsmall()
                    .label(label)
                    .tooltip("笔记操作")
                    .dropdown_menu(move |menu, _, _| {
                        menu.menu("切换笔记…", Box::new(QuickSwitch))
                            .separator()
                            .menu("新建笔记", Box::new(NewNote))
                            .menu("重命名", Box::new(RenameNote))
                            .menu("删除当前笔记", Box::new(DeleteNote))
                            .separator()
                            .menu("学习标题规则…", Box::new(LearnTitle))
                            .menu("拉黑前台应用", Box::new(BlockForeground))
                            .separator()
                            .menu("窗口置顶", Box::new(ToggleTop))
                            .menu("退出", Box::new(Quit))
                    }),
            )
            .content(div().flex_1())
            .child(
                Button::new("preview")
                    .icon(IconName::Eye)
                    .selected(self.preview)
                    .tooltip("编辑 / 预览 (Ctrl+Shift+P)")
                    .on_click(cx.listener(|app, _, window, cx| {
                        app.toggle_preview(window, cx);
                    })),
            )
            .child(
                Button::new("settings")
                    .icon(IconName::Settings)
                    .tooltip("设置")
                    .on_click(cx.listener(|app, _, window, cx| app.open_settings(window, cx))),
            );
        let tabs = TabBar::new("note-tabs")
            .small()
            .selected_index(self.session.active_tab)
            .menu(true)
            .max_width(px(150.))
            .on_click(cx.listener(|app, index, window, cx| app.activate_tab(*index, window, cx)))
            .children(self.session.tabs.iter().enumerate().map(|(index, tab)| {
                Tab::new().label(self.session.label(tab)).suffix(
                    Button::new(("close-tab", index))
                        .ghost()
                        .xsmall()
                        .icon(IconName::Close)
                        .tooltip("关闭标签")
                        .on_click(cx.listener(move |app, _, window, cx| {
                            cx.stop_propagation();
                            app.close_tab(index, window, cx);
                        })),
                )
            }));
        let paste_editor = self.editor.clone();
        let body = if self.preview {
            div()
                .id("markdown-preview")
                .flex_1()
                .min_h_0()
                .overflow_y_scroll()
                .p_3()
                .child(
                    gpui_kit::base::TextView::markdown("note-preview", self.session.text.clone())
                        .style(theme::preview_style(&self.session.cfg, cx))
                        .selectable(true),
                )
                .into_any_element()
        } else {
            div()
                .flex_1()
                .min_h_0()
                .child(
                    Editor::new(&self.editor)
                        .h_full()
                        .appearance(false)
                        .bordered(false)
                        .aria_label("笔记编辑器")
                        .on_paste(move |item, window, cx| {
                            if let Some(text) =
                                item.text().filter(|t| crate::editor_support::image_url(t))
                            {
                                paste_editor.update(cx, |state, cx| {
                                    state.insert(format!("![图片]({})\n", text.trim()), window, cx)
                                });
                                true
                            } else {
                                false
                            }
                        }),
                )
                .into_any_element()
        };
        let cursor = self.editor.read(cx).cursor_position();
        let status = StatusBar::new()
            .text_xs()
            .left(self.status.clone())
            .right(format!(
                "{} 字 · {}:{}",
                self.session.text.chars().count(),
                cursor.line + 1,
                cursor.character + 1
            ));
        let title_visible = !self.session.cfg.autohide_title_bar || self.title_hover;
        div()
            .id("nxnote-main")
            .key_context("NxNote")
            .track_focus(&self.focus)
            .v_flex()
            .size_full()
            .bg(cx.theme().background)
            .text_color(cx.theme().foreground)
            .font_family(cx.theme().font_family.clone())
            .text_size(cx.theme().font_size)
            .on_action(cx.listener(|app, _: &Save, window, cx| {
                app.save(window, cx);
            }))
            .on_action(cx.listener(|app, _: &QuickSwitch, window, cx| app.quick_switch(window, cx)))
            .on_action(
                cx.listener(|app, _: &NewNote, window, cx| app.note_dialog(false, window, cx)),
            )
            .on_action(
                cx.listener(|app, _: &RenameNote, window, cx| app.note_dialog(true, window, cx)),
            )
            .on_action(cx.listener(|app, _: &DeleteNote, window, cx| app.delete_dialog(window, cx)))
            .on_action(cx.listener(|app, _: &Settings, window, cx| app.open_settings(window, cx)))
            .on_action(cx.listener(|app, _: &LearnTitle, window, cx| app.learn_title(window, cx)))
            .on_action(
                cx.listener(|app, _: &BlockForeground, window, cx| {
                    app.block_foreground(window, cx)
                }),
            )
            .on_action(cx.listener(|app, _: &MoveLineUp, w, c| {
                app.edit_lines(crate::editor_support::LineAction::MoveUp, w, c)
            }))
            .on_action(cx.listener(|app, _: &MoveLineDown, w, c| {
                app.edit_lines(crate::editor_support::LineAction::MoveDown, w, c)
            }))
            .on_action(cx.listener(|app, _: &CopyLineUp, w, c| {
                app.edit_lines(crate::editor_support::LineAction::CopyUp, w, c)
            }))
            .on_action(cx.listener(|app, _: &CopyLineDown, w, c| {
                app.edit_lines(crate::editor_support::LineAction::CopyDown, w, c)
            }))
            .on_action(cx.listener(|app, _: &DeleteLines, w, c| {
                app.edit_lines(crate::editor_support::LineAction::Delete, w, c)
            }))
            .on_action(cx.listener(|app, _: &Quit, window, cx| app.quit(window, cx)))
            .on_action(cx.listener(|app, _: &TogglePreview, window, cx| {
                app.toggle_preview(window, cx);
            }))
            .on_action(cx.listener(|app, _: &ToggleTop, window, cx| {
                app.session.cfg.always_on_top = !app.session.cfg.always_on_top;
                platform::set_always_on_top(app.hwnd, app.session.cfg.always_on_top);
                if let Err(e) = app.session.cfg.save() {
                    app.error(e, window, cx);
                }
                cx.notify();
            }))
            .on_action(cx.listener(|app, _: &NextTab, window, cx| app.cycle_tab(1, window, cx)))
            .on_action(
                cx.listener(|app, _: &PreviousTab, window, cx| app.cycle_tab(-1, window, cx)),
            )
            .on_action(cx.listener(|app, _: &CloseTab, window, cx| {
                app.close_tab(app.session.active_tab, window, cx)
            }))
            .on_action(cx.listener(|app, _: &ZoomIn, window, cx| app.zoom(1., window, cx)))
            .on_action(cx.listener(|app, _: &ZoomOut, window, cx| app.zoom(-1., window, cx)))
            .on_action(cx.listener(|app, _: &Tab1, w, c| app.activate_tab(0, w, c)))
            .on_action(cx.listener(|app, _: &Tab2, w, c| app.activate_tab(1, w, c)))
            .on_action(cx.listener(|app, _: &Tab3, w, c| app.activate_tab(2, w, c)))
            .on_action(cx.listener(|app, _: &Tab4, w, c| app.activate_tab(3, w, c)))
            .on_action(cx.listener(|app, _: &Tab5, w, c| app.activate_tab(4, w, c)))
            .on_action(cx.listener(|app, _: &Tab6, w, c| app.activate_tab(5, w, c)))
            .on_action(cx.listener(|app, _: &Tab7, w, c| app.activate_tab(6, w, c)))
            .on_action(cx.listener(|app, _: &Tab8, w, c| app.activate_tab(7, w, c)))
            .on_action(cx.listener(|app, _: &Tab9, w, c| app.activate_tab(8, w, c)))
            .child(
                div()
                    .id("window-header")
                    .v_flex()
                    .flex_shrink_0()
                    .on_hover(cx.listener(|app, hovered, _, cx| {
                        app.title_hover = *hovered;
                        cx.notify();
                    }))
                    .when(title_visible, |header| {
                        header.child(TitleBar::new().child("NxNote").on_close_window(
                            cx.listener(|app, _, window, cx| app.request_close(window, cx)),
                        ))
                    })
                    .when(!title_visible, |header| header.child(div().h(px(3.))))
                    .child(toolbar),
            )
            .child(tabs)
            .child(body)
            .child(status)
    }
}

fn confirm_footer() -> DialogFooter {
    DialogFooter::new()
        .child(DialogClose::new().trigger(|button| button.label("取消")))
        .child(DialogAction::new().child(Button::new("confirm").primary().label("确认")))
}
