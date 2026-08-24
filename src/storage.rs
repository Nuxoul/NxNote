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
    /// 速记本（全局文件夹）下的用户笔记；scratch 恒存在，不在此列
    #[serde(default)]
    pub global_notes: Vec<String>,
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
        crate::fsutil::write_atomic(&index_path(), s.as_bytes())?;
        Ok(())
    }

    /// 某个 folder 下的全部候选笔记名（不含 scratch，除非 folder 就是全局）。
    pub fn notes_of(&self, folder_key: &str) -> Vec<String> {
        if folder_key == GLOBAL_FOLDER {
            let mut v = vec![SCRATCH_NOTE.to_string()];
            for n in &self.global_notes {
                if n != SCRATCH_NOTE && !v.contains(n) {
                    v.push(n.clone());
                }
            }
            v
        } else {
            self.apps
                .get(folder_key)
                .map(|e| e.notes.clone())
                .unwrap_or_default()
        }
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
    crate::fsutil::write_atomic(&p, content.as_bytes())?;
    Ok(())
}

/// 把笔记移入回收目录（notes/.trash/<folder>/名字-时间戳.md），而非直接删除。
pub fn trash_note(folder_key: &str, note_name: &str) -> Result<PathBuf> {
    let src = note_path(folder_key, note_name);
    if !src.exists() {
        return Ok(src);
    }
    let dst_dir = notes_root().join(".trash").join(folder_key);
    std::fs::create_dir_all(&dst_dir)?;
    let ts = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    // 文件名里的非法字符已经 sanitize 过，这里只需防重名
    let dst = dst_dir.join(format!("{}-{}.md", note_name, ts));
    std::fs::rename(&src, &dst)?;
    Ok(dst)
}

/// 剪贴板快速捕获：把文本追加到速记本文件（当前视图不在速记本时由后台直写）。
pub fn append_scratch(text: &str) -> Result<()> {
    let p = note_path(GLOBAL_FOLDER, SCRATCH_NOTE);
    let mut cur = std::fs::read_to_string(&p).unwrap_or_default();
    if !cur.is_empty() && !cur.ends_with('\n') {
        cur.push('\n');
    }
    cur.push_str(text);
    if !cur.ends_with('\n') {
        cur.push('\n');
    }
    crate::fsutil::write_atomic(&p, cur.as_bytes())?;
    Ok(())
}

/// 启动恢复：校验 folder/note 真实存在（磁盘上有文件），不存在则回退。
/// 返回 (folder_key, display_name, note_name)。
pub fn resolve_startup(index: &AppIndex) -> (String, String, String) {
    let scratch = || (GLOBAL_FOLDER.to_string(), "速记".to_string(), SCRATCH_NOTE.to_string());
    let (Some(f), Some(n), Some(d)) = (
        index.last_folder_key.clone(),
        index.last_note_name.clone(),
        index.last_display_name.clone(),
    ) else {
        return scratch();
    };
    if f != GLOBAL_FOLDER && !index.apps.contains_key(&f) {
        return scratch();
    }
    // 目标文件还在 → 直接用
    if note_path(&f, &n).exists() {
        return (f, d, n);
    }
    // 否则退到该 folder 的第一个存在的笔记，再不行回速记
    for cand in index.notes_of(&f) {
        if note_path(&f, &cand).exists() {
            return (f, d, cand);
        }
    }
    scratch()
}

pub const GLOBAL_FOLDER: &str = "_global";
pub const DEFAULT_NOTE: &str = "_default";
pub const SCRATCH_NOTE: &str = "scratch";
