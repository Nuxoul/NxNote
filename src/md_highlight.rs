//! 行级 + inline 双层扫描的 markdown live 高亮。
//! 输出 LayoutJob：每段文本带自己的 TextFormat（字号/颜色/底色）。
//! 关键：原始字符（# ** ` 等语法标记）保持可见，只是样式不同。

use egui::text::LayoutJob;
use egui::{Color32, FontFamily, FontId, Stroke, TextFormat};

use crate::theme::Palette;

#[derive(Clone, Copy)]
pub struct Styles {
    pub p: Palette,
    pub base: f32,
}

impl Styles {
    fn fmt(&self, size: f32, color: Color32, family: FontFamily) -> TextFormat {
        TextFormat {
            font_id: FontId::new(size, family),
            color,
            ..Default::default()
        }
    }
    fn fmt_bg(&self, size: f32, color: Color32, bg: Color32, family: FontFamily) -> TextFormat {
        TextFormat {
            font_id: FontId::new(size, family),
            color,
            background: bg,
            ..Default::default()
        }
    }

    fn normal(&self) -> TextFormat {
        // DEBUG: 临时改成粉色，确认 layouter 是否运行
        self.fmt(
            self.base,
            egui::Color32::from_rgb(255, 100, 200),
            FontFamily::Monospace,
        )
    }
    fn syntax(&self) -> TextFormat {
        // 语法标记：弱色
        self.fmt(self.base, self.p.text_weak, FontFamily::Monospace)
    }
    fn heading(&self, level: u8) -> (TextFormat, TextFormat) {
        // 极其显眼：H1 2.0x、H2 1.6x，统一用 accent 色
        let scale = match level {
            1 => 2.0,
            2 => 1.6,
            3 => 1.35,
            4 => 1.2,
            5 => 1.1,
            _ => 1.05,
        };
        let size = self.base * scale;
        let body = self.fmt(size, self.p.accent, FontFamily::Proportional);
        let marker = TextFormat {
            font_id: FontId::new(size, FontFamily::Proportional),
            color: self.p.text_weak,
            ..Default::default()
        };
        (marker, body)
    }
    fn code_inline(&self) -> TextFormat {
        // 加强对比：明显的暗底 + 暖色文字
        self.fmt_bg(
            self.base,
            egui::Color32::from_rgb(220, 170, 100),
            egui::Color32::from_rgb(50, 42, 32),
            FontFamily::Monospace,
        )
    }
    fn code_block(&self) -> TextFormat {
        self.fmt_bg(
            self.base,
            egui::Color32::from_rgb(220, 215, 200),
            egui::Color32::from_rgb(50, 42, 32),
            FontFamily::Monospace,
        )
    }
    fn bold(&self) -> TextFormat {
        // 用纯白拉强对比
        self.fmt(self.base, egui::Color32::WHITE, FontFamily::Monospace)
    }
    fn italic(&self) -> TextFormat {
        TextFormat {
            font_id: FontId::new(self.base, FontFamily::Monospace),
            color: self.p.text,
            italics: true,
            ..Default::default()
        }
    }
    fn link(&self) -> TextFormat {
        TextFormat {
            font_id: FontId::new(self.base, FontFamily::Monospace),
            color: self.p.accent,
            underline: Stroke::new(1.0, self.p.accent),
            ..Default::default()
        }
    }
    fn quote(&self) -> TextFormat {
        TextFormat {
            font_id: FontId::new(self.base, FontFamily::Monospace),
            color: self.p.text_weak,
            italics: true,
            ..Default::default()
        }
    }
    fn hr(&self) -> TextFormat {
        self.fmt(self.base, self.p.stroke, FontFamily::Monospace)
    }
    fn list_marker(&self) -> TextFormat {
        self.fmt(self.base, self.p.accent, FontFamily::Monospace)
    }
}

pub fn build(text: &str, s: Styles) -> LayoutJob {
    let mut job = LayoutJob::default();
    let mut in_code_block = false;

    // 空文本：至少 append 一个空段，确保 galley 有合理 metric
    if text.is_empty() {
        job.append("", 0.0, s.normal());
        return job;
    }

    // 按 \n 分割，再分别 append 换行
    let lines: Vec<&str> = text.split('\n').collect();
    let last = lines.len() - 1;
    for (i, line) in lines.iter().enumerate() {
        process_line(&mut job, line, s, &mut in_code_block);
        if i < last {
            // 段落间的 \n 用代码块色 / 普通色
            if in_code_block {
                job.append("\n", 0.0, s.code_block());
            } else {
                job.append("\n", 0.0, s.normal());
            }
        }
    }
    job
}

fn process_line(job: &mut LayoutJob, line: &str, s: Styles, in_code_block: &mut bool) {
    // 代码块界限
    if line.trim_start().starts_with("```") {
        *in_code_block = !*in_code_block;
        job.append(line, 0.0, s.syntax());
        return;
    }
    if *in_code_block {
        job.append(line, 0.0, s.code_block());
        return;
    }

    // ATX 标题
    if let Some((level, prefix_len)) = atx_heading(line) {
        let (marker_fmt, body_fmt) = s.heading(level);
        job.append(&line[..prefix_len], 0.0, marker_fmt);
        append_inline_with(job, &line[prefix_len..], s, body_fmt);
        return;
    }

    // 水平分隔线
    if is_hr(line) {
        job.append(line, 0.0, s.hr());
        return;
    }

    // 块引用
    if let Some(rest_idx) = blockquote_prefix(line) {
        job.append(&line[..rest_idx], 0.0, s.syntax());
        append_inline_with(job, &line[rest_idx..], s, s.quote());
        return;
    }

    // 无序列表
    if let Some(marker_end) = unordered_list_marker(line) {
        job.append(&line[..marker_end], 0.0, s.list_marker());
        append_inline_with(job, &line[marker_end..], s, s.normal());
        return;
    }

    // 有序列表
    if let Some(marker_end) = ordered_list_marker(line) {
        job.append(&line[..marker_end], 0.0, s.list_marker());
        append_inline_with(job, &line[marker_end..], s, s.normal());
        return;
    }

    // 普通段落
    append_inline_with(job, line, s, s.normal());
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
        Some((n as u8, n + 1))
    } else {
        None
    }
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

fn unordered_list_marker(line: &str) -> Option<usize> {
    let b = line.as_bytes();
    let mut i = 0;
    while i < b.len() && (b[i] == b' ' || b[i] == b'\t') {
        i += 1;
    }
    if i > 8 {
        return None;
    }
    if i >= b.len() {
        return None;
    }
    let c = b[i];
    if !matches!(c, b'-' | b'*' | b'+') {
        return None;
    }
    if b.get(i + 1) == Some(&b' ') {
        Some(i + 2)
    } else {
        None
    }
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
    if b.get(i + 1) != Some(&b' ') {
        return None;
    }
    Some(i + 2)
}

/// 扫描 `code`, **bold**, *italic*, [text](url)。
/// `default` 应用于未匹配到任何标记的纯文本片段。
fn append_inline_with(job: &mut LayoutJob, text: &str, s: Styles, default: TextFormat) {
    let bytes = text.as_bytes();
    let mut i = 0;
    let mut buf_start = 0;

    while i < bytes.len() {
        let c = bytes[i];

        // `code`
        if c == b'`' {
            if let Some(end) = find_single_char_close(bytes, i + 1, b'`') {
                flush(job, text, &mut buf_start, i, &default);
                job.append(&text[i..=end], 0.0, s.code_inline());
                i = end + 1;
                buf_start = i;
                continue;
            }
        }
        // **bold**
        if c == b'*' && bytes.get(i + 1) == Some(&b'*') {
            if let Some(end) = find_double_star(bytes, i + 2) {
                flush(job, text, &mut buf_start, i, &default);
                job.append("**", 0.0, s.syntax());
                if end > i + 2 {
                    job.append(&text[i + 2..end], 0.0, s.bold());
                }
                job.append("**", 0.0, s.syntax());
                i = end + 2;
                buf_start = i;
                continue;
            }
        }
        // *italic*  (单星，避免吃掉双星)
        if c == b'*'
            && bytes.get(i + 1) != Some(&b'*')
            && (i == 0 || bytes[i - 1] != b'*')
        {
            if let Some(end) = find_single_star(bytes, i + 1) {
                if end > i + 1 {
                    flush(job, text, &mut buf_start, i, &default);
                    job.append("*", 0.0, s.syntax());
                    job.append(&text[i + 1..end], 0.0, s.italic());
                    job.append("*", 0.0, s.syntax());
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
                job.append("[", 0.0, s.syntax());
                if close_text > i + 1 {
                    job.append(&text[i + 1..close_text], 0.0, s.link());
                }
                job.append("](", 0.0, s.syntax());
                if close_url > close_text + 2 {
                    job.append(&text[close_text + 2..close_url], 0.0, s.syntax());
                }
                job.append(")", 0.0, s.syntax());
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
