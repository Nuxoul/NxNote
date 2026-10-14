//! UI-independent line actions and image URL classification.
use std::ops::Range;

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
