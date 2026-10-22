//! External image storage and backward-compatible path/filename registry.
use anyhow::{Context, Result};
use std::{
    collections::BTreeMap,
    io::Cursor,
    path::{Path, PathBuf},
};

static REGISTRY_WRITE: std::sync::Mutex<()> = std::sync::Mutex::new(());
#[derive(Default, Clone)]
pub struct Attachments {
    labels: BTreeMap<String, String>,
}
impl Attachments {
    pub fn load() -> Result<Self> {
        let path = crate::config::data_dir().join("attachments.json");
        match std::fs::read(&path) {
            Ok(bytes) => Ok(Self {
                labels: serde_json::from_slice(&bytes).context("图片引用清单损坏，未覆盖原文件")?,
            }),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Self::default()),
            Err(e) => Err(e.into()),
        }
    }
    pub fn register(&mut self, path: &Path, label: String) -> Result<String> {
        let path = path.to_string_lossy().to_string();
        anyhow::ensure!(
            !path.chars().any(char::is_control),
            "图片路径不能含控制字符"
        );
        let _guard = REGISTRY_WRITE
            .lock()
            .map_err(|_| anyhow::anyhow!("图片引用清单锁不可用"))?;
        let mut next = Self::load()?.labels;
        next.extend(self.labels.clone());
        next.insert(path.clone(), label);
        crate::fsutil::write_atomic(
            &crate::config::data_dir().join("attachments.json"),
            &serde_json::to_vec_pretty(&next)?,
        )?;
        self.labels = next;
        Ok(path)
    }
    pub fn save_image(&mut self, bytes: &[u8]) -> Result<String> {
        let image = image::load_from_memory(bytes).context("无法解码剪贴板图片")?;
        let mut png = Cursor::new(Vec::new());
        image.write_to(&mut png, image::ImageFormat::Png)?;
        let hash = sha1_smol::Sha1::from(png.get_ref().as_slice())
            .digest()
            .to_string();
        let dir = crate::config::data_dir().join("attachments");
        std::fs::create_dir_all(&dir)?;
        let path = dir.join(format!("image-{}.png", &hash[..12]));
        if !path.exists() {
            crate::fsutil::write_atomic(&path, png.get_ref())?;
        }
        let path = std::fs::canonicalize(&path).unwrap_or(path);
        // Use ordinary Windows paths, not extended-length \\?\ paths in copied prose.
        let path = PathBuf::from(
            path.to_string_lossy()
                .strip_prefix(r"\\?\")
                .unwrap_or(&path.to_string_lossy())
                .to_string(),
        );
        self.register(&path, "image.png".into())
    }
}

pub fn is_image_file(path: &Path) -> bool {
    path.extension().and_then(|e| e.to_str()).is_some_and(|e| {
        matches!(
            e.to_ascii_lowercase().as_str(),
            "png" | "jpg" | "jpeg" | "gif" | "webp" | "bmp"
        )
    })
}
