//! Extension overlays preserve source coordinates and the parent document state.
use super::*;
use crate::markdown_extensions::{BlockKind, Extensions, InlineKind};

impl Builder<'_> {
    pub(super) fn color_code_rows(&mut self, first: usize, range: Range<usize>, language: &str) {
        let spans = crate::markdown_renderers::highlight_code(&self.source[range.clone()], language);
        for row in &mut self.rows[first..] {
            for cell in &mut row.cells {
                let mut colored = Vec::new();
                for inline in std::mem::take(&mut cell.inline) {
                    if let Inline::Text { text, map, marks } = inline {
                        for character in map {
                            let Some(value) = text.get(character.display.clone()) else { continue; };
                            let tone = spans.iter().find(|span| range.start + span.source.start <= character.source.start
                                && range.start + span.source.end >= character.source.end).map_or(0, |span| span.tone as u8);
                            colored.push(Inline::Text { text: value.to_string(), map: vec![SourceChar { display: 0..value.len(), source: character.source }],
                                marks: Marks { tone, ..marks } });
                        }
                    } else { colored.push(inline); }
                }
                cell.inline = colored;
            }
        }
    }
    pub(super) fn apply_extensions(&mut self, extensions: &Extensions, metadata_expanded: bool, links: &mut Vec<DocumentLink>) {
        for block in &extensions.blocks {
            match &block.kind {
                BlockKind::Callout { kind, title } => {
                    let editing_header = self.cursor >= block.source.start && self.cursor <= block.content.start;
                    if !editing_header {
                        self.rows.retain(|row| row.source.start < block.source.start || row.source.start >= block.content.start);
                    }
                    let mut first = true;
                    for row in self.rows.iter_mut().filter(|row| row.source.start < block.source.end && row.source.end >= block.source.start) {
                        row.callout = Some(kind.clone());
                        if first { row.annotation = Some(if title.is_empty() { kind.clone() } else { format!("{kind} · {title}") }); first = false; }
                    }
                    if first {
                        let mut row = self.row(block.source.clone(), vec![Cell { source: block.content.clone(), ..Default::default() }], 0, 0);
                        row.callout = Some(kind.clone()); row.annotation = Some(if title.is_empty() { kind.clone() } else { title.clone() });
                        self.rows.push(row);
                    }
                }
                BlockKind::FootnoteDefinition { id } => {
                    self.rows.retain(|row| row.source.end < block.source.start || row.source.start >= block.source.end);
                    let first = self.rows.len(); self.plain(block.source.clone(), 0, 0, false);
                    if let Some(row) = self.rows.get_mut(first) { row.annotation = Some(format!("脚注 {id} · Ctrl+点击引用跳转")); }
                }
                _ => {
                    // Remove only rows within the overlay. Parent paragraph spans
                    // may touch this block's end; preserve their later lines.
                    self.rows.retain(|row| row.source.end < block.source.start || row.source.start >= block.source.end);
                    let editing = self.cursor >= block.source.start && self.cursor < block.source.end;
                    let widget = match &block.kind {
                        BlockKind::FrontMatter if !metadata_expanded => Some(WidgetKind::Metadata { text: self.source[block.content.clone()].to_string() }),
                        BlockKind::Math if !editing && !self.composing => Some(WidgetKind::Math { formula: self.source[block.content.clone()].trim().to_string(), display: true }),
                        BlockKind::Mermaid if !editing && !self.composing => Some(WidgetKind::Mermaid { diagram: self.source[block.content.clone()].to_string() }),
                        _ => None,
                    };
                    if let Some(kind) = widget {
                        let cell = Cell { source: block.source.clone(), inline: vec![Inline::Widget { kind, source: block.source.clone() }], ..Default::default() };
                        self.rows.push(self.row(block.source.clone(), vec![cell], 0, 0));
                    } else {
                        let first = self.rows.len(); self.plain(block.source.clone(), 0, 0, true);
                        if matches!(&block.kind, BlockKind::FrontMatter) {
                            self.color_code_rows(first, block.source.clone(), "yaml");
                            if let Some(row) = self.rows.get_mut(first) { row.annotation = Some("Front Matter · 点击折叠".into()); }
                        }
                    }
                }
            }
        }
        for extension in &extensions.inline {
            let editing = self.cursor >= extension.source.start && self.cursor <= extension.source.end || self.marked.as_ref()
                .is_some_and(|marked| marked.start < extension.source.end && marked.end > extension.source.start);
            let target = match &extension.kind {
                InlineKind::WikiLink { target, .. } | InlineKind::NoteEmbed { target } => Some(LinkTarget::Note(target.clone())),
                InlineKind::Footnote { id } => Some(LinkTarget::Footnote(id.clone())),
                _ => None,
            };
            if let Some(target) = target { links.push(DocumentLink { source: extension.source.clone(), target }); }
            for row in &mut self.rows {
                for cell in &mut row.cells {
                    let covered = cell.inline.iter().any(|inline| inline_start(inline) <= extension.source.start && inline_end(inline) > extension.source.start);
                    if !covered { continue; }
                    let mut output = Vec::new(); let mut inserted = false;
                    for inline in std::mem::take(&mut cell.inline) {
                        if inline_end(&inline) <= extension.source.start || inline_start(&inline) >= extension.source.end { output.push(inline); continue; }
                        if let Inline::Text { text, map, marks } = inline {
                            let before = map.iter().filter(|character| character.source.end <= extension.source.start).cloned().collect::<Vec<_>>();
                            append_mapped(&text, &before, marks, &mut output);
                            if !inserted {
                                let mut style = marks;
                                let value = self.source[extension.content.clone()].to_string();
                                let generated = match &extension.kind {
                                    InlineKind::Math if !editing && !self.composing => Some(WidgetKind::Math { formula: value.clone(), display: false }),
                                    InlineKind::NoteEmbed { target } if !editing && !self.composing => Some(WidgetKind::NoteEmbed { target: target.clone() }),
                                    _ => None,
                                };
                                if let Some(kind) = generated { output.push(Inline::Widget { kind, source: extension.source.clone() }); }
                                else {
                                    match &extension.kind {
                                        InlineKind::Highlight => style.highlight = true,
                                        InlineKind::Superscript => style.script = 1,
                                        InlineKind::Subscript => style.script = -1,
                                        InlineKind::WikiLink { .. } | InlineKind::Footnote { .. } | InlineKind::NoteEmbed { .. } => style.link = true,
                                        _ => {}
                                    }
                                    let (display, source) = if editing { (self.source[extension.source.clone()].to_string(), extension.source.clone()) }
                                        else { match &extension.kind {
                                            InlineKind::WikiLink { label, .. } => (label.clone(), extension.source.clone()),
                                            InlineKind::Footnote { id } => (format!("[{id}]"), extension.source.clone()),
                                            _ => (value, extension.content.clone()),
                                        } };
                                    let direct = self.source.get(source.clone()) == Some(display.as_str());
                                    let map = display.char_indices().map(|(offset, ch)| SourceChar { display: offset..offset + ch.len_utf8(),
                                        source: if direct { source.start + offset..source.start + offset + ch.len_utf8() } else { source.clone() } }).collect();
                                    output.push(Inline::Text { text: display, map, marks: style });
                                }
                                inserted = true;
                            }
                            let after = map.iter().filter(|character| character.source.start >= extension.source.end).cloned().collect::<Vec<_>>();
                            append_mapped(&text, &after, marks, &mut output);
                        } else { output.push(inline); }
                    }
                    cell.inline = output;
                }
            }
        }
        self.rows.sort_by_key(|row| (row.source.start, row.source.end));
    }
}
fn append_mapped(text: &str, characters: &[SourceChar], marks: Marks, output: &mut Vec<Inline>) {
    let mut value = String::new(); let mut map = Vec::new();
    for character in characters {
        if let Some(display) = text.get(character.display.clone()) {
            let start = value.len(); value.push_str(display);
            map.push(SourceChar { display: start..value.len(), source: character.source.clone() });
        }
    }
    if !value.is_empty() { output.push(Inline::Text { text: value, map, marks }); }
}
