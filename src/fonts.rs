use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

const DEFAULT_CANDIDATES: &[&str] = &[
    "msyh.ttc", "msyh.ttf", "msyhbd.ttc", "Deng.ttf", "simhei.ttf", "simsun.ttc",
];

const MATERIAL_ICONS: &[u8] = include_bytes!("../assets/MaterialIcons-Regular.ttf");

fn system_fonts_dir() -> PathBuf {
    std::env::var_os("WINDIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("C:\\Windows"))
        .join("Fonts")
}

fn user_fonts_dir() -> Option<PathBuf> {
    dirs::data_local_dir().map(|p| p.join("Microsoft").join("Windows").join("Fonts"))
}

fn find_default_cjk() -> Option<(String, Vec<u8>)> {
    let dir = system_fonts_dir();
    for name in DEFAULT_CANDIDATES {
        let p = dir.join(name);
        if let Ok(bytes) = std::fs::read(&p) {
            return Some((name.to_string(), bytes));
        }
    }
    None
}

fn default_cjk() -> Option<(&'static str, &'static [u8])> {
    static CACHE: OnceLock<Option<(String, Vec<u8>)>> = OnceLock::new();
    CACHE.get_or_init(find_default_cjk).as_ref().map(|(n, b)| (n.as_str(), b.as_slice()))
}

/// 枚举系统已安装字体（系统目录 + 当前用户目录）。
pub fn list_system_fonts() -> &'static [PathBuf] {
    static CACHE: OnceLock<Vec<PathBuf>> = OnceLock::new();
    CACHE.get_or_init(|| {
        let mut dirs = vec![system_fonts_dir()];
        if let Some(u) = user_fonts_dir() {
            if u.exists() {
                dirs.push(u);
            }
        }
        let mut fonts = Vec::new();
        for dir in dirs {
            if let Ok(entries) = std::fs::read_dir(&dir) {
                for entry in entries.flatten() {
                    let path = entry.path();
                    if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
                        let ext = ext.to_lowercase();
                        if matches!(ext.as_str(), "ttf" | "ttc" | "otf") {
                            fonts.push(path);
                        }
                    }
                }
            }
        }
        fonts.sort_by(|a, b| {
            font_display_name(a)
                .to_lowercase()
                .cmp(&font_display_name(b).to_lowercase())
        });
        fonts.dedup_by(|a, b| a == b);
        fonts
    })
}

fn read_font_family_name(path: &Path) -> Option<String> {
    let bytes = std::fs::read(path).ok()?;
    // 对 TTC，依次尝试前几个 face
    let face_count = ttf_parser::fonts_in_collection(&bytes).unwrap_or(1);
    for face_idx in 0..face_count.min(8) {
        if let Ok(face) = ttf_parser::Face::parse(&bytes, face_idx) {
            let names = face.names();
            let mut zh: Option<String> = None;
            let mut en: Option<String> = None;
            let mut other: Option<String> = None;
            for i in 0..names.len() {
                let Some(rec) = names.get(i) else { continue };
                // name_id 1 = Family，16 = Typographic Family（更准确）
                if rec.name_id != 1 && rec.name_id != 16 {
                    continue;
                }
                let Some(s) = rec.to_string() else { continue };
                let lang = rec.language_id;
                // Windows 平台 (3) zh-CN 0x0804，zh-TW 0x0404，zh-HK 0x0C04
                if matches!(lang, 0x0804 | 0x0404 | 0x0C04 | 0x1004 | 0x1404) {
                    if rec.name_id == 16 {
                        return Some(s); // 中文首选名优先返回
                    }
                    zh = Some(s);
                } else if lang == 0x0409 {
                    if en.is_none() || rec.name_id == 16 {
                        en = Some(s);
                    }
                } else if other.is_none() {
                    other = Some(s);
                }
            }
            if let Some(n) = zh.or(en).or(other) {
                return Some(n);
            }
        }
    }
    None
}

pub fn font_display_name(path: &Path) -> String {
    static CACHE: OnceLock<Mutex<HashMap<PathBuf, String>>> = OnceLock::new();
    let cache = CACHE.get_or_init(|| Mutex::new(HashMap::new()));
    if let Ok(map) = cache.lock() {
        if let Some(s) = map.get(path) {
            return s.clone();
        }
    }
    let name = read_font_family_name(path).unwrap_or_else(|| {
        path.file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("?")
            .to_string()
    });
    if let Ok(mut map) = cache.lock() {
        map.insert(path.to_path_buf(), name.clone());
    }
    name
}

fn load_user_font(path: &str) -> Option<(String, Vec<u8>)> {
    let bytes = std::fs::read(path).ok()?;
    let display = font_display_name(Path::new(path));
    Some((display.replace([' ', '\t'], "_"), bytes))
}

/// 安装字体：UI 字体（列表前者优先）写入 Proportional，编辑器字体写入 Monospace；
/// 默认 CJK 与图标字体作为最终兜底。
pub fn install_with_lists(ctx: &egui::Context, ui_fonts: &[String], editor_fonts: &[String]) {
    let mut fonts = egui::FontDefinitions::default();

    // 工具：把一组字体路径按顺序插入指定 family
    let mut push_into = |family: egui::FontFamily, paths: &[String], key_prefix: &str| {
        for (i, path) in paths.iter().enumerate() {
            if let Some((name, bytes)) = load_user_font(path) {
                let key = format!("{}_{}_{}", key_prefix, i, name);
                fonts
                    .font_data
                    .insert(key.clone(), egui::FontData::from_owned(bytes));
                fonts.families.entry(family.clone()).or_default().insert(i, key);
            }
        }
    };

    push_into(egui::FontFamily::Proportional, ui_fonts, "ui");
    push_into(egui::FontFamily::Monospace, editor_fonts, "editor");

    // 默认 CJK 字体作为兜底
    if let Some((name, bytes)) = default_cjk() {
        let key = format!("default_cjk_{name}");
        fonts
            .font_data
            .insert(key.clone(), egui::FontData::from_static(bytes));
        fonts
            .families
            .entry(egui::FontFamily::Proportional)
            .or_default()
            .push(key.clone());
        fonts
            .families
            .entry(egui::FontFamily::Monospace)
            .or_default()
            .push(key);
    }

    // 图标字体
    let icon_key = crate::icons::ICON_FAMILY.to_string();
    fonts
        .font_data
        .insert(icon_key.clone(), egui::FontData::from_static(MATERIAL_ICONS));
    fonts.families.insert(
        egui::FontFamily::Name(crate::icons::ICON_FAMILY.into()),
        vec![icon_key.clone()],
    );
    fonts
        .families
        .entry(egui::FontFamily::Proportional)
        .or_default()
        .push(icon_key);

    ctx.set_fonts(fonts);
}

#[allow(dead_code)]
pub fn install_cjk_fonts(ctx: &egui::Context) {
    install_with_lists(ctx, &[], &[]);
}
