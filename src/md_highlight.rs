//! 行级 + inline markdown live 高亮。
//! 每段文本带自己的 TextFormat（字号/颜色/底色/下划线）。
//! 原始字符（# ** ` 等语法标记）保持可见，只是样式不同。
//!
//! 样式定位（极强对比，避免"看起来没渲染"）：
//! - heading: 大字号 + 鲜亮琥珀
//! - inline code: 暖琥珀字 + 深巧克力色底
//! - bold: 纯白（最亮）
//! - italic: 斜体 + 浅琥珀
//! - link: 青色 + 下划线
//! - list marker: 鲜亮琥珀
//! - quote: 引用斜体灰
//! - hr: 暗色

use egui::text::LayoutJob;
use egui::{Color32, FontFamily, FontId, Stroke, TextFormat};

use crate::theme::Palette;

#[derive(Clone, Copy)]
pub struct Styles {
    pub p: Palette,
    pub base: f32,
}

const AMBER_BRIGHT: Color32 = Color32::from_rgb(245, 175, 90);
const CODE_TEXT: Color32 = Color32::from_rgb(245, 180, 110);
const CODE_BG: Color32 = Color32::from_rgb(52, 36, 22);
const LINK_CYAN: Color32 = Color32::from_rgb(120, 200, 220);

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
    fn syntax(&self) -> TextFormat {
        self.mono(self.base, self.p.text_weak)
    }
    fn heading(&self, level: u8) -> (TextFormat, TextFormat) {
        let scale = match level {
            1 => 1.9,
            2 => 1.55,
            3 => 1.3,
            4 => 1.15,
            5 => 1.08,
            _ => 1.04,
        };
        let size = self.base * scale;
        let body = self.prop(size, AMBER_BRIGHT);
        let marker = TextFormat {
            font_id: FontId::new(size, FontFamily::Proportional),
            color: self.p.text_weak,
            ..Default::default()
        };
        (marker, body)
    }
    fn code_inline(&self) -> TextFormat {
        TextFormat {
            font_id: FontId::new(self.base, FontFamily::Monospace),
            color: CODE_TEXT,
            background: CODE_BG,
            ..Default::default()
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
    fn link(&self) -> TextFormat {
        TextFormat {
            font_id: FontId::new(self.base, FontFamily::Monospace),
            color: LINK_CYAN,
            underline: Stroke::new(1.0, LINK_CYAN),
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
        self.mono(self.base, self.p.stroke)
    }
    fn list_marker(&self) -> TextFormat {
        self.mono(self.base, AMBER_BRIGHT)
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
        process_line(&mut job, line, s, &mut in_code_block);
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

fn process_line(job: &mut LayoutJob, line: &str, s: Styles, in_code_block: &mut bool) {
    if line.trim_start().starts_with("```") {
        *in_code_block = !*in_code_block;
        job.append(line, 0.0, s.syntax());
        return;
    }
    if *in_code_block {
        job.append(line, 0.0, s.code_block());
        return;
    }

    if let Some((level, prefix_len)) = atx_heading(line) {
        let (marker_fmt, body_fmt) = s.heading(level);
        job.append(&line[..prefix_len], 0.0, marker_fmt);
        append_inline_with(job, &line[prefix_len..], s, body_fmt);
        return;
    }

    if is_hr(line) {
        job.append(line, 0.0, s.hr());
        return;
    }

    if let Some(rest_idx) = blockquote_prefix(line) {
        job.append(&line[..rest_idx], 0.0, s.syntax());
        append_inline_with(job, &line[rest_idx..], s, s.quote());
        return;
    }

    if let Some(marker_end) = unordered_list_marker(line) {
        job.append(&line[..marker_end], 0.0, s.list_marker());
        append_inline_with(job, &line[marker_end..], s, s.normal());
        return;
    }

    if let Some(marker_end) = ordered_list_marker(line) {
        job.append(&line[..marker_end], 0.0, s.list_marker());
        append_inline_with(job, &line[marker_end..], s, s.normal());
        return;
    }

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
    // 接受 ASCII 空格或中文全角空格（U+3000，UTF-8 = E3 80 80）
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
    // ASCII 空格
    if b.get(i + 1) == Some(&b' ') {
        return Some(i + 2);
    }
    // 中文全角空格 U+3000 (E3 80 80)
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

fn append_inline_with(job: &mut LayoutJob, text: &str, s: Styles, default: TextFormat) {
    let bytes = text.as_bytes();
    let mut i = 0;
    let mut buf_start = 0;

    while i < bytes.len() {
        let c = bytes[i];

        if c == b'`' {
            if let Some(end) = find_single_char_close(bytes, i + 1, b'`') {
                flush(job, text, &mut buf_start, i, &default);
                job.append(&text[i..=end], 0.0, s.code_inline());
                i = end + 1;
                buf_start = i;
                continue;
            }
        }
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
