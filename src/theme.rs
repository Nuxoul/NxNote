use egui::{Color32, Stroke};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[derive(Default)]
pub enum ThemeMode {
    System,
    Light,
    #[default]
    Dark,
}


#[derive(Clone, Copy)]
pub struct Palette {
    pub bg: Color32,
    pub bg_alt: Color32,
    pub panel: Color32,
    pub stroke: Color32,
    pub text: Color32,
    pub text_weak: Color32,
    pub text_strong: Color32,
    pub accent: Color32,
    pub accent_text: Color32,
    pub hover: Color32,
    pub danger: Color32,
}

pub fn palette(mode: ThemeMode) -> Palette {
    match resolve(mode) {
        ResolvedMode::Dark => Palette {
            bg: Color32::from_rgb(30, 30, 32),
            bg_alt: Color32::from_rgb(38, 38, 40),
            panel: Color32::from_rgb(34, 34, 36),
            stroke: Color32::from_rgb(60, 58, 54),
            text: Color32::from_rgb(216, 210, 196),
            text_weak: Color32::from_rgb(140, 132, 118),
            text_strong: Color32::from_rgb(238, 230, 210),
            accent: Color32::from_rgb(178, 132, 76),
            accent_text: Color32::from_rgb(32, 24, 16),
            hover: Color32::from_rgb(54, 50, 44),
            danger: Color32::from_rgb(196, 60, 56),
        },
        ResolvedMode::Light => Palette {
            bg: Color32::from_rgb(244, 240, 232),
            bg_alt: Color32::from_rgb(234, 228, 216),
            panel: Color32::from_rgb(250, 246, 238),
            stroke: Color32::from_rgb(210, 200, 180),
            text: Color32::from_rgb(40, 36, 30),
            text_weak: Color32::from_rgb(120, 110, 96),
            text_strong: Color32::from_rgb(20, 18, 14),
            accent: Color32::from_rgb(178, 132, 76),
            accent_text: Color32::from_rgb(252, 248, 238),
            hover: Color32::from_rgb(224, 216, 198),
            danger: Color32::from_rgb(196, 60, 56),
        },
    }
}

enum ResolvedMode {
    Dark,
    Light,
}

#[cfg(windows)]
fn system_prefers_light_uncached() -> bool {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    let Ok(out) = std::process::Command::new("reg")
        .args([
            "query",
            r"HKCU\Software\Microsoft\Windows\CurrentVersion\Themes\Personalize",
            "/v",
            "AppsUseLightTheme",
        ])
        .creation_flags(CREATE_NO_WINDOW)
        .output()
    else {
        return false;
    };
    let s = String::from_utf8_lossy(&out.stdout);
    // 形如 "    AppsUseLightTheme    REG_DWORD    0x1"
    let toks: Vec<&str> = s.split_whitespace().collect();
    toks.iter()
        .position(|t| *t == "REG_DWORD")
        .and_then(|i| toks.get(i + 1))
        .and_then(|v| v.strip_prefix("0x"))
        .and_then(|v| u32::from_str_radix(v, 16).ok())
        .map(|v| v == 1)
        .unwrap_or(false)
}

#[cfg(not(windows))]
fn system_prefers_light_uncached() -> bool {
    false
}

/// 读注册表 AppsUseLightTheme（缓存 2 秒，避免每帧起进程）。
pub fn system_prefers_light() -> bool {
    static CACHE: OnceLock<Mutex<(Instant, bool)>> = OnceLock::new();
    let cache = CACHE.get_or_init(|| Mutex::new((Instant::now() - Duration::from_secs(10), false)));
    let mut guard = match cache.lock() {
        Ok(g) => g,
        Err(p) => p.into_inner(),
    };
    if guard.0.elapsed() >= Duration::from_secs(2) {
        guard.1 = system_prefers_light_uncached();
        guard.0 = Instant::now();
    }
    guard.1
}

fn resolve(mode: ThemeMode) -> ResolvedMode {
    match mode {
        ThemeMode::Light => ResolvedMode::Light,
        ThemeMode::Dark => ResolvedMode::Dark,
        ThemeMode::System => {
            if system_prefers_light() {
                ResolvedMode::Light
            } else {
                ResolvedMode::Dark
            }
        }
    }
}

/// 解析后的实际模式是否为浅色（System 会查系统设置）。
pub fn resolved_is_light(mode: ThemeMode) -> bool {
    matches!(resolve(mode), ResolvedMode::Light)
}

pub fn apply(ctx: &egui::Context, mode: ThemeMode, font_size: f32) {
    let p = palette(mode);

    let mut visuals = match resolve(mode) {
        ResolvedMode::Dark => egui::Visuals::dark(),
        ResolvedMode::Light => egui::Visuals::light(),
    };

    visuals.window_fill = p.bg;
    visuals.panel_fill = p.bg;
    visuals.window_stroke = Stroke::new(1.0, p.stroke);
    visuals.window_rounding = egui::Rounding::same(2.0);
    visuals.menu_rounding = egui::Rounding::same(4.0);

    visuals.widgets.noninteractive.bg_fill = p.bg;
    visuals.widgets.noninteractive.weak_bg_fill = p.bg;
    visuals.widgets.noninteractive.bg_stroke = Stroke::new(1.0, p.stroke);
    visuals.widgets.noninteractive.fg_stroke = Stroke::new(1.0, p.text);

    visuals.widgets.inactive.bg_fill = p.bg_alt;
    visuals.widgets.inactive.weak_bg_fill = p.bg_alt;
    visuals.widgets.inactive.bg_stroke = Stroke::NONE;
    visuals.widgets.inactive.fg_stroke = Stroke::new(1.0, p.text);
    visuals.widgets.inactive.rounding = egui::Rounding::same(3.0);

    visuals.widgets.hovered.bg_fill = p.hover;
    visuals.widgets.hovered.weak_bg_fill = p.hover;
    visuals.widgets.hovered.bg_stroke = Stroke::NONE;
    visuals.widgets.hovered.fg_stroke = Stroke::new(1.0, p.text_strong);
    visuals.widgets.hovered.rounding = egui::Rounding::same(3.0);

    visuals.widgets.active.bg_fill = p.accent;
    visuals.widgets.active.weak_bg_fill = p.accent;
    visuals.widgets.active.bg_stroke = Stroke::NONE;
    visuals.widgets.active.fg_stroke = Stroke::new(1.0, p.accent_text);
    visuals.widgets.active.rounding = egui::Rounding::same(3.0);

    visuals.widgets.open.bg_fill = p.bg_alt;
    visuals.widgets.open.weak_bg_fill = p.bg_alt;

    visuals.selection.bg_fill = p.accent;
    visuals.selection.stroke = Stroke::NONE;

    visuals.override_text_color = Some(p.text);
    visuals.extreme_bg_color = p.panel;

    visuals.hyperlink_color = p.accent;

    ctx.set_visuals(visuals);

    let mut style = (*ctx.style()).clone();
    for (_, font_id) in style.text_styles.iter_mut() {
        font_id.size = font_size.max(10.0);
    }
    style.spacing.item_spacing = egui::vec2(6.0, 4.0);
    style.spacing.button_padding = egui::vec2(8.0, 4.0);
    style.spacing.window_margin = egui::Margin::same(0.0);
    style.visuals.widgets.inactive.expansion = 0.0;
    style.visuals.widgets.hovered.expansion = 0.0;
    style.visuals.widgets.active.expansion = 0.0;
    ctx.set_style(style);
}
