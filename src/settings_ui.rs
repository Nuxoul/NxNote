use std::{path::Path, rc::Rc};

use anyhow::anyhow;
use gpui_kit::component::{
    button::{Button, ButtonVariants as _},
    color_picker::{ColorPicker, ColorPickerEvent, ColorPickerState},
    combobox::{Combobox, ComboboxEvent, ComboboxState},
    form::{Field, Form},
    input::{Input, InputEvent, InputState, NumberInput, Textarea, TextareaState},
    notification::{Notification, NotificationType},
    searchable_list::{SearchableListItem, SearchableVec},
    switch::Switch,
    tab::{Tab, TabBar},
    ActiveTheme as _, StyledExt as _, IndexPath as ComponentIndexPath, TitleBar, WindowExt as _,
};
use gpui_kit::{
    div, prelude::*, px, size, App, Bounds, Context, Entity, Hsla, IntoElement, Render, Rgba,
    SharedString, Subscription, TitlebarOptions, Window, WindowBounds, WindowOptions,
};

use crate::{
    config::{Config, MdColors},
    fonts,
    theme::ThemeMode,
};

type FontState = ComboboxState<SearchableVec<FontChoice>>;
type ApplyCallback = dyn Fn(Config, &mut Window, &mut App) -> bool;

const COLOR_LABELS: [&str; 11] = [
    "正文",
    "标题",
    "粗体",
    "斜体",
    "行内代码",
    "代码背景",
    "链接",
    "引用文字",
    "引用边线",
    "列表标记",
    "语法标点",
];

#[derive(Clone)]
struct FontChoice {
    path: String,
    label: SharedString,
}

impl SearchableListItem for FontChoice {
    type Value = String;

    fn title(&self) -> SharedString {
        self.label.clone()
    }

    fn value(&self) -> &Self::Value {
        &self.path
    }

    fn matches(&self, query: &str) -> bool {
        let query = query.to_lowercase();
        self.label.to_lowercase().contains(&query) || self.path.to_lowercase().contains(&query)
    }
}

pub struct SettingsView {
    draft: Config,
    tab: usize,
    font_size: Entity<InputState>,
    ui_font_size: Entity<InputState>,
    line_height: Entity<InputState>,
    window_width: Entity<InputState>,
    window_height: Entity<InputState>,
    autosave_delay: Entity<InputState>,
    poll_interval: Entity<InputState>,
    hotkey: Entity<InputState>,
    hotkey_capture: Entity<InputState>,
    blocked_apps: Entity<TextareaState>,
    ui_fonts: Entity<FontState>,
    editor_fonts: Entity<FontState>,
    dark_colors: Vec<Entity<ColorPickerState>>,
    light_colors: Vec<Entity<ColorPickerState>>,
    on_apply: Rc<ApplyCallback>,
    _subscriptions: Vec<Subscription>,
    _font_loader: gpui_kit::Task<()>,
}

pub fn open(
    cfg: Config,
    on_apply: impl Fn(Config, &mut Window, &mut App) -> bool + 'static,
    cx: &mut App,
) -> anyhow::Result<Entity<SettingsView>> {
    let options = WindowOptions {
        window_bounds: Some(WindowBounds::Windowed(Bounds::centered(
            None,
            size(px(720.), px(650.)),
            cx,
        ))),
        titlebar: Some(TitlebarOptions {
            title: Some("NxNote 设置".into()),
            appears_transparent: true,
            ..Default::default()
        }),
        ..Default::default()
    };
    let callback: Rc<ApplyCallback> = Rc::new(on_apply);
    let (_, view) = gpui_kit::open_window(options, cx, move |window, cx| {
        let cfg = cfg.clone();
        let callback = callback.clone();
        cx.new(|cx| SettingsView::new(cfg, callback, window, cx))
    })
    .map_err(|error| anyhow!("无法打开设置窗口: {error}"))?;
    Ok(view)
}

impl SettingsView {
    fn new(
        draft: Config,
        on_apply: Rc<ApplyCallback>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let font_size = integer_font_state(draft.font_size.round(), window, cx);
        let ui_font_size = integer_font_state(draft.ui_font_size as f32, window, cx);
        let line_height = cx.new(|cx| InputState::new(window, cx)
            .default_value(format!("{:.1}", crate::config::normalize_line_height(draft.line_height)))
            .step(0.1).min(crate::config::MIN_LINE_HEIGHT as f64).max(crate::config::MAX_LINE_HEIGHT as f64)
            .mask_pattern(gpui_kit::base::input::MaskPattern::Number { separator: None, fraction: Some(1) }));
        let window_width = number_state(draft.window_width as f64, 220., 1600., 1., window, cx);
        let window_height = number_state(draft.window_height as f64, 200., 1600., 1., window, cx);
        let autosave_delay = number_state(
            draft.autosave_delay_ms as f64,
            200.,
            10_000.,
            50.,
            window,
            cx,
        );
        let poll_interval =
            number_state(draft.poll_interval_ms as f64, 100., 5_000., 50., window, cx);
        let hotkey = cx.new(|cx| InputState::new(window, cx).default_value(draft.hotkey.clone()));
        let hotkey_capture =
            cx.new(|cx| InputState::new(window, cx).default_value(draft.hotkey_capture.clone()));
        let blocked_apps = cx.new(|cx| {
            TextareaState::new(window, cx)
                .rows(7)
                .default_value(draft.blocked_apps.join("\n"))
        });
        let ui_fonts = font_state(&draft.ui_fonts, window, cx);
        let editor_fonts = font_state(&draft.editor_fonts, window, cx);
        let dark_colors = color_states(&draft.md_dark, window, cx);
        let light_colors = color_states(&draft.md_light, window, cx);

        let mut subscriptions = Vec::new();
        for state in [
            &font_size,
            &ui_font_size,
            &line_height,
            &window_width,
            &window_height,
            &autosave_delay,
            &poll_interval,
            &hotkey,
            &hotkey_capture,
        ] {
            subscriptions.push(cx.subscribe(state, |_: &mut Self, _, _: &InputEvent, cx| {
                cx.notify();
            }));
        }
        subscriptions.push(
            cx.subscribe(&blocked_apps, |_: &mut Self, _, _: &InputEvent, cx| {
                cx.notify()
            }),
        );
        subscriptions.push(cx.subscribe(
            &ui_fonts,
            |_: &mut Self, _, _: &ComboboxEvent<SearchableVec<FontChoice>>, cx| cx.notify(),
        ));
        subscriptions.push(cx.subscribe(
            &editor_fonts,
            |_: &mut Self, _, _: &ComboboxEvent<SearchableVec<FontChoice>>, cx| cx.notify(),
        ));
        for state in dark_colors.iter().chain(light_colors.iter()) {
            subscriptions.push(
                cx.subscribe(state, |_: &mut Self, _, _: &ColorPickerEvent, cx| {
                    cx.notify()
                }),
            );
        }

        let selected_paths: Vec<String> = draft
            .ui_fonts
            .iter()
            .chain(&draft.editor_fonts)
            .cloned()
            .collect();
        let load = cx.background_executor().spawn(async move {
            let mut paths = selected_paths;
            paths.extend(
                fonts::list_system_fonts()
                    .iter()
                    .map(|p| p.to_string_lossy().to_string()),
            );
            paths.sort();
            paths.dedup();
            paths
                .into_iter()
                .map(|path| {
                    let label = fonts::font_display_name(Path::new(&path));
                    (path, label)
                })
                .collect::<Vec<_>>()
        });
        let font_loader = cx.spawn_in(window, async move |view, cx| {
            let catalog = load.await;
            let _ = cx.update(|window, cx| {
                if let Some(view) = view.upgrade() {
                    view.update(cx, |settings, cx| {
                        for state in [&settings.ui_fonts, &settings.editor_fonts] {
                            let selected = state.read(cx).selected_values();
                            let choices = ordered_font_choices(&selected, &catalog);
                            state.update(cx, |state, cx| {
                                state.set_items(SearchableVec::new(choices), window, cx);
                                state.set_selected_values(&selected, window, cx);
                            });
                        }
                        cx.notify();
                    });
                }
            });
        });
        Self {
            draft,
            tab: 0,
            font_size,
            ui_font_size,
            line_height,
            window_width,
            window_height,
            autosave_delay,
            poll_interval,
            hotkey,
            hotkey_capture,
            blocked_apps,
            ui_fonts,
            editor_fonts,
            dark_colors,
            light_colors,
            on_apply,
            _subscriptions: subscriptions,
            _font_loader: font_loader,
        }
    }

    fn apply(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        match self.collected_config(cx) {
            Ok(config) => {
                if (self.on_apply)(config.clone(), window, cx) {
                    self.draft = config;
                    window.push_notification(
                        Notification::new()
                            .with_type(NotificationType::Success)
                            .message("设置已应用"),
                        cx,
                    );
                } else {
                    window.push_notification(
                        Notification::new()
                            .with_type(NotificationType::Error)
                            .message("设置未能保存；修改仍保留在此窗口中"),
                        cx,
                    );
                }
            }
            Err(error) => window.push_notification(
                Notification::new()
                    .with_type(NotificationType::Error)
                    .message(error.to_string()),
                cx,
            ),
        }
    }

    fn collected_config(&self, cx: &App) -> anyhow::Result<Config> {
        let font_size = read_font_size(&self.font_size, "正文字号", cx)?;
        let ui_font_size = read_font_size(&self.ui_font_size, "界面字号", cx)? as u16;
        let line_height = crate::config::normalize_line_height(read_number(
            &self.line_height, "正文行间距", crate::config::MIN_LINE_HEIGHT as f64,
            crate::config::MAX_LINE_HEIGHT as f64, cx,
        )? as f32);
        let window_width = read_number(&self.window_width, "默认宽度", 220., 1600., cx)? as f32;
        let window_height = read_number(&self.window_height, "默认高度", 200., 1600., cx)? as f32;
        let autosave_delay_ms =
            read_number(&self.autosave_delay, "保存延迟", 200., 10_000., cx)? as u64;
        let poll_interval_ms =
            read_number(&self.poll_interval, "前台轮询", 100., 5_000., cx)? as u64;

        let hotkey = self.hotkey.read(cx).value().trim().to_string();
        let hotkey_capture = self.hotkey_capture.read(cx).value().trim().to_string();
        if crate::hotkey::parse(&hotkey).is_none() {
            return Err(anyhow!("显示 / 隐藏热键格式无效"));
        }
        if crate::hotkey::parse(&hotkey_capture).is_none() {
            return Err(anyhow!("速记捕获热键格式无效"));
        }
        if hotkey.eq_ignore_ascii_case(&hotkey_capture) {
            return Err(anyhow!("两个全局热键不能重复"));
        }

        let mut blocked_apps = Vec::new();
        for line in self.blocked_apps.read(cx).value().lines() {
            let value = line.trim();
            if !value.is_empty()
                && !blocked_apps
                    .iter()
                    .any(|existing: &String| existing.eq_ignore_ascii_case(value))
            {
                blocked_apps.push(value.to_string());
            }
        }

        let mut config = self.draft.clone();
        config.font_size = font_size;
        config.ui_font_size = ui_font_size;
        config.line_height = line_height;
        config.window_width = window_width;
        config.window_height = window_height;
        config.autosave_delay_ms = autosave_delay_ms;
        config.poll_interval_ms = poll_interval_ms;
        config.hotkey = hotkey;
        config.hotkey_capture = hotkey_capture;
        config.blocked_apps = blocked_apps;
        config.ui_fonts = self.ui_fonts.read(cx).selected_values();
        config.editor_fonts = self.editor_fonts.read(cx).selected_values();
        config.md_dark = colors_from_states(&self.dark_colors, "深色", cx)?;
        config.md_light = colors_from_states(&self.light_colors, "浅色", cx)?;
        Ok(config)
    }

    fn render_general(&self, cx: &mut Context<Self>) -> impl IntoElement {
        Form::horizontal()
            .label_width(px(150.))
            .child(
                Field::new()
                    .label("主题")
                    .description("跟随系统或固定外观")
                    .child(
                        TabBar::new("theme-mode")
                            .segmented()
                            .selected_index(match self.draft.theme_mode {
                                ThemeMode::System => 0,
                                ThemeMode::Light => 1,
                                ThemeMode::Dark => 2,
                            })
                            .children([
                                Tab::new().label("系统"),
                                Tab::new().label("浅色"),
                                Tab::new().label("深色"),
                            ])
                            .on_click(cx.listener(|this, index, _, cx| {
                                this.draft.theme_mode = match *index {
                                    1 => ThemeMode::Light,
                                    2 => ThemeMode::Dark,
                                    _ => ThemeMode::System,
                                };
                                cx.notify();
                            })),
                    ),
            )
            .child(
                Field::new()
                    .label("界面字号")
                    .description("标题、菜单、标签和设置，整数 10–24 px")
                    .child(NumberInput::new(&self.ui_font_size).suffix("px")),
            )
            .child(
                Field::new()
                    .label("正文字号")
                    .description("笔记正文及 Markdown 排版基准，整数 10–24 px")
                    .child(NumberInput::new(&self.font_size).suffix("px")),
            )
            .child(
                Field::new()
                    .label("正文行间距")
                    .description("行高为字号的 1.0–2.5 倍，步长 0.1；默认 1.5，不改变段落间距")
                    .child(NumberInput::new(&self.line_height).suffix("倍")),
            )
            .child(
                Field::new()
                    .label("界面字体")
                    .description("可搜索、多选；选择顺序即回退顺序")
                    .child(
                        Combobox::new(&self.ui_fonts)
                            .placeholder("选择界面字体")
                            .search_placeholder("搜索字体或路径")
                            .menu_max_h(px(320.))
                            .w_full(),
                    ),
            )
            .child(
                Field::new()
                    .label("编辑字体")
                    .description("可搜索、多选；选择顺序即回退顺序")
                    .child(
                        Combobox::new(&self.editor_fonts)
                            .placeholder("选择编辑字体")
                            .search_placeholder("搜索字体或路径")
                            .menu_max_h(px(320.))
                            .w_full(),
                    ),
            )
            .child(Field::new().label("自动隐藏标题栏").child(settings_switch(
                "autohide-titlebar",
                self.draft.autohide_title_bar,
                cx.listener(|this, checked, _, cx| {
                    this.draft.autohide_title_bar = *checked;
                    cx.notify();
                }),
            )))
            .child(
                Field::new()
                    .label("默认宽度")
                    .child(NumberInput::new(&self.window_width).suffix("px")),
            )
            .child(
                Field::new()
                    .label("默认高度")
                    .child(NumberInput::new(&self.window_height).suffix("px")),
            )
            .child(Field::new().label("窗口始终置顶").child(settings_switch(
                "always-on-top",
                self.draft.always_on_top,
                cx.listener(|this, checked, _, cx| {
                    this.draft.always_on_top = *checked;
                    cx.notify();
                }),
            )))
    }

    fn render_behavior(&self, cx: &mut Context<Self>) -> impl IntoElement {
        Form::horizontal()
            .label_width(px(170.))
            .child(
                Field::new()
                    .label("开机自启")
                    .description("登录 Windows 后以隐藏态启动")
                    .child(settings_switch(
                        "autostart",
                        self.draft.autostart,
                        cx.listener(|this, checked, _, cx| {
                            this.draft.autostart = *checked;
                            cx.notify();
                        }),
                    )),
            )
            .child(
                Field::new()
                    .label("关闭最小化到托盘")
                    .child(settings_switch(
                        "close-to-tray",
                        self.draft.close_to_tray,
                        cx.listener(|this, checked, _, cx| {
                            this.draft.close_to_tray = *checked;
                            cx.notify();
                        }),
                    )),
            )
            .child(
                Field::new()
                    .label("随前台切换笔记")
                    .description("自动跟随当前前台应用")
                    .child(settings_switch(
                        "auto-follow",
                        self.draft.auto_follow_foreground,
                        cx.listener(|this, checked, _, cx| {
                            this.draft.auto_follow_foreground = *checked;
                            cx.notify();
                        }),
                    )),
            )
            .child(
                Field::new()
                    .label("保存延迟")
                    .description("停止输入后写盘等待，200–10000 ms")
                    .child(NumberInput::new(&self.autosave_delay).suffix("ms")),
            )
            .child(
                Field::new()
                    .label("前台轮询")
                    .description("检测前台应用间隔，100–5000 ms")
                    .child(NumberInput::new(&self.poll_interval).suffix("ms")),
            )
            .child(
                Field::new()
                    .label("应用黑名单")
                    .description("每行一个 exe 文件名或完整路径；空行会忽略")
                    .items_start()
                    .child(Textarea::new(&self.blocked_apps).w_full()),
            )
    }

    fn render_hotkeys(&self) -> impl IntoElement {
        Form::horizontal()
            .label_width(px(170.))
            .child(
                Field::new()
                    .label("显示 / 隐藏")
                    .description("例如 Ctrl+Alt+N")
                    .child(Input::new(&self.hotkey).w_full()),
            )
            .child(
                Field::new()
                    .label("速记捕获")
                    .description("例如 Ctrl+Alt+Shift+C")
                    .child(Input::new(&self.hotkey_capture).w_full()),
            )
            .child(Field::new().label_indent(false).child(
                div().text_sm().child(
                    "应用时校验热键格式；两个全局热键不能重复。实际系统重注册由主程序完成。",
                ),
            ))
    }

    fn render_colors(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let dark = color_form("深色主题 Markdown 颜色", &self.dark_colors);
        let light = color_form("浅色主题 Markdown 颜色", &self.light_colors);
        div().v_flex().gap_4()
            .child(div().flex().items_center().justify_between().gap_3()
                .child(div().text_sm().text_color(cx.theme().muted_foreground)
                    .child("选区、焦点与控件状态统一使用主题蓝；恢复默认后需点击应用。"))
                .child(Button::new("reset-document-colors").label("恢复新版默认配色")
                    .on_click(cx.listener(|this, _, window, cx| {
                        for (state, rgb) in this.dark_colors.iter().zip(md_color_values(&MdColors::default_dark())) {
                            state.update(cx, |state, cx| state.set_value(rgb_to_hsla(rgb), window, cx));
                        }
                        for (state, rgb) in this.light_colors.iter().zip(md_color_values(&MdColors::default_light())) {
                            state.update(cx, |state, cx| state.set_value(rgb_to_hsla(rgb), window, cx));
                        }
                        cx.notify();
                    }))))
            .child(div().flex().gap_6().items_start()
                .child(div().flex_1().min_w_0().child(dark))
                .child(div().flex_1().min_w_0().child(light)))
    }
}

impl Render for SettingsView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        window.set_rem_size(cx.theme().font_size);
        let content = match self.tab {
            0 => self.render_general(cx).into_any_element(),
            1 => self.render_behavior(cx).into_any_element(),
            2 => self.render_hotkeys().into_any_element(),
            _ => self.render_colors(cx).into_any_element(),
        };

        div()
            .size_full()
            .flex()
            .flex_col()
            .bg(cx.theme().background)
            .text_color(cx.theme().foreground)
            .font_family(cx.theme().font_family.clone())
            .text_size(cx.theme().font_size)
            .child(TitleBar::new().child("NxNote 设置"))
            .child(
                div().px_6().pt_5().pb_3().child(
                    TabBar::new("settings-sections")
                        .selected_index(self.tab)
                        .children([
                            Tab::new().label("外观与窗口"),
                            Tab::new().label("自动行为"),
                            Tab::new().label("热键"),
                            Tab::new().label("Markdown 颜色"),
                        ])
                        .on_click(cx.listener(|this, index, _, cx| {
                            this.tab = *index;
                            cx.notify();
                        })),
                ),
            )
            .child(
                div()
                    .id("settings-scroll")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .px_6()
                    .pb_5()
                    .child(content),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .border_t_1()
                    .border_color(cx.theme().border)
                    .px_6()
                    .py_3()
                    .child(
                        div()
                            .text_sm()
                            .text_color(cx.theme().muted_foreground)
                            .child("修改仅在点击“应用”后交给主程序保存。"),
                    )
                    .child(
                        div()
                            .flex()
                            .gap_2()
                            .child(
                                Button::new("cancel-settings")
                                    .label("取消")
                                    .on_click(|_, window, _| window.remove_window()),
                            )
                            .child(
                                Button::new("apply-settings")
                                    .primary()
                                    .label("应用")
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        this.apply(window, cx);
                                    })),
                            ),
                    ),
            )
    }
}

fn integer_font_state(value: f32, window: &mut Window, cx: &mut Context<SettingsView>) -> Entity<InputState> {
    cx.new(|cx| InputState::new(window, cx)
        .default_value(format!("{:.0}", value.round().clamp(10., 24.)))
        .step(1.).min(10.).max(24.)
        .mask_pattern(gpui_kit::base::input::MaskPattern::Number { separator: None, fraction: Some(0) })
        .validate(|text, _| text.is_empty() || text.bytes().all(|b| b.is_ascii_digit())))
}
fn read_font_size(state: &Entity<InputState>, label: &str, cx: &App) -> anyhow::Result<f32> {
    let value = state.read(cx).value().trim().parse::<u16>()
        .map_err(|_| anyhow!("{label}必须是整数"))?;
    if !(10..=24).contains(&value) { return Err(anyhow!("{label}必须在 10–24 之间")); }
    Ok(value as f32)
}
fn number_state(
    value: f64,
    min: f64,
    max: f64,
    step: f64,
    window: &mut Window,
    cx: &mut Context<SettingsView>,
) -> Entity<InputState> {
    cx.new(|cx| {
        InputState::new(window, cx)
            .default_value(value.to_string())
            .step(step)
            .min(min)
            .max(max)
    })
}

fn read_number(
    state: &Entity<InputState>,
    label: &str,
    min: f64,
    max: f64,
    cx: &App,
) -> anyhow::Result<f64> {
    let text = state.read(cx).value();
    let value = text
        .trim()
        .parse::<f64>()
        .map_err(|_| anyhow!("{label}不是有效数字"))?;
    if !value.is_finite() || value < min || value > max {
        return Err(anyhow!("{label}必须在 {min}–{max} 之间"));
    }
    Ok(value)
}

fn settings_switch(
    id: &'static str,
    checked: bool,
    on_change: impl Fn(&bool, &mut Window, &mut App) + 'static,
) -> Switch {
    Switch::new(id).checked(checked).on_change(on_change)
}

fn font_state(
    selected: &[String],
    window: &mut Window,
    cx: &mut Context<SettingsView>,
) -> Entity<FontState> {
    let choices = ordered_font_choices(selected, &[]);
    let selected_count = choices.len();
    let selected_indices = (0..selected_count)
        .map(|row| ComponentIndexPath::default().row(row))
        .collect();
    cx.new(|cx| {
        ComboboxState::new(SearchableVec::new(choices), selected_indices, window, cx)
            .multiple(true)
            .searchable(true)
    })
}

fn ordered_font_choices(selected: &[String], catalog: &[(String, String)]) -> Vec<FontChoice> {
    let mut choices = Vec::<FontChoice>::new();
    for path in selected {
        if choices.iter().any(|choice| &choice.path == path) {
            continue;
        }
        let label = catalog
            .iter()
            .find(|(p, _)| p == path)
            .map(|(_, label)| label.clone())
            .unwrap_or_else(|| {
                Path::new(path)
                    .file_name()
                    .and_then(|n| n.to_str())
                    .unwrap_or(path)
                    .to_string()
            });
        choices.push(FontChoice {
            path: path.clone(),
            label: label.into(),
        });
    }
    for (path, label) in catalog {
        if !choices.iter().any(|choice| &choice.path == path) {
            choices.push(FontChoice {
                path: path.clone(),
                label: label.clone().into(),
            });
        }
    }
    choices
}

fn color_states(
    colors: &MdColors,
    window: &mut Window,
    cx: &mut Context<SettingsView>,
) -> Vec<Entity<ColorPickerState>> {
    md_color_values(colors)
        .into_iter()
        .map(|rgb| cx.new(|cx| ColorPickerState::new(window, cx).default_value(rgb_to_hsla(rgb))))
        .collect()
}

fn color_form(title: &'static str, states: &[Entity<ColorPickerState>]) -> impl IntoElement {
    let mut form = Form::horizontal().label_width(px(86.));
    for (label, state) in COLOR_LABELS.iter().zip(states.iter()) {
        form = form.child(
            Field::new()
                .label(*label)
                .child(ColorPicker::new(state).accessibility_label(format!("{title}：{label}"))),
        );
    }
    div()
        .flex()
        .flex_col()
        .gap_3()
        .child(
            div()
                .font_weight(gpui_kit::FontWeight::SEMIBOLD)
                .child(title),
        )
        .child(form)
}

fn md_color_values(colors: &MdColors) -> [[u8; 3]; 11] {
    [
        colors.text,
        colors.heading,
        colors.bold,
        colors.italic,
        colors.code_text,
        colors.code_bg,
        colors.link,
        colors.quote_text,
        colors.quote_bar,
        colors.list_marker,
        colors.syntax,
    ]
}

fn colors_from_states(
    states: &[Entity<ColorPickerState>],
    theme: &str,
    cx: &App,
) -> anyhow::Result<MdColors> {
    let mut values = [[0_u8; 3]; 11];
    for (index, state) in states.iter().enumerate() {
        let color = state
            .read(cx)
            .value()
            .ok_or_else(|| anyhow!("{theme}主题的“{}”颜色未设置", COLOR_LABELS[index]))?;
        values[index] = hsla_to_rgb(color);
    }
    Ok(MdColors {
        text: values[0],
        heading: values[1],
        bold: values[2],
        italic: values[3],
        code_text: values[4],
        code_bg: values[5],
        link: values[6],
        quote_text: values[7],
        quote_bar: values[8],
        list_marker: values[9],
        syntax: values[10],
    })
}

fn rgb_to_hsla(rgb: [u8; 3]) -> Hsla {
    Rgba {
        r: rgb[0] as f32 / 255.,
        g: rgb[1] as f32 / 255.,
        b: rgb[2] as f32 / 255.,
        a: 1.,
    }
    .into()
}

fn hsla_to_rgb(color: Hsla) -> [u8; 3] {
    let rgba = Rgba::from(color);
    let channel = |value: f32| (value.clamp(0., 1.) * 255.).round() as u8;
    [channel(rgba.r), channel(rgba.g), channel(rgba.b)]
}

#[cfg(test)]
mod font_tests {
    use super::*;
    #[test]
    fn async_catalog_preserves_selected_order_and_missing_paths() {
        let selected = vec![
            "second.ttf".into(),
            "missing.ttf".into(),
            "first.ttf".into(),
        ];
        let catalog = vec![
            ("first.ttf".into(), "First".into()),
            ("second.ttf".into(), "Second".into()),
            ("third.ttf".into(), "Third".into()),
        ];
        let choices = ordered_font_choices(&selected, &catalog);
        assert_eq!(
            choices.iter().map(|c| c.path.as_str()).collect::<Vec<_>>(),
            vec!["second.ttf", "missing.ttf", "first.ttf", "third.ttf"]
        );
    }
}
