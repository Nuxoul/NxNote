//! 图片链接的抓取与展示支持：磁盘缓存 + 后台下载 + egui 纹理。
//! 下载走 PowerShell Invoke-WebRequest（不引入 HTTP 依赖，隐藏窗口）。

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::mpsc::{self, Receiver, Sender};
use std::thread;

use crate::config::data_dir;

pub struct ImageCache {
    tx: Sender<String>,
    rx: Receiver<(String, Result<PathBuf, String>)>,
    inflight: HashSet<String>,
    pub failed: HashSet<String>,
    /// url → 已就绪纹理
    pub ready: HashMap<String, egui::TextureHandle>,
}

impl Default for ImageCache {
    fn default() -> Self {
        Self::new()
    }
}

impl ImageCache {
    pub fn new() -> Self {
        let (tx_req, rx_req) = mpsc::channel::<String>();
        let (tx_done, rx_done) = mpsc::channel::<(String, Result<PathBuf, String>)>();
        // 单个下载线程串行处理，避免同时拉一堆图打满带宽
        thread::spawn(move || {
            while let Ok(url) = rx_req.recv() {
                let path = cache_path_for(&url);
                if !path.exists() {
                    if let Err(e) = download(&url, &path) {
                        let _ = tx_done.send((url, Err(e)));
                        continue;
                    }
                }
                let _ = tx_done.send((url, Ok(path)));
            }
        });
        Self {
            tx: tx_req,
            rx: rx_done,
            inflight: HashSet::new(),
            failed: HashSet::new(),
            ready: HashMap::new(),
        }
    }

    pub fn is_pending(&self, url: &str) -> bool {
        self.inflight.contains(url)
    }

    pub fn pending_count(&self) -> usize {
        self.inflight.len()
    }

    pub fn request(&mut self, url: &str) {
        if self.ready.contains_key(url)
            || self.failed.contains(url)
            || self.inflight.contains(url)
        {
            return;
        }
        // 磁盘缓存已有且能解码的，直接同步入 ready
        let path = cache_path_for(url);
        if path.exists() && path.metadata().map(|m| m.len() > 0).unwrap_or(false) {
            self.inflight.insert(url.to_string()); // 借用同一完成通道解码
            let _ = self.tx.send(url.to_string());
            return;
        }
        self.inflight.insert(url.to_string());
        let _ = self.tx.send(url.to_string());
    }

    /// 每帧收割完成的下载并解码为纹理。返回本帧新就绪的数量。
    pub fn drain(&mut self, ctx: &egui::Context) -> usize {
        let mut got = 0usize;
        while let Ok((url, result)) = self.rx.try_recv() {
            self.inflight.remove(&url);
            match result {
                Err(_) => {
                    self.failed.insert(url);
                }
                Ok(path) => match decode_to_texture(ctx, &url, &path) {
                    Ok(tex) => {
                        self.ready.insert(url, tex);
                        got += 1;
                    }
                    Err(_) => {
                        self.failed.insert(url);
                    }
                },
            }
        }
        got
    }

    pub fn texture(&self, url: &str) -> Option<&egui::TextureHandle> {
        self.ready.get(url)
    }
}

fn cache_path_for(url: &str) -> PathBuf {
    let hash = sha1_smol::Sha1::from(url.as_bytes()).digest().to_string();
    let ext = url
        .split('?')
        .next()
        .unwrap_or("")
        .rsplit('.')
        .next()
        .and_then(|e| {
            let e = e.to_lowercase();
            matches!(e.as_str(), "png" | "jpg" | "jpeg" | "gif" | "webp" | "bmp" | "avif")
                .then_some(e)
        })
        .unwrap_or_else(|| "img".to_string());
    data_dir().join("cache").join("images").join(format!("{hash}.{ext}"))
}

#[cfg(windows)]
fn download(url: &str, dst: &PathBuf) -> Result<(), String> {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    if let Some(p) = dst.parent() {
        let _ = std::fs::create_dir_all(p);
    }
    let tmp = dst.with_extension("part");
    let status = std::process::Command::new("powershell")
        .args([
            "-NoProfile",
            "-NonInteractive",
            "-ExecutionPolicy",
            "Bypass",
            "-Command",
            "$ProgressPreference='SilentlyContinue'; \
             [Net.ServicePointManager]::SecurityProtocol=[Net.SecurityProtocolType]::Tls12; \
             Invoke-WebRequest -UseBasicParsing -TimeoutSec 20 \
             -Uri $env:NX_IMG_URL -OutFile $env:NX_IMG_OUT",
        ])
        .env("NX_IMG_URL", url)
        .env("NX_IMG_OUT", &tmp)
        .creation_flags(CREATE_NO_WINDOW)
        .output()
        .map_err(|e| e.to_string())?;
    if !status.status.success() || !tmp.exists() {
        let _ = std::fs::remove_file(&tmp);
        return Err(format!("download failed: {}", String::from_utf8_lossy(&status.stderr)));
    }
    std::fs::rename(&tmp, dst).map_err(|e| e.to_string())
}

#[cfg(not(windows))]
fn download(_url: &str, _dst: &PathBuf) -> Result<(), String> {
    Err("unsupported platform".to_string())
}

fn decode_to_texture(
    ctx: &egui::Context,
    name: &str,
    path: &PathBuf,
) -> Result<egui::TextureHandle, String> {
    let bytes = std::fs::read(path).map_err(|e| e.to_string())?;
    let img = image::load_from_memory(&bytes).map_err(|e| e.to_string())?;
    let size_px = [img.width() as usize, img.height() as usize];
    if img.width() == 0 || img.height() == 0 {
        return Err("empty image".to_string());
    }
    let color = egui::ColorImage::from_rgba_unmultiplied(size_px, &img.to_rgba8());
    Ok(ctx.load_texture(name.to_string(), color, egui::TextureOptions::LINEAR))
}
