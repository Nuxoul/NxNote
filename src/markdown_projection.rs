//! Editable Markdown projection. Every displayed character retains its source range.
//! This module contains no input state: the entire document has one selection and IME handler.
use crate::attachments::Attachments;
use markdown::{mdast::{AlignKind, Node}, ParseOptions};
use std::{collections::BTreeMap, ops::Range};
#[path = "markdown_extension_projection.rs"]
mod extension_projection;

#[derive(Clone, Copy, Default, PartialEq, Eq)]
pub struct Marks {
    pub bold: bool,
    pub italic: bool,
    pub strike: bool,
    pub code: bool,
    pub link: bool,
    pub syntax: bool,
    pub highlight: bool,
    /// -1 subscript, 1 superscript, 0 normal baseline.
    pub script: i8,
    pub tone: u8,
}
#[derive(Clone)]
pub struct SourceChar {
    pub display: Range<usize>,
    pub source: Range<usize>,
}
#[derive(Clone, Debug)]
pub enum WidgetKind {
    Math { formula: String, display: bool },
    Mermaid { diagram: String },
    Metadata { text: String },
    NoteEmbed { target: String },
}
impl WidgetKind {
    pub fn key(&self) -> String { format!("{self:?}") }
}
#[derive(Clone)]
pub enum LinkTarget { External(String), Note(String), Footnote(String) }
#[derive(Clone)]
pub struct DocumentLink { pub source: Range<usize>, pub target: LinkTarget }
#[derive(Clone)]
pub enum Inline {
    Text { text: String, map: Vec<SourceChar>, marks: Marks },
    Image { path: String, label: String, source: Range<usize> },
    Widget { kind: WidgetKind, source: Range<usize> },
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
    /// Reserve the marker column for actual list content, including explicit continuations.
    pub list_content: bool,
    pub code: bool,
    pub rule: bool,
    pub table: bool,
    pub table_head: bool,
    pub gap: bool,
    pub callout: Option<String>,
    pub annotation: Option<String>,
}
impl Row {
    /// The first list level uses the marker column, not an extra nesting level.
    pub fn content_inset(&self) -> f32 {
        let levels = if self.list_content { self.indent.saturating_sub(1) } else { self.indent };
        levels as f32 * 22. + self.quote as f32 * 14. + if self.list_content { 26. } else { 0. }
    }
}
#[derive(Clone, Default)]
pub struct Document {
    pub rows: Vec<Row>,
    pub links: Vec<DocumentLink>,
    pub footnotes: BTreeMap<String, Range<usize>>,
}
impl Document {
    pub fn new(source: &str, cursor: usize, marked: Option<Range<usize>>, attachments: &Attachments) -> Self {
        Self::with_options(source, cursor, marked, attachments, false)
    }
    pub fn with_options(source: &str, cursor: usize, marked: Option<Range<usize>>, attachments: &Attachments, metadata_expanded: bool) -> Self {
        let extensions = crate::markdown_extensions::Extensions::parse(source);
        let composing = marked.is_some();
        let mut builder = Builder { source, cursor, composing, marked: marked.clone(), attachments, rows: Vec::new(), definitions: BTreeMap::new(), masked_ranges: Vec::new() };
        let mut masked_ranges = extensions.inline.iter().map(|extension| extension.source.clone()).collect::<Vec<_>>();
        let mut parse_source = extensions.mask_inline(source).into_bytes();
        for block in &extensions.blocks {
            if matches!(&block.kind, crate::markdown_extensions::BlockKind::FrontMatter | crate::markdown_extensions::BlockKind::Math | crate::markdown_extensions::BlockKind::Mermaid) {
                masked_ranges.push(block.source.clone());
                for byte in &mut parse_source[block.source.clone()] { if *byte != b'\n' && *byte != b'\r' { *byte = b'x'; } }
            }
            if matches!(&block.kind, crate::markdown_extensions::BlockKind::FootnoteDefinition { .. }) {
                parse_source[block.source.start..block.content.start].fill(b'x');
                masked_ranges.push(block.source.start..block.content.start);
            }
        }
        if let Some(marked) = marked.filter(|range| range.start <= range.end && range.end <= source.len()) {
            let mut offset = marked.start;
            while offset < marked.end {
                if source[offset..].starts_with("\\|") { offset += 2; continue; }
                if source[offset..].starts_with("<br>") { offset += 4; continue; }
                let ch = source[offset..].chars().next().unwrap();
                if ch.is_ascii_punctuation() && ch != '|' { parse_source[offset] = b'x'; }
                masked_ranges.push(offset..offset + ch.len_utf8());
                offset += ch.len_utf8();
            }
        }
        let mut protected = Vec::new();
        if let Ok(root) = markdown::to_mdast(source, &ParseOptions::gfm()) { protected_ranges(&root, &mut protected); }
        for (range, _) in attachments.references(source) {
            if protected.iter().any(|protected: &Range<usize>| range.start >= protected.start && range.end <= protected.end) { continue; }
            parse_source[range.clone()].fill(b'x');
            masked_ranges.push(range);
        }
        let parse_source = String::from_utf8(parse_source).unwrap_or_else(|_| source.to_string());
        let mut links = Vec::new();
        builder.masked_ranges = masked_ranges;
        if let Ok(root) = markdown::to_mdast(source, &ParseOptions::gfm()) {
            collect_definitions(&root, &mut builder.definitions);
            collect_links(&root, &builder.definitions, &mut links);
        }
        match markdown::to_mdast(&parse_source, &ParseOptions::gfm()) {
            Ok(Node::Root(root)) => {
                for node in &root.children {
                    builder.block(node, 0, 0, None, None);
                }
            }
            _ => builder.plain(0..source.len(), 0, 0, false),
        }
        builder.apply_extensions(&extensions, metadata_expanded, &mut links);
        builder.fill_blank_lines();
        if builder.rows.is_empty() {
            builder.plain(0..0, 0, 0, false);
        }
        Self { rows: builder.rows, links, footnotes: extensions.footnotes }
    }
    pub fn widgets(&self) -> impl Iterator<Item = (&WidgetKind, &Range<usize>)> {
        self.rows.iter().flat_map(|row| &row.cells).flat_map(|cell| &cell.inline).filter_map(|inline| {
            if let Inline::Widget { kind, source } = inline { Some((kind, source)) } else { None }
        })
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
    marked: Option<Range<usize>>,
    attachments: &'a Attachments,
    rows: Vec<Row>,
    definitions: BTreeMap<String, String>,
    masked_ranges: Vec<Range<usize>>,
}
impl Builder<'_> {
    fn row(&self, source: Range<usize>, cells: Vec<Cell>, indent: usize, quote: usize) -> Row {
        Row { source, cells, heading: 0, indent, quote, marker: None, task: None, list_content: false,
            code: false, rule: false, table: false, table_head: false, gap: false, callout: None, annotation: None }
    }
    fn push_text_lines(&mut self, mut row: Row) {
        // Reading parsers can omit final whitespace from block positions. The
        // editor owns the physical line suffix, not merely the parsed content.
        let physical_end = self.source[row.source.end..].find('\n').map_or(self.source.len(), |offset| row.source.end + offset);
        let suffix = &self.source[row.source.end..physical_end];
        if suffix.chars().all(|ch| matches!(ch, ' ' | '\t' | '\r')) {
            row.source.end += suffix.trim_end_matches('\r').len();
        }
        // Physical source lines, not parser Text nodes, define editable rows.
        // Blank-line reconciliation is independent of AST container spans.
        let multiline_atom = row.cells.iter().flat_map(|cell| &cell.inline).any(|inline| match inline {
            Inline::Image { source, .. } | Inline::Widget { source, .. } => self.source.get(source.clone()).is_some_and(|text| text.contains('\n')),
            Inline::Text { text, map, .. } => map.iter().any(|character| {
                self.source.get(character.source.clone()).is_some_and(|source| source.contains('\n'))
                    && text.get(character.display.clone()).is_some_and(|display| display != "\n")
            }),
        });
        if multiline_atom { self.rows.push(row); return; }
        let mut start = row.source.start;
        for line in self.source[row.source.clone()].split_inclusive('\n') {
            let end = start + line.trim_end_matches(['\r', '\n']).len();
            let mut current = row.clone();
            current.source = start..end;
            current.gap = end < row.source.end;
            let mut inline = Vec::new();
            for item in &row.cells[0].inline {
                match item {
                    Inline::Image { source, .. } | Inline::Widget { source, .. } if source.start >= start && source.end <= end => inline.push(item.clone()),
                    Inline::Text { text, map, marks } => {
                        let mut value = String::new();
                        let mut selected = Vec::new();
                        for character in map.iter().filter(|character| character.source.start >= start && character.source.end <= end) {
                            let Some(display) = text.get(character.display.clone()) else { continue; };
                            let offset = value.len(); value.push_str(display);
                            selected.push(SourceChar { display: offset..value.len(), source: character.source.clone() });
                        }
                        if !value.is_empty() { inline.push(Inline::Text { text: value, map: selected, marks: *marks }); }
                    }
                    _ => {}
                }
            }
            if row.indent == 0 && row.quote == 0 && row.heading == 0 {
                let first = inline.first().map(inline_start).unwrap_or(end);
                let leading_end = start + self.source[start..end].len() - self.source[start..end].trim_start_matches([' ', '\t']).len();
                if start < first.min(leading_end) {
                    let mut prefix = Vec::new(); self.raw_text(start..first.min(leading_end), Marks::default(), &mut prefix);
                    prefix.extend(inline); inline = prefix;
                }
            }
            // Preserve whitespace excluded from Text values (including hard-
            // break spaces), without revealing concealed Markdown delimiters.
            let mapped_end = inline.last().map(inline_end).unwrap_or(start);
            let whitespace_start = start + self.source[start..end].trim_end_matches([' ', '\t']).len();
            let missing_start = mapped_end.max(whitespace_start);
            if missing_start < end && (!inline.is_empty() || whitespace_start == start) {
                self.raw_text(missing_start..end, Marks::default(), &mut inline);
            }
            let content_start = inline.first().map(inline_start).unwrap_or(start);
            let content_end = inline.last().map(inline_end).unwrap_or(content_start);
            current.cells = vec![Cell { source: content_start..content_end, inline, ..Default::default() }];
            if start != row.source.start { current.marker = None; current.task = None; }
            self.rows.push(current);
            start += line.len();
        }
        if row.source.is_empty() { self.rows.push(row); }
    }
    fn fill_blank_lines(&mut self) {
        // AST container ranges can include blank lines with no child node (lists
        // in particular). Reconcile physical blank lines against painted rows,
        // rather than assuming every omitted line is between top-level blocks.
        let line_start = |offset: usize| self.source[..offset].rfind('\n').map_or(0, |index| index + 1);
        let mut covered = std::collections::BTreeSet::new();
        for row in &self.rows {
            covered.insert(line_start(row.source.start));
            for inline in row.cells.iter().flat_map(|cell| &cell.inline) {
                if let Inline::Widget { source, .. } = inline {
                    let mut offset = source.start;
                    for line in self.source[source.clone()].split_inclusive('\n') {
                        covered.insert(line_start(offset)); offset += line.len();
                    }
                }
                if let Inline::Text { text, map, .. } = inline {
                    for character in map {
                        if text.get(character.display.clone()).is_some_and(|text| text.contains('\n')) {
                            covered.insert(line_start(character.source.end));
                        }
                    }
                }
            }
        }
        let mut offset = 0;
        for line in self.source.split('\n') {
            let end = offset + line.trim_end_matches('\r').len();
            if line.trim_matches([' ', '\t', '\r']).is_empty() && !covered.contains(&offset) {
                let source = offset..end;
                let mut inline = Vec::new();
                self.raw(source.clone(), Marks::default(), &mut inline);
                let mut row = self.row(source.clone(), vec![Cell { source, inline, ..Default::default() }], 0, 0);
                row.gap = true;
                self.rows.push(row);
            }
            offset += line.len() + 1;
        }
        self.rows.sort_by_key(|row| (row.source.start, row.source.end));
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
        let mut range = node_range(node);
        if matches!(node, Node::Heading(heading) if heading.children.is_empty()) {
            let line_end = self.source[range.start..].find('\n').map_or(self.source.len(), |end| range.start + end);
            range.end = range.start + self.source[range.start..line_end].trim_end_matches('\r').len();
        }
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
                    let first_row = self.rows.len();
                    if item.children.is_empty() {
                        let line_end = self.source[item_range.start..].find('\n').map_or(self.source.len(), |end| item_range.start + end);
                        let empty_range = item_range.start..line_end;
                        if !self.composing && self.cursor >= empty_range.start && self.cursor <= empty_range.end {
                            self.plain(empty_range.clone(), indent, quote, false);
                        } else {
                            self.plain(empty_range.end..empty_range.end, indent + 1, quote, false);
                            if let Some(row) = self.rows.last_mut() { row.marker = Some(marker.clone()); row.task = task.clone(); }
                        }
                    }
                    for (child_index, child) in item.children.iter().enumerate() {
                        self.block(child, indent + 1, quote,
                            (child_index == 0).then(|| marker.clone()),
                            if child_index == 0 { task.clone() } else { None });
                    }
                    let content_start = item.children.first().map(node_range).map_or(item_range.end, |range| range.start);
                    let physical_start = self.source[..item_range.start].rfind('\n').map_or(0, |offset| offset + 1);
                    let line_end = self.source[physical_start..].find('\n').map_or(self.source.len(), |offset| physical_start + offset);
                    let prefix = &self.source[physical_start..content_start.min(line_end)];
                    let prefix = prefix.rsplit('>').next().unwrap_or(prefix);
                    // Checkbox syntax is content of a task item, not continuation indentation.
                    let required_indent = prefix.find('[').unwrap_or(prefix.len());
                    for row in &mut self.rows[first_row..] {
                        // A nested list owns its own marker column and depth.
                        if row.indent != indent + 1 { continue; }
                        let line_start = self.source[..row.source.start].rfind('\n').map_or(0, |offset| offset + 1);
                        if line_start == physical_start {
                            row.list_content = true;
                        } else {
                            let end = self.source[line_start..].find('\n').map_or(self.source.len(), |offset| line_start + offset);
                            let line = &self.source[line_start..end];
                            let body = line.rsplit_once('>').filter(|(prefix, _)| prefix.chars().all(|ch| matches!(ch, '>' | ' ' | '\t')))
                                .map_or(line, |(_, body)| body);
                            let columns = body.chars().take_while(|ch| matches!(*ch, ' ' | '\t'))
                                .fold(0, |columns, ch| if ch == '\t' { columns + 4 - columns % 4 } else { columns + 1 });
                            row.list_content = columns >= required_indent;
                            // GFM permits lazy, unindented list continuations. In
                            // this editor they read as normal physical text lines.
                            if !row.list_content { row.indent = 0; }
                        }
                    }
                    if !item.children.is_empty() {
                        let content_start = node_range(&item.children[0]).start;
                        if let Some(row) = self.rows.get_mut(first_row) { row.source.start = item_range.start; }
                        if !self.composing && self.cursor >= item_range.start && self.cursor < content_start {
                            let mut prefix = Vec::new();
                            self.raw(item_range.start..content_start, Marks { syntax: true, ..Default::default() }, &mut prefix);
                            if let Some(row) = self.rows.get_mut(first_row) {
                                if let Some(cell) = row.cells.first_mut() {
                                    prefix.append(&mut cell.inline); cell.inline = prefix;
                                }
                                row.marker = None; row.task = None; row.list_content = false;
                                row.indent = indent;
                            }
                        }
                    }
                }
            }
            Node::Blockquote(block) => {
                let first_row = self.rows.len();
                for child in &block.children { self.block(child, indent, quote + 1, None, None); }
                if self.rows.len() == first_row { self.plain(range.clone(), indent, quote, false); }
                for index in first_row..self.rows.len() {
                    let row_start = self.rows[index].source.start;
                    let line_start = self.source[..row_start].rfind('\n').map_or(0, |offset| offset + 1);
                    let prefix = &self.source[line_start..row_start];
                    if !self.composing && self.cursor >= line_start && self.cursor <= row_start && prefix.contains('>') {
                        let mut inline = Vec::new();
                        self.raw(line_start..row_start, Marks { syntax: true, ..Default::default() }, &mut inline);
                        let row = &mut self.rows[index];
                        if let Some(cell) = row.cells.first_mut() {
                            inline.append(&mut cell.inline); cell.inline = inline;
                        }
                        row.source.start = line_start; row.quote = 0;
                    }
                }
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
            Node::Code(code) => {
                let source = &self.source[range.clone()];
                let lines: Vec<&str> = source.split_inclusive('\n').collect();
                let fenced = source.trim_start().starts_with("```") || source.trim_start().starts_with("~~~");
                let opening_end = range.start + lines.first().map_or(0, |line| line.trim_end_matches(['\r', '\n']).len());
                let last_start = range.start + lines.iter().take(lines.len().saturating_sub(1)).map(|line| line.len()).sum::<usize>();
                let closing = lines.len() > 1 && lines.last().is_some_and(|line| line.trim_start().starts_with(&source.trim_start().chars().next().unwrap_or('`').to_string().repeat(3)));
                if fenced && !self.composing && ((self.cursor >= range.start && self.cursor <= opening_end)
                    || closing && self.cursor >= last_start && self.cursor <= range.end) {
                    let first = self.rows.len();
                    self.plain(range.clone(), indent, quote, true);
                    self.color_code_rows(first, range.clone(), code.lang.as_deref().unwrap_or("")); return;
                }
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
                self.color_code_rows(initial_rows, range.clone(), code.lang.as_deref().unwrap_or(""));
            }
            Node::ThematicBreak(_) => {
                if !self.composing && self.cursor >= range.start && self.cursor <= range.end {
                    self.plain(range, indent, quote, false);
                } else {
                    let mut row = self.row(range.clone(), vec![Cell { source: range, ..Default::default() }], indent, quote);
                    row.rule = true; self.rows.push(row);
                }
            }
            Node::Definition(_) => self.plain(range, indent, quote, false),
            _ => {
                if matches!(node, Node::Paragraph(_)) && indent == 0 && quote == 0 && marker.is_none() {
                    let physical_start = self.source[..range.start].rfind('\n').map_or(0, |offset| offset + 1);
                    if self.source[physical_start..range.start].chars().all(|ch| matches!(ch, ' ' | '\t')) {
                        range.start = physical_start;
                    }
                }
                let mut inline = Vec::new();
                if let Some(children) = node.children() {
                    for child in children { self.inline(child, Marks::default(), &mut inline); }
                } else { self.raw(range.clone(), Marks::default(), &mut inline); }
                let heading = if let Node::Heading(heading) = node { heading.depth } else { 0 };
                if matches!(node, Node::Paragraph(_)) {
                    let first = inline.first().map(inline_start).unwrap_or(range.start);
                    if first > range.start && self.source[range.start..first].chars().all(|ch| matches!(ch, ' ' | '\t')) {
                        let mut prefix = Vec::new(); self.raw_text(range.start..first, Marks::default(), &mut prefix);
                        prefix.extend(inline); inline = prefix;
                    }
                }
                let visible_start = inline.first().map(inline_start).unwrap_or(if heading > 0 { range.end } else { range.start });
                if heading > 0 && self.cursor >= range.start && self.cursor <= visible_start && !self.composing {
                    let mut prefix = Vec::new();
                    self.raw(range.start..visible_start, Marks { syntax: true, ..Default::default() }, &mut prefix);
                    prefix.extend(inline); inline = prefix;
                }
                let visible_end = inline.last().map(inline_end).unwrap_or(visible_start);
                if heading > 0 && visible_end < range.end {
                    let suffix = &self.source[visible_end..range.end];
                    // Setext underline lines remain editable source rows. ATX
                    // closing hashes reveal only when the caret touches them.
                    if suffix.contains('\n') || !self.composing && self.cursor >= visible_end && self.cursor <= range.end {
                        self.raw(visible_end..range.end, Marks { syntax: true, ..Default::default() }, &mut inline);
                    }
                }
                let cell_source = if heading > 0 { visible_start..range.end } else { range.clone() };
                let mut row = self.row(range.clone(), vec![Cell { source: cell_source, inline, ..Default::default() }], indent, quote);
                row.heading = heading; row.marker = marker; row.task = task;
                self.push_text_lines(row);
            }
        }
    }
    fn restored_text(&self, range: Range<usize>, marks: Marks, output: &mut Vec<Inline>) {
        let mut spans = self.masked_ranges.iter().filter(|masked| masked.start < range.end && masked.end > range.start)
            .map(|masked| masked.start.max(range.start)..masked.end.min(range.end)).collect::<Vec<_>>();
        spans.sort_by_key(|span| (span.start, span.end));
        let mut offset = range.start;
        for span in spans {
            if offset < span.start { self.unmasked_text(offset..span.start, marks, output); }
            let start = offset.max(span.start);
            if start < span.end { self.raw(start..span.end, marks, output); }
            offset = offset.max(span.end);
        }
        if offset < range.end { self.unmasked_text(offset..range.end, marks, output); }
    }
    fn unmasked_text(&self, range: Range<usize>, marks: Marks, output: &mut Vec<Inline>) {
        let mut offset = range.start;
        while offset < range.end {
            let rest = &self.source[offset..range.end];
            if rest.starts_with('\\') && rest[1..].chars().next().is_some_and(|ch| ch.is_ascii_punctuation()) {
                let value = rest[1..2].to_string();
                output.push(Inline::Text { text: value, map: vec![SourceChar { display: 0..1, source: offset..offset + 2 }], marks });
                offset += 2; continue;
            }
            if rest.starts_with('&') {
                if let Some(end) = rest.find(';').filter(|end| *end <= 32) {
                    if let Some(value) = decode_entity(&rest[..end + 1]) {
                        let source = offset..offset + end + 1;
                        let map = value.char_indices().map(|(start, ch)| SourceChar { display: start..start + ch.len_utf8(), source: source.clone() }).collect();
                        output.push(Inline::Text { text: value, map, marks }); offset += end + 1; continue;
                    }
                }
            }
            let ch = rest.chars().next().unwrap();
            self.raw_text(offset..offset + ch.len_utf8(), marks, output); offset += ch.len_utf8();
        }
    }
    fn inline(&self, node: &Node, mut marks: Marks, output: &mut Vec<Inline>) {
        let range = node_range(node);
        if matches!(node, Node::Text(_)) && self.masked_ranges.iter().any(|masked| masked.start < range.end && masked.end > range.start) {
            self.restored_text(range, marks, output); return;
        }
        if matches!(node, Node::Text(_)) && self.marked.as_ref().is_some_and(|marked| marked.start < range.end && marked.end > range.start) {
            self.raw(range, marks, output); return;
        }
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
                if !self.composing && ((self.cursor >= range.start && self.cursor <= inner.start)
                    || (self.cursor >= inner.end && self.cursor <= range.end)) {
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
                let reveal = !self.composing && ((self.cursor >= range.start && self.cursor <= open)
                    || (self.cursor >= close && self.cursor <= range.end));
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
                    let decoded = decode_entity(entity).unwrap_or_else(|| entity.to_string());
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
                let Some(source) = found else { self.raw(range, marks, output); return; };
                map.push(SourceChar { display: display..display + ch.len_utf8(), source });
            }
            output.push(Inline::Text { text: value.to_string(), map, marks });
        }
    }
}
pub(crate) fn decode_entity(entity: &str) -> Option<String> {
    let root = markdown::to_mdast(entity, &ParseOptions::gfm()).ok()?;
    let paragraph = root.children()?.first()?;
    let children = paragraph.children()?;
    let [Node::Text(text)] = children.as_slice() else { return None; };
    (text.value != entity).then(|| text.value.clone())
}
pub fn node_range(node: &Node) -> Range<usize> {
    node.position().map(|p| p.start.offset..p.end.offset).unwrap_or(0..0)
}
fn inline_start(inline: &Inline) -> usize {
    match inline {
        Inline::Text { map, .. } => map.first().map_or(0, |m| m.source.start),
        Inline::Image { source, .. } | Inline::Widget { source, .. } => source.start,
    }
}
fn inline_end(inline: &Inline) -> usize {
    match inline {
        Inline::Text { map, .. } => map.last().map_or(0, |m| m.source.end),
        Inline::Image { source, .. } | Inline::Widget { source, .. } => source.end,
    }
}
fn collect_links(node: &Node, definitions: &BTreeMap<String, String>, links: &mut Vec<DocumentLink>) {
    let target = match node {
        Node::Link(link) => Some(link.url.clone()),
        Node::LinkReference(link) => definitions.get(&link.identifier.to_lowercase()).cloned(),
        _ => None,
    };
    if let Some(target) = target { links.push(DocumentLink { source: node_range(node), target: LinkTarget::External(target) }); }
    if let Some(children) = node.children() { for child in children { collect_links(child, definitions, links); } }
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keeps_every_blank_line_after_markdown() {
        for prefix in ["- 东蒙嵩", "# 标题", "-", "#", ""] {
            let source = format!("{prefix}{}", "\n".repeat(16));
            let document = Document::new(&source, source.len(), None, &Attachments::default());
            for (index, _) in source.match_indices('\n') {
                let start = index + 1;
                assert!(document.rows.iter().any(|row| row.source.start == start),
                    "missing editable blank line at {start} in {source:?}");
            }
            assert_eq!(document.rows.len(), 17, "{source:?}");
        }
    }

    #[test]
    fn keeps_leading_and_nested_list_blank_lines() {
        for source in ["\n\n# 标题", "- first\n\n  second\n\n- third", "- first\n\n\n- second", "- first\n  \n  second"] {
            let document = Document::new(source, source.len(), None, &Attachments::default());
            let mut start = 0;
            for line in source.split('\n') {
                if line.trim().is_empty() {
                    let visible = document.rows.iter().any(|row| row.source.start == start)
                        || document.rows.iter().flat_map(|row| &row.cells).flat_map(|cell| &cell.inline).any(|inline| {
                            if let Inline::Text { text, map, .. } = inline {
                                map.iter().any(|character| character.source.end == start
                                    && text.get(character.display.clone()).is_some_and(|text| text.contains('\n')))
                            } else { false }
                        });
                    assert!(visible, "{source:?} at {start}");
                }
                start += line.len() + 1;
            }
        }
    }

    #[test]
    fn reveals_empty_heading_and_list_markers_at_caret() {
        for source in ["#", "##", "# ", "##  ", "-", "- "] {
            let document = Document::new(source, source.len(), None, &Attachments::default());
            let visible: String = document.rows.iter().flat_map(|row| &row.cells).flat_map(|cell| &cell.inline)
                .filter_map(|inline| if let Inline::Text { text, .. } = inline { Some(text.as_str()) } else { None }).collect();
            assert_eq!(visible, source, "{source:?}");
        }
    }
}

#[cfg(test)]
mod structural_tests {
    use super::*;
    fn text(document: &Document) -> String {
        document.rows.iter().flat_map(|row| &row.cells).flat_map(|cell| &cell.inline)
            .filter_map(|inline| if let Inline::Text { text, .. } = inline { Some(text.as_str()) } else { None }).collect()
    }
    #[test]
    fn physical_paragraph_lines_have_distinct_rows() {
        let source = "first\nsecond\nthird";
        let document = Document::new(source, source.len(), None, &Attachments::default());
        assert_eq!(document.rows.iter().map(|row| row.source.start).collect::<Vec<_>>(), vec![0, 6, 13]);
        assert_eq!(text(&document), "firstsecondthird");
    }
    #[test]
    fn quote_markers_and_fences_are_editable_at_their_edges() {
        for source in ["> text", "> > text", "```\ncode\n```", "---"] {
            let document = Document::new(source, 0, None, &Attachments::default());
            assert!(text(&document).starts_with(source.split('\n').next().unwrap()), "{source:?}");
        }
    }
}

#[cfg(test)]
mod list_alignment_tests {
    use super::*;
    fn row_at<'a>(document: &'a Document, source: &str, value: &str) -> &'a Row {
        let start = source.find(value).unwrap();
        document.rows.iter().find(|row| row.source.start <= start && row.source.end >= start).unwrap()
    }
    #[test]
    fn top_level_marker_has_no_extra_nesting_indent_and_plain_line_resets() {
        let source = "- 23\n- 32\n- 32\n1. 32\n你好\n";
        let document = Document::new(source, source.len(), None, &Attachments::default());
        for row in document.rows.iter().filter(|row| row.marker.is_some()) {
            assert!(row.list_content);
            assert_eq!(row.content_inset(), 26.);
        }
        let plain = row_at(&document, source, "你好");
        assert!(!plain.list_content);
        assert_eq!(plain.content_inset(), 0.);
    }
    #[test]
    fn explicitly_indented_continuation_aligns_with_list_text() {
        for source in ["- item\n  continuation\nplain", "1. item\n   continuation\nplain", "- [x] item\n  continuation\nplain"] {
            let document = Document::new(source, source.len(), None, &Attachments::default());
            assert_eq!(row_at(&document, source, "item").content_inset(), 26.);
            assert_eq!(row_at(&document, source, "continuation").content_inset(), 26.);
            assert_eq!(row_at(&document, source, "plain").content_inset(), 0.);
        }
    }
    #[test]
    fn nested_lists_keep_only_the_real_nesting_offset() {
        let source = "- outer\n  - inner\n\nplain";
        let document = Document::new(source, source.len(), None, &Attachments::default());
        assert_eq!(row_at(&document, source, "outer").content_inset(), 26.);
        assert_eq!(row_at(&document, source, "inner").content_inset(), 48.);
        assert_eq!(row_at(&document, source, "plain").content_inset(), 0.);
    }
    #[test]
    fn revealed_list_source_starts_at_the_normal_text_edge() {
        let source = "- item";
        let document = Document::new(source, 0, None, &Attachments::default());
        assert_eq!(document.rows[0].content_inset(), 0.);
        assert!(document.rows[0].marker.is_none());
    }
}

#[cfg(test)]
mod whitespace_editing_tests {
    use super::*;
    #[test]
    fn ordinary_marker_like_text_keeps_each_inserted_trailing_space() {
        for prefix in ["-1", "1.32", "1234", "中文🙂", "- item", "1. item"] {
            for count in 1..=4 {
                let source = format!("{prefix}{}", " ".repeat(count));
                let document = Document::new(&source, source.len(), None, &Attachments::default());
                for offset in prefix.len()..source.len() {
                    assert!(document.rows.iter().flat_map(|row| &row.cells).flat_map(|cell| &cell.inline).any(|inline| {
                        if let Inline::Text { text, map, .. } = inline {
                            map.iter().any(|character| character.source == (offset..offset + 1)
                                && text.get(character.display.clone()) == Some(" "))
                        } else { false }
                    }), "missing space at {offset} in {source:?}");
                }
                assert_eq!(document.rows.last().unwrap().cells.last().unwrap().source.end, source.len());
            }
        }
    }
    #[test]
    fn paragraph_leading_whitespace_is_editable_source_not_hidden_syntax() {
        let source = "  -1 ";
        let document = Document::new(source, source.len(), None, &Attachments::default());
        let text: String = document.rows.iter().flat_map(|row| &row.cells).flat_map(|cell| &cell.inline)
            .filter_map(|inline| if let Inline::Text { text, .. } = inline { Some(text.as_str()) } else { None }).collect();
        assert_eq!(text, source);
    }
    #[test]
    fn non_list_prefixes_stay_plain_before_and_after_space_input() {
        for source in ["-1", "-1 ", "1.32", "1.32 "] {
            let document = Document::new(source, source.len(), None, &Attachments::default());
            assert!(document.rows.iter().all(|row| row.marker.is_none() && !row.list_content));
            let visible: String = document.rows.iter().flat_map(|row| &row.cells).flat_map(|cell| &cell.inline)
                .filter_map(|inline| if let Inline::Text { text, .. } = inline { Some(text.as_str()) } else { None }).collect();
            assert_eq!(visible, source);
        }
    }
}

#[cfg(test)]
mod source_mapping_invariants {
    use super::*;
    fn visible(document: &Document) -> String {
        document.rows.iter().flat_map(|row| &row.cells).flat_map(|cell| &cell.inline)
            .filter_map(|inline| if let Inline::Text { text, .. } = inline { Some(text.as_str()) } else { None }).collect()
    }
    #[test]
    fn extension_masking_never_becomes_display_text() {
        let source = "plain ==高亮== &amp; [[目标|别名]] tail ";
        let document = Document::new(source, source.len(), None, &Attachments::default());
        assert_eq!(visible(&document), "plain 高亮 & 别名 tail ");
        for cell in document.rows.iter().flat_map(|row| &row.cells) {
            for inline in &cell.inline {
                if let Inline::Text { text, map, .. } = inline {
                    for character in map {
                        assert!(text.get(character.display.clone()).is_some());
                        assert!(!character.source.is_empty());
                        assert!(source.get(character.source.clone()).is_some());
                    }
                }
            }
        }
    }
    #[test]
    fn reference_definition_keeps_original_destination_and_editable_metadata() {
        let source = "[go][target]\n\n[target]: https://example.com/#part";
        let document = Document::new(source, source.len(), None, &Attachments::default());
        assert!(visible(&document).contains("[target]: https://example.com/#part"));
        assert!(document.links.iter().any(|link| matches!(&link.target, LinkTarget::External(url) if url == "https://example.com/#part")));
    }
    #[test]
    fn composition_punctuation_is_exact_source_not_parser_placeholder() {
        let source = "word **中";
        let document = Document::new(source, source.len(), Some(5..source.len()), &Attachments::default());
        assert_eq!(visible(&document), source);
    }
}

#[cfg(test)]
mod whitespace_line_invariants {
    use super::*;
    #[test]
    fn whitespace_only_and_continuation_lines_keep_every_space() {
        for source in ["first\n   \nlast", "first\n  second ", "-1 \n1.32  "] {
            let document = Document::new(source, source.len(), None, &Attachments::default());
            for (offset, ch) in source.char_indices().filter(|(_, ch)| *ch == ' ') {
                assert!(document.rows.iter().flat_map(|row| &row.cells).flat_map(|cell| &cell.inline).any(|inline| {
                    if let Inline::Text { text, map, .. } = inline {
                        map.iter().any(|character| character.source == (offset..offset + ch.len_utf8())
                            && text.get(character.display.clone()) == Some(" "))
                    } else { false }
                }), "missing space at {offset} in {source:?}");
            }
        }
    }
}
