//! Typora 风格的 inline markdown live preview。
//! 颜色全部来自 Config.md_dark / md_light，可在「颜色配置」三级窗口里改。
//!
//! - 不在光标行的 `# ** ` [ ]` 等语法标记 字号→0.1pt 不可见也不占位
//! - 无序列表 `-` `*` `+` 的字符整段透明（保宽度），由 draw_editor 在原位
//!   overlay 一个 · 字形——所以即使光标在该行，看到的也是 ·
//! - 任务列表 `- [ ] ` / `- [x] ` 整段透明保宽，draw_editor 在原位画复选框
//! - 有序列表 `1.` `2.` 始终用 list_marker 颜色（数字本身有内容感）
//! - `~~删除线~~` 支持；链接区间可经 collect_links() 导出供点击打开

use egui::text::LayoutJob;
use egui::{Color32, FontFamily, FontId, Stroke, TextFormat};

use crate::config::MdColors;
use crate::theme::Palette;

#[derive(Clone, Copy)]
pub struct Styles<'a> {
    pub p: Palette,
    pub base: f32,
    pub cursor_line: Option<usize>,
    /// 图片占位字符的字号（撑起图片行行高）；<=0 表示不启用
    pub img_ph_size: f32,
    pub c: &'a MdColors,
}

/// 隐藏 marker 时使用的极小字号 —— Galley 按字号算宽度，0.1pt 视觉上接近 0
const HIDDEN_SIZE: f32 = 0.1;

fn rgb(c: [u8; 3]) -> Color32 {
    Color32::from_rgb(c[0], c[1], c[2])
}

fn hidden_fmt(family: FontFamily) -> TextFormat {
    TextFormat {
        font_id: FontId::new(HIDDEN_SIZE, family),
        color: Color32::TRANSPARENT,
        ..Default::default()
    }
}

impl<'a> Styles<'a> {
    fn mono(&self, size: f32, color: Color32) -> TextFormat {
        TextFormat {
            font_id: FontId::new(size, FontFamily::Monospace),
            color,
            ..Default::default()
        }
    }
    fn prop(&self, size: f32, color: Color32) -> TextFormat {
        TextFormat {
            font_id: FontId::new(size, FontFamily::Proportional),
            color,
            ..Default::default()
        }
    }

    fn normal(&self) -> TextFormat {
        self.mono(self.base, rgb(self.c.text))
    }
    fn syntax(&self, visible: bool) -> TextFormat {
        if visible {
            self.mono(self.base, rgb(self.c.syntax))
        } else {
            hidden_fmt(FontFamily::Monospace)
        }
    }
    fn heading_marker(&self, level: u8, visible: bool) -> TextFormat {
        if visible {
            let size = self.base * heading_scale(level);
            TextFormat {
                font_id: FontId::new(size, FontFamily::Proportional),
                color: rgb(self.c.syntax),
                ..Default::default()
            }
        } else {
            hidden_fmt(FontFamily::Proportional)
        }
    }
    fn heading_body(&self, level: u8) -> TextFormat {
        let size = self.base * heading_scale(level);
        self.prop(size, rgb(self.c.heading))
    }
    fn code_inline_text(&self) -> TextFormat {
        TextFormat {
            font_id: FontId::new(self.base, FontFamily::Monospace),
            color: rgb(self.c.code_text),
            background: rgb(self.c.code_bg),
            ..Default::default()
        }
    }
    fn code_inline_marker(&self, visible: bool) -> TextFormat {
        if visible {
            TextFormat {
                font_id: FontId::new(self.base, FontFamily::Monospace),
                color: rgb(self.c.syntax),
                background: rgb(self.c.code_bg),
                ..Default::default()
            }
        } else {
            hidden_fmt(FontFamily::Monospace)
        }
    }
    fn code_block(&self) -> TextFormat {
        TextFormat {
            font_id: FontId::new(self.base, FontFamily::Monospace),
            color: rgb(self.c.code_text),
            background: rgb(self.c.code_bg),
            ..Default::default()
        }
    }
    fn bold(&self) -> TextFormat {
        self.mono(self.base, rgb(self.c.bold))
    }
    fn italic(&self) -> TextFormat {
        TextFormat {
            font_id: FontId::new(self.base, FontFamily::Monospace),
            color: rgb(self.c.italic),
            italics: true,
            ..Default::default()
        }
    }
    fn strike(&self) -> TextFormat {
        TextFormat {
            font_id: FontId::new(self.base, FontFamily::Monospace),
            color: rgb(self.c.text),
            strikethrough: Stroke::new(1.0, rgb(self.c.syntax)),
            ..Default::default()
        }
    }
    fn link_text(&self) -> TextFormat {
        TextFormat {
            font_id: FontId::new(self.base, FontFamily::Monospace),
            color: rgb(self.c.link),
            underline: Stroke::new(1.0, rgb(self.c.link)),
            ..Default::default()
        }
    }
    fn quote_marker(&self, visible: bool) -> TextFormat {
        if visible {
            TextFormat {
                font_id: FontId::new(self.base, FontFamily::Monospace),
                color: rgb(self.c.quote_bar),
                ..Default::default()
            }
        } else {
            hidden_fmt(FontFamily::Monospace)
        }
    }
    fn quote(&self) -> TextFormat {
        self.mono(self.base, rgb(self.c.quote_text))
    }
    fn hr(&self) -> TextFormat {
        self.mono(self.base, self.p.stroke)
    }
    fn list_marker_ordered(&self) -> TextFormat {
        self.mono(self.base, rgb(self.c.list_marker))
    }
    /// 无序 marker：完全透明保宽——draw_editor 会画一个 · 覆盖
    fn list_marker_unordered_hidden(&self) -> TextFormat {
        TextFormat {
            font_id: FontId::new(self.base, FontFamily::Monospace),
            color: Color32::TRANSPARENT,
            ..Default::default()
        }
    }
    /// 图片占位字符：透明 + 超大字号 —— 撑起整行行高给内嵌图片留位
    fn img_placeholder(&self) -> TextFormat {
        TextFormat {
            font_id: FontId::new(self.img_ph_size.max(1.0), FontFamily::Monospace),
            color: Color32::TRANSPARENT,
            ..Default::default()
        }
    }
}

fn heading_scale(level: u8) -> f32 {
    match level {
        1 => 1.9,
        2 => 1.55,
        3 => 1.3,
        4 => 1.15,
        5 => 1.08,
        _ => 1.04,
    }
}

/// 任务列表标记：`- [ ] ` / `- [x] ` / `* [X]`。
/// 返回 (indent_end, 勾选字符字节位, marker_end(含尾随空格), 是否已勾选)
pub fn task_marker(line: &str) -> Option<(usize, usize, usize, bool)> {
    let b = line.as_bytes();
    let mut i = 0;
    while i < b.len() && (b[i] == b' ' || b[i] == b'\t') {
        i += 1;
    }
    if i > 8 {
        return None;
    }
    if !matches!(b.get(i), Some(b'-' | b'*' | b'+')) || b.get(i + 1) != Some(&b' ') {
        return None;
    }
    if b.get(i + 2) != Some(&b'[') {
        return None;
    }
    let flag = *b.get(i + 3)?;
    if flag != b' ' && flag != b'x' && flag != b'X' {
        return None;
    }
    match b.get(i + 4) {
        Some(b']') => {}
        _ => return None,
    }
    // marker_end："] " 或行尾 "]"
    let marker_end = if b.get(i + 5) == Some(&b' ') {
        i + 6
    } else if i + 5 == b.len() {
        i + 5
    } else {
        return None;
    };
    Some((i, i + 3, marker_end, flag != b' '))
}

pub fn build(text: &str, s: Styles<'_>) -> LayoutJob {
    let mut job = LayoutJob::default();
    let mut in_code_block = false;

    if text.is_empty() {
        job.append("", 0.0, s.normal());
        return job;
    }

    let lines: Vec<&str> = text.split('\n').collect();
    let last = lines.len() - 1;
    for (i, line) in lines.iter().enumerate() {
        let on_cursor = s.cursor_line == Some(i);
        process_line(&mut job, line, on_cursor, s, &mut in_code_block);
        if i < last {
            if in_code_block {
                job.append("\n", 0.0, s.code_block());
            } else {
                job.append("\n", 0.0, s.normal());
            }
        }
    }
    job
}

/// 整行就是一张图片引用的两种形式：
/// A) 可选缩进 + ![alt](url)，行尾无其它内容
/// B) 整行为一个裸图片 URL（png/jpg/... 结尾）
/// 返回 (行内起始字节, 行内结束字节, url)。
pub fn image_line_span(line: &str) -> Option<(usize, usize, String)> {
    let trimmed = line.trim();
    if trimmed.is_empty() {
        return None;
    }
    // 形式 B：裸图片 URL
    if !trimmed.starts_with("![") && is_image_url(trimmed) {
        let start = line.len() - line.trim_start().len();
        return Some((start, start + trimmed.len(), trimmed.to_string()));
    }
    // 形式 A：markdown 图片
    if !trimmed.starts_with("![") {
        return None;
    }
    let ind = line.len() - line.trim_start().len();
    let b = line.as_bytes();
    let i = ind;
    if b.get(i) != Some(&b'!') || b.get(i + 1) != Some(&b'[') {
        return None;
    }
    let (_ct, cu) = find_link(b, i + 1)?;
    // ]( 之后必须直接到行尾（允许尾随空白）
    let rest = line[cu + 1..].trim();
    if !rest.is_empty() {
        return None;
    }
    let raw = &line[i + 2..cu];
    let url = raw.split_once("](").map(|(_, u)| u.trim().to_string())?;
    if url.is_empty() {
        return None;
    }
    Some((i, cu + 1, url))
}

/// 只扫描链接区间（不做排版）。点击编辑器时用，避免依赖 layout 结果。
/// 返回相对整篇文本的 (byte_start, byte_end_exclusive)。
pub fn collect_links(text: &str) -> Vec<(usize, usize)> {
    let mut out = Vec::new();
    let mut base = 0usize;
    for line in text.split('\n') {
        scan_links(line.as_bytes(), base, &mut out);
        base += line.len() + 1;
    }
    out
}

fn scan_links(bytes: &[u8], base: usize, out: &mut Vec<(usize, usize)>) {
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'[' {
            if let Some((_close_text, close_url)) = find_link(bytes, i) {
                out.push((base + i, base + close_url + 1));
                i = close_url + 1;
                continue;
            }
        }
        i += 1;
    }
}

fn process_line(
    job: &mut LayoutJob,
    line: &str,
    on_cursor: bool,
    s: Styles<'_>,
    in_code_block: &mut bool,
) {
    if line.trim_start().starts_with("```") {
        *in_code_block = !*in_code_block;
        job.append(line, 0.0, s.syntax(on_cursor));
        return;
    }
    if *in_code_block {
        job.append(line, 0.0, s.code_block());
        return;
    }

    if let Some((level, prefix_len)) = atx_heading(line) {
        let marker = s.heading_marker(level, on_cursor);
        let body = s.heading_body(level);
        job.append(&line[..prefix_len], 0.0, marker);
        append_inline_with(job, &line[prefix_len..], s, body, on_cursor);
        return;
    }

    if is_hr(line) {
        job.append(line, 0.0, s.hr());
        return;
    }

    if let Some(rest_idx) = blockquote_prefix(line) {
        job.append(&line[..rest_idx], 0.0, s.quote_marker(on_cursor));
        append_inline_with(job, &line[rest_idx..], s, s.quote(), on_cursor);
        return;
    }

    if let Some((ind, _flag_byte, marker_end, _checked)) = task_marker(line) {
        // 整段 "- [ ] " 透明保宽，draw_editor 在原位画复选框
        if ind > 0 {
            job.append(&line[..ind], 0.0, s.normal());
        }
        job.append(&line[ind..marker_end], 0.0, s.list_marker_unordered_hidden());
        append_inline_with(job, &line[marker_end..], s, s.normal(), on_cursor);
        return;
    }

    // 整行图片：光标行显示原始 Markdown；非光标行用超大透明字符撑行高，
    // draw_editor 把真实纹理贴到这块预留区域（Typora 式文内预览）
    if let Some((sp, ep, _url)) = image_line_span(line) {
        job.append(&line[..sp], 0.0, s.normal());
        if on_cursor {
            job.append(&line[sp..], 0.0, s.syntax(true));
        } else {
            // 首字符大字号撑高，其余极小隐形
            job.append(&line[sp..sp + 1], 0.0, s.img_placeholder());
            job.append(&line[sp + 1..ep], 0.0, hidden_fmt(FontFamily::Monospace));
            if line.len() > ep {
                job.append(&line[ep..], 0.0, s.list_marker_unordered_hidden());
            }
        }
        return;
    }

    if let Some(parts) = unordered_list_marker(line) {
        let (indent_end, marker_end) = parts;
        // 缩进保正常
        if indent_end > 0 {
            job.append(&line[..indent_end], 0.0, s.normal());
        }
        // 单字符 marker 透明保宽 (draw_editor 在这位置 overlay ·)
        job.append(&line[indent_end..indent_end + 1], 0.0, s.list_marker_unordered_hidden());
        // marker 后的空格 / 全角空格也用 normal
        if marker_end > indent_end + 1 {
            job.append(&line[indent_end + 1..marker_end], 0.0, s.normal());
        }
        append_inline_with(job, &line[marker_end..], s, s.normal(), on_cursor);
        return;
    }

    if let Some(marker_end) = ordered_list_marker(line) {
        job.append(&line[..marker_end], 0.0, s.list_marker_ordered());
        append_inline_with(job, &line[marker_end..], s, s.normal(), on_cursor);
        return;
    }

    append_inline_with(job, line, s, s.normal(), on_cursor);
}

fn atx_heading(line: &str) -> Option<(u8, usize)> {
    let b = line.as_bytes();
    let mut n = 0;
    while n < b.len() && n < 6 && b[n] == b'#' {
        n += 1;
    }
    if n == 0 {
        return None;
    }
    if b.get(n) == Some(&b' ') {
        return Some((n as u8, n + 1));
    }
    if b.get(n) == Some(&0xE3) && b.get(n + 1) == Some(&0x80) && b.get(n + 2) == Some(&0x80) {
        return Some((n as u8, n + 3));
    }
    None
}

fn is_hr(line: &str) -> bool {
    let t = line.trim();
    if t.len() < 3 {
        return false;
    }
    let c = t.as_bytes()[0];
    if !matches!(c, b'-' | b'*' | b'_') {
        return false;
    }
    t.bytes().all(|x| x == c || x == b' ' || x == b'\t')
}

fn blockquote_prefix(line: &str) -> Option<usize> {
    let b = line.as_bytes();
    if b.first() != Some(&b'>') {
        return None;
    }
    if b.get(1) == Some(&b' ') {
        Some(2)
    } else {
        Some(1)
    }
}

/// 返回 (indent_end, marker_end). marker 字节位置 = indent_end (single ASCII byte)
pub fn unordered_list_marker(line: &str) -> Option<(usize, usize)> {
    let b = line.as_bytes();
    let mut i = 0;
    while i < b.len() && (b[i] == b' ' || b[i] == b'\t') {
        i += 1;
    }
    if i > 8 || i >= b.len() {
        return None;
    }
    let c = b[i];
    if !matches!(c, b'-' | b'*' | b'+') {
        return None;
    }
    let indent_end = i;
    if b.get(i + 1) == Some(&b' ') {
        return Some((indent_end, i + 2));
    }
    if b.get(i + 1) == Some(&0xE3)
        && b.get(i + 2) == Some(&0x80)
        && b.get(i + 3) == Some(&0x80)
    {
        return Some((indent_end, i + 4));
    }
    None
}

fn ordered_list_marker(line: &str) -> Option<usize> {
    let b = line.as_bytes();
    let mut i = 0;
    while i < b.len() && (b[i] == b' ' || b[i] == b'\t') {
        i += 1;
    }
    if i > 8 {
        return None;
    }
    let start = i;
    while i < b.len() && b[i].is_ascii_digit() {
        i += 1;
    }
    if i == start || i - start > 9 {
        return None;
    }
    if b.get(i) != Some(&b'.') {
        return None;
    }
    if b.get(i + 1) == Some(&b' ') {
        return Some(i + 2);
    }
    if b.get(i + 1) == Some(&0xE3)
        && b.get(i + 2) == Some(&0x80)
        && b.get(i + 3) == Some(&0x80)
    {
        return Some(i + 4);
    }
    None
}

/// 判断是否为图片链接（http/https 且扩展名为常见图片格式）。
pub fn is_image_url(s: &str) -> bool {
    let t = s.trim();
    let lower = t.to_lowercase();
    (lower.starts_with("http://") || lower.starts_with("https://"))
        && ["png", "jpg", "jpeg", "gif", "webp", "bmp", "avif"]
            .iter()
            .any(|ext| {
                lower
                    .split('?')
                    .next()
                    .unwrap_or("")
                    .ends_with(&format!(".{ext}"))
            })
}

/// 收集全文中的图片引用：`![alt](url)` 以及"整行就是一个裸图片 URL"两种形式。
/// 返回 (byte_start, byte_end, url)。
pub fn collect_images(text: &str) -> Vec<(usize, usize, String)> {
    let mut out = Vec::new();
    let mut base = 0usize;
    for line in text.split('\n') {
        let bytes = line.as_bytes();
        let mut i = 0;
        while i < bytes.len() {
            if bytes[i] == b'!' && bytes.get(i + 1) == Some(&b'[') {
                if let Some((_ct, cu)) = find_link(bytes, i + 1) {
                    let raw = &line[i + 2..cu]; // alt](url 的内部
                    if let Some(url) = raw.split_once("](").map(|(_, u)| u.trim().to_string()) {
                        if !url.is_empty() {
                            out.push((base + i, base + cu + 1, url));
                        }
                    }
                    i = cu + 1;
                    continue;
                }
            }
            i += 1;
        }
        // 裸图片 URL 行
        let trimmed = line.trim();
        if is_image_url(trimmed) {
            let s = line.len() - line.trim_start().len();
            out.push((base + s, base + line.len(), trimmed.to_string()));
        }
        base += line.len() + 1;
    }
    out
}

fn append_inline_with(
    job: &mut LayoutJob,
    text: &str,
    s: Styles<'_>,
    default: TextFormat,
    on_cursor: bool,
) {
    let bytes = text.as_bytes();
    let mut i = 0;
    let mut buf_start = 0;

    while i < bytes.len() {
        let c = bytes[i];

        // ![alt](url) 图片：非光标行整段隐藏保宽（draw_editor 画图片图标芯片）
        if c == b'!' && bytes.get(i + 1) == Some(&b'[') {
            if let Some((_ct, cu)) = find_link(bytes, i + 1) {
                flush(job, text, &mut buf_start, i, &default);
                let fmt = if on_cursor { s.syntax(true) } else { hidden_fmt(FontFamily::Monospace) };
                job.append(&text[i..cu + 1], 0.0, fmt);
                i = cu + 1;
                buf_start = i;
                continue;
            }
        }
        // `code`
        if c == b'`' {
            if let Some(end) = find_single_char_close(bytes, i + 1, b'`') {
                flush(job, text, &mut buf_start, i, &default);
                job.append("`", 0.0, s.code_inline_marker(on_cursor));
                if end > i + 1 {
                    job.append(&text[i + 1..end], 0.0, s.code_inline_text());
                }
                job.append("`", 0.0, s.code_inline_marker(on_cursor));
                i = end + 1;
                buf_start = i;
                continue;
            }
        }
        // **bold**
        if c == b'*' && bytes.get(i + 1) == Some(&b'*') {
            if let Some(end) = find_double_star(bytes, i + 2) {
                flush(job, text, &mut buf_start, i, &default);
                job.append("**", 0.0, s.syntax(on_cursor));
                if end > i + 2 {
                    job.append(&text[i + 2..end], 0.0, s.bold());
                }
                job.append("**", 0.0, s.syntax(on_cursor));
                i = end + 2;
                buf_start = i;
                continue;
            }
        }
        // ~~strike~~
        if c == b'~' && bytes.get(i + 1) == Some(&b'~') {
            if let Some(end) = find_double_tilde(bytes, i + 2) {
                if end > i + 2 {
                    flush(job, text, &mut buf_start, i, &default);
                    job.append("~~", 0.0, s.syntax(on_cursor));
                    job.append(&text[i + 2..end], 0.0, s.strike());
                    job.append("~~", 0.0, s.syntax(on_cursor));
                    i = end + 2;
                    buf_start = i;
                    continue;
                }
            }
        }
        // *italic*
        if c == b'*'
            && bytes.get(i + 1) != Some(&b'*')
            && (i == 0 || bytes[i - 1] != b'*')
        {
            if let Some(end) = find_single_star(bytes, i + 1) {
                if end > i + 1 {
                    flush(job, text, &mut buf_start, i, &default);
                    job.append("*", 0.0, s.syntax(on_cursor));
                    job.append(&text[i + 1..end], 0.0, s.italic());
                    job.append("*", 0.0, s.syntax(on_cursor));
                    i = end + 1;
                    buf_start = i;
                    continue;
                }
            }
        }
        // [text](url)
        if c == b'[' {
            if let Some((close_text, close_url)) = find_link(bytes, i) {
                flush(job, text, &mut buf_start, i, &default);
                job.append("[", 0.0, s.syntax(on_cursor));
                if close_text > i + 1 {
                    job.append(&text[i + 1..close_text], 0.0, s.link_text());
                }
                job.append("](", 0.0, s.syntax(on_cursor));
                if close_url > close_text + 2 {
                    job.append(&text[close_text + 2..close_url], 0.0, s.syntax(on_cursor));
                }
                job.append(")", 0.0, s.syntax(on_cursor));
                i = close_url + 1;
                buf_start = i;
                continue;
            }
        }
        i += 1;
    }
    flush(job, text, &mut buf_start, bytes.len(), &default);
}

fn flush(job: &mut LayoutJob, text: &str, start: &mut usize, end: usize, fmt: &TextFormat) {
    if end > *start {
        job.append(&text[*start..end], 0.0, fmt.clone());
    }
    *start = end;
}

fn find_single_char_close(bytes: &[u8], from: usize, target: u8) -> Option<usize> {
    let mut i = from;
    while i < bytes.len() {
        if bytes[i] == target {
            return Some(i);
        }
        if bytes[i] == b'\n' {
            return None;
        }
        i += 1;
    }
    None
}

fn find_double_star(bytes: &[u8], from: usize) -> Option<usize> {
    let mut i = from;
    while i + 1 < bytes.len() {
        if bytes[i] == b'\n' {
            return None;
        }
        if bytes[i] == b'*' && bytes[i + 1] == b'*' {
            return Some(i);
        }
        i += 1;
    }
    None
}

fn find_double_tilde(bytes: &[u8], from: usize) -> Option<usize> {
    let mut i = from;
    while i + 1 < bytes.len() {
        if bytes[i] == b'\n' {
            return None;
        }
        if bytes[i] == b'~' && bytes[i + 1] == b'~' {
            return Some(i);
        }
        i += 1;
    }
    None
}

fn find_single_star(bytes: &[u8], from: usize) -> Option<usize> {
    let mut i = from;
    while i < bytes.len() {
        if bytes[i] == b'\n' {
            return None;
        }
        if bytes[i] == b'*' && bytes.get(i + 1) != Some(&b'*') {
            return Some(i);
        }
        i += 1;
    }
    None
}

fn find_link(bytes: &[u8], from: usize) -> Option<(usize, usize)> {
    let close_text = find_link_text_end(bytes, from + 1)?;
    if bytes.get(close_text + 1) != Some(&b'(') {
        return None;
    }
    let close_url = find_link_url_end(bytes, close_text + 2)?;
    Some((close_text, close_url))
}

fn find_link_text_end(bytes: &[u8], from: usize) -> Option<usize> {
    let mut i = from;
    while i < bytes.len() {
        match bytes[i] {
            b']' => return Some(i),
            b'\n' => return None,
            _ => i += 1,
        }
    }
    None
}

fn find_link_url_end(bytes: &[u8], from: usize) -> Option<usize> {
    let mut i = from;
    while i < bytes.len() {
        match bytes[i] {
            b')' => return Some(i),
            b'\n' | b' ' => return None,
            _ => i += 1,
        }
    }
    None
}
