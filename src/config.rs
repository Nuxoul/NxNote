use serde::{Deserialize, Serialize};
use std::path::PathBuf;

use crate::theme::ThemeMode;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct MdColors {
    pub text: [u8; 3],
    pub heading: [u8; 3],
    pub bold: [u8; 3],
    pub italic: [u8; 3],
    pub code_text: [u8; 3],
    pub code_bg: [u8; 3],
    pub link: [u8; 3],
    pub quote_text: [u8; 3],
    pub quote_bar: [u8; 3],
    pub list_marker: [u8; 3],
    pub syntax: [u8; 3],
}

impl MdColors {
    pub fn default_dark() -> Self {
        Self {
            text: [229, 234, 242],
            heading: [241, 245, 251],
            bold: [241, 245, 251],
            italic: [212, 224, 239],
            code_text: [209, 233, 222],
            code_bg: [36, 48, 63],
            link: [210, 230, 255],
            quote_text: [212, 224, 239],
            quote_bar: [125, 164, 214],
            list_marker: [202, 220, 243],
            syntax: [212, 224, 239],
        }
    }
    pub fn default_light() -> Self {
        Self {
            text: [32, 43, 60],
            heading: [21, 35, 57],
            bold: [21, 35, 57],
            italic: [67, 83, 108],
            code_text: [30, 92, 78],
            code_bg: [234, 240, 246],
            link: [23, 79, 150],
            quote_text: [58, 78, 103],
            quote_bar: [72, 111, 164],
            list_marker: [23, 79, 150],
            syntax: [67, 83, 107],
        }
    }
    /// Only replace the exact old factory palette, never a user's edits.
    pub fn migrate_factory_palettes(dark: &mut Self, light: &mut Self) {
        let old_dark = Self {
            text: [216,210,196], heading: [245,175,90], bold: [255,255,255], italic: [245,175,90],
            code_text: [245,180,110], code_bg: [52,36,22], link: [120,200,220],
            quote_text: [140,170,200], quote_bar: [110,145,180], list_marker: [245,175,90], syntax: [140,132,118],
        };
        let old_light = Self {
            text: [40,36,30], heading: [178,110,40], bold: [10,8,6], italic: [178,110,40],
            code_text: [148,92,42], code_bg: [240,226,200], link: [50,110,180],
            quote_text: [70,90,130], quote_bar: [110,145,180], list_marker: [178,110,40], syntax: [150,140,124],
        };
        if *dark == old_dark { *dark = Self::default_dark(); }
        if *light == old_light { *light = Self::default_light(); }
    }
}

fn default_md_dark() -> MdColors {
    MdColors::default_dark()
}
fn default_md_light() -> MdColors {
    MdColors::default_light()
}

/// 一个打开的标签页 = (所属文件夹, 笔记名)。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct OpenTab {
    pub folder_key: String,
    pub note_name: String,
}

/// --data-dir 覆盖（便携模式 / 隔离测试）。main 里在 Config::load 之前设置。
static DATA_DIR_OVERRIDE: std::sync::OnceLock<PathBuf> = std::sync::OnceLock::new();

pub fn set_data_dir_override(dir: PathBuf) {
    let _ = DATA_DIR_OVERRIDE.set(dir);
}

pub fn data_dir() -> PathBuf {
    if let Some(dir) = DATA_DIR_OVERRIDE.get() {
        let _ = std::fs::create_dir_all(dir);
        return dir.clone();
    }
    let base = dirs::data_dir().unwrap_or_else(|| PathBuf::from("."));
    let dir = base.join("NxNote");
    let _ = std::fs::create_dir_all(&dir);
    dir
}

pub fn config_path() -> PathBuf {
    data_dir().join("config.toml")
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    pub theme_mode: ThemeMode,
    pub window_width: f32,
    pub window_height: f32,
    /// 上次窗口位置（外框左上角），退出/隐藏时更新，启动时恢复
    #[serde(default)]
    pub window_pos: Option<[f32; 2]>,
    pub always_on_top: bool,
    /// Keep the legacy field name for body text; old fractional values are rounded on load.
    #[serde(serialize_with = "serialize_body_font_size")]
    pub font_size: f32,
    #[serde(default = "default_ui_font_size")]
    pub ui_font_size: u16,
    /// Body line-box height as a font-size multiplier; independent of paragraph gaps.
    #[serde(default = "default_line_height")]
    pub line_height: f32,
    pub hotkey: String,
    /// 剪贴板快速捕获热键：按下后把剪贴板文本追加进速记本
    #[serde(default = "default_hotkey_capture")]
    pub hotkey_capture: String,
    pub autosave_delay_ms: u64,
    pub poll_interval_ms: u64,
    #[serde(default)]
    pub ui_fonts: Vec<String>,
    #[serde(default)]
    pub editor_fonts: Vec<String>,
    #[serde(default = "default_autohide")]
    pub autohide_title_bar: bool,
    /// 自动随前台应用切换笔记（默认关闭，需要时由用户在设置里打开）
    #[serde(default)]
    pub auto_follow_foreground: bool,
    /// 黑名单：完整 exe 路径或裸文件名（不区分大小写）。命中后视为未绑定，落回 scratch。
    #[serde(default)]
    pub blocked_apps: Vec<String>,
    #[serde(default = "default_md_dark")]
    pub md_dark: MdColors,
    #[serde(default = "default_md_light")]
    pub md_light: MdColors,
    /// 开机自启动：写入 HKCU\...\Run，指向当前 exe + `--hidden`
    #[serde(default)]
    pub autostart: bool,
    /// 关闭按钮 / Alt+F4 时最小化到托盘而不是退出
    #[serde(default = "default_close_to_tray")]
    pub close_to_tray: bool,
    /// 打开的标签页（退出时落盘，启动时恢复）
    #[serde(default)]
    pub open_tabs: Vec<OpenTab>,
    /// 激活的标签页下标（越界时启动时会被 clamp）
    #[serde(default)]
    pub active_tab: usize,
}

fn default_ui_font_size() -> u16 { 16 }
pub const MIN_LINE_HEIGHT: f32 = 1.;
pub const MAX_LINE_HEIGHT: f32 = 2.5;
fn default_line_height() -> f32 { 1.5 }
pub fn normalize_line_height(value: f32) -> f32 {
    if value.is_finite() {
        (value.clamp(MIN_LINE_HEIGHT, MAX_LINE_HEIGHT) * 10.).round() / 10.
    } else {
        default_line_height()
    }
}
fn serialize_body_font_size<S: serde::Serializer>(value: &f32, serializer: S) -> Result<S::Ok, S::Error> {
    serializer.serialize_u16(value.round().clamp(10., 24.) as u16)
}
fn default_close_to_tray() -> bool {
    true
}

fn default_hotkey_capture() -> String {
    "Ctrl+Alt+Shift+C".to_string()
}

fn default_autohide() -> bool {
    true
}

impl Default for Config {
    fn default() -> Self {
        Self {
            theme_mode: ThemeMode::Dark,
            window_width: 256.0,
            window_height: 256.0,
            window_pos: None,
            always_on_top: false,
            font_size: 13.0,
            ui_font_size: default_ui_font_size(),
            line_height: default_line_height(),
            hotkey: "Ctrl+Alt+N".to_string(),
            hotkey_capture: default_hotkey_capture(),
            autosave_delay_ms: 1500,
            poll_interval_ms: 500,
            ui_fonts: Vec::new(),
            editor_fonts: Vec::new(),
            autohide_title_bar: true,
            auto_follow_foreground: false,
            blocked_apps: Vec::new(),
            md_dark: MdColors::default_dark(),
            md_light: MdColors::default_light(),
            autostart: false,
            close_to_tray: true,
            open_tabs: Vec::new(),
            active_tab: 0,
        }
    }
}

impl Config {
    pub fn load() -> Self {
        let p = config_path();
        match std::fs::read_to_string(&p) {
            Ok(s) => {
                let mut cfg: Self = toml::from_str(&s).unwrap_or_default();
                cfg.font_size = if cfg.font_size.is_finite() { cfg.font_size.round().clamp(10., 24.) } else { 13. };
                cfg.ui_font_size = cfg.ui_font_size.clamp(10, 24);
                cfg.line_height = normalize_line_height(cfg.line_height);
                MdColors::migrate_factory_palettes(&mut cfg.md_dark, &mut cfg.md_light);
                cfg
            }
            Err(_) => {
                let c = Self::default();
                let _ = c.save();
                c
            }
        }
    }

    pub fn save(&self) -> anyhow::Result<()> {
        let s = toml::to_string_pretty(self)?;
        crate::fsutil::write_atomic(&config_path(), s.as_bytes())?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn legacy_config_still_loads() {
        let legacy = r#"
theme_mode = "Dark"
window_width = 256.0
window_height = 256.0
always_on_top = false
font_size = 13.0
hotkey = "Ctrl+Alt+N"
autosave_delay_ms = 1500
poll_interval_ms = 500
"#;
        let cfg: Config = toml::from_str(legacy).unwrap();
        assert_eq!(cfg.theme_mode, ThemeMode::Dark);
        assert!(cfg.close_to_tray);
        assert_eq!(cfg.hotkey_capture, "Ctrl+Alt+Shift+C");
        assert_eq!(cfg.md_dark, MdColors::default_dark());
        assert_eq!(cfg.line_height, default_line_height());
    }
    #[test]
    fn config_roundtrip_preserves_paths_tabs_and_colors() {
        let mut cfg = Config {
            ui_fonts: vec![r"C:\Fonts\中文.ttf".into()],
            ..Config::default()
        };
        cfg.open_tabs.push(OpenTab {
            folder_key: "_global".into(),
            note_name: "中文🙂".into(),
        });
        cfg.md_dark.heading = [1, 2, 3];
        cfg.line_height = 1.8;
        let restored: Config = toml::from_str(&toml::to_string_pretty(&cfg).unwrap()).unwrap();
        assert_eq!(restored.ui_fonts, cfg.ui_fonts);
        assert_eq!(restored.open_tabs, cfg.open_tabs);
        assert_eq!(restored.md_dark, cfg.md_dark);
        assert_eq!(restored.line_height, 1.8);
    }
    #[test]
    fn line_height_normalization_is_bounded_and_finite() {
        assert_eq!(normalize_line_height(0.5), 1.);
        assert_eq!(normalize_line_height(3.), 2.5);
        assert_eq!(normalize_line_height(1.84), 1.8);
        assert_eq!(normalize_line_height(f32::NAN), 1.5);
        assert_eq!(normalize_line_height(f32::INFINITY), 1.5);
    }
    #[test]
    fn factory_palette_migration_preserves_custom_colors() {
        let mut dark = MdColors::default_dark();
        let mut light = MdColors::default_light();
        dark.heading = [1, 2, 3];
        light.link = [7, 8, 9];
        let custom = (dark.clone(), light.clone());
        MdColors::migrate_factory_palettes(&mut dark, &mut light);
        assert_eq!((dark, light), custom);
    }
    #[test]
    fn old_factory_palettes_adopt_new_defaults() {
        let mut dark = MdColors {
            text: [216,210,196], heading: [245,175,90], bold: [255,255,255], italic: [245,175,90],
            code_text: [245,180,110], code_bg: [52,36,22], link: [120,200,220],
            quote_text: [140,170,200], quote_bar: [110,145,180], list_marker: [245,175,90], syntax: [140,132,118],
        };
        let mut light = MdColors {
            text: [40,36,30], heading: [178,110,40], bold: [10,8,6], italic: [178,110,40],
            code_text: [148,92,42], code_bg: [240,226,200], link: [50,110,180],
            quote_text: [70,90,130], quote_bar: [110,145,180], list_marker: [178,110,40], syntax: [150,140,124],
        };
        MdColors::migrate_factory_palettes(&mut dark, &mut light);
        assert_eq!(dark, MdColors::default_dark());
        assert_eq!(light, MdColors::default_light());
    }
}
