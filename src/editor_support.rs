//! Markdown editing rules layered over the Kit editor, without a second text engine.
use std::ops::Range;

pub fn continuation_edit(text: &str, cursor: usize) -> Option<(Range<usize>, String)> {
    if cursor == 0
        || cursor > text.len()
        || !text.is_char_boundary(cursor)
        || text.as_bytes()[cursor - 1] != b'\n'
    {
        return None;
    }
    let end = cursor - 1;
    let start = text[..end].rfind('\n').map(|i| i + 1).unwrap_or(0);
    let line = &text[start..end];
    // Don't continue a list inside a fenced code block.
    let fences = text[..start]
        .lines()
        .filter(|l| l.trim_start().starts_with("```") || l.trim_start().starts_with("~~~"))
        .count();
    if fences % 2 != 0 {
        return None;
    }
    let trimmed = line.trim_start_matches([' ', '\t']);
    let indent = &line[..line.len() - trimmed.len()];
    let (marker, content) = if let Some(rest) = trimmed
        .strip_prefix("- [ ] ")
        .or_else(|| trimmed.strip_prefix("- [x] "))
        .or_else(|| trimmed.strip_prefix("- [X] "))
    {
        ("- [ ] ".to_string(), rest)
    } else if trimmed.len() >= 2
        && matches!(trimmed.as_bytes()[0], b'-' | b'*' | b'+')
        && trimmed.as_bytes()[1] == b' '
    {
        (trimmed[..2].to_string(), &trimmed[2..])
    } else {
        let digits = trimmed.bytes().take_while(u8::is_ascii_digit).count();
        if digits == 0 || !trimmed[digits..].starts_with(". ") {
            return None;
        }
        let number = trimmed[..digits].parse::<u64>().ok()?.checked_add(1)?;
        (format!("{number}. "), &trimmed[digits + 2..])
    };
    if content.trim().is_empty() {
        Some((start..cursor, indent.to_string()))
    } else {
        Some((cursor..cursor, format!("{indent}{marker}")))
    }
}

pub fn image_url(text: &str) -> bool {
    let lower = text.trim().to_ascii_lowercase();
    if !(lower.starts_with("https://") || lower.starts_with("http://"))
        || lower.chars().any(char::is_whitespace)
    {
        return false;
    }
    let path = lower.split(['?', '#']).next().unwrap_or("");
    [".png", ".jpg", ".jpeg", ".gif", ".webp", ".bmp", ".avif"]
        .iter()
        .any(|ext| path.ends_with(ext))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn unicode_list_continues() {
        let text = "- 中文🙂\n";
        assert_eq!(
            continuation_edit(text, text.len()),
            Some((text.len()..text.len(), "- ".into()))
        );
    }
    #[test]
    fn ordered_and_task_lists_continue() {
        assert_eq!(continuation_edit("9. item\n", 8).unwrap().1, "10. ");
        assert_eq!(
            continuation_edit("  - [x] done\n", 13).unwrap().1,
            "  - [ ] "
        );
    }
    #[test]
    fn empty_list_exits() {
        assert_eq!(continuation_edit("- \n", 3), Some((0..3, "".into())));
    }
    #[test]
    fn code_and_non_newline_are_unchanged() {
        assert!(continuation_edit("```\n- code\n", 11).is_none());
        assert!(continuation_edit("- 中文", 5).is_none());
    }
    #[test]
    fn image_urls_handle_query_and_case() {
        assert!(image_url("https://example.com/图片.PNG?size=2"));
        assert!(!image_url("file:///image.png"));
        assert!(!image_url("https://example.com/a.png text"));
    }
}

#[derive(Clone, Copy)]
pub enum LineAction {
    MoveUp,
    MoveDown,
    CopyUp,
    CopyDown,
    Delete,
}

pub fn edit_lines(
    text: &str,
    selection: Range<usize>,
    action: LineAction,
) -> Option<(String, Range<usize>)> {
    if selection.end > text.len()
        || selection.start > selection.end
        || !text.is_char_boundary(selection.start)
        || !text.is_char_boundary(selection.end)
    {
        return None;
    }
    let mut lines: Vec<&str> = text.split('\n').collect();
    let lo = text[..selection.start]
        .bytes()
        .filter(|b| *b == b'\n')
        .count();
    let last = if selection.end > selection.start && text[..selection.end].ends_with('\n') {
        selection.end - 1
    } else {
        selection.end
    };
    let hi = text[..last].bytes().filter(|b| *b == b'\n').count();
    let start = lines[..lo].iter().map(|l| l.len() + 1).sum::<usize>();
    let end = start + lines[lo..=hi].iter().map(|l| l.len() + 1).sum::<usize>() - 1;
    let block = lines[lo..=hi].to_vec();
    let count = block.len();
    let new_lo = match action {
        LineAction::MoveUp => {
            if lo == 0 {
                return None;
            }
            lines[lo - 1..=hi].rotate_left(1);
            lo - 1
        }
        LineAction::MoveDown => {
            if hi + 1 == lines.len() {
                return None;
            }
            lines[lo..=hi + 1].rotate_right(1);
            lo + 1
        }
        LineAction::CopyUp => {
            lines.splice(lo..lo, block);
            lo + count
        }
        LineAction::CopyDown => {
            lines.splice(hi + 1..hi + 1, block);
            lo
        }
        LineAction::Delete => {
            lines.drain(lo..=hi);
            if lines.is_empty() {
                lines.push("");
            }
            lo.min(lines.len() - 1)
        }
    };
    let new_start = lines[..new_lo].iter().map(|l| l.len() + 1).sum::<usize>();
    let new_text = lines.join("\n");
    let new_selection = if matches!(action, LineAction::Delete) {
        new_start..new_start
    } else {
        (new_start + selection.start - start).min(new_text.len())
            ..(new_start + selection.end.min(end) - start).min(new_text.len())
    };
    Some((new_text, new_selection))
}

#[cfg(test)]
mod line_tests {
    use super::*;
    #[test]
    fn moves_unicode_block_and_selection() {
        let text = "first\n中文🙂\nlast";
        let end = "first\n中文🙂".len();
        let (out, selection) = edit_lines(text, 6..end, LineAction::MoveUp).unwrap();
        assert_eq!(out, "中文🙂\nfirst\nlast");
        assert_eq!(&out[selection], "中文🙂");
    }
    #[test]
    fn deletes_and_copies_lines() {
        assert_eq!(
            edit_lines("a\nb", 0..0, LineAction::CopyDown).unwrap().0,
            "a\na\nb"
        );
        assert_eq!(edit_lines("a\nb", 0..0, LineAction::Delete).unwrap().0, "b");
        assert!(edit_lines("a", 0..0, LineAction::MoveUp).is_none());
    }
}
