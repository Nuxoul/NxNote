//! Read-only logical text-line numbers, positioned by the native frame geometry.
//! Soft wraps do not increment; table cells in the same row share their numbers.
use std::collections::HashMap;

use gpui::{App, Bounds, ContentMask, Pixels, TextAlign, TextStyle, Window, fill, point, px, size};

use super::{
    document::EditorDocument, frame::FrameLayout, id::BlockId,
    kind::BlockKindData, position::BlockPosition, style::BlockEditorStyle,
};

// Fixed layout allocation: digit-count transitions never reflow the document.
pub(crate) const WIDTH: f32 = 64.;
const HANDLE_WIDTH: f32 = 16.;

#[derive(Default)]
pub(crate) struct LineNumbers {
    leaves: HashMap<BlockId, NumberedLeaf>,
    pub(crate) total: usize,
}
struct NumberedLeaf {
    first: usize,
    starts: Vec<usize>,
}
fn line_starts(text: &str) -> Vec<usize> {
    std::iter::once(0)
        .chain(text.match_indices('\n').map(|(offset, _)| offset + 1))
        .collect()
}

// UI chrome must not inherit Markdown fonts, emphasis or decorations.
fn number_style(typography: &TextStyle, window: &Window) -> (Pixels, gpui::Font) {
    let body = f32::from(typography.font_size.to_pixels(window.rem_size()));
    let mut font = gpui::font("Consolas");
    font.weight = gpui::FontWeight::NORMAL;
    font.style = gpui::FontStyle::Normal;
    (px((body * 0.85).clamp(10., 14.)), font)
}
impl LineNumbers {
    pub(crate) fn new(doc: &EditorDocument) -> Self {
        let mut result = Self::default();
        let mut table_row = None;
        let mut row_first = 1;
        let mut row_lines = 0;
        for id in doc.leaves() {
            let Some(text) = doc.leaf_text(*id) else { continue; };
            let starts = line_starts(text);
            let row = if matches!(doc.kind(*id), Some(BlockKindData::TableCell)) {
                doc.parent(*id)
            } else {
                None
            };
            let first = if let Some(row) = row {
                if table_row != Some(row) {
                    table_row = Some(row);
                    row_first = result.total + 1;
                    row_lines = 0;
                }
                row_lines = row_lines.max(starts.len());
                result.total = row_first + row_lines - 1;
                row_first
            } else {
                table_row = None;
                let first = result.total + 1;
                result.total += starts.len();
                first
            };
            result.leaves.insert(*id, NumberedLeaf { first, starts });
        }
        result
    }
    fn at(&self, pos: BlockPosition) -> Option<usize> {
        let leaf = self.leaves.get(&pos.block)?;
        let line = leaf.starts.partition_point(|start| *start <= pos.offset).saturating_sub(1);
        Some(leaf.first + line)
    }
    pub(crate) fn paint(
        &self, frame: &FrameLayout, bounds: Bounds<Pixels>, width: Pixels, caret: Option<BlockPosition>,
        typography: &TextStyle, _style: &BlockEditorStyle, window: &mut Window, cx: &mut App,
    ) {
        let Some(content) = frame.content else { return; };
        if width <= px(0.) { return; }
        // Use the native root, not inset text bounds, so the lane reaches the
        // viewport's left edge and includes its top/bottom presentation space.
        let gutter = Bounds::new(bounds.origin, size((content.left() - bounds.left() - px(HANDLE_WIDTH)).max(px(0.)), bounds.size.height));
        let active = caret.and_then(|pos| self.at(pos));
        // One number per table row, not once per cell. Keep the top-most row
        // anchor when cells have different wrapping/inline metrics.
        let mut rows: std::collections::BTreeMap<usize, (Pixels, Pixels, Pixels, Pixels)> = std::collections::BTreeMap::new();
        for leaf in &frame.leaves {
            let Some(numbered) = self.leaves.get(&leaf.info.block) else { continue; };
            for (index, offset) in numbered.starts.iter().enumerate() {
                let y = leaf.caret_point(*offset).y;
                let bottom = numbered.starts.get(index + 1)
                    .map(|next| leaf.caret_point(*next).y)
                    .unwrap_or_else(|| leaf.bounds().bottom());
                let row = (y, leaf.line_height(), leaf.caret_height(), bottom.max(y + leaf.line_height()));
                rows.entry(numbered.first + index)
                    .and_modify(|prev| {
                        let bottom = prev.3.max(row.3);
                        if y < prev.0 { *prev = row; }
                        prev.3 = bottom;
                    })
                    .or_insert(row);
            }
        }
        let (font_size, font) = number_style(typography, window);
        let label_height = font_size * 1.5;
        let tokens = gpui_kit::base::Theme::global(cx).tokens.colors;
        let mut background = tokens.background;
        background.a = 1.;
        window.with_content_mask(Some(ContentMask { bounds: gutter }), |window| {
            window.paint_quad(fill(gutter, background));
            // A semantic solid surface, not cumulative translucent gray.
            let scale = window.scale_factor();
            let thickness = px(1. / scale);
            let edge = px((f32::from(gutter.right()) * scale).floor() / scale);
            window.paint_quad(fill(
                Bounds::new(point(edge - thickness, gutter.top()), size(thickness, gutter.size.height)),
                tokens.border.opacity(0.55),
            ));
            let clip = window.content_mask().bounds;
            for (number, (y, line_height, _caret_height, bottom)) in rows {
                if bottom <= clip.top() || y >= clip.bottom() { continue; }
                let selected = active == Some(number);
                // Emphasize only the active number; no boxed cell background.
                let text = number.to_string();
                let mut run = typography.to_run(text.len());
                run.font = font.clone();
                run.color = if selected { tokens.foreground } else { tokens.muted_foreground.opacity(0.8) };
                run.background_color = None;
                run.underline = None;
                run.strikethrough = None;
                let label = window.text_system().shape_line(text.into(), font_size, &[run], None);
                let centre = y + line_height / 2.;
                let scale = window.scale_factor();
                let snap = |value: Pixels| px((f32::from(value) * scale).round() / scale);
                let origin = point(snap(gutter.right() - px(6.) - label.width), snap(centre - label_height / 2.));
                let _ = label.paint(origin, label_height, TextAlign::Left, None, window, cx);
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::line_starts;
    #[test]
    fn logical_lines_include_empty_and_trailing_rows() {
        assert_eq!(line_starts(""), vec![0]);
        assert_eq!(line_starts("中文\n\n🙂\n"), vec![0, 7, 8, 13]);
        assert_eq!(line_starts("long line without a hard break"), vec![0]);
    }
}
