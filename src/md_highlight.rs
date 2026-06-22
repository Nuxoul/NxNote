//! Typora 风格的 inline markdown live preview。
//! - 不在光标行的 `# ** ` [ ]` 等语法标记 alpha=0 → 不可见但仍占位置
//! - 光标移到该行才显示标记
//! - 标记的可见性切换不引起内容重排（字号 / 宽度都保留）
//! - 列表 marker `- ` `1. ` 始终可见（属于"内容感"，不是噪声）
//! - blockquote `> ` 用 slate 蓝色，CJK 字体没斜体则颜色顶替

use egui::text::LayoutJob;
use egui::{Color32, FontFamily, FontId, Stroke, TextFormat};

use crate::theme::Palette;

#[derive(Clone, Copy)]
pub struct Styles {
    pub p: Palette,
    pub base: f32,
    pub cursor_line: Option<usize>,
}

const AMBER_BRIGHT: Color32 = Color32::from_rgb(245, 175, 90);
const CODE_TEXT: Color32 = Color32::from_rgb(245, 180, 110);
const CODE_BG: Color32 = Color32::from_rgb(52, 36, 22);
const LINK_CYAN: Color32 = Color32::from_rgb(120, 200, 220);
const QUOTE_COLOR: Color32 = Color32::from_rgb(140, 170, 200);
const QUOTE_BAR: Color32 = Color32::from_rgb(110, 145, 180);

/// 隐藏 marker 时使用的极小字号 —— Galley 按字号算宽度，0.1pt 视觉上接近 0
const HIDDEN_SIZE: f32 = 0.1;

fn hidden_fmt(family: FontFamily) -> TextFormat {
    TextFormat {
        font_id: FontId::new(HIDDEN_SIZE, family),
        color: Color32::TRANSPARENT,
        ..Default::default()
    }
}

impl Styles {
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
        self.mono(self.base, self.p.text)
    }
    fn syntax(&self, visible: bool) -> TextFormat {
        if visible {
            self.mono(self.base, self.p.text_weak)
        } else {
            hidden_fmt(FontFamily::Monospace)
        }
    }
    fn heading_marker(&self, level: u8, visible: bool) -> TextFormat {
        if visible {
            let size = self.base * heading_scale(level);
            TextFormat {
                font_id: FontId::new(size, FontFamily::Proportional),
                color: self.p.text_weak,
                ..Default::default()
            }
        } else {
            hidden_fmt(FontFamily::Proportional)
        }
    }
    fn heading_body(&self, level: u8) -> TextFormat {
        let size = self.base * heading_scale(level);
        self.prop(size, AMBER_BRIGHT)
    }
    fn code_inline_text(&self) -> TextFormat {
        TextFormat {
            font_id: FontId::new(self.base, FontFamily::Monospace),
            color: CODE_TEXT,
            background: CODE_BG,
            ..Default::default()
        }
    }
    fn code_inline_marker(&self, visible: bool) -> TextFormat {
        if visible {
            // 反引号也加上 CODE_BG 让背景连续
            TextFormat {
                font_id: FontId::new(self.base, FontFamily::Monospace),
                color: self.p.text_weak,
                background: CODE_BG,
                ..Default::default()
            }
        } else {
            hidden_fmt(FontFamily::Monospace)
        }
    }
    fn code_block(&self) -> TextFormat {
        TextFormat {
            font_id: FontId::new(self.base, FontFamily::Monospace),
            color: Color32::from_rgb(225, 220, 205),
            background: CODE_BG,
            ..Default::default()
        }
    }
    fn bold(&self) -> TextFormat {
        self.mono(self.base, Color32::WHITE)
    }
    fn italic(&self) -> TextFormat {
        TextFormat {
            font_id: FontId::new(self.base, FontFamily::Monospace),
            color: AMBER_BRIGHT,
            italics: true,
            ..Default::default()
        }
    }
    fn link_text(&self) -> TextFormat {
        TextFormat {
            font_id: FontId::new(self.base, FontFamily::Monospace),
            color: LINK_CYAN,
            underline: Stroke::new(1.0, LINK_CYAN),
            ..Default::default()
        }
    }
    fn quote_marker(&self, visible: bool) -> TextFormat {
        if visible {
            TextFormat {
                font_id: FontId::new(self.base, FontFamily::Monospace),
                color: QUOTE_BAR,
                ..Default::default()
            }
        } else {
            hidden_fmt(FontFamily::Monospace)
        }
    }
    fn quote(&self) -> TextFormat {
        self.mono(self.base, QUOTE_COLOR)
    }
    fn hr(&self) -> TextFormat {
        self.mono(self.base, self.p.stroke)
    }
    fn list_marker(&self) -> TextFormat {
        self.mono(self.base, AMBER_BRIGHT)
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

pub fn build(text: &str, s: Styles) -> LayoutJob {
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

fn process_line(job: &mut LayoutJob, line: &str, on_cursor: bool, s: Styles, in_code_block: &mut bool) {
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

    if let Some(marker_end) = unordered_list_marker(line) {
        // 列表 marker 始终可见
        job.append(&line[..marker_end], 0.0, s.list_marker());
        append_inline_with(job, &line[marker_end..], s, s.normal(), on_cursor);
        return;
    }

    if let Some(marker_end) = ordered_list_marker(line) {
        job.append(&line[..marker_end], 0.0, s.list_marker());
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

fn unordered_list_marker(line: &str) -> Option<usize> {
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

fn append_inline_with(
    job: &mut LayoutJob,
    text: &str,
    s: Styles,
    default: TextFormat,
    on_cursor: bool,
) {
    let bytes = text.as_bytes();
    let mut i = 0;
    let mut buf_start = 0;

    while i < bytes.len() {
        let c = bytes[i];

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
