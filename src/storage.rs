use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use crate::config::data_dir;

pub fn notes_root() -> PathBuf {
    let p = data_dir().join("notes");
    let _ = std::fs::create_dir_all(&p);
    p
}

pub fn index_path() -> PathBuf {
    data_dir().join("index.json")
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum TitleRule {
    SplitIndex { sep: String, index: usize },
}

impl TitleRule {
    pub fn extract(&self, title: &str) -> Option<String> {
        match self {
            TitleRule::SplitIndex { sep, index } => {
                let parts: Vec<&str> = title.split(sep.as_str()).collect();
                parts.get(*index).map(|s| s.trim().to_string()).filter(|s| !s.is_empty())
            }
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct AppEntry {
    pub exe_path: String,
    pub display_name: String,
    #[serde(default)]
    pub title_rule: Option<TitleRule>,
    #[serde(default)]
    pub notes: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct AppIndex {
    #[serde(default)]
    pub apps: BTreeMap<String, AppEntry>,
    /// 上次打开的笔记（启动时自动恢复）
    #[serde(default)]
    pub last_folder_key: Option<String>,
    #[serde(default)]
    pub last_note_name: Option<String>,
    #[serde(default)]
    pub last_display_name: Option<String>,
}

impl AppIndex {
    pub fn load() -> Self {
        match std::fs::read_to_string(index_path()) {
            Ok(s) => serde_json::from_str(&s).unwrap_or_default(),
            Err(_) => Self::default(),
        }
    }

    pub fn save(&self) -> Result<()> {
        let s = serde_json::to_string_pretty(self)?;
        std::fs::write(index_path(), s)?;
        Ok(())
    }
}

pub fn folder_key_for(exe: &Path) -> String {
    let stem = exe
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("unknown");
    let safe: String = stem
        .chars()
        .map(|c| if c.is_alphanumeric() || c == '_' || c == '-' { c } else { '_' })
        .collect();
    let path_str = exe.to_string_lossy();
    let hash = sha1_smol::Sha1::from(path_str.as_bytes()).digest().to_string();
    format!("{}_{}", safe, &hash[..6])
}

pub fn sanitize_note_name(name: &str) -> String {
    let trimmed = name.trim();
    if trimmed.is_empty() {
        return "未命名".to_string();
    }
    trimmed
        .chars()
        .map(|c| match c {
            '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|' => '_',
            _ => c,
        })
        .collect()
}

pub fn note_path(folder_key: &str, note_name: &str) -> PathBuf {
    let dir = notes_root().join(folder_key);
    let _ = std::fs::create_dir_all(&dir);
    dir.join(format!("{}.md", note_name))
}

pub fn load_note(folder_key: &str, note_name: &str) -> String {
    let p = note_path(folder_key, note_name);
    std::fs::read_to_string(p).unwrap_or_default()
}

pub fn save_note(folder_key: &str, note_name: &str, content: &str) -> Result<()> {
    let p = note_path(folder_key, note_name);
    std::fs::write(p, content)?;
    Ok(())
}

pub const GLOBAL_FOLDER: &str = "_global";
pub const DEFAULT_NOTE: &str = "_default";
pub const SCRATCH_NOTE: &str = "scratch";
