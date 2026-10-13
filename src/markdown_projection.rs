//! Editable Markdown projection. Every displayed character retains its source range.
//! This module contains no input state: the entire document has one selection and IME handler.
use crate::attachments::Attachments;
use markdown::{mdast::{AlignKind, Node}, ParseOptions};
use std::{collections::BTreeMap, ops::Range};

#[derive(Clone, Copy, Default, PartialEq, Eq)]
pub struct Marks {
    pub bold: bool,
    pub italic: bool,
    pub strike: bool,
    pub code: bool,
    pub link: bool,
    pub syntax: bool,
}
#[derive(Clone)]
pub struct SourceChar {
    pub display: Range<usize>,
    pub source: Range<usize>,
}
#[derive(Clone)]
pub enum Inline {
    Text { text: String, map: Vec<SourceChar>, marks: Marks },
    Image { path: String, label: String, source: Range<usize> },
}
#[derive(Clone)]
pub struct Cell {
    pub source: Range<usize>,
    pub inline: Vec<Inline>,
    pub align: AlignKind,
}
impl Default for Cell {
    fn default() -> Self { Self { source: 0..0, inline: Vec::new(), align: AlignKind::None } }
}
#[derive(Clone)]
pub struct Row {
    pub source: Range<usize>,
    pub cells: Vec<Cell>,
    pub heading: u8,
    pub indent: usize,
    pub quote: usize,
    pub marker: Option<String>,
    pub task: Option<(bool, Range<usize>)>,
    pub code: bool,
    pub rule: bool,
    pub table: bool,
    pub table_head: bool,
    pub gap: bool,
}
#[derive(Clone, Default)]
pub struct Document {
    pub rows: Vec<Row>,
}
impl Document {
    pub fn new(source: &str, cursor: usize, composing: bool, attachments: &Attachments) -> Self {
        let mut builder = Builder { source, cursor, composing, attachments, rows: Vec::new(), definitions: BTreeMap::new() };
        let mut parse_source = source.as_bytes().to_vec();
        let mut protected = Vec::new();
        if let Ok(root) = markdown::to_mdast(source, &ParseOptions::gfm()) { protected_ranges(&root, &mut protected); }
        for (range, _) in attachments.references(source) {
            if protected.iter().any(|protected: &Range<usize>| range.start >= protected.start && range.end <= protected.end) { continue; }
            parse_source[range].fill(b'x');
        }
        let parse_source = String::from_utf8(parse_source).unwrap_or_else(|_| source.to_string());
        match markdown::to_mdast(&parse_source, &ParseOptions::gfm()) {
            Ok(Node::Root(root)) => {
                collect_definitions(&Node::Root(root.clone()), &mut builder.definitions);
                let mut end = 0;
                for node in &root.children {
                    let range = node_range(node);
                    builder.blank_between(end, range.start);
                    builder.block(node, 0, 0, None, None);
                    end = range.end;
                }
                builder.blank_between(end, source.len());
            }
            _ => builder.plain(0..source.len(), 0, 0, false),
        }
        if builder.rows.is_empty() {
            builder.plain(0..0, 0, 0, false);
        }
        // A terminal newline always has an editable following line.
        if source.ends_with('\n') && builder.rows.last().is_none_or(|r| r.source.start != source.len()) {
            builder.plain(source.len()..source.len(), 0, 0, false);
        }
        Self { rows: builder.rows }
    }
    pub fn images(&self) -> impl Iterator<Item = &Range<usize>> {
        self.rows.iter().flat_map(|r| &r.cells).flat_map(|c| &c.inline).filter_map(|i| {
            if let Inline::Image { source, .. } = i { Some(source) } else { None }
        })
    }
    pub fn cell_at(&self, offset: usize) -> Option<(usize, usize)> {
        self.rows.iter().enumerate().filter(|(_, r)| r.table).find_map(|(row, r)| {
            r.cells.iter().enumerate().find(|(_, c)| offset >= c.source.start && offset <= c.source.end).map(|(cell, _)| (row, cell))
        })
    }
    pub fn code_at(&self, offset: usize) -> bool {
        self.rows.iter().any(|row| row.code && offset >= row.source.start && offset <= row.source.end)
    }
}
struct Builder<'a> {
    source: &'a str,
    cursor: usize,
    composing: bool,
    attachments: &'a Attachments,
    rows: Vec<Row>,
    definitions: BTreeMap<String, String>,
}
impl Builder<'_> {
    fn row(&self, source: Range<usize>, cells: Vec<Cell>, indent: usize, quote: usize) -> Row {
        Row { source, cells, heading: 0, indent, quote, marker: None, task: None,
            code: false, rule: false, table: false, table_head: false, gap: false }
    }
    fn blank_between(&mut self, start: usize, end: usize) {
        let Some(text) = self.source.get(start..end) else { return; };
        let mut offset = start;
        // One line break is a block boundary, further line breaks are editable blank lines.
        let mut first_break = true;
        for line in text.split_inclusive('\n') {
            if line.ends_with('\n') {
                if !first_break {
                    let mut row = self.row(offset..offset + line.len() - 1,
                        vec![Cell { source: offset..offset, ..Default::default() }], 0, 0);
                    row.gap = true;
                    self.rows.push(row);
                }
                first_break = false;
            }
            offset += line.len();
        }
    }
    fn plain(&mut self, range: Range<usize>, indent: usize, quote: usize, code: bool) {
        let text = self.source.get(range.clone()).unwrap_or("");
        let mut start = range.start;
        if text.is_empty() {
            let mut row = self.row(range.clone(), vec![Cell { source: range, ..Default::default() }], indent, quote);
            row.code = code;
            self.rows.push(row);
            return;
        }
        for line in text.split_inclusive('\n') {
            let len = line.trim_end_matches(['\r', '\n']).len();
            let source = start..start + len;
            let mut inline = Vec::new();
            self.raw(source.clone(), Marks { code, ..Default::default() }, &mut inline);
            let mut row = self.row(source.clone(), vec![Cell { source, inline, ..Default::default() }], indent, quote);
            row.code = code;
            self.rows.push(row);
            start += line.len();
        }
    }
    fn block(&mut self, node: &Node, indent: usize, quote: usize, marker: Option<String>, task: Option<(bool, Range<usize>)>) {
        let range = node_range(node);
        match node {
            Node::List(list) => {
                for (index, item) in list.children.iter().enumerate() {
                    let Node::ListItem(item) = item else { continue; };
                    let marker = if list.ordered { format!("{}.", list.start.unwrap_or(1) as usize + index) } else { "•".into() };
                    let item_range = item.position.as_ref().map(|p| p.start.offset..p.end.offset).unwrap_or(range.clone());
                    let task = item.checked.and_then(|checked| {
                        let prefix_end = item.children.first().map(node_range).map_or(item_range.end, |r| r.start);
                        let prefix = self.source.get(item_range.start..prefix_end)?;
                        let bracket = prefix.find('[')?;
                        Some((checked, item_range.start + bracket + 1..item_range.start + bracket + 2))
                    });
                    if item.children.is_empty() {
                        self.plain(item_range.end..item_range.end, indent + 1, quote, false);
                        if let Some(row) = self.rows.last_mut() { row.marker = Some(marker.clone()); row.task = task.clone(); }
                    }
                    for (child_index, child) in item.children.iter().enumerate() {
                        self.block(child, indent + 1, quote,
                            (child_index == 0).then(|| marker.clone()),
                            if child_index == 0 { task.clone() } else { None });
                    }
                }
            }
            Node::Blockquote(block) => {
                for child in &block.children { self.block(child, indent, quote + 1, None, None); }
            }
            Node::Table(table) => {
                for (index, row_node) in table.children.iter().enumerate() {
                    let Node::TableRow(table_row) = row_node else { continue; };
                    let mut cells = Vec::new();
                    for (column, cell_node) in table_row.children.iter().enumerate() {
                        let source = node_range(cell_node);
                        let mut inline = Vec::new();
                        if let Some(children) = cell_node.children() {
                            for child in children { self.inline(child, Marks { bold: index == 0, ..Default::default() }, &mut inline); }
                        }
                        let visible_start = inline.first().map(inline_start).unwrap_or_else(|| {
                            let raw = self.source.get(source.clone()).unwrap_or("");
                            source.start + raw.len() - raw.trim_start().len()
                        });
                        let visible_end = inline.last().map(inline_end).unwrap_or(visible_start);
                        cells.push(Cell { source: visible_start..visible_end, inline, align: table.align.get(column).copied().unwrap_or(AlignKind::None) });
                    }
                    while cells.len() < table.align.len() {
                        let source = node_range(row_node);
                        let raw = self.source.get(source.clone()).unwrap_or("");
                        let end = if raw.ends_with('|') { source.end - 1 } else { source.end };
                        cells.push(Cell { source: end..end, inline: Vec::new(), align: table.align[cells.len()] });
                    }
                    let mut row = self.row(node_range(row_node), cells, indent, quote);
                    row.table = true; row.table_head = index == 0;
                    self.rows.push(row);
                }
            }
            Node::Code(_) => {
                let source = &self.source[range.clone()];
                let lines: Vec<&str> = source.split_inclusive('\n').collect();
                let fenced = source.trim_start().starts_with("```") || source.trim_start().starts_with("~~~");
                let mut offset = range.start;
                let fence = source.trim_start().chars().next().unwrap_or('`');
                let initial_rows = self.rows.len();
                for (index, line) in lines.iter().enumerate() {
                    let closing = fenced && index > 0 && index + 1 == lines.len() && line.trim_start().starts_with(&fence.to_string().repeat(3));
                    if !(fenced && index == 0 || closing) {
                        let stripped = if fenced { 0 } else { line.bytes().take_while(|b| *b == b' ').count().min(4) };
                        let end = offset + line.trim_end_matches(['\r', '\n']).len();
                        self.plain(offset + stripped..end, indent, quote, true);
                    }
                    offset += line.len();
                }
                if self.rows.len() == initial_rows {
                    let end = range.start + lines.first().map_or(0, |line| line.len());
                    self.plain(end..end, indent, quote, true);
                }
            }
            Node::ThematicBreak(_) => {
                let mut row = self.row(range.clone(), vec![Cell { source: range, ..Default::default() }], indent, quote);
                row.rule = true; self.rows.push(row);
            }
            Node::Definition(_) => {} // Reference definitions are source metadata, not a separate reading block.
            _ => {
                let mut inline = Vec::new();
                if let Some(children) = node.children() {
                    for child in children { self.inline(child, Marks::default(), &mut inline); }
                } else { self.raw(range.clone(), Marks::default(), &mut inline); }
                let heading = if let Node::Heading(heading) = node { heading.depth } else { 0 };
                let visible_start = inline.first().map(inline_start).unwrap_or(range.start);
                if heading > 0 && self.cursor >= range.start && self.cursor < visible_start && !self.composing {
                    let mut prefix = Vec::new();
                    self.raw(range.start..visible_start, Marks { syntax: true, ..Default::default() }, &mut prefix);
                    prefix.extend(inline); inline = prefix;
                }
                let cell_source = if heading > 0 { visible_start..range.end } else { range.clone() };
                let mut row = self.row(range.clone(), vec![Cell { source: cell_source, inline, ..Default::default() }], indent, quote);
                row.heading = heading; row.marker = marker; row.task = task;
                self.rows.push(row);
            }
        }
    }
    fn inline(&self, node: &Node, mut marks: Marks, output: &mut Vec<Inline>) {
        let range = node_range(node);
        if matches!(node, Node::Text(_)) && !self.attachments.references(self.source.get(range.clone()).unwrap_or("")).is_empty() {
            self.raw(range, marks, output); return;
        }
        match node {
            Node::Text(text) => self.decoded(&text.value, range, marks, output),
            Node::Image(image) => {
                let label = self.attachments.token(&image.url).label().to_string();
                output.push(Inline::Image { path: image.url.clone(), label, source: range });
            }
            Node::ImageReference(image) => {
                if let Some(path) = self.definitions.get(&image.identifier.to_lowercase()) {
                    let label = self.attachments.token(path).label().to_string();
                    output.push(Inline::Image { path: path.clone(), label, source: range });
                } else { self.raw(range, marks, output); }
            }
            Node::Break(_) => self.decoded("\n", range, marks, output),
            Node::Html(html) if matches!(html.value.to_ascii_lowercase().as_str(), "<br>" | "<br/>" | "<br />") => {
                let map = vec![SourceChar { display: 0..1, source: range }];
                output.push(Inline::Text { text: "\n".into(), map, marks });
            }
            Node::InlineCode(code) => {
                marks.code = true;
                let raw = &self.source[range.clone()];
                let ticks = raw.bytes().take_while(|b| *b == b'`').count();
                let inner = range.start + ticks..range.end.saturating_sub(ticks);
                if (self.cursor > range.start && self.cursor < inner.start)
                    || (self.cursor > inner.end && self.cursor < range.end) {
                    self.raw(range, marks, output);
                } else { self.decoded(&code.value, inner, marks, output); }
            }
            Node::Strong(_) | Node::Emphasis(_) | Node::Delete(_) | Node::Link(_) | Node::LinkReference(_) => {
                match node {
                    Node::Strong(_) => marks.bold = true,
                    Node::Emphasis(_) => marks.italic = true,
                    Node::Delete(_) => marks.strike = true,
                    _ => marks.link = true,
                }
                let Some(children) = node.children() else { return; };
                let open = children.first().map(node_range).map_or(range.end, |r| r.start);
                let close = children.last().map(node_range).map_or(range.start, |r| r.end);
                let reveal = self.cursor > range.start && self.cursor < range.end && !self.composing;
                if reveal { self.raw(range.start..open, Marks { syntax: true, ..marks }, output); }
                for child in children { self.inline(child, marks, output); }
                if reveal { self.raw(close..range.end, Marks { syntax: true, ..marks }, output); }
            }
            _ => self.raw(range, marks, output),
        }
    }
    fn raw(&self, range: Range<usize>, marks: Marks, output: &mut Vec<Inline>) {
        let Some(text) = self.source.get(range.clone()) else { return; };
        if !marks.code {
            let references = self.attachments.references(text);
            if !references.is_empty() {
                let mut end = 0;
                for (reference, path) in references {
                    self.raw_text(range.start + end..range.start + reference.start, marks, output);
                    let label = self.attachments.token(&path).label().to_string();
                    output.push(Inline::Image { path, label, source: range.start + reference.start..range.start + reference.end });
                    end = reference.end;
                }
                self.raw_text(range.start + end..range.end, marks, output);
                return;
            }
        }
        self.raw_text(range, marks, output);
    }
    fn raw_text(&self, range: Range<usize>, marks: Marks, output: &mut Vec<Inline>) {
        let Some(text) = self.source.get(range.clone()).filter(|s| !s.is_empty()) else { return; };
        let map = text.char_indices().map(|(start, c)| SourceChar {
            display: start..start + c.len_utf8(), source: range.start + start..range.start + start + c.len_utf8(),
        }).collect();
        output.push(Inline::Text { text: text.to_string(), map, marks });
    }
    fn decoded(&self, value: &str, range: Range<usize>, marks: Marks, output: &mut Vec<Inline>) {
        if !marks.code && !self.attachments.references(self.source.get(range.clone()).unwrap_or("")).is_empty() {
            self.raw(range, marks, output); return;
        }
        if self.source.get(range.clone()) == Some(value) { self.raw(range, marks, output); return; }
        // CommonMark escapes and entities are decoded, while preserving their entire source spans.
        let source = self.source.get(range.clone()).unwrap_or("");
        let mut chars = Vec::new();
        let mut index = 0;
        while index < source.len() {
            let start = index;
            let ch = source[index..].chars().next().unwrap();
            index += ch.len_utf8();
            if ch == '\n' && source != value {
                let newline_end = index;
                // Markdown container indentation and quote markers are outside
                // the visible text, even when mdast includes them in this span.
                let mut prefix = index;
                while source[prefix..].starts_with(' ') || source[prefix..].starts_with('\t') { prefix += 1; }
                while source[prefix..].starts_with('>') {
                    prefix += 1;
                    if source[prefix..].starts_with(' ') { prefix += 1; }
                }
                index = prefix;
                chars.push(("\n".into(), range.start + start..range.start + newline_end));
                continue;
            }
            if ch == '\\' {
                if let Some(next) = source[index..].chars().next().filter(|c| c.is_ascii_punctuation()) {
                    index += next.len_utf8(); chars.push((next.to_string(), range.start + start..range.start + index)); continue;
                }
            }
            if ch == '&' {
                if let Some(end) = source[index..].find(';').filter(|end| *end < 32) {
                    let entity_end = index + end + 1;
                    let entity = &source[start..entity_end];
                    let decoded = markdown::to_html(entity);
                    let decoded = decoded.trim_end_matches('\n');
                    let decoded = decoded.strip_prefix("<p>").and_then(|s| s.strip_suffix("</p>")).unwrap_or(entity);
                    let decoded = decoded.replace("&amp;", "&").replace("&lt;", "<").replace("&gt;", ">");
                    if decoded != entity {
                        chars.push((decoded, range.start + start..range.start + entity_end));
                        index = entity_end; continue;
                    }
                }
            }
            chars.push((ch.to_string(), range.start + start..range.start + index));
        }
        let reconstructed: String = chars.iter().map(|(s, _)| s.as_str()).collect();
        if reconstructed == value {
            let mut map = Vec::new(); let mut display = 0;
            for (text, source) in chars {
                for ch in text.chars() { map.push(SourceChar { display: display..display + ch.len_utf8(), source: source.clone() }); display += ch.len_utf8(); }
            }
            output.push(Inline::Text { text: value.to_string(), map, marks });
        } else if !value.is_empty() {
            // Container prefixes and code whitespace normalization can remove bytes
            // from an AST Text span. Match decoded characters in order, retaining
            // the actual source occurrence rather than mapping the whole fragment.
            let mut map = Vec::new();
            let mut source_index = 0;
            let flattened: Vec<_> = chars.into_iter().flat_map(|(text, source)| {
                text.chars().map(|ch| (ch, source.clone())).collect::<Vec<_>>()
            }).collect();
            for (display, ch) in value.char_indices() {
                let mut found = None;
                while let Some((decoded, source)) = flattened.get(source_index) {
                    source_index += 1;
                    if *decoded == ch || ch == ' ' && (*decoded == '\n' || *decoded == '\r') {
                        found = Some(source.clone()); break;
                    }
                }
                let source = found.unwrap_or_else(|| range.end..range.end);
                map.push(SourceChar { display: display..display + ch.len_utf8(), source });
            }
            output.push(Inline::Text { text: value.to_string(), map, marks });
        }
    }
}
pub fn node_range(node: &Node) -> Range<usize> {
    node.position().map(|p| p.start.offset..p.end.offset).unwrap_or(0..0)
}
fn inline_start(inline: &Inline) -> usize {
    match inline {
        Inline::Text { map, .. } => map.first().map_or(0, |m| m.source.start),
        Inline::Image { source, .. } => source.start,
    }
}
fn inline_end(inline: &Inline) -> usize {
    match inline {
        Inline::Text { map, .. } => map.last().map_or(0, |m| m.source.end),
        Inline::Image { source, .. } => source.end,
    }
}
fn collect_definitions(node: &Node, definitions: &mut BTreeMap<String, String>) {
    if let Node::Definition(definition) = node { definitions.insert(definition.identifier.to_lowercase(), definition.url.clone()); }
    if let Some(children) = node.children() { for child in children { collect_definitions(child, definitions); } }
}
fn protected_ranges(node: &Node, ranges: &mut Vec<Range<usize>>) {
    if matches!(node, Node::Image(_) | Node::Link(_) | Node::Definition(_) | Node::Code(_) | Node::InlineCode(_)) {
        ranges.push(node_range(node));
    } else if let Some(children) = node.children() {
        for child in children { protected_ranges(child, ranges); }
    }
}
