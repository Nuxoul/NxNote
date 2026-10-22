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
    input::{Input, InputEvent, InputState},
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

#[derive(Clone)]
struct DragNoteTab { tab: OpenTab, label: SharedString }
impl Render for DragNoteTab {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div().px_3().py_2().rounded_md().border_1().border_color(cx.theme().border)
            .bg(cx.theme().background).text_color(cx.theme().foreground)
            .text_size(cx.theme().font_size).child(self.label.clone())
    }
}

actions!(
    nxnote,
    [
        Save,
        QuickSwitch,
        NewNote,
        RenameNote,
        DeleteNote,
        Settings,
        NextTab,
        PreviousTab,
        CloseTab,
        Quit,
        LearnTitle,
        BlockForeground,
        ToggleTop,
        ZoomIn,
        ZoomOut,
        MoveBlockUp,
        MoveBlockDown,
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
        KeyBinding::new("alt-up", MoveBlockUp, Some("NxNoteEditor")),
        KeyBinding::new("alt-down", MoveBlockDown, Some("NxNoteEditor")),
    ]);
}

pub struct NxNoteApp {
    pub session: Session,
    editor: Entity<crate::note_editor::NoteEditor>,
    editor_events: Subscription,
    editor_observer: Subscription,
    save_task: Option<Task<()>>,
    _events: Task<()>,
    _tray: Option<crate::tray::TrayHandle>,
    hotkey: Option<crate::hotkey::HotkeyHandle>,
    poll_ms: Arc<AtomicU64>,
    hwnd: isize,
    hidden: bool,
    title_hover: bool,
    status: String,
    focus: FocusHandle,
    replacing: bool,
    tab_drop: Option<(OpenTab, bool)>,
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
            title_hover: false,
            status,
            focus: cx.focus_handle(),
            replacing: false,
            tab_drop: None,
        };
        // Never leave an unreachable hidden process when both recovery mechanisms failed.
        if hidden && app._tray.is_none() && app.hotkey.is_none() {
            platform::set_visible(hwnd, true);
            app.hidden = false;
        }
        app
    }

    fn make_editor(
        text: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Entity<crate::note_editor::NoteEditor> {
        cx.new(|cx| crate::note_editor::NoteEditor::new(text, window, cx))
    }

    fn watch_editor(
        editor: &Entity<crate::note_editor::NoteEditor>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> (Subscription, Subscription) {
        let events = cx.subscribe_in(
            editor,
            window,
            |app, editor, event: &InputEvent, window, cx| {
                if matches!(event, InputEvent::Change | InputEvent::Blur)
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
            self.editor.update(cx, |state, cx| state.focus(window, cx));
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
        self.editor.update(cx, |state, cx| state.focus(window, cx));
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
                            state.select_end(cx);
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
            Ok(false) => {
                let editor = self.editor.clone();
                window.on_next_frame(move |window, cx| {
                    editor.update(cx, |editor, cx| editor.focus(window, cx));
                });
            }
            Err(e) => self.error(e, window, cx),
        }
    }

    fn drop_tab(&mut self, drag: &DragNoteTab, target: &OpenTab, window: &mut Window, cx: &mut Context<Self>) {
        let after = self.tab_drop.take().filter(|(tab, _)| tab == target).is_some_and(|(_, after)| after);
        match self.session.reorder_tab(&drag.tab, target, after) {
            Ok(_) => { self.editor.update(cx, |editor, cx| editor.focus(window, cx)); cx.notify(); }
            Err(error) => self.error(format!("标签顺序保存失败: {error:#}"), window, cx),
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
                        app.editor.update(cx, |state, cx| state.focus(window, cx));
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

    fn move_block(
        &mut self,
        down: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.editor.focus_handle(cx).is_focused(window) {
            return;
        }
        self.editor.update(cx, |state, cx| state.move_block(down, window, cx));
    }


    fn zoom(&mut self, delta: f32, window: &mut Window, cx: &mut Context<Self>) {
        self.session.cfg.font_size = (self.session.cfg.font_size.round() + delta).clamp(10., 24.);
        theme::apply(&self.session.cfg, Some(window), cx);
        if let Err(e) = self.session.cfg.save() {
            self.error(e, window, cx);
        }
        cx.notify();
    }
}

impl Render for NxNoteApp {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let ui_size = cx.theme().font_size;
        _window.set_rem_size(ui_size);
        let label = format!(
            "{} / {}",
            self.session.display_name(&self.session.current.folder_key),
            self.session.label(&self.session.current)
        );
        let toolbar = Toolbar::new("note-toolbar")
            .small()
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
                    .small()
                    .text_size(ui_size)
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
                Button::new("settings")
                    .icon(IconName::Settings)
                    .tooltip("设置")
                    .on_click(cx.listener(|app, _, window, cx| app.open_settings(window, cx))),
            );
        let tabs = TabBar::new("note-tabs")
            .large()
            .selected_index(self.session.active_tab)
            .menu(true)
            .max_width(px(150.))
            .on_click(cx.listener(|app, index, window, cx| app.activate_tab(*index, window, cx)))
            .children(self.session.tabs.iter().enumerate().map(|(index, tab)| {
                let target = tab.clone();
                let move_target = target.clone();
                let drag = DragNoteTab { tab: tab.clone(), label: self.session.label(tab).into() };
                let drop_after = self.tab_drop.as_ref().filter(|(candidate, _)| candidate == tab).is_some_and(|(_, after)| *after);
                let drag_owner = cx.weak_entity();
                Tab::new().label(self.session.label(tab)).text_size(ui_size)
                    .on_drag(drag, move |drag, _, _, cx| {
                        cx.stop_propagation();
                        if let Some(owner) = drag_owner.upgrade() { owner.update(cx, |app, _| app.tab_drop = None); }
                        cx.new(|_| drag.clone())
                    })
                    .on_drag_move(cx.listener(move |app, event: &DragMoveEvent<DragNoteTab>, _, cx| {
                        let after = event.event.position.x > event.bounds.left() + event.bounds.size.width / 2.;
                        let next = (move_target.clone(), after);
                        if app.tab_drop.as_ref() != Some(&next) { app.tab_drop = Some(next); cx.notify(); }
                    }))
                    .drag_over::<DragNoteTab>(move |style, _, _, cx| {
                        let style = style.border_color(cx.theme().primary);
                        if drop_after { style.border_r_2().border_l_0() } else { style.border_l_2().border_r_0() }
                    })
                    .on_drop(cx.listener(move |app, drag: &DragNoteTab, window, cx| app.drop_tab(drag, &target, window, cx)))
                    .suffix(Button::new(("close-tab", index)).ghost().xsmall().icon(IconName::Close)
                        .tooltip("关闭标签")
                        .on_mouse_down(MouseButton::Left, |_, window, cx| { window.prevent_default(); cx.stop_propagation(); })
                        .on_click(cx.listener(move |app, _, window, cx| {
                            cx.stop_propagation(); app.close_tab(index, window, cx);
                        })))
            }));
        let body = div()
            .flex_1()
            .min_h_0()
            .child(self.editor.clone());
        let status_detail = format!("{} 字 · Markdown 块编辑", self.session.text.chars().count());
        let status = StatusBar::new()
            .text_size(ui_size)
            .left(self.status.clone())
            .right(status_detail);
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
            .on_action(cx.listener(|app, _: &MoveBlockUp, window, cx| {
                app.move_block(false, window, cx)
            }))
            .on_action(cx.listener(|app, _: &MoveBlockDown, window, cx| {
                app.move_block(true, window, cx)
            }))
            .on_action(cx.listener(|app, _: &Quit, window, cx| app.quit(window, cx)))
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
                        header.child(TitleBar::new().text_size(ui_size).child("NxNote").on_close_window(
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
