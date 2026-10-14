//! UI-independent note/session state. Switching never discards a failed save.
use crate::config::{Config, OpenTab};
use crate::storage::{self, AppEntry, AppIndex, DEFAULT_NOTE, GLOBAL_FOLDER, SCRATCH_NOTE};
use crate::watcher::ForegroundInfo;
use anyhow::{bail, Result};

pub struct Session {
    pub cfg: Config,
    pub index: AppIndex,
    pub current: OpenTab,
    pub text: String,
    pub dirty: bool,
    pub tabs: Vec<OpenTab>,
    pub active_tab: usize,
    pub pinned: bool,
    pub foreground: Option<ForegroundInfo>,
}

impl Session {
    pub fn new(cfg: Config) -> Result<Self> {
        let index = AppIndex::load();
        let mut tabs: Vec<OpenTab> = Vec::new();
        for tab in &cfg.open_tabs {
            if (tab.folder_key == GLOBAL_FOLDER || index.apps.contains_key(&tab.folder_key))
                && storage::note_path(&tab.folder_key, &tab.note_name).is_file()
                && !tabs.contains(tab)
            {
                tabs.push(tab.clone());
            }
        }
        let active_tab = cfg.active_tab.min(tabs.len().saturating_sub(1));
        let current = if let Some(tab) = tabs.get(active_tab) {
            tab.clone()
        } else {
            let (folder_key, _, note_name) = storage::resolve_startup(&index);
            let tab = OpenTab {
                folder_key,
                note_name,
            };
            tabs.push(tab.clone());
            tab
        };
        let text = storage::load_note(&current.folder_key, &current.note_name)?;
        Ok(Self {
            cfg,
            index,
            current,
            text,
            dirty: false,
            tabs,
            active_tab,
            pinned: false,
            foreground: None,
        })
    }

    pub fn edit(&mut self, text: String) {
        if self.text != text {
            self.text = text;
            self.dirty = true;
        }
    }

    pub fn save_with(&mut self, write: impl FnOnce(&str, &str, &str) -> Result<()>) -> Result<()> {
        if self.dirty {
            write(
                &self.current.folder_key,
                &self.current.note_name,
                &self.text,
            )?;
            self.dirty = false;
        }
        Ok(())
    }

    pub fn save(&mut self) -> Result<()> {
        self.save_with(storage::save_note)
    }

    pub fn persist(&mut self) -> Result<()> {
        self.save()?;
        self.cfg.open_tabs = self.tabs.clone();
        self.cfg.active_tab = self.active_tab;
        self.index.last_folder_key = Some(self.current.folder_key.clone());
        self.index.last_note_name = Some(self.current.note_name.clone());
        self.index.last_display_name = Some(self.display_name(&self.current.folder_key));
        self.index.save()?;
        self.cfg.save()
    }

    pub fn switch(&mut self, tab: OpenTab) -> Result<bool> {
        if tab == self.current {
            return Ok(false);
        }
        self.save()?;
        self.text = storage::load_note(&tab.folder_key, &tab.note_name)?;
        self.current = tab.clone();
        self.active_tab = if let Some(i) = self.tabs.iter().position(|t| t == &tab) {
            i
        } else {
            self.tabs.push(tab);
            self.tabs.len() - 1
        };
        self.persist()?;
        Ok(true)
    }

    /// Reorder by note identity. Persist config first; never reload/edit the current document.
    pub fn reorder_tab(&mut self, source: &OpenTab, target: &OpenTab, after: bool) -> Result<bool> {
        if source == target { return Ok(false); }
        let Some(from) = self.tabs.iter().position(|tab| tab == source) else { return Ok(false); };
        let mut next = self.tabs.clone();
        let moved = next.remove(from);
        let Some(target_index) = next.iter().position(|tab| tab == target) else { return Ok(false); };
        next.insert(target_index + usize::from(after), moved);
        if next == self.tabs { return Ok(false); }
        let active = next.iter().position(|tab| tab == &self.current).unwrap_or(0);
        let mut cfg = self.cfg.clone();
        cfg.open_tabs = next.clone(); cfg.active_tab = active;
        cfg.save()?;
        self.tabs = next; self.active_tab = active; self.cfg = cfg;
        Ok(true)
    }

    pub fn close_tab(&mut self, index: usize) -> Result<bool> {
        if index >= self.tabs.len() {
            return Ok(false);
        }
        if self.tabs.len() == 1 {
            bail!("至少保留一个标签页");
        }
        self.save()?;
        let active = index == self.active_tab;
        let next = if active {
            Some(
                self.tabs[if index + 1 < self.tabs.len() {
                    index + 1
                } else {
                    index - 1
                }]
                .clone(),
            )
        } else {
            None
        };
        let next_text = next
            .as_ref()
            .map(|tab| storage::load_note(&tab.folder_key, &tab.note_name))
            .transpose()?;
        self.tabs.remove(index);
        if self.active_tab > index {
            self.active_tab -= 1;
        }
        if active {
            self.active_tab = index.min(self.tabs.len() - 1);
            self.current = next.expect("active tab has a successor");
            self.text = next_text.expect("successor was read before removing tab");
        }
        self.persist()?;
        Ok(active)
    }

    pub fn display_name(&self, folder: &str) -> String {
        if folder == GLOBAL_FOLDER {
            "速记".into()
        } else {
            self.index
                .apps
                .get(folder)
                .map(|a| a.display_name.clone())
                .unwrap_or_else(|| folder.into())
        }
    }

    pub fn label(&self, tab: &OpenTab) -> String {
        if tab.folder_key == GLOBAL_FOLDER && tab.note_name == SCRATCH_NOTE {
            "速记".into()
        } else {
            tab.note_name.clone()
        }
    }

    pub fn all_notes(&self) -> Vec<OpenTab> {
        let mut notes = Vec::new();
        for folder in
            std::iter::once(GLOBAL_FOLDER).chain(self.index.apps.keys().map(String::as_str))
        {
            for note_name in self.index.notes_of(folder) {
                notes.push(OpenTab {
                    folder_key: folder.into(),
                    note_name,
                });
            }
        }
        notes
    }

    pub fn create(&mut self, name: &str) -> Result<bool> {
        validate_name(name)?;
        let name = name.trim().to_string();
        let folder = self.current.folder_key.clone();
        if self.index.notes_of(&folder).contains(&name)
            || storage::note_path(&folder, &name).exists()
        {
            bail!("同名笔记已存在");
        }
        self.save()?;
        storage::save_note(&folder, &name, "")?;
        self.notes_mut(&folder)?.push(name.clone());
        self.switch(OpenTab {
            folder_key: folder,
            note_name: name,
        })
    }

    fn notes_mut(&mut self, folder: &str) -> Result<&mut Vec<String>> {
        if folder == GLOBAL_FOLDER {
            Ok(&mut self.index.global_notes)
        } else {
            Ok(&mut self
                .index
                .apps
                .get_mut(folder)
                .ok_or_else(|| anyhow::anyhow!("应用不存在"))?
                .notes)
        }
    }

    pub fn rename(&mut self, name: &str) -> Result<()> {
        validate_name(name)?;
        let name = name.trim();
        if self.current.note_name == SCRATCH_NOTE || self.current.note_name == DEFAULT_NOTE {
            bail!("内置笔记不能重命名");
        }
        if name == self.current.note_name {
            return Ok(());
        }
        let folder = self.current.folder_key.clone();
        if self.index.notes_of(&folder).iter().any(|n| n == name)
            || storage::note_path(&folder, name).exists()
        {
            bail!("同名笔记已存在");
        }
        self.save()?;
        let old = self.current.note_name.clone();
        std::fs::rename(
            storage::note_path(&folder, &old),
            storage::note_path(&folder, name),
        )?;
        for n in self.notes_mut(&folder)? {
            if n == &old {
                *n = name.into();
            }
        }
        for tab in &mut self.tabs {
            if tab.folder_key == folder && tab.note_name == old {
                tab.note_name = name.into();
            }
        }
        self.current.note_name = name.into();
        self.persist()
    }

    pub fn delete(&mut self) -> Result<()> {
        if self.current.note_name == SCRATCH_NOTE || self.current.note_name == DEFAULT_NOTE {
            bail!("内置笔记不能删除");
        }
        self.save()?;
        let old = self.current.clone();
        storage::trash_note(&old.folder_key, &old.note_name)?;
        self.notes_mut(&old.folder_key)?
            .retain(|n| n != &old.note_name);
        self.tabs.retain(|t| t != &old);
        let next = self
            .tabs
            .get(self.active_tab.min(self.tabs.len().saturating_sub(1)))
            .cloned()
            .unwrap_or(OpenTab {
                folder_key: GLOBAL_FOLDER.into(),
                note_name: SCRATCH_NOTE.into(),
            });
        self.switch(next)?;
        Ok(())
    }

    pub fn capture(&mut self, text: &str) -> Result<bool> {
        if text.trim().is_empty() {
            return Ok(false);
        }
        if self.current.folder_key == GLOBAL_FOLDER && self.current.note_name == SCRATCH_NOTE {
            if !self.text.is_empty() && !self.text.ends_with('\n') {
                self.text.push('\n');
            }
            self.text.push_str(text.trim_end());
            self.text.push('\n');
            self.dirty = true;
            // Save even while the main window is hidden.
            self.save()?;
            Ok(true)
        } else {
            storage::append_scratch(text.trim_end())?;
            Ok(false)
        }
    }

    pub fn foreground(&mut self, info: ForegroundInfo) -> Result<bool> {
        self.foreground = Some(info.clone());
        if self.pinned {
            return Ok(false);
        }
        if self.cfg.auto_follow_foreground && app_blocked(&self.cfg.blocked_apps, &info.exe_path) {
            return self.switch(OpenTab {
                folder_key: GLOBAL_FOLDER.into(),
                note_name: SCRATCH_NOTE.into(),
            });
        }
        let folder = storage::folder_key_for(&info.exe_path);
        let display = info
            .exe_path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("应用")
            .to_string();
        let entry = self
            .index
            .apps
            .entry(folder.clone())
            .or_insert_with(|| AppEntry {
                exe_path: info.exe_path.to_string_lossy().into(),
                display_name: display,
                title_rule: None,
                notes: vec![DEFAULT_NOTE.into()],
            });
        if entry.notes.is_empty() {
            entry.notes.push(DEFAULT_NOTE.into());
        }
        let target = entry
            .title_rule
            .as_ref()
            .and_then(|r| r.extract(&info.title))
            .map(|n| storage::sanitize_note_name(&n))
            .unwrap_or_else(|| entry.notes[0].clone());
        if !entry.notes.contains(&target) {
            entry.notes.push(target.clone());
        }
        self.index.save()?;
        if self.cfg.auto_follow_foreground {
            self.switch(OpenTab {
                folder_key: folder,
                note_name: target,
            })
        } else {
            Ok(false)
        }
    }
}

pub fn validate_name(name: &str) -> Result<()> {
    let name = name.trim();
    if name.is_empty()
        || name == "."
        || name == ".."
        || name.ends_with('.')
        || name.chars().any(|c| c.is_control())
    {
        bail!("请输入有效笔记名称");
    }
    if storage::sanitize_note_name(name) != name {
        bail!("名称不能包含 / \\ : * ? \" < > | ");
    }
    let stem = name.split('.').next().unwrap_or("").to_ascii_uppercase();
    if matches!(stem.as_str(), "CON" | "PRN" | "AUX" | "NUL")
        || (stem.len() == 4
            && (stem.starts_with("COM") || stem.starts_with("LPT"))
            && matches!(stem.as_bytes()[3], b'1'..=b'9'))
    {
        bail!("名称为 Windows 保留名称");
    }
    Ok(())
}

pub fn app_blocked(blocked: &[String], exe: &std::path::Path) -> bool {
    let full = exe.to_string_lossy().replace('/', "\\").to_lowercase();
    let name = exe
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("")
        .to_lowercase();
    let stem = exe
        .file_stem()
        .and_then(|n| n.to_str())
        .unwrap_or("")
        .to_lowercase();
    blocked.iter().any(|raw| {
        let b = raw.trim().replace('/', "\\").to_lowercase();
        !b.is_empty()
            && (full == b
                || full.ends_with(&format!("\\{b}"))
                || name == b
                || stem == b
                || b.strip_suffix(".exe").is_some_and(|s| s == stem))
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    fn session() -> Session {
        Session {
            cfg: Config::default(),
            index: AppIndex::default(),
            current: OpenTab {
                folder_key: GLOBAL_FOLDER.into(),
                note_name: SCRATCH_NOTE.into(),
            },
            text: "中文🙂".into(),
            dirty: true,
            tabs: vec![],
            active_tab: 0,
            pinned: false,
            foreground: None,
        }
    }
    #[test]
    fn failed_save_keeps_dirty_text() {
        let mut s = session();
        assert!(s.save_with(|_, _, _| bail!("disk full")).is_err());
        assert!(s.dirty);
        assert_eq!(s.text, "中文🙂");
    }
    #[test]
    fn successful_save_clears_dirty() {
        let mut s = session();
        s.save_with(|folder, note, text| {
            assert_eq!(folder, GLOBAL_FOLDER);
            assert_eq!(note, SCRATCH_NOTE);
            assert_eq!(text, "中文🙂");
            Ok(())
        })
        .unwrap();
        assert!(!s.dirty);
    }
    #[test]
    fn unchanged_text_stays_clean() {
        let mut s = session();
        s.dirty = false;
        s.edit(s.text.clone());
        assert!(!s.dirty);
    }
    #[test]
    fn names_are_windows_safe() {
        for name in ["", "..", "a/b", "NUL", "COM1.md", "trailing."] {
            assert!(validate_name(name).is_err(), "{name}");
        }
        for name in ["中文笔记", "项目 1", "normal.md"] {
            assert!(validate_name(name).is_ok(), "{name}");
        }
    }
    #[test]
    fn disk_workflow_preserves_notes_and_session() {
        let root = std::env::temp_dir().join(format!(
            "nxnote-session-test-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        crate::config::set_data_dir_override(root.clone());
        let mut s = Session::new(Config::default()).unwrap();
        s.create("测试笔记").unwrap();
        s.edit("# 中文🙂\n- item\n".into());
        s.save().unwrap();
        s.rename("重命名").unwrap();
        assert_eq!(
            storage::load_note(GLOBAL_FOLDER, "重命名").unwrap(),
            "# 中文🙂\n- item\n"
        );
        s.persist().unwrap();
        let restored = Session::new(Config::load()).unwrap();
        assert_eq!(restored.current, s.current);
        assert_eq!(restored.text, s.text);
        s.close_tab(s.active_tab).unwrap();
        assert_eq!(s.current.note_name, SCRATCH_NOTE);
        s.capture("捕获🙂").unwrap();
        assert_eq!(
            storage::load_note(GLOBAL_FOLDER, SCRATCH_NOTE).unwrap(),
            "捕获🙂\n"
        );
        s.switch(OpenTab {
            folder_key: GLOBAL_FOLDER.into(),
            note_name: "重命名".into(),
        })
        .unwrap();
        s.delete().unwrap();
        assert!(!storage::note_path(GLOBAL_FOLDER, "重命名").exists());
        assert!(std::fs::read_dir(root.join("notes/.trash/_global"))
            .unwrap()
            .next()
            .is_some());
        let bad = storage::note_path(GLOBAL_FOLDER, "损坏");
        std::fs::write(&bad, [0xff, 0xfe]).unwrap();
        let original = s.current.clone();
        let original_text = s.text.clone();
        assert!(s
            .switch(OpenTab {
                folder_key: GLOBAL_FOLDER.into(),
                note_name: "损坏".into()
            })
            .is_err());
        assert_eq!(s.current, original);
        assert_eq!(s.text, original_text);
        let scratch = storage::note_path(GLOBAL_FOLDER, SCRATCH_NOTE);
        std::fs::write(&scratch, [0xff, 0xfe]).unwrap();
        assert!(storage::append_scratch("不能覆盖").is_err());
        assert_eq!(std::fs::read(&scratch).unwrap(), vec![0xff, 0xfe]);
        std::fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn blocklist_matches_case_insensitively() {
        assert!(app_blocked(
            &["APP.EXE".into()],
            std::path::Path::new("app.exe")
        ));
        assert!(!app_blocked(
            &["other.exe".into()],
            std::path::Path::new("app.exe")
        ));
    }
}
