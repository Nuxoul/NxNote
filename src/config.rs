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
            text: [216, 210, 196],
            heading: [245, 175, 90],
            bold: [255, 255, 255],
            italic: [245, 175, 90],
            code_text: [245, 180, 110],
            code_bg: [52, 36, 22],
            link: [120, 200, 220],
            quote_text: [140, 170, 200],
            quote_bar: [110, 145, 180],
            list_marker: [245, 175, 90],
            syntax: [140, 132, 118],
        }
    }
    pub fn default_light() -> Self {
        Self {
            text: [40, 36, 30],
            heading: [178, 110, 40],
            bold: [10, 8, 6],
            italic: [178, 110, 40],
            code_text: [148, 92, 42],
            code_bg: [240, 226, 200],
            link: [50, 110, 180],
            quote_text: [70, 90, 130],
            quote_bar: [110, 145, 180],
            list_marker: [178, 110, 40],
            syntax: [150, 140, 124],
        }
    }
}

fn default_md_dark() -> MdColors {
    MdColors::default_dark()
}
fn default_md_light() -> MdColors {
    MdColors::default_light()
}

pub fn data_dir() -> PathBuf {
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
    pub always_on_top: bool,
    pub font_size: f32,
    pub hotkey: String,
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
            always_on_top: false,
            font_size: 13.0,
            hotkey: "Ctrl+Alt+N".to_string(),
            autosave_delay_ms: 1500,
            poll_interval_ms: 500,
            ui_fonts: Vec::new(),
            editor_fonts: Vec::new(),
            autohide_title_bar: true,
            auto_follow_foreground: false,
            blocked_apps: Vec::new(),
            md_dark: MdColors::default_dark(),
            md_light: MdColors::default_light(),
        }
    }
}

impl Config {
    pub fn load() -> Self {
        let p = config_path();
        match std::fs::read_to_string(&p) {
            Ok(s) => toml::from_str(&s).unwrap_or_default(),
            Err(_) => {
                let c = Self::default();
                let _ = c.save();
                c
            }
        }
    }

    pub fn save(&self) -> anyhow::Result<()> {
        let s = toml::to_string_pretty(self)?;
        std::fs::write(config_path(), s)?;
        Ok(())
    }
}
