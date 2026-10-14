//! Non-HTML Markdown extensions, expressed exclusively as original UTF-8 byte ranges.
//!
//! Block delimiters are physical lines (LF or CRLF), with at most three leading
//! spaces. Fences follow CommonMark's same-character, at-least-opening-length
//! closing rule. Unclosed custom blocks remain ordinary Markdown. Callout bodies
//! retain their `>` prefixes: a single contiguous range cannot strip every line.
//! Footnote bodies likewise retain continuation indentation; duplicate IDs use
//! the first definition. Callout kinds are uppercase and absent titles are empty.
//!
//! Inline extensions do not nest: the first complete outer span wins. Ordinary
//! emphasis inside their contents is left untouched. Superscript/subscript reject
//! unescaped whitespace; math is single-line. Dollar matching uses Pandoc-style
//! whitespace/digit heuristics, not a currency or TeX parser (`$5$` is valid math).
//! Number-leading dollar contents with whitespace are conservatively currency,
//! unless they contain an explicit mathematical operator.
//! Wiki targets retain heading fragments; an omitted label is the full target.
//! Escaped ASCII punctuation in wiki strings is decoded, not in source ranges.
//! Existing GFM AST positions only identify protected Markdown (including nested
//! code containers); extension recognition itself is a source-range scanner.
//! Images are wholly opaque; link labels may contain extensions, destinations
//! and reference syntax may not. Raw HTML nodes are opaque, never interpreted or
//! rendered. This module does not sanitize or render any source, including HTML.

use markdown::{mdast::Node, ParseOptions};
use std::{collections::BTreeMap, ops::Range};

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum InlineKind {
    Highlight,
    Superscript,
    Subscript,
    Math,
    WikiLink { target: String, label: String },
    NoteEmbed { target: String },
    Footnote { id: String },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InlineExtension {
    pub source: Range<usize>,
    pub content: Range<usize>,
    pub kind: InlineKind,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum BlockKind {
    FrontMatter,
    Math,
    Mermaid,
    Callout { kind: String, title: String },
    FootnoteDefinition { id: String },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BlockExtension {
    pub source: Range<usize>,
    pub content: Range<usize>,
    pub kind: BlockKind,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Extensions {
    pub inline: Vec<InlineExtension>,
    pub blocks: Vec<BlockExtension>,
    pub footnotes: BTreeMap<String, Range<usize>>,
}

impl Extensions {
    pub fn parse(source: &str) -> Self {
        let lines = physical_lines(source);
        let mut result = Self::default();
        let mut protected = Vec::new();
        let mut code = Vec::new();
        if let Ok(root) = markdown::to_mdast(source, &ParseOptions::gfm()) {
            collect_code(&root, &mut code);
        }
        result.scan_blocks(source, &lines, &code, &mut protected);

        // Special block interiors must not change how the ordinary Markdown
        // parser recognizes subsequent code, links, or HTML. Masking is bytewise
        // and newline-preserving, so its AST offsets still refer to the source.
        let mut opaque: Vec<_> = result.blocks.iter().filter(|block| {
            matches!(&block.kind, BlockKind::FrontMatter | BlockKind::Math | BlockKind::Mermaid)
        }).map(|block| block.source.clone()).collect();
        // Prevent ordinary reference-definition parsing from hiding a footnote
        // body. The original bytes and the definition's content remain intact.
        opaque.extend(result.blocks.iter().filter_map(|block| {
            matches!(&block.kind, BlockKind::FootnoteDefinition { .. })
                .then(|| block.source.start..block.content.start)
        }));
        let parse_source = mask_ranges(source, &opaque);
        if let Ok(root) = markdown::to_mdast(&parse_source, &ParseOptions::gfm()) {
            collect_protected(&root, &mut protected);
        }
        protected.retain(|range| valid_range(source, range));
        merge_ranges(&mut protected);
        result.scan_inline(source, &protected);
        // Both scanners advance past accepted spans; sorting states that contract
        // explicitly, including for callers relying on ordered source edits.
        result.blocks.sort_by_key(|block| block.source.start);
        result.inline.sort_by_key(|inline| inline.source.start);
        result
    }

    /// Hide complete inline atoms from the original Markdown parser. Newline
    /// bytes (including CRLF's CR) and total byte length are unchanged. Since the
    /// model is public, malformed or non-UTF-8-boundary ranges are ignored safely.
    /// Block spans are deliberately not masked by this API.
    pub fn mask_inline(&self, source: &str) -> String {
        let ranges: Vec<_> = self.inline.iter().map(|inline| inline.source.clone()).collect();
        mask_ranges(source, &ranges)
    }

    fn scan_blocks(
        &mut self,
        source: &str,
        lines: &[Line],
        code: &[Range<usize>],
        protected: &mut Vec<Range<usize>>,
    ) {
        let mut i = 0;
        // A UTF-8 BOM is allowed before the first document-start YAML marker.
        if let Some(first) = lines.first() {
            let text = source[first.start..first.end].strip_prefix('\u{feff}')
                .unwrap_or(&source[first.start..first.end]);
            if text.trim_end_matches([' ', '\t']) == "---" {
                if let Some(close) = (1..lines.len()).find(|&j| {
                    matches!(source[lines[j].start..lines[j].end].trim_end_matches([' ', '\t']), "---" | "...")
                }) {
                    self.push_block(lines, 0, close, first.next..lines[close].start, BlockKind::FrontMatter);
                    protected.push(self.blocks.last().unwrap().source.clone());
                    i = close + 1;
                }
            }
        }
        while i < lines.len() {
            let line = lines[i];
            let text = &source[line.start..line.end];
            let Some((indent, body)) = block_body(text) else {
                i += 1;
                continue;
            };
            let marker = line.start + indent;
            // AST code guards also cover fences/indented code inside lists and
            // quotes. Ignore stale AST code that originated inside a previously
            // recognized opaque block (e.g. backticks in a math expression).
            let containing_code = code.iter().find(|range| {
                range.start <= marker && marker < range.end && !self.blocks.iter().any(|block| {
                    matches!(&block.kind, BlockKind::FrontMatter | BlockKind::Math | BlockKind::Mermaid)
                        && block.source.contains(&range.start)
                })
            });
            if let Some(fence) = fence_open(body) {
                if containing_code.is_some_and(|range| range.start != marker && range.start != line.start) {
                    i += 1;
                    continue;
                }
                let close = (i + 1..lines.len()).find(|&j| {
                    block_body(&source[lines[j].start..lines[j].end])
                        .is_some_and(|(_, body)| fence_close(body, fence.character, fence.length))
                });
                let last = close.unwrap_or(lines.len() - 1);
                let range = line.start..lines[last].next;
                if fence.info == "mermaid" && close.is_some() {
                    self.push_block(lines, i, last, line.next..lines[last].start, BlockKind::Mermaid);
                }
                // An unclosed mermaid fence is still ordinary fenced code.
                protected.push(range);
                i = last + 1;
                continue;
            }
            if containing_code.is_some() {
                i += 1;
                continue;
            }
            if body.trim_end_matches([' ', '\t']) == "$$" {
                if let Some(close) = (i + 1..lines.len()).find(|&j| {
                    block_body(&source[lines[j].start..lines[j].end])
                        .is_some_and(|(_, body)| body.trim_end_matches([' ', '\t']) == "$$")
                }) {
                    self.push_block(lines, i, close, line.next..lines[close].start, BlockKind::Math);
                    protected.push(self.blocks.last().unwrap().source.clone());
                    i = close + 1;
                    continue;
                }
            }
            if let Some((kind, title)) = callout_header(body) {
                let mut last = i;
                while last + 1 < lines.len() && quote_body(&source[lines[last + 1].start..lines[last + 1].end]).is_some() {
                    last += 1;
                }
                let content = if last == i { line.end..line.end } else { line.next..lines[last].end };
                self.push_block(lines, i, last, content, BlockKind::Callout { kind, title });
                // Inline scanning still visits quote bodies separately; block
                // scanning skips them so nested headers cannot overlap this span.
                i = last + 1;
                continue;
            }
            if let Some((id, after_colon)) = footnote_definition(body) {
                let mut last = i;
                let mut next = i + 1;
                while next < lines.len() {
                    let continuation = &source[lines[next].start..lines[next].end];
                    if continuation.trim_matches([' ', '\t']).is_empty() {
                        next += 1;
                    } else if indentation(continuation) >= 4 {
                        last = next;
                        next += 1;
                    } else {
                        break;
                    }
                }
                let content_start = marker + after_colon;
                let content = content_start..lines[last].end;
                self.footnotes.entry(id.clone()).or_insert_with(|| content.clone());
                self.push_block(lines, i, last, content, BlockKind::FootnoteDefinition { id });
                protected.push(marker..content_start);
                i = last + 1;
                continue;
            }
            i += 1;
        }
    }

    fn push_block(&mut self, lines: &[Line], first: usize, last: usize, content: Range<usize>, kind: BlockKind) {
        self.blocks.push(BlockExtension {
            source: lines[first].start..lines[last].next,
            content,
            kind,
        });
    }

    fn scan_inline(&mut self, source: &str, protected: &[Range<usize>]) {
        let bytes = source.as_bytes();
        let mut i = 0;
        while i < bytes.len() {
            if let Some(range) = protected_at(protected, i) {
                i = range.end;
                continue;
            }
            if !escaped(bytes, i) {
                let remaining = &source[i..];
                let atom = if remaining.starts_with("![[") {
                    wiki(source, i, true, protected)
                } else if remaining.starts_with("[[") && !(i > 0 && bytes[i - 1] == b'!') {
                    wiki(source, i, false, protected)
                } else if remaining.starts_with("[^") {
                    footnote_reference(source, i, protected)
                } else if exact_run(bytes, i, b'=', 2) {
                    delimited(source, i, b'=', 2, InlineKind::Highlight, protected)
                } else if exact_run(bytes, i, b'^', 1) {
                    delimited(source, i, b'^', 1, InlineKind::Superscript, protected)
                } else if exact_run(bytes, i, b'~', 1) {
                    delimited(source, i, b'~', 1, InlineKind::Subscript, protected)
                } else if exact_run(bytes, i, b'$', 1) {
                    delimited(source, i, b'$', 1, InlineKind::Math, protected)
                } else {
                    None
                };
                if let Some(atom) = atom.filter(|atom| {
                    valid_range(source, &atom.source) && valid_range(source, &atom.content)
                        && !protected.iter().any(|range| {
                            range.start < atom.source.end && atom.source.start < range.end
                        })
                }) {
                    i = atom.source.end;
                    self.inline.push(atom);
                    continue;
                }
            }
            i += source[i..].chars().next().unwrap().len_utf8();
        }
    }
}

#[derive(Clone, Copy)]
struct Line {
    start: usize,
    end: usize,
    next: usize,
}

fn physical_lines(source: &str) -> Vec<Line> {
    let mut result = Vec::new();
    let mut start = 0;
    for text in source.split_inclusive('\n') {
        let next = start + text.len();
        let end = if text.ends_with("\r\n") { next - 2 }
            else if text.ends_with('\n') { next - 1 }
            else { next };
        result.push(Line { start, end, next });
        start = next;
    }
    result
}

fn block_body(text: &str) -> Option<(usize, &str)> {
    let spaces = text.bytes().take_while(|byte| *byte == b' ').count();
    if spaces > 3 || text.as_bytes().get(spaces) == Some(&b'\t') { return None; }
    Some((spaces, &text[spaces..]))
}

struct Fence<'a> {
    character: u8,
    length: usize,
    info: &'a str,
}

fn fence_open(body: &str) -> Option<Fence<'_>> {
    let character = *body.as_bytes().first()?;
    if !matches!(character, b'`' | b'~') { return None; }
    let length = body.bytes().take_while(|byte| *byte == character).count();
    if length < 3 { return None; }
    let info = body[length..].trim_matches([' ', '\t']);
    if character == b'`' && info.contains('`') { return None; }
    Some(Fence { character, length, info })
}

fn fence_close(body: &str, character: u8, length: usize) -> bool {
    let run = body.bytes().take_while(|byte| *byte == character).count();
    run >= length && body[run..].trim_matches([' ', '\t']).is_empty()
}

fn quote_body(text: &str) -> Option<&str> {
    let (_, body) = block_body(text)?;
    let body = body.strip_prefix('>')?;
    Some(body.strip_prefix([' ', '\t']).unwrap_or(body))
}

fn callout_header(body: &str) -> Option<(String, String)> {
    let text = quote_body(body)?.strip_prefix("[!")?;
    let close = text.find(']')?;
    let kind = &text[..close];
    if !matches!(kind, "NOTE" | "TIP" | "IMPORTANT" | "WARNING" | "CAUTION") { return None; }
    let title = &text[close + 1..];
    if !title.is_empty() && !title.starts_with([' ', '\t']) { return None; }
    Some((kind.to_string(), title.trim_matches([' ', '\t']).to_string()))
}

fn indentation(text: &str) -> usize {
    let mut columns = 0;
    for byte in text.bytes() {
        match byte {
            b' ' => columns += 1,
            b'\t' => columns += 4 - columns % 4,
            _ => break,
        }
    }
    columns
}

fn valid_id(id: &str) -> bool {
    !id.is_empty() && !id.chars().any(|ch| ch.is_whitespace() || matches!(ch, '[' | ']'))
}

fn footnote_definition(body: &str) -> Option<(String, usize)> {
    let rest = body.strip_prefix("[^")?;
    let close = rest.find("]:")?;
    let id = &rest[..close];
    if !valid_id(id) || escaped(body.as_bytes(), close + 2) { return None; }
    let after = close + 4;
    let spaces = body[after..].bytes().take_while(|byte| matches!(*byte, b' ' | b'\t')).count();
    Some((id.to_string(), after + spaces))
}

fn escaped(bytes: &[u8], at: usize) -> bool {
    let mut start = at;
    while start > 0 && bytes[start - 1] == b'\\' { start -= 1; }
    (at - start) % 2 == 1
}

fn exact_run(bytes: &[u8], at: usize, delimiter: u8, count: usize) -> bool {
    (at == 0 || bytes[at - 1] != delimiter)
        && bytes.get(at..at + count).is_some_and(|run| run.iter().all(|byte| *byte == delimiter))
        && bytes.get(at + count) != Some(&delimiter)
}

fn delimited(
    source: &str,
    start: usize,
    delimiter: u8,
    width: usize,
    kind: InlineKind,
    protected: &[Range<usize>],
) -> Option<InlineExtension> {
    let bytes = source.as_bytes();
    let content_start = start + width;
    let first = source.get(content_start..)?.chars().next()?;
    if first.is_whitespace() { return None; }
    let math = matches!(&kind, InlineKind::Math);
    let script = matches!(&kind, InlineKind::Superscript | InlineKind::Subscript);
    if math && start > 0 && bytes[start - 1].is_ascii_digit() { return None; }
    let mut i = content_start;
    while i < bytes.len() {
        if protected_at(protected, i).is_some() { return None; }
        if exact_run(bytes, i, delimiter, width) && !escaped(bytes, i) {
            let content = content_start..i;
            if content.is_empty() || source[content.clone()].chars().next_back()?.is_whitespace() {
                return None;
            }
            if math && bytes.get(i + width).is_some_and(u8::is_ascii_digit) { return None; }
            if math && first.is_ascii_digit() {
                let text = &source[content.clone()];
                if text.chars().any(char::is_whitespace)
                    && !text.contains(['+', '-', '*', '/', '=', '^', '_', '\\']) {
                    return None;
                }
            }
            return Some(InlineExtension { source: start..i + width, content, kind });
        }
        let ch = source[i..].chars().next()?;
        if matches!(ch, '\r' | '\n') {
            if math || script { return None; }
            // Highlights may cross soft line breaks, but not paragraph breaks.
            let next = if ch == '\r' && bytes.get(i + 1) == Some(&b'\n') { i + 2 } else { i + ch.len_utf8() };
            if source[next..].trim_start_matches([' ', '\t']).starts_with(['\r', '\n']) { return None; }
            i = next;
            continue;
        } else if script && ch.is_whitespace() && !escaped(bytes, i) {
            return None;
        }
        i += ch.len_utf8();
    }
    None
}

fn wiki(source: &str, start: usize, embed: bool, protected: &[Range<usize>]) -> Option<InlineExtension> {
    let content_start = start + if embed { 3 } else { 2 };
    let bytes = source.as_bytes();
    let mut i = content_start;
    let mut pipe = None;
    while i < bytes.len() {
        if protected_at(protected, i).is_some() { return None; }
        let ch = source[i..].chars().next()?;
        if matches!(ch, '\r' | '\n') { return None; }
        if !escaped(bytes, i) {
            if source[i..].starts_with("]]") {
                let target_range = trimmed_range(source, content_start..pipe.unwrap_or(i));
                if target_range.is_empty() { return None; }
                let target = decode_escapes(&source[target_range.clone()]);
                let (kind, content) = if embed {
                    // Note embeds have a target, not an image-style caption.
                    if pipe.is_some() { return None; }
                    (InlineKind::NoteEmbed { target }, target_range)
                } else {
                    let label_range = pipe.map_or(target_range.clone(), |pipe| trimmed_range(source, pipe + 1..i));
                    if label_range.is_empty() { return None; }
                    let label = decode_escapes(&source[label_range.clone()]);
                    (InlineKind::WikiLink { target, label }, label_range)
                };
                return Some(InlineExtension { source: start..i + 2, content, kind });
            }
            if matches!(ch, '[' | ']') { return None; }
            if ch == '|' && pipe.is_none() { pipe = Some(i); }
        }
        i += ch.len_utf8();
    }
    None
}

fn trimmed_range(source: &str, range: Range<usize>) -> Range<usize> {
    let text = &source[range.clone()];
    let start = range.start + text.len() - text.trim_start().len();
    let end = range.end - (text.len() - text.trim_end().len());
    start..end.max(start)
}

fn decode_escapes(text: &str) -> String {
    let mut result = String::new();
    let mut chars = text.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch == '\\' && chars.peek().is_some_and(|next| next.is_ascii_punctuation()) {
            result.push(chars.next().unwrap());
        } else {
            result.push(ch);
        }
    }
    result
}

fn footnote_reference(source: &str, start: usize, protected: &[Range<usize>]) -> Option<InlineExtension> {
    let content_start = start + 2;
    let mut i = content_start;
    while i < source.len() {
        if protected_at(protected, i).is_some() { return None; }
        let ch = source[i..].chars().next()?;
        if ch == ']' && !escaped(source.as_bytes(), i) {
            let id = &source[content_start..i];
            if !valid_id(id) || source.as_bytes().get(i + 1) == Some(&b':') { return None; }
            return Some(InlineExtension {
                source: start..i + 1,
                content: content_start..i,
                kind: InlineKind::Footnote { id: id.to_string() },
            });
        }
        if ch.is_whitespace() || ch == '[' { return None; }
        i += ch.len_utf8();
    }
    None
}

fn node_range(node: &Node) -> Option<Range<usize>> {
    node.position().map(|position| position.start.offset..position.end.offset)
}

fn collect_code(node: &Node, ranges: &mut Vec<Range<usize>>) {
    if matches!(node, Node::Code(_) | Node::InlineCode(_) | Node::Html(_) | Node::Image(_) | Node::ImageReference(_) | Node::Link(_) | Node::LinkReference(_)) {
        if let Some(range) = node_range(node) { ranges.push(range); }
        return;
    }
    if let Some(children) = node.children() {
        for child in children { collect_code(child, ranges); }
    }
}

fn collect_protected(node: &Node, ranges: &mut Vec<Range<usize>>) {
    if matches!(node, Node::Code(_) | Node::InlineCode(_) | Node::Html(_) | Node::Image(_) | Node::ImageReference(_) | Node::Definition(_)) {
        if let Some(range) = node_range(node) { ranges.push(range); }
        return;
    }
    if matches!(node, Node::Link(_) | Node::LinkReference(_)) {
        if let Some(range) = node_range(node) {
            // Only label children are scanned. For autolinks their text occupies
            // the destination too, so protect those wholly instead.
            let children = node.children().map(Vec::as_slice).unwrap_or(&[]);
            let first = children.first().and_then(node_range);
            let last = children.last().and_then(node_range);
            if let (Some(first), Some(last)) = (first, last) {
                if first.start <= range.start || (first.start == range.start + 1 && last.end + 1 == range.end) {
                    // Covers <https://...> and <email@...> autolinks.
                    // A collapsed [label] reference has the same shape; keeping
                    // it opaque avoids treating reference IDs as display syntax.
                    ranges.push(range);
                    return;
                }
                ranges.push(range.start..first.start);
                ranges.push(last.end..range.end);
            } else {
                ranges.push(range);
                return;
            }
        }
    }
    if let Some(children) = node.children() {
        for child in children { collect_protected(child, ranges); }
    }
}

fn merge_ranges(ranges: &mut Vec<Range<usize>>) {
    ranges.retain(|range| !range.is_empty());
    ranges.sort_by_key(|range| (range.start, range.end));
    let mut merged: Vec<Range<usize>> = Vec::new();
    for range in ranges.drain(..) {
        if let Some(last) = merged.last_mut().filter(|last| range.start <= last.end) {
            last.end = last.end.max(range.end);
        } else {
            merged.push(range);
        }
    }
    *ranges = merged;
}

fn protected_at(ranges: &[Range<usize>], offset: usize) -> Option<&Range<usize>> {
    let index = ranges.partition_point(|range| range.end <= offset);
    ranges.get(index).filter(|range| range.start <= offset)
}

fn valid_range(source: &str, range: &Range<usize>) -> bool {
    range.start <= range.end && range.end <= source.len()
        && source.is_char_boundary(range.start) && source.is_char_boundary(range.end)
}

fn mask_ranges(source: &str, ranges: &[Range<usize>]) -> String {
    let mut bytes = source.as_bytes().to_vec();
    for range in ranges {
        if !valid_range(source, range) { continue; }
        for byte in &mut bytes[range.clone()] {
            if !matches!(*byte, b'\r' | b'\n') { *byte = b'x'; }
        }
    }
    // Each replacement covers complete Unicode characters with ASCII bytes.
    String::from_utf8(bytes).expect("boundary-checked masks preserve UTF-8")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn contents<'a>(source: &'a str, extensions: &Extensions) -> Vec<&'a str> {
        extensions.inline.iter().map(|inline| &source[inline.content.clone()]).collect()
    }

    fn assert_ranges(source: &str, extensions: &Extensions) {
        for inline in &extensions.inline {
            assert!(valid_range(source, &inline.source));
            assert!(valid_range(source, &inline.content));
            assert!(inline.source.start <= inline.content.start && inline.content.end <= inline.source.end);
        }
        for block in &extensions.blocks {
            assert!(valid_range(source, &block.source));
            assert!(valid_range(source, &block.content));
            assert!(block.source.start <= block.content.start && block.content.end <= block.source.end);
        }
        assert!(extensions.inline.windows(2).all(|pair| pair[0].source.end <= pair[1].source.start));
        assert!(extensions.blocks.windows(2).all(|pair| pair[0].source.end <= pair[1].source.start));
    }

    #[test]
    fn unicode_mixed_markdown_and_nested_ordinary_emphasis() {
        let source = "中文 **==高亮 *强调*==** 与 ^上标^ H~下标~ $α + β$ [[笔记#标题]] [[目录|标签]] ![[嵌入]] [^注]";
        let extensions = Extensions::parse(source);
        assert_eq!(contents(source, &extensions), vec!["高亮 *强调*", "上标", "下标", "α + β", "笔记#标题", "标签", "嵌入", "注"]);
        assert_eq!(extensions.inline[4].kind, InlineKind::WikiLink { target: "笔记#标题".into(), label: "笔记#标题".into() });
        assert_eq!(extensions.inline[5].kind, InlineKind::WikiLink { target: "目录".into(), label: "标签".into() });
        assert_eq!(extensions.inline[6].kind, InlineKind::NoteEmbed { target: "嵌入".into() });
        assert_ranges(source, &extensions);
    }

    #[test]
    fn escaped_delimiters_and_strikethrough_are_not_extensions() {
        let source = r"\==no== \^no^ \~no~ \$no$ \[[no]] \![[no]] \[^no] ~~strike~~ ===no=== ^^no^^ $$no$$ ==yes==";
        let extensions = Extensions::parse(source);
        assert_eq!(contents(source, &extensions), vec!["yes"]);
        let source = r"==a\==b== [[a\|b|c\]d]] \\^yes^";
        let extensions = Extensions::parse(source);
        assert_eq!(contents(source, &extensions), vec![r"a\==b", r"c\]d", "yes"]);
        assert_eq!(extensions.inline[1].kind, InlineKind::WikiLink { target: "a|b".into(), label: "c]d".into() });
    }

    #[test]
    fn code_and_destinations_are_opaque_but_link_labels_are_not() {
        let source = concat!(
            "`==inline==` ``^code^``\n\n",
            "```rust\n==fenced== [[note]]\n```\n\n",
            "    ==indented==\n\n",
            "> ```\n> ==quoted code==\n> ```\n\n",
            "[==label==](https://example.test/^hidden^ \"==title==\")\n",
            "![==image==](image~hidden~.png)\n",
            "[normal][==ref==]\n\n[==ref==]: https://example.test/$hidden$\n\n",
            "==visible==\n"
        );
        let extensions = Extensions::parse(source);
        assert_eq!(contents(source, &extensions), vec!["label", "visible"]);
        assert_ranges(source, &extensions);
    }

    #[test]
    fn dollars_and_script_whitespace_use_conservative_matching() {
        let source = r"$5 and $10; cost 5$ more; $ x$ $x $ $x$2 $a\$b$ $5$ $x + y$ ^two words^ ~two words~ ^two\ words^";
        let extensions = Extensions::parse(source);
        assert_eq!(contents(source, &extensions), vec![r"a\$b", "5", "x + y", r"two\ words"]);
        assert!(extensions.inline[..3].iter().all(|inline| inline.kind == InlineKind::Math));
    }

    #[test]
    fn crlf_blocks_have_exact_original_byte_ranges() {
        let source = concat!(
            "---\r\ntitle: 中文 ==hidden==\r\n...\r\n",
            "$$\r\nα + ^hidden^\r\n$$\r\n",
            "~~~~mermaid\r\ngraph TD; A-->B\r\n~~~\r\n~~~~~\r\n",
            "> [!WARNING] 注意\r\n> ==正文==\r\n> 下一行\r\n",
            "[^注]: 说明\r\n    延续\r\n\r\n\t末行\r\n",
            "最后 [^注]\r\n"
        );
        let extensions = Extensions::parse(source);
        assert_eq!(extensions.blocks.len(), 5);
        assert_eq!(&source[extensions.blocks[0].content.clone()], "title: 中文 ==hidden==\r\n");
        assert_eq!(&source[extensions.blocks[1].content.clone()], "α + ^hidden^\r\n");
        assert_eq!(&source[extensions.blocks[2].content.clone()], "graph TD; A-->B\r\n~~~\r\n");
        assert_eq!(extensions.blocks[3].kind, BlockKind::Callout { kind: "WARNING".into(), title: "注意".into() });
        assert_eq!(&source[extensions.blocks[3].content.clone()], "> ==正文==\r\n> 下一行");
        assert_eq!(&source[extensions.footnotes["注"].clone()], "说明\r\n    延续\r\n\r\n\t末行");
        assert_eq!(contents(source, &extensions), vec!["正文", "注"]);
        assert_ranges(source, &extensions);
    }

    #[test]
    fn front_matter_is_only_at_document_start_and_requires_a_close() {
        assert!(matches!(Extensions::parse("---\nx: y\n---\n").blocks[0].kind, BlockKind::FrontMatter));
        assert!(matches!(Extensions::parse("\u{feff}---\nx: y\n...\n").blocks[0].kind, BlockKind::FrontMatter));
        assert!(Extensions::parse("\n---\nx: y\n---\n").blocks.is_empty());
        assert!(Extensions::parse("---\nx: y\n").blocks.is_empty());
    }

    #[test]
    fn ordinary_fences_hide_block_markers_and_unclosed_fences_remain_code() {
        let source = "````text\n```mermaid\n$$\n> [!NOTE]\n[^x]: no\n````\n==yes==\n";
        let extensions = Extensions::parse(source);
        assert!(extensions.blocks.is_empty());
        assert_eq!(contents(source, &extensions), vec!["yes"]);
        let source = "```mermaid\n==no==\n~~~\n";
        let extensions = Extensions::parse(source);
        assert!(extensions.blocks.is_empty());
        assert!(extensions.inline.is_empty());
        let extensions = Extensions::parse("```mermaid\nx\n```\n~~~mermaid\ny\n~~~\n");
        assert_eq!(extensions.blocks.len(), 2);
        assert!(extensions.blocks.iter().all(|block| block.kind == BlockKind::Mermaid));
    }

    #[test]
    fn callout_kinds_titles_and_quote_termination() {
        for kind in ["NOTE", "TIP", "IMPORTANT", "WARNING", "CAUTION"] {
            let source = format!("> [!{kind}]\n> body\n>\n> more\nnot quoted\n");
            let extensions = Extensions::parse(&source);
            assert_eq!(extensions.blocks.len(), 1);
            assert_eq!(extensions.blocks[0].kind, BlockKind::Callout { kind: kind.into(), title: String::new() });
            assert_eq!(&source[extensions.blocks[0].content.clone()], "> body\n>\n> more");
        }
        assert!(Extensions::parse("> [!UNKNOWN]\n> no\n").blocks.is_empty());
        let extensions = Extensions::parse("> [!TIP] Solo");
        assert!(extensions.blocks[0].content.is_empty());
    }

    #[test]
    fn footnote_duplicates_first_wins_and_trailing_blanks_are_not_content() {
        let source = "[^a]: first ==body==\n\n    continued\n\nplain [^a]\n\n[^a]: second\n";
        let extensions = Extensions::parse(source);
        assert_eq!(extensions.blocks.len(), 2);
        assert_eq!(&source[extensions.footnotes["a"].clone()], "first ==body==\n\n    continued");
        assert_eq!(contents(source, &extensions), vec!["body", "a"]);
        assert_ranges(source, &extensions);
    }

    #[test]
    fn masking_preserves_bytes_crlf_and_blocks_not_unicode_char_count() {
        let source = "---\r\nx: 中文\r\n---\r\n==中文\r\n第二行== [^注]";
        let extensions = Extensions::parse(source);
        let masked = extensions.mask_inline(source);
        assert_eq!(masked.len(), source.len());
        assert_eq!(&masked[..extensions.blocks[0].source.end], &source[..extensions.blocks[0].source.end]);
        for (i, byte) in source.bytes().enumerate() {
            if matches!(byte, b'\r' | b'\n') { assert_eq!(masked.as_bytes()[i], byte); }
            else if extensions.inline.iter().any(|inline| inline.source.contains(&i)) {
                assert_eq!(masked.as_bytes()[i], b'x');
            } else { assert_eq!(masked.as_bytes()[i], byte); }
        }
        assert_ranges(source, &extensions);
    }

    #[test]
    fn malformed_public_ranges_are_ignored_by_masking() {
        let source = "中文 ==ok==";
        let mut extensions = Extensions::parse(source);
        extensions.inline.push(InlineExtension { source: 1..2, content: 1..2, kind: InlineKind::Math });
        extensions.inline.push(InlineExtension { source: 90..100, content: 90..100, kind: InlineKind::Math });
        assert_eq!(extensions.mask_inline(source), "中文 xxxxxx");
        assert!(Extensions::parse("").inline.is_empty());
    }

    #[test]
    fn raw_html_is_not_an_extension_or_a_renderer() {
        let source = "<script>\n==not rendered==\n$not math$\n</script>\n\n==plain==\n";
        let extensions = Extensions::parse(source);
        assert!(extensions.blocks.is_empty());
        assert_eq!(contents(source, &extensions), vec!["plain"]);
        assert!(extensions.mask_inline(source).starts_with("<script>\n==not rendered=="));
        assert!(Extensions::parse("<mark>text</mark>\n").inline.is_empty());
    }

    #[test]
    fn callout_body_headers_do_not_create_overlapping_blocks() {
        let source = "> [!NOTE] Outer\n> ==body==\n> [!WARNING] Inner text\n> more\n";
        let extensions = Extensions::parse(source);
        assert_eq!(extensions.blocks.len(), 1);
        assert_eq!(contents(source, &extensions), vec!["body"]);
        assert_ranges(source, &extensions);
    }

    #[test]
    fn html_and_multiline_inline_code_hide_block_delimiters() {
        let source = "<script>\n$$\n> [!WARNING]\n$$\n</script>\n\n`literal\n$$\n> [!NOTE]\n$$\ncode`\n";
        let extensions = Extensions::parse(source);
        assert!(extensions.blocks.is_empty());
        assert!(extensions.inline.is_empty());
    }

    #[test]
    fn currency_pairs_and_numeric_equations() {
        let source = "$5 and $10 cost 5$ more; $2 + 3$; $5$";
        let extensions = Extensions::parse(source);
        assert_eq!(contents(source, &extensions), vec!["2 + 3", "5"]);
        assert_ranges(source, &extensions);
    }

    #[test]
    fn custom_interiors_do_not_leak_unclosed_markdown_fences() {
        let source = "---\nx: ```\n---\n$$\n```\n$$\n==visible==\n";
        let extensions = Extensions::parse(source);
        assert_eq!(extensions.blocks.len(), 2);
        assert_eq!(contents(source, &extensions), vec!["visible"]);
        assert_ranges(source, &extensions);
    }
}
