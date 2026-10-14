//! Formatting edits operate on source delimiters, not painted glyphs.
use crate::attachments::Attachments;
use markdown::{mdast::Node, ParseOptions};
use std::ops::Range;
use unicode_segmentation::UnicodeSegmentation;

struct Format {
    full: Range<usize>,
    content: Range<usize>,
    markers: Vec<Range<usize>>,
    block: bool,
}
pub struct FormatEdit {
    pub text: String,
    pub selection: Range<usize>,
}

/// Backspace at a formatted run's start or Delete at its end unwraps the
/// smallest matching format. A delimiter itself can also be deleted explicitly.
#[cfg(test)]
pub fn at_boundary(source: &str, cursor: usize, backwards: bool, attachments: &Attachments) -> Option<FormatEdit> {
    boundary_edit(source, cursor, backwards, formats(source, attachments))
}

/// Only inline delimiters unwrap in both directions. Block ends are not format
/// deletion boundaries: Delete there must join lines, not clear the heading.
pub fn inline_boundary(source: &str, cursor: usize, backwards: bool, attachments: &Attachments) -> Option<FormatEdit> {
    let formats = formats(source, attachments).into_iter().filter(|format| !format.block).collect();
    boundary_edit(source, cursor, backwards, formats)
}
pub fn block_start(source: &str, cursor: usize, attachments: &Attachments) -> Option<FormatEdit> {
    let formats = formats(source, attachments);
    let format = formats.iter().filter(|format| format.block && (cursor == format.content.start || cursor == format.full.start))
        .min_by_key(|format| format.full.len())?;
    remove(source, cursor..cursor, format.markers.clone())
}
fn boundary_edit(source: &str, cursor: usize, backwards: bool, formats: Vec<Format>) -> Option<FormatEdit> {
    if let Some(format) = formats.iter().filter(|format| {
        (backwards && cursor == format.content.start) || (!backwards && cursor == format.content.end)
    }).min_by_key(|format| format.full.len()) {
        return remove(source, cursor..cursor, format.markers.clone());
    }
    let marker = formats.iter().flat_map(|format| &format.markers).filter(|marker| {
        if backwards { cursor > marker.start && cursor <= marker.end }
        else { cursor >= marker.start && cursor < marker.end }
    }).min_by_key(|marker| marker.len())?;
    let deletion = if backwards {
        let start = source[marker.start..cursor].grapheme_indices(true).next_back()?.0 + marker.start;
        start..cursor
    } else {
        let end = cursor + source[cursor..marker.end].graphemes(true).next()?.len();
        cursor..end
    };
    remove(source, cursor..cursor, vec![deletion])
}

/// Clear formats intersecting the selected text, or the innermost enclosing
/// format at the caret. No replacement of the content or attachment references.
pub fn clear(source: &str, selection: Range<usize>, attachments: &Attachments) -> Option<FormatEdit> {
    let formats = formats(source, attachments);
    let markers = if selection.is_empty() {
        formats.iter().filter(|format| selection.start >= format.content.start && selection.start <= format.content.end)
            .min_by_key(|format| format.full.len())?.markers.clone()
    } else {
        formats.iter().filter(|format| selection.start < format.content.end && selection.end > format.content.start)
            .flat_map(|format| format.markers.clone()).collect()
    };
    remove(source, selection, markers)
}
fn remove(source: &str, selection: Range<usize>, mut markers: Vec<Range<usize>>) -> Option<FormatEdit> {
    markers.retain(|range| range.start < range.end && range.end <= source.len()
        && source.is_char_boundary(range.start) && source.is_char_boundary(range.end));
    markers.sort_by_key(|range| (range.start, range.end));
    let mut merged: Vec<Range<usize>> = Vec::new();
    for range in markers {
        if let Some(previous) = merged.last_mut() {
            if range.start <= previous.end { previous.end = previous.end.max(range.end); continue; }
        }
        merged.push(range);
    }
    if merged.is_empty() { return None; }
    let map = |offset: usize| offset - merged.iter().map(|range| offset.saturating_sub(range.start).min(range.len())).sum::<usize>();
    let mapped = map(selection.start)..map(selection.end);
    let mut text = source.to_string();
    for range in merged.into_iter().rev() { text.replace_range(range, ""); }
    Some(FormatEdit { text, selection: mapped })
}
fn formats(source: &str, attachments: &Attachments) -> Vec<Format> {
    let mut output = Vec::new();
    if let Ok(root) = markdown::to_mdast(source, &ParseOptions::gfm()) {
        collect(&root, source, attachments, &mut output);
    }
    for extension in crate::markdown_extensions::Extensions::parse(source).inline {
        if matches!(&extension.kind, crate::markdown_extensions::InlineKind::Highlight
            | crate::markdown_extensions::InlineKind::Superscript | crate::markdown_extensions::InlineKind::Subscript) {
            let markers = vec![extension.source.start..extension.content.start, extension.content.end..extension.source.end];
            add(extension.source, extension.content, markers, &mut output);
        }
    }
    let images = attachments.references(source);
    output.retain(|format| !format.markers.iter().any(|marker| {
        images.iter().any(|(image, _)| marker.start < image.end && marker.end > image.start)
    }));
    output
}
fn range(node: &Node) -> Option<Range<usize>> { node.position().map(|p| p.start.offset..p.end.offset) }
fn add(full: Range<usize>, content: Range<usize>, markers: Vec<Range<usize>>, output: &mut Vec<Format>) {
    if markers.iter().any(|r| !r.is_empty()) { output.push(Format { full, content, markers, block: false }); }
}
fn collect(node: &Node, source: &str, attachments: &Attachments, output: &mut Vec<Format>) {
    let Some(full) = range(node) else { return; };
    let Some(raw) = source.get(full.clone()) else { return; };
    let first_format = output.len();
    match node {
        Node::Heading(heading) if heading.children.is_empty() => {
            let line_end = source[full.start..].find('\n').map_or(source.len(), |end| full.start + end);
            let end = full.start + source[full.start..line_end].trim_end_matches('\r').len();
            add(full.start..end, end..end, vec![full.start..end], output);
        }
        Node::Strong(_) | Node::Emphasis(_) | Node::Delete(_) | Node::Link(_) | Node::LinkReference(_) | Node::Heading(_) => {
            if let Some(children) = node.children() {
                if let (Some(first), Some(last)) = (children.first().and_then(range), children.last().and_then(range)) {
                    // A bare image path containing underscores must not be parsed as emphasis.
                    let attachment_syntax = !matches!(node, Node::Heading(_)) && attachments.references(raw).iter().any(|(path, _)| {
                        path.start < first.start - full.start || path.end > last.end - full.start
                    });
                    if !attachment_syntax {
                        add(full.clone(), first.start..last.end, vec![full.start..first.start, last.end..full.end], output);
                    }
                }
            }
        }
        Node::InlineCode(_) => {
            let ticks = raw.bytes().take_while(|b| *b == b'`').count();
            if ticks > 0 && raw.len() >= ticks * 2 {
                let content = full.start + ticks..full.end - ticks;
                add(full.clone(), content.clone(), vec![full.start..content.start, content.end..full.end], output);
            }
        }
        Node::Code(_) => {
            let trimmed = raw.trim_start();
            let fenced = trimmed.starts_with("```") || trimmed.starts_with("~~~");
            if fenced {
                let opening_end = raw.find('\n').map(|end| full.start + end + 1).unwrap_or(full.end);
                let last_start = raw.rfind('\n').map(|start| full.start + start + 1).unwrap_or(full.end);
                let fence = trimmed.chars().next().unwrap_or('`');
                let closing = source.get(last_start..full.end).is_some_and(|line| line.trim_start().starts_with(&fence.to_string().repeat(3)));
                let closing_start = if closing { last_start } else { full.end };
                let content_end = if closing {
                    opening_end + source[opening_end..closing_start].trim_end_matches(['\r', '\n']).len()
                } else { full.end };
                add(full.clone(), opening_end..content_end.max(opening_end),
                    vec![full.start..opening_end, closing_start..full.end], output);
            } else {
                let markers = line_prefixes(source, full.clone(), false);
                let start = markers.first().map_or(full.start, |marker| marker.end);
                add(full.clone(), start..full.end, markers, output);
            }
        }
        Node::ListItem(item) => {
            let line_end = source[full.start..].find('\n').map_or(source.len(), |end| full.start + end);
            let start = item.children.first().and_then(range).map_or(line_end, |r| r.start);
            let content_end = if item.children.is_empty() { line_end } else { full.end };
            add(full.start..full.end.max(content_end), start..content_end, vec![full.start..start], output);
        }
        Node::Blockquote(_) => {
            let markers = line_prefixes(source, full.clone(), true);
            let start = markers.first().map_or(full.start, |marker| marker.end);
            add(full.clone(), start..full.end, markers, output);
        }
        _ => {}
    }
    for format in &mut output[first_format..] {
        format.block = matches!(node, Node::Heading(_) | Node::ListItem(_) | Node::Blockquote(_) | Node::Code(_));
    }
    if let Some(children) = node.children() { for child in children { collect(child, source, attachments, output); } }
}
fn line_prefixes(source: &str, full: Range<usize>, quote: bool) -> Vec<Range<usize>> {
    let mut markers = Vec::new();
    let mut offset = full.start;
    for line in source[full.clone()].split_inclusive('\n') {
        let spaces = line.bytes().take_while(|b| *b == b' ').count();
        let length = if quote {
            if line[spaces..].starts_with('>') { spaces + 1 + usize::from(line[spaces + 1..].starts_with(' ')) } else { 0 }
        } else if line.starts_with('\t') { 1 } else { spaces.min(4) };
        if length > 0 { markers.push(offset..offset + length); }
        offset += line.len();
    }
    markers
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deletes_empty_markers_without_eating_following_blank_lines() {
        for marker in ["#", "##", "-", "- "] {
            let source = format!("{marker}\n\n\n");
            let edit = at_boundary(&source, marker.len(), true, &Attachments::default()).expect("empty format");
            assert_eq!(edit.text, "\n\n\n");
            assert_eq!(edit.selection, 0..0);
        }
    }

    #[test]
    fn unwraps_heading_and_list_without_deleting_text() {
        for (source, cursor, expected) in [("# 中文", 2, "中文"), ("- 中文", 2, "中文")] {
            let edit = at_boundary(source, cursor, true, &Attachments::default()).expect("format boundary");
            assert_eq!(edit.text, expected);
            assert_eq!(edit.selection, 0..0);
        }
    }

    #[test]
    fn deletes_only_one_character_inside_a_marker() {
        let edit = at_boundary("### text", 2, true, &Attachments::default()).expect("marker");
        assert_eq!(edit.text, "## text");
        assert_eq!(edit.selection, 1..1);
    }
}
