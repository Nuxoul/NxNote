use serde::{Deserialize, Serialize};
use std::path::PathBuf;

use crate::theme::ThemeMode;

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
