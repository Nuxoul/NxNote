//! Source-only Markdown editing transactions. Layout never chooses what to delete.
//! Rules informed by Velotype's structural events and Lockbook's own-prefix edits;
//! implementation is local and preserves untouched Markdown bytes.
use crate::{attachments::Attachments, markdown_format::FormatEdit};
use markdown::{mdast::Node, ParseOptions};
use std::ops::Range;
use unicode_segmentation::UnicodeSegmentation;

#[derive(Clone)]
enum PrefixKind {
    Quote,
    List { next: String },
    Heading,
}
#[derive(Clone)]
struct Prefix {
    range: Range<usize>,
    indent: Range<usize>,
    kind: PrefixKind,
}
struct Line {
    range: Range<usize>,
    content: usize,
    prefixes: Vec<Prefix>,
    code: bool,
    table: bool,
    inline_code: bool,
    rule: bool,
}
#[derive(Default)]
struct Syntax {
    quotes: usize,
    list_starts: Vec<usize>,
    heading: bool,
    code: bool,
    table: bool,
    inline_code: bool,
    rule: bool,
}
fn node_range(node: &Node) -> Range<usize> {
    node.position().map_or(0..0, |position| position.start.offset..position.end.offset)
}
fn inspect(node: &Node, line: &Range<usize>, cursor: usize, syntax: &mut Syntax) {
    let range = node_range(node);
    if range.end < line.start || range.start > line.end { return; }
    let contains = cursor >= range.start && cursor <= range.end;
    match node {
        Node::Code(_) => syntax.code = true,
        Node::Table(_) => syntax.table = true,
        Node::InlineCode(_) if contains => syntax.inline_code = true,
        Node::Blockquote(_) => syntax.quotes += 1,
        Node::ListItem(_) if range.start >= line.start && range.start <= line.end => syntax.list_starts.push(range.start),
        Node::Heading(_) => syntax.heading = true,
        Node::ThematicBreak(_) => syntax.rule = true,
        _ => {}
    }
    if let Some(children) = node.children() {
        for child in children { inspect(child, line, cursor, syntax); }
    }
}
fn line_at(source: &str, cursor: usize) -> Line {
    let start = source[..cursor].rfind('\n').map_or(0, |index| index + 1);
    let end = source[cursor..].find('\n').map_or(source.len(), |index| cursor + index);
    let range = start..end;
    let mut syntax = Syntax::default();
    if let Ok(root) = markdown::to_mdast(source, &ParseOptions::gfm()) {
        inspect(&root, &range, cursor, &mut syntax);
    }
    let mut line = Line { range, content: start, prefixes: Vec::new(), code: syntax.code,
        table: syntax.table, inline_code: syntax.inline_code, rule: syntax.rule };
    if line.code || line.table { return line; }
    let mut position = start;
    let mut quotes = 0;
    let mut list_seen = false;
    let mut heading_seen = false;
    loop {
        let indent_start = position;
        while position < end && matches!(source.as_bytes()[position], b' ' | b'\t') { position += 1; }
        let marker_start = position;
        let remaining = &source[position..end];
        let parsed = if quotes < syntax.quotes && remaining.starts_with('>') {
            quotes += 1;
            let length = 1 + usize::from(remaining[1..].starts_with([' ', '\t']));
            Some((length, PrefixKind::Quote))
        } else if !list_seen && syntax.list_starts.iter().any(|start| *start >= indent_start && *start <= marker_start) {
            let parsed = list_marker(remaining).map(|(length, next)| (length, PrefixKind::List { next }));
            list_seen = parsed.is_some(); parsed
        } else if syntax.heading && !heading_seen {
            let hashes = remaining.bytes().take_while(|byte| *byte == b'#').count();
            if (1..=6).contains(&hashes) && (hashes == remaining.len() || remaining[hashes..].starts_with([' ', '\t'])) {
                let spaces = remaining[hashes..].bytes().take_while(|byte| matches!(*byte, b' ' | b'\t')).count();
                heading_seen = true;
                Some((hashes + spaces, PrefixKind::Heading))
            } else { None }
        } else { None };
        let Some((length, kind)) = parsed else { break; };
        position += length;
        line.prefixes.push(Prefix { range: marker_start..position, indent: indent_start..marker_start, kind });
        line.content = position;
    }
    line
}
fn list_marker(source: &str) -> Option<(usize, String)> {
    let (marker_end, next) = if source.starts_with(['-', '*', '+']) {
        (1, source[..1].to_string())
    } else {
        let digits = source.bytes().take_while(u8::is_ascii_digit).count();
        if digits == 0 || digits > 9 { return None; }
        let delimiter = *source.as_bytes().get(digits)?;
        if !matches!(delimiter, b'.' | b')') { return None; }
        let number = source[..digits].parse::<u64>().ok()?.checked_add(1)?;
        (digits + 1, format!("{number}{}", delimiter as char))
    };
    if marker_end < source.len() && !source[marker_end..].starts_with([' ', '\t']) { return None; }
    let spaces = source[marker_end..].bytes().take_while(|byte| matches!(*byte, b' ' | b'\t')).count();
    let mut end = marker_end + spaces;
    let mut next = format!("{next}{}", if spaces == 0 { " " } else { &source[marker_end..end] });
    if source[end..].starts_with("[ ]") || source[end..].starts_with("[x]") || source[end..].starts_with("[X]") {
        let task_end = end + 3;
        if task_end == source.len() || source[task_end..].starts_with([' ', '\t']) {
            let spaces = source[task_end..].bytes().take_while(|byte| matches!(*byte, b' ' | b'\t')).count();
            end = task_end + spaces;
            next.push_str("[ ]");
            next.push_str(if spaces == 0 { " " } else { &source[task_end..end] });
        }
    }
    Some((end, next))
}
fn valid(source: &str, range: &Range<usize>) -> bool {
    range.start <= range.end && range.end <= source.len()
        && source.is_char_boundary(range.start) && source.is_char_boundary(range.end)
}
fn replace(source: &str, range: Range<usize>, value: &str) -> Option<FormatEdit> {
    if !valid(source, &range) || source.get(range.clone()) == Some(value) { return None; }
    let mut text = source.to_string();
    text.replace_range(range.clone(), value);
    let cursor = range.start + value.len();
    Some(FormatEdit { text, selection: cursor..cursor })
}
fn outdent_range(source: &str, prefix: &Prefix) -> Range<usize> {
    if matches!(&prefix.kind, PrefixKind::List { .. }) && !prefix.indent.is_empty() {
        let indentation = &source[prefix.indent.clone()];
        let length = if indentation.starts_with('\t') { 1 } else { indentation.len().min(2) };
        prefix.indent.start..prefix.indent.start + length
    } else { prefix.range.clone() }
}
fn remove_prefix(source: &str, cursor: usize, range: Range<usize>) -> Option<FormatEdit> {
    let mapped = cursor - cursor.saturating_sub(range.start).min(range.len());
    let mut edit = replace(source, range, "")?;
    edit.selection = mapped..mapped;
    Some(edit)
}
fn continuation(source: &str, line: &Line, soft: bool) -> String {
    let mut result = String::new();
    for prefix in &line.prefixes {
        result.push_str(&source[prefix.indent.clone()]);
        match &prefix.kind {
            PrefixKind::Quote => result.push_str(&source[prefix.range.clone()]),
            PrefixKind::List { next } if !soft => result.push_str(next),
            PrefixKind::List { .. } => result.push_str(&" ".repeat(prefix.range.len())),
            PrefixKind::Heading => break,
        }
    }
    result
}

/// Enter is a single transaction: terminate one empty container, or split content.
/// Table navigation belongs to the host; table soft breaks remain source <br>.
pub fn newline(source: &str, selection: Range<usize>, shift: bool, attachments: &Attachments) -> Option<FormatEdit> {
    if !valid(source, &selection) { return None; }
    let extensions = crate::markdown_extensions::Extensions::parse(source);
    if extensions.blocks.iter().any(|block| matches!(&block.kind, crate::markdown_extensions::BlockKind::Math
        | crate::markdown_extensions::BlockKind::Mermaid | crate::markdown_extensions::BlockKind::FrontMatter)
        && selection.start >= block.content.start && selection.end <= block.content.end) {
        return replace(source, selection, "\n");
    }
    let selected = !selection.is_empty();
    let mut text = source.to_string();
    text.replace_range(selection.clone(), "");
    let cursor = selection.start;
    let line = line_at(&text, cursor);
    if line.table { return replace(source, selection, "<br>"); }
    if !shift && !selected && !line.code && cursor >= line.content
        && text[line.content..line.range.end].trim().is_empty() {
        if let Some(prefix) = line.prefixes.last() {
            return remove_prefix(source, cursor, outdent_range(source, prefix));
        }
    }
    let indent = if line.code {
        text[line.range.start..cursor].chars().take_while(|ch| matches!(*ch, ' ' | '\t')).collect::<String>()
    } else { continuation(&text, &line, shift) };
    let break_text = if shift && !line.code && !line.inline_code { "  \n" } else { "\n" };
    let mut insertion = cursor;
    let mut closing = String::new();
    let mut opening = String::new();
    if !line.code && !line.inline_code {
        (insertion, closing, opening) = inline_split(&text, cursor, attachments);
    }
    let replacement = format!("{closing}{break_text}{indent}{opening}");
    text.insert_str(insertion, &replacement);
    let cursor = insertion + closing.len() + break_text.len() + indent.len() + opening.len();
    Some(FormatEdit { text, selection: cursor..cursor })
}
struct InlineFormat {
    full: Range<usize>,
    content: Range<usize>,
    open: String,
    close: String,
}
fn inline_formats(node: &Node, source: &str, output: &mut Vec<InlineFormat>) {
    if matches!(node, Node::Strong(_) | Node::Emphasis(_) | Node::Delete(_) | Node::Link(_) | Node::LinkReference(_)) {
        if let Some(children) = node.children() {
            if let (Some(first), Some(last)) = (children.first(), children.last()) {
                let full = node_range(node);
                let content = node_range(first).start..node_range(last).end;
                if valid(source, &full) && full.start <= content.start && content.end <= full.end {
                    output.push(InlineFormat { open: source[full.start..content.start].into(),
                        close: source[content.end..full.end].into(), full, content });
                }
            }
        }
    }
    // Do not interpret apparent formatting in code or image destinations.
    if matches!(node, Node::Code(_) | Node::InlineCode(_) | Node::Image(_) | Node::ImageReference(_)) { return; }
    if let Some(children) = node.children() { for child in children { inline_formats(child, source, output); } }
}
fn inline_split(source: &str, cursor: usize, attachments: &Attachments) -> (usize, String, String) {
    let mut formats = Vec::new();
    if let Ok(root) = markdown::to_mdast(source, &ParseOptions::gfm()) { inline_formats(&root, source, &mut formats); }
    let images = attachments.references(source);
    formats.retain(|format| !images.iter().any(|(image, _)| image.start < format.full.end && image.end > format.full.start));
    let mut insertion = cursor;
    // At the beginning, put the break before the opening delimiter. Repeat for
    // nested formats whose outer content begins at the inner marker boundary.
    loop {
        let previous = formats.iter().filter(|format| format.content.start == insertion && format.content.end > insertion)
            .map(|format| format.full.start).min().unwrap_or(insertion);
        if previous == insertion { break; }
        insertion = previous;
    }
    // At the end of a formatted run, split after its closing delimiter, not inside it.
    loop {
        let next = formats.iter().filter(|format| format.content.end == insertion && format.content.start < insertion)
            .map(|format| format.full.end).max().unwrap_or(insertion);
        if next == insertion { break; }
        insertion = next;
    }
    let mut active: Vec<_> = formats.iter().filter(|format| format.content.start < insertion && insertion < format.content.end).collect();
    active.sort_by_key(|format| std::cmp::Reverse(format.full.len()));
    let opening = active.iter().map(|format| format.open.as_str()).collect::<String>();
    let closing = active.iter().rev().map(|format| format.close.as_str()).collect::<String>();
    (insertion, closing, opening)
}

/// Deletion order: selection/atomic references, own structural prefix, format
/// boundary, adjacent text unit, or a deliberate neighbouring-line merge.
pub fn delete(source: &str, selection: Range<usize>, backwards: bool, word: bool,
    attachments: &Attachments, atoms: &[Range<usize>]) -> Option<FormatEdit> {
    if !valid(source, &selection) { return None; }
    if !selection.is_empty() {
        let mut range = selection;
        for atom in atoms {
            if range.start < atom.end && range.end > atom.start {
                range.start = range.start.min(atom.start); range.end = range.end.max(atom.end);
            }
        }
        return replace(source, range, "");
    }
    let cursor = selection.start;
    if let Some(atom) = atoms.iter().filter(|atom| valid(source, atom)).find(|atom| {
        if backwards { cursor > atom.start && cursor <= atom.end } else { cursor >= atom.start && cursor < atom.end }
    }) { return replace(source, atom.clone(), ""); }
    let line = line_at(source, cursor);
    let extensions = crate::markdown_extensions::Extensions::parse(source);
    if !word {
        if let Some(extension) = extensions.inline.iter().find(|extension| matches!(&extension.kind,
            crate::markdown_extensions::InlineKind::Highlight | crate::markdown_extensions::InlineKind::Superscript
            | crate::markdown_extensions::InlineKind::Subscript)
            && ((backwards && cursor == extension.content.start) || (!backwards && cursor == extension.content.end))) {
            let mut text = source.to_string();
            text.replace_range(extension.content.end..extension.source.end, "");
            text.replace_range(extension.source.start..extension.content.start, "");
            let mapped = cursor - (extension.content.start - extension.source.start);
            return Some(FormatEdit { text, selection: mapped..mapped });
        }
    }
    if !line.code && !line.table {
        if backwards && (cursor == line.content || cursor == line.range.start) {
            if let Some(prefix) = line.prefixes.last() {
                return remove_prefix(source, cursor, outdent_range(source, prefix));
            }
            if let Some(edit) = crate::markdown_format::block_start(source, cursor, attachments) { return Some(edit); }
        }
        if line.rule && (cursor == line.range.start || cursor == line.range.end) {
            return replace(source, line.range, "");
        }
        if !word {
            if let Some(edit) = crate::markdown_format::inline_boundary(source, cursor, backwards, attachments) { return Some(edit); }
        }
    } else if line.code && backwards {
        if let Some(edit) = crate::markdown_format::block_start(source, cursor, attachments) { return Some(edit); }
    }
    if line.code && !backwards && cursor == line.range.end && cursor < source.len() {
        if !line_at(source, cursor + 1).code { return None; }
    }
    if !line.code && !line.table && backwards && cursor == line.range.start && cursor > 0 {
        let previous = line_at(source, cursor - 1);
        if previous.code || previous.table { return None; }
    }
    if !line.table && !line.code && !backwards && cursor == line.range.end && cursor < source.len() {
        let next_start = cursor + 1;
        let next = line_at(source, next_start);
        // Never flatten a code fence or a table into the preceding paragraph.
        if next.code || next.table { return None; }
        return replace(source, cursor..next.content.max(next_start), "");
    }
    let target = if word && cursor > line.content && backwards {
        source[line.content..cursor].unicode_word_indices().next_back().map_or(line.content, |(start, _)| line.content + start)
    } else if word && cursor < line.range.end && !backwards {
        source[cursor..line.range.end].unicode_word_indices().next().map_or(line.range.end, |(start, word)| cursor + start + word.len())
    } else if backwards {
        source[..cursor].grapheme_indices(true).next_back().map_or(cursor, |(start, _)| start)
    } else {
        source[cursor..].graphemes(true).next().map_or(cursor, |grapheme| cursor + grapheme.len())
    };
    if target == cursor { return None; }
    let mut range = cursor.min(target)..cursor.max(target);
    if !word && !line.code && !line.inline_code { range = decoded_unit(source, cursor, backwards).unwrap_or(range); }
    for atom in atoms.iter().filter(|atom| valid(source, atom)) {
        if range.start < atom.end && range.end > atom.start {
            range.start = range.start.min(atom.start); range.end = range.end.max(atom.end);
        }
    }
    if line.table && (range.start < line.content || range.end > line.range.end) { return None; }
    replace(source, range, "")
}
fn decoded_unit(source: &str, cursor: usize, backwards: bool) -> Option<Range<usize>> {
    fn find(node: &Node, source: &str, cursor: usize, backwards: bool) -> Option<Range<usize>> {
        if let Node::Text(_) = node {
            let full = node_range(node);
            if cursor < full.start || cursor > full.end { return None; }
            let mut offset = full.start;
            while offset < full.end {
                let rest = &source[offset..full.end];
                let length = if rest.starts_with('\\') && rest[1..].chars().next().is_some_and(|ch| ch.is_ascii_punctuation()) { 2 }
                    else if rest.starts_with('&') {
                        rest.find(';').filter(|end| *end <= 32).map_or(1, |end| {
                            let candidate = &rest[..end + 1];
                            let recognized = markdown::to_mdast(candidate, &ParseOptions::gfm()).ok().is_some_and(|root| {
                                root.children().and_then(|children| children.first()).and_then(Node::children)
                                    .and_then(|children| children.first()).is_some_and(|node| {
                                        matches!(node, Node::Text(text) if text.value != candidate)
                                    })
                            });
                            if recognized { end + 1 } else { 1 }
                        })
                    } else { rest.graphemes(true).next()?.len() };
                let range = offset..offset + length;
                if (backwards && range.end == cursor) || (!backwards && range.start == cursor) { return Some(range); }
                offset += length;
            }
        }
        if matches!(node, Node::Code(_) | Node::InlineCode(_)) { return None; }
        if let Some(children) = node.children() {
            for child in children { if let Some(range) = find(child, source, cursor, backwards) { return Some(range); } }
        }
        None
    }
    let root = markdown::to_mdast(source, &ParseOptions::gfm()).ok()?;
    find(&root, source, cursor, backwards)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn enter(source: &str, cursor: usize) -> FormatEdit { newline(source, cursor..cursor, false, &Attachments::default()).unwrap() }
    fn backspace(source: &str, cursor: usize) -> FormatEdit {
        delete(source, cursor..cursor, true, false, &Attachments::default(), &[]).unwrap()
    }
    #[test]
    fn empty_list_exit_preserves_every_existing_newline() {
        let edit = enter("- text\n- \n\n", 9);
        assert_eq!(edit.text, "- text\n\n\n");
        assert_eq!(edit.selection, 7..7);
        let mut edit = enter("- ", 2);
        assert_eq!(edit.text, "");
        for _ in 0..16 { edit = enter(&edit.text, edit.selection.start); }
        assert_eq!(edit.text, "\n".repeat(16));
    }
    #[test]
    fn continues_numbered_task_and_quoted_lists() {
        for (source, expected) in [("9) 中文", "9) 中文\n10) "), ("- [x] done", "- [x] done\n- [ ] "),
            ("> - item", "> - item\n> - ")] {
            assert_eq!(enter(source, source.len()).text, expected);
        }
    }
    #[test]
    fn empty_nested_containers_downgrade_one_level() {
        assert_eq!(enter("  - ", 4).text, "- ");
        assert_eq!(enter("> - ", 4).text, "> ");
        assert_eq!(enter("> > ", 4).text, "> ");
        assert_eq!(backspace("> > text", 4).text, "> text");
    }
    #[test]
    fn downgrade_is_not_character_deletion_or_paragraph_merge() {
        assert_eq!(backspace("# 中文\n\n", 2).text, "中文\n\n");
        assert_eq!(backspace("- 中文", 2).text, "中文");
        assert_eq!(backspace("### text", 2).text, "## text");
        assert_eq!(backspace("12 apples", 3).text, "12apples");
        assert_eq!(backspace("- text\n\n", 8).text, "- text\n");
    }
    #[test]
    fn delete_at_heading_end_does_not_remove_heading_format() {
        let source = "# first\n# second";
        let edit = delete(source, 7..7, false, false, &Attachments::default(), &[]).unwrap();
        assert_eq!(edit.text, "# firstsecond");
    }
    #[test]
    fn paragraph_split_balances_inline_formatting() {
        assert_eq!(enter("**abcd**", 4).text, "**ab**\n**cd**");
        assert_eq!(enter("**text**", 6).text, "**text**\n");
        assert_eq!(enter("- **abcd**", 6).text, "- **ab**\n- **cd**");
        assert_eq!(enter("# title", 7).text, "# title\n");
    }
    #[test]
    fn selection_enter_is_one_edit_and_uses_remaining_content() {
        let edit = newline("- abcd", 3..5, false, &Attachments::default()).unwrap();
        assert_eq!(edit.text, "- a\n- d");
        assert_eq!(edit.selection, 6..6);
    }
    #[test]
    fn code_keeps_literal_markers_and_quote_preserves_soft_continuation() {
        assert_eq!(enter("```\n- code\n```", 10).text, "```\n- code\n\n```");
        assert_eq!(backspace("```\n# code\n```", 6).text, "```\n#code\n```");
        assert_eq!(newline("> text", 6..6, true, &Attachments::default()).unwrap().text, "> text  \n> ");
    }
    #[test]
    fn image_reference_and_graphemes_are_atomic_without_geometry() {
        let source = "a path.png b";
        assert_eq!(delete(source, 10..10, true, false, &Attachments::default(), &[2..10]).unwrap().text, "a  b");
        assert_eq!(backspace("🙂\n\n", 6).text, "🙂\n");
        assert_eq!(backspace("a\\*b", 3).text, "ab");
    }
}

#[cfg(test)]
mod sequence_tests {
    use super::*;
    #[test]
    fn continuous_enter_after_heading_keeps_blank_lines_and_caret() {
        let attachments = Attachments::default();
        let mut text = "# title".to_string();
        let mut cursor = text.len();
        for _ in 0..16 {
            let edit = newline(&text, cursor..cursor, false, &attachments).unwrap();
            text = edit.text; cursor = edit.selection.start;
        }
        assert_eq!(text, format!("# title{}", "\n".repeat(16)));
        assert_eq!(cursor, text.len());
        let document = crate::markdown_projection::Document::new(&text, cursor, None, &attachments);
        assert_eq!(document.rows.len(), 17);
    }
    #[test]
    fn list_exit_and_repeated_backspace_do_not_eat_prior_text() {
        let attachments = Attachments::default();
        let source = "- 中文\n- \n\n";
        let cursor = "- 中文\n- ".len();
        let edit = newline(source, cursor..cursor, false, &attachments).unwrap();
        assert_eq!(edit.text, "- 中文\n\n\n");
        assert_eq!(edit.selection.start, "- 中文\n".len());
        let edit = delete(&edit.text, edit.selection, true, false, &attachments, &[]).unwrap();
        assert_eq!(edit.text, "- 中文\n\n");
    }
    #[test]
    fn link_split_keeps_destination_on_both_fragments() {
        let edit = newline("[abcd](url)", 3..3, false, &Attachments::default()).unwrap();
        assert_eq!(edit.text, "[ab](url)\n[cd](url)");
    }
}

#[cfg(test)]
mod format_edge_tests {
    use super::*;
    #[test]
    fn splits_before_opening_markers_and_after_closing_markers() {
        for (source, cursor, expected) in [("**text**", 2, "\n**text**"),
            ("[text](url)", 1, "\n[text](url)"), ("***text***", 3, "\n***text***")] {
            assert_eq!(newline(source, cursor..cursor, false, &Attachments::default()).unwrap().text, expected);
        }
    }
    #[test]
    fn backspace_at_inline_code_start_unwraps_without_deleting_content() {
        let edit = delete("`code`", 1..1, true, false, &Attachments::default(), &[]).unwrap();
        assert_eq!(edit.text, "code");
    }
}

#[cfg(test)]
mod source_unit_tests {
    use super::*;
    #[test]
    fn headings_do_not_consume_literal_hashes_in_content() {
        assert_eq!(delete("# # title", 2..2, true, false, &Attachments::default(), &[]).unwrap().text, "# title");
    }
    #[test]
    fn entities_delete_as_display_units_but_unknown_names_do_not() {
        assert_eq!(delete("a&amp;b", 6..6, true, false, &Attachments::default(), &[]).unwrap().text, "ab");
        assert_eq!(delete("a&unknown;b", 10..10, true, false, &Attachments::default(), &[]).unwrap().text, "a&unknownb");
    }
}
