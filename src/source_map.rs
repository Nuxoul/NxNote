//! Current-projection source mapping, independent of frames and window geometry.
//! The projection is the seam: navigation and edits consume the same units that
//! shaping consumes. Concealed delimiters are not accidental deletion targets.
use crate::markdown_projection::{Document, Inline};
use std::ops::Range;
use unicode_segmentation::UnicodeSegmentation;

#[derive(Clone, Default)]
pub struct SourceMap {
    stops: Vec<usize>,
    units: Vec<Range<usize>>,
    atoms: Vec<Range<usize>>,
}
impl SourceMap {
    pub fn new(source: &str, document: &Document) -> Self {
        let mut result = Self::default();
        for row in &document.rows {
            for cell in &row.cells {
                if cell.inline.is_empty() { result.stops.extend([cell.source.start, cell.source.end]); }
                for inline in &cell.inline {
                    match inline {
                        Inline::Image { source: range, .. } | Inline::Widget { source: range, .. } => {
                            if valid(source, range) { result.units.push(range.clone()); result.atoms.push(range.clone()); }
                        }
                        Inline::Text { text, map, .. } => {
                            for (start, grapheme) in text.grapheme_indices(true) {
                                let end = start + grapheme.len();
                                let first = map.partition_point(|character| character.display.end <= start);
                                let last = map.partition_point(|character| character.display.start < end);
                                let entries = &map[first..last];
                                let mut range: Option<Range<usize>> = None;
                                for character in entries {
                                    if !valid(source, &character.source) { continue; }
                                    if let Some(range) = &mut range {
                                        range.start = range.start.min(character.source.start);
                                        range.end = range.end.max(character.source.end);
                                    } else { range = Some(character.source.clone()); }
                                }
                                if let Some(range) = range.filter(|range| !range.is_empty()) {
                                    if source.get(range.clone()) != Some(grapheme) { result.atoms.push(range.clone()); }
                                    result.units.push(range);
                                }
                            }
                        }
                    }
                }
            }
        }
        // Syntax-color and style boundaries may split a combining grapheme.
        // Source identity, not a TextRun boundary, defines its editable unit.
        let boundaries = source.grapheme_indices(true).map(|(offset, _)| offset)
            .chain(std::iter::once(source.len())).collect::<Vec<_>>();
        for unit in &mut result.units {
            let first = boundaries.partition_point(|offset| *offset <= unit.start);
            unit.start = boundaries[first.saturating_sub(1)];
            let last = boundaries.partition_point(|offset| *offset < unit.end);
            unit.end = boundaries.get(last).copied().unwrap_or(source.len());
            if source[unit.clone()].chars().count() > 1 && source[unit.clone()].graphemes(true).count() == 1 {
                result.atoms.push(unit.clone());
            }
        }
        for atom in &mut result.atoms {
            let first = boundaries.partition_point(|offset| *offset <= atom.start);
            atom.start = boundaries[first.saturating_sub(1)];
            let last = boundaries.partition_point(|offset| *offset < atom.end);
            atom.end = boundaries.get(last).copied().unwrap_or(source.len());
        }
        result.units.sort_by_key(|range| (range.start, range.end)); result.units.dedup();
        for range in &result.units { result.stops.extend([range.start, range.end]); }
        result.stops.retain(|offset| *offset <= source.len() && source.is_char_boundary(*offset));
        result.stops.sort_unstable(); result.stops.dedup();
        result.units.sort_by_key(|range| (range.start, range.end)); result.units.dedup();
        result.atoms.sort_by_key(|range| (range.start, range.end)); result.atoms.dedup();
        result
    }
    pub fn line_edge(&self, source: &str, cursor: usize, end: bool) -> usize {
        let cursor = boundary(source, cursor);
        let start = source[..cursor].rfind('\n').map_or(0, |offset| offset + 1);
        let last = source[cursor..].find('\n').map_or(source.len(), |offset| cursor + offset);
        if end { self.stops.iter().copied().rev().find(|offset| *offset >= start && *offset <= last).unwrap_or(last) }
        else { self.stops.iter().copied().find(|offset| *offset >= start && *offset <= last).unwrap_or(start) }
    }
    pub fn vertical(&self, source: &str, cursor: usize, direction: i32) -> usize {
        let cursor = boundary(source, cursor);
        let start = source[..cursor].rfind('\n').map_or(0, |offset| offset + 1);
        let column = source[start..cursor].graphemes(true).count();
        let target_start = if direction < 0 {
            if start == 0 { return cursor; }
            source[..start - 1].rfind('\n').map_or(0, |offset| offset + 1)
        } else {
            let Some(end) = source[cursor..].find('\n') else { return cursor; };
            cursor + end + 1
        };
        let target_end = source[target_start..].find('\n').map_or(source.len(), |offset| target_start + offset);
        let target = source[target_start..target_end].grapheme_indices(true).nth(column).map_or(target_end, |(offset, _)| target_start + offset);
        self.stops.iter().copied().filter(|offset| *offset >= target_start && *offset <= target_end)
            .min_by_key(|offset| offset.abs_diff(target)).unwrap_or(target)
    }
    pub fn step(&self, source: &str, cursor: usize, backwards: bool, word: bool) -> usize {
        let cursor = boundary(source, cursor);
        let target = if word {
            if backwards { source[..cursor].unicode_word_indices().next_back().map_or(0, |(offset, _)| offset) }
            else { source[cursor..].unicode_word_indices().next().map_or(source.len(), |(offset, word)| cursor + offset + word.len()) }
        } else { cursor };
        let mapped = if backwards {
            self.stops.iter().copied().rev().find(|offset| if word { *offset <= target && *offset < cursor } else { *offset < cursor })
        } else {
            self.stops.iter().copied().find(|offset| if word { *offset >= target && *offset > cursor } else { *offset > cursor })
        };
        mapped.unwrap_or_else(|| if word { target } else if backwards {
            source[..cursor].grapheme_indices(true).next_back().map_or(cursor, |(offset, _)| offset)
        } else { source[cursor..].graphemes(true).next().map_or(cursor, |grapheme| cursor + grapheme.len()) })
    }
    /// Only a unit touching the caret may be deleted; never search past a newline.
    pub fn adjacent(&self, cursor: usize, backwards: bool) -> Option<Range<usize>> {
        if backwards { self.units.iter().filter(|range| range.end == cursor).max_by_key(|range| range.start).cloned() }
        else { self.units.iter().filter(|range| range.start == cursor).min_by_key(|range| range.end).cloned() }
    }
    pub fn atoms(&self) -> &[Range<usize>] { &self.atoms }
    /// Replacement ranges cannot cut a rendered atom or a Unicode grapheme in half.
    /// An insertion inside one goes after it; insertion at either edge is unchanged.
    pub fn replacement_range(&self, source: &str, range: Range<usize>) -> Range<usize> {
        let start = boundary(source, range.start);
        let end = boundary(source, range.end).max(start);
        let mut range = start..end;
        if range.is_empty() {
            if let Some(unit) = self.units.iter().filter(|unit| unit.start < start && start < unit.end).max_by_key(|unit| unit.end) {
                return unit.end..unit.end;
            }
            return range;
        }
        loop {
            let previous = range.clone();
            for unit in &self.units {
                if range.start < unit.end && range.end > unit.start {
                    range.start = range.start.min(unit.start); range.end = range.end.max(unit.end);
                }
            }
            if range == previous { return range; }
        }
    }
}
fn boundary(source: &str, offset: usize) -> usize {
    let mut offset = offset.min(source.len());
    while !source.is_char_boundary(offset) { offset -= 1; }
    offset
}
fn valid(source: &str, range: &Range<usize>) -> bool {
    range.start <= range.end && range.end <= source.len()
        && source.is_char_boundary(range.start) && source.is_char_boundary(range.end)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::attachments::Attachments;
    fn project(source: &str, cursor: usize) -> SourceMap {
        SourceMap::new(source, &Document::new(source, cursor, None, &Attachments::default()))
    }
    #[test]
    fn navigation_uses_current_source_without_a_painted_frame() {
        for source in ["-1    ", "1.32  ", "中文🙂  "] {
            let map = project(source, source.len());
            assert_eq!(map.step(source, source.len(), true, false), source.len() - 1);
            assert_eq!(map.adjacent(source.len(), true), Some(source.len() - 1..source.len()));
        }
    }
    #[test]
    fn rendered_entities_and_labels_have_atomic_source_ranges() {
        let source = "a&amp;b [[目标|别名]] end";
        let map = project(source, source.len());
        assert_eq!(map.adjacent(6, true), Some(1..6));
        assert_eq!(map.replacement_range(source, 2..3), 1..6);
        let start = source.find("[[").unwrap(); let end = source.find("]]").unwrap() + 2;
        assert_eq!(map.replacement_range(source, start + 3..start + 3), end..end);
    }
    #[test]
    fn plain_unicode_units_preserve_combining_graphemes() {
        let source = "ae\u{301}🙂";
        let map = project(source, source.len());
        assert_eq!(map.replacement_range(source, 2..2), 4..4);
        assert_eq!(map.replacement_range(source, 2..4), 1..4);
    }
    #[test]
    fn adjacent_delete_never_jumps_over_an_empty_line() {
        let source = "a\n\nb"; let map = project(source, 2);
        assert_eq!(map.adjacent(2, true), None);
        assert_eq!(map.adjacent(2, false), None);
    }
}

#[cfg(test)]
mod transaction_tests {
    use super::*;
    use crate::{attachments::Attachments, markdown_edit};
    #[test]
    fn projected_label_delete_removes_only_its_own_reference() {
        let source = "before [[目标|别名]] after";
        let end = source.find("]]").unwrap() + 2;
        let document = Document::new(source, source.len(), None, &Attachments::default());
        let map = SourceMap::new(source, &document);
        let edit = markdown_edit::delete(source, end..end, true, false, &Attachments::default(), map.atoms()).unwrap();
        assert_eq!(edit.text, "before  after");
    }
    #[test]
    fn line_navigation_without_geometry_keeps_empty_lines() {
        let source = "first  \n\nlast";
        let document = Document::new(source, source.len(), None, &Attachments::default());
        let map = SourceMap::new(source, &document);
        assert_eq!(map.line_edge(source, 3, true), 7);
        assert_eq!(map.line_edge(source, 3, false), 0);
        assert_eq!(map.vertical(source, 3, 1), 8);
        assert_eq!(map.vertical(source, 8, 1), 9);
    }
    #[test]
    fn literal_markers_keep_typing_and_deleting_spaces_in_the_same_source_unit() {
        for prefix in ["-1", "1.32", "#word"] {
            let source = format!("{prefix}  ");
            let document = Document::new(&source, source.len(), None, &Attachments::default());
            let map = SourceMap::new(&source, &document);
            let edit = markdown_edit::delete(&source, source.len()..source.len(), true, false, &Attachments::default(), map.atoms()).unwrap();
            assert_eq!(edit.text, format!("{prefix} "));
        }
    }
}
