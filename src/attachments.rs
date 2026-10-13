//! Images are external files; document text always contains their real paths.
use anyhow::{Context, Result};
use gpui_kit::component::input::{InlineToken, InputContent};
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
    pub fn token(&self, path: &str) -> InlineToken {
        InlineToken::new(path.to_string(), path.to_string()).with_label(
            self.labels.get(path).cloned().unwrap_or_else(|| {
                Path::new(path)
                    .file_name()
                    .and_then(|n| n.to_str())
                    .unwrap_or("image.png")
                    .into()
            }),
        )
    }
    pub fn content(&self, text: &str) -> InputContent {
        let mut content = InputContent::new(text.to_string());
        let mut ranges = Vec::new();
        for path in self.labels.keys() {
            for (start, _) in text.match_indices(path) {
                ranges.push((start..start + path.len(), path));
            }
        }
        ranges.sort_by(|(a, _), (b, _)| a.start.cmp(&b.start).then(b.end.cmp(&a.end)));
        let mut end = 0;
        for (range, path) in ranges {
            if range.start < end {
                continue;
            }
            end = range.end;
            if let Ok(next) = content.clone().with_token(range, self.token(path)) {
                content = next;
            }
        }
        content
    }
    pub fn preview_markdown(&self, text: &str) -> String {
        let mut output = text.to_string();
        for (path, label) in &self.labels {
            let src = path.replace('\\', "/").replace(' ', "%20");
            output = output.replace(
                path,
                &format!("![{}](file:///{src})", label.replace(['[', ']'], "")),
            );
        }
        output
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

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn tokens_preserve_source_paths_and_unicode() {
        let path = r"C:\图片\my image.png";
        let attachments = Attachments {
            labels: BTreeMap::from([(path.into(), "my image.png".into())]),
        };
        let text = format!("中文 {path} 后文 {path}");
        let content = attachments.content(&text);
        assert_eq!(content.text().as_ref(), text);
        assert_eq!(content.tokens().len(), 2);
        assert_eq!(content.tokens()[0].token().text().as_ref(), path);
        assert_eq!(content.tokens()[0].token().label().as_ref(), "my image.png");
    }
    #[test]
    fn ordinary_text_is_not_a_token() {
        assert!(Attachments::default()
            .content("image.png 普通文字")
            .tokens()
            .is_empty());
    }
}
