//! Native, source-mapped document flow. The parent owns all text input and selection.
//! Geometry is replaced during prepaint, without notifying the editor or scheduling frames.
use crate::markdown_projection::{Inline, Marks, Row, SourceChar};
use gpui_kit::base::TextViewStyle;
use gpui_kit::component::{button::{Button, ButtonVariants}, hover_card::HoverCard, Sizable};
use gpui_kit::{prelude::*, *};
use markdown::mdast::AlignKind;
use std::{cell::RefCell, collections::BTreeMap, ops::Range, path::PathBuf, rc::Rc, time::Duration};
use unicode_segmentation::UnicodeSegmentation;
#[path = "markdown_widget_layout.rs"]
mod widget_layout;

#[derive(Clone, Copy)]
pub struct CaretStop {
    pub source: usize,
    pub position: Point<Pixels>,
    pub height: Pixels,
}

#[derive(Clone)]
struct SourceQuad {
    source: Range<usize>,
    bounds: Bounds<Pixels>,
}
#[derive(Clone)]
struct VisualGeometry {
    bounds: Bounds<Pixels>,
    cell: usize,
    stops: Vec<CaretStop>,
    spans: Vec<SourceQuad>,
}
#[derive(Clone)]
struct RowGeometry {
    source: Range<usize>,
    bounds: Bounds<Pixels>,
    cells: Vec<Bounds<Pixels>>,
    lines: Vec<VisualGeometry>,
}

#[derive(Default)]
pub struct DocumentGeometry {
    rows: BTreeMap<usize, RowGeometry>,
}
impl DocumentGeometry {
    /// Clear when rebuilding the projection, so removed rows cannot retain stale geometry.
    pub fn clear(&mut self) { self.rows.clear(); }

    pub fn hit(&self, point: Point<Pixels>) -> Option<usize> {
        let row = self.rows.values().min_by_key(|row| distance_to_interval(point.y, row.bounds.top(), row.bounds.bottom()))?;
        let cell = row.cells.iter().enumerate().min_by_key(|(_, bounds)| distance_to_interval(point.x, bounds.left(), bounds.right()))?.0;
        let line = row.lines.iter().filter(|line| line.cell == cell)
            .min_by_key(|line| distance_to_interval(point.y, line.bounds.top(), line.bounds.bottom()))?;
        // Hidden-syntax aliases share positions with visible glyphs, but clicks on
        // those glyphs should prefer their actual source boundaries.
        let visible: Vec<_> = line.stops.iter().copied().filter(|stop| line.spans.iter()
            .any(|span| stop.source == span.source.start || stop.source == span.source.end)).collect();
        closest_x(if visible.is_empty() { &line.stops } else { &visible }, point.x).map(|s| s.source)
    }

    pub fn caret(&self, source: usize) -> Option<CaretStop> {
        // Prefer the downstream position at a soft-wrap boundary.
        let mut exact = None;
        for row in self.rows.values() {
            for line in &row.lines {
                for stop in &line.stops {
                    if stop.source == source { exact = Some(*stop); }
                }
            }
        }
        if exact.is_some() { return exact; }
        let row = self.rows.values().find(|r| source >= r.source.start && source <= r.source.end);
        let nearest = |rows: Vec<&RowGeometry>| rows.into_iter().flat_map(|r| &r.lines)
            .flat_map(|l| &l.stops).min_by_key(|s| s.source.abs_diff(source)).copied();
        nearest(row.into_iter().collect()).or_else(|| nearest(self.rows.values().collect()))
    }

    pub fn vertical(&self, source: usize, preferred_x: Pixels, direction: i32) -> Option<CaretStop> {
        let current = self.caret(source)?;
        if direction == 0 { return Some(current); }
        let lines: Vec<_> = self.rows.values().flat_map(|r| &r.lines).filter(|line| {
            if direction < 0 { line.bounds.top() < current.position.y - px(0.5) }
            else { line.bounds.top() > current.position.y + px(0.5) }
        }).collect();
        let target_y = lines.iter().map(|l| l.bounds.top()).min_by(|a, b| {
            (*a - current.position.y).abs().partial_cmp(&(*b - current.position.y).abs())
                .unwrap_or(std::cmp::Ordering::Equal)
        })?;
        let candidates: Vec<_> = lines.into_iter().filter(|l| (l.bounds.top() - target_y).abs() < px(0.5))
            .flat_map(|l| l.stops.iter().copied()).collect();
        closest_x(&candidates, preferred_x)
    }

    pub fn line_edge(&self, source: usize, end: bool) -> Option<usize> {
        let caret = self.caret(source)?;
        let line = self.rows.values().flat_map(|r| &r.lines).find(|l| {
            l.stops.iter().any(|s| s.source == caret.source && s.position == caret.position)
        })?;
        let stop = if end {
            line.stops.iter().max_by(|a, b| a.position.x.partial_cmp(&b.position.x).unwrap_or(std::cmp::Ordering::Equal))
        } else {
            line.stops.iter().min_by(|a, b| a.position.x.partial_cmp(&b.position.x).unwrap_or(std::cmp::Ordering::Equal))
        }?;
        Some(stop.source)
    }


    pub fn selection(&self, range: Range<usize>) -> Vec<Bounds<Pixels>> {
        self.rows.values().flat_map(|r| selection_quads(r, &range)).collect()
    }
    /// Native IME geometry is anchored to one visual line, just as GPUI/Zed
    /// candidate placement is. Do not combine the next line's x coordinate.
    pub fn input_bounds(&self, range: Range<usize>) -> Option<Bounds<Pixels>> {
        let start = self.caret(range.start)?;
        let quads = self.selection(range.clone());
        let mut left = start.position.x;
        let mut right = left + px(1.);
        let mut height = start.height;
        for quad in quads.iter().filter(|quad| (quad.top() - start.position.y).abs() < px(0.5)) {
            left = left.min(quad.left()); right = right.max(quad.right()); height = height.max(quad.size.height);
        }
        if let Some(end) = self.caret(range.end).filter(|end| (end.position.y - start.position.y).abs() < px(0.5)) {
            right = right.max(end.position.x);
        }
        Some(Bounds::new(point(left, start.position.y), size((right - left).max(px(1.)), height)))
    }
}
fn distance_to_interval(value: Pixels, start: Pixels, end: Pixels) -> Pixels {
    if value < start { start - value } else if value > end { value - end } else { px(0.) }
}
fn closest_x(stops: &[CaretStop], x: Pixels) -> Option<CaretStop> {
    stops.iter().min_by(|a, b| {
        let distance = (a.position.x - x).abs().partial_cmp(&(b.position.x - x).abs()).unwrap_or(std::cmp::Ordering::Equal);
        distance.then_with(|| if x <= a.position.x.min(b.position.x) { a.source.cmp(&b.source) } else { b.source.cmp(&a.source) })
    }).copied()
}
fn selection_quads(row: &RowGeometry, range: &Range<usize>) -> Vec<Bounds<Pixels>> {
    if range.is_empty() { return Vec::new(); }
    let mut result = Vec::new();
    for line in &row.lines {
        if line.spans.is_empty() {
            if let Some(stop) = line.stops.first().filter(|s| range.start <= s.source && range.end > s.source) {
                result.push(Bounds::new(stop.position, size(px(2.), stop.height)));
            }
        }
        let mut quads: Vec<_> = line.spans.iter().filter(|s| s.source.start < range.end && s.source.end > range.start)
            .map(|s| s.bounds).collect();
        quads.sort_by(|a, b| a.left().partial_cmp(&b.left()).unwrap_or(std::cmp::Ordering::Equal));
        let mut merged: Option<Bounds<Pixels>> = None;
        for quad in quads {
            if let Some(previous) = merged.as_mut() {
                if quad.left() <= previous.right() + px(0.5) {
                    previous.size.width = previous.size.width.max(quad.right() - previous.left());
                    continue;
                }
            }
            if let Some(previous) = merged.replace(quad) { result.push(previous); }
        }
        if let Some(previous) = merged { result.push(previous); }
    }
    result
}

/// One rendered, editable row. There is no independent input state in this element.
pub struct DocumentRow {
    pub editor: Entity<crate::note_editor::NoteEditor>,
    pub row: Row,
    pub index: usize,
    pub geometry: Rc<RefCell<DocumentGeometry>>,
    pub style: TextViewStyle,
    pub selection: Range<usize>,
    pub cursor: usize,
    pub marked: Option<Range<usize>>,
    pub focused: bool,
    pub caret_visible: bool,
    pub font: Font,
    pub font_size: Pixels,
    pub gutter_width: Pixels,
    pub widgets: crate::note_editor::WidgetViews,
}

#[derive(Clone, Default)]
struct MappedText {
    text: String,
    map: Vec<SourceChar>,
    styles: Vec<(Range<usize>, Marks)>,
}
impl MappedText {
    fn append(&mut self, text: &str, map: &[SourceChar], marks: Marks) {
        let offset = self.text.len();
        self.text.push_str(text);
        self.map.extend(map.iter().map(|m| SourceChar {
            display: m.display.start + offset..m.display.end + offset,
            source: m.source.clone(),
        }));
        self.styles.push((offset..self.text.len(), marks));
    }
    fn source_for(&self, display: Range<usize>) -> Option<Range<usize>> {
        let start = self.map.partition_point(|m| m.display.end <= display.start);
        let end = self.map.partition_point(|m| m.display.start < display.end);
        let entries = self.map.get(start..end)?;
        Some(entries.iter().map(|m| m.source.start).min()?..entries.iter().map(|m| m.source.end).max()?)
    }
}
#[derive(Clone)]
enum Atom { Text(MappedText), Chip(usize) }
#[derive(Clone)]
struct FlowCell { source: Range<usize>, align: AlignKind, atoms: Vec<Atom> }
struct Child {
    element: AnyElement,
    size: Size<Pixels>,
    source: Range<usize>,
}
#[derive(Clone)]
struct TextFragment {
    line: ShapedLine,
    // Relative x and exact source spans, already restricted to grapheme boundaries.
    graphemes: Vec<(Range<usize>, Pixels, Pixels)>,
    baseline_offset: Pixels,
}
#[derive(Clone)]
enum PlacedContent { Text(TextFragment), Chip(usize) }
#[derive(Clone)]
struct Placed { x: Pixels, content: PlacedContent }
#[derive(Clone)]
struct FlowLine {
    cell: usize,
    origin: Point<Pixels>,
    width: Pixels,
    height: Pixels,
    used: Pixels,
    empty_source: usize,
    items: Vec<Placed>,
}
#[derive(Clone, Default)]
struct FlowLayout {
    width: Pixels,
    height: Pixels,
    lines: Vec<FlowLine>,
    cells: Vec<Bounds<Pixels>>,
}
#[derive(Clone)]
struct Metrics {
    font: Font,
    size: Pixels,
    line_height: Pixels,
    left: Pixels,
    padding: Pixels,
    bottom: Pixels,
    style: TextViewStyle,
    code: bool,
    table: bool,
}
/// Element lifecycle state; public only because it is an Element associated type.
pub struct RowLayoutState {
    children: Vec<Child>,
    task: Option<usize>,
    annotation: Option<usize>,
    cells: Vec<FlowCell>,
    metrics: Metrics,
    layout: FlowLayout,
}
/// Element lifecycle state; all document coordinates are absolute window coordinates.
pub struct RowPrepaintState { geometry: RowGeometry, marker: Option<ShapedLine> }

impl IntoElement for DocumentRow {
    type Element = Self;
    fn into_element(self) -> Self { self }
}
impl DocumentRow {
    fn chip(&self, path: &str, label: &str, source: Range<usize>, maximum_width: Pixels) -> AnyElement {
        let editor = self.editor.clone();
        let start = source.start;
        let image: ImageSource = if path.starts_with("http://") || path.starts_with("https://") {
            SharedString::from(path.to_owned()).into()
        } else { PathBuf::from(path).into() };
        let label: SharedString = if label.is_empty() { "image.png".into() } else { label.to_owned().into() };
        // The button remains a normal Kit button; only its pointer events are isolated.
        let remove = Button::new(("document-image-remove", start)).ghost().xsmall().label("×").w(px(18.)).min_w_0().px_0()
            .on_mouse_down(MouseButton::Left, |_, window, cx| { window.prevent_default(); cx.stop_propagation(); })
            .on_mouse_up(MouseButton::Left, |_, window, cx| { window.prevent_default(); cx.stop_propagation(); })
            .on_click(move |_, window, cx| {
                window.prevent_default(); cx.stop_propagation();
                editor.update(cx, |editor, cx| editor.remove_image(source.clone(), window, cx));
            });
        HoverCard::new(("document-image", start)).open_delay(Duration::from_millis(180))
            .close_delay(Duration::from_millis(100))
            .trigger(div().flex().items_center().gap(px(3.)).px(px(5.)).h(px(24.)).max_w(maximum_width).rounded_md()
                .bg(self.style.code_background()).text_color(self.style.foreground()).text_size(px(12.))
                .child(div().min_w_0().flex_shrink(1.).truncate().child(label.clone())).child(remove))
            .content(move |_, _, _| div().flex().flex_col().gap(px(6.)).w(px(280.)).p(px(8.))
                .child(img(image.clone()).w_full().h(px(180.)).object_fit(ObjectFit::Contain))
                .child(div().text_size(px(12.)).child(label.clone())))
            .into_any_element()
    }
    fn make_state(&self, window: &Window) -> RowLayoutState {
        let (scale, line_height) = crate::theme::markdown_metrics(self.row.heading);
        let font_size = if self.row.heading > 0 {
            self.style.heading(self.row.heading).text.font_size
                .map_or(self.font_size * scale, |size| size.to_pixels(window.rem_size()))
        } else { self.font_size };
        let padding = if self.row.table { px(8.) } else if self.row.code { px(6.) } else { px(2.) };
        let mut row_style = self.style.clone();
        if self.row.heading > 0 {
            if let Some(color) = self.style.heading(self.row.heading).text.color { row_style = row_style.with_foreground(color); }
        } else if self.row.code {
            if let Some(color) = self.style.code_block().text.color { row_style = row_style.with_foreground(color); }
        } else if self.row.quote > 0 { row_style = row_style.with_foreground(self.style.muted_foreground()); }
        let mut row_font = self.font.clone();
        if self.row.heading > 0 {
            row_font.weight = self.style.heading(self.row.heading).text.font_weight.unwrap_or(FontWeight::SEMIBOLD);
        }
        let metrics = Metrics {
            font: row_font, size: font_size, line_height: font_size * line_height,
            left: self.gutter_width + px(self.row.content_inset()),
            padding, bottom: if self.row.table || self.row.code || self.row.gap { px(0.) }
                else { self.style.paragraph_gap().to_pixels(window.rem_size()) },
            style: row_style, code: self.row.code, table: self.row.table,
        };
        let mut state = RowLayoutState { children: Vec::new(), task: None, annotation: None, cells: Vec::new(), metrics, layout: FlowLayout::default() };
        for cell in &self.row.cells {
            let mut atoms = Vec::new();
            let mut text = MappedText::default();
            for inline in &cell.inline {
                match inline {
                    Inline::Text { text: value, map, marks } => {
                        if marks.script != 0 {
                            if !text.text.is_empty() { atoms.push(Atom::Text(std::mem::take(&mut text))); }
                            let mut script = MappedText::default(); script.append(value, map, *marks); atoms.push(Atom::Text(script));
                        } else { text.append(value, map, *marks); }
                    }
                    Inline::Image { path, label, source } => {
                        if !text.text.is_empty() { atoms.push(Atom::Text(std::mem::take(&mut text))); }
                        let index = state.children.len();
                        let columns = if self.row.table { self.row.cells.len().max(1) } else { 1 };
                        let maximum_width = ((window.viewport_size().width - px(24.) - state.metrics.left) / columns as f32 - state.metrics.padding * 2.).max(px(28.));
                        state.children.push(Child { element: self.chip(path, label, source.clone(), maximum_width), size: size(px(0.), px(0.)), source: source.clone() });
                        atoms.push(Atom::Chip(index));
                    }
                    Inline::Widget { kind, source } => {
                        if !text.text.is_empty() { atoms.push(Atom::Text(std::mem::take(&mut text))); }
                        let index = state.children.len();
                        let maximum_width = (window.viewport_size().width - px(24.) - state.metrics.left - state.metrics.padding * 2.).max(px(28.));
                        state.children.push(Child { element: self.widget(kind, source.clone(), maximum_width), size: size(px(0.), px(0.)), source: source.clone() });
                        atoms.push(Atom::Chip(index));
                    }
                }
            }
            if !text.text.is_empty() { atoms.push(Atom::Text(text)); }
            state.cells.push(FlowCell { source: cell.source.clone(), align: cell.align, atoms });
        }
        if state.cells.is_empty() {
            state.cells.push(FlowCell { source: self.row.source.clone(), align: AlignKind::None, atoms: Vec::new() });
        }
        if let Some((checked, range)) = &self.row.task {
            let editor = self.editor.clone(); let range = range.clone(); let checked = *checked;
            let element = div().id(("document-task", range.start)).w(px(18.)).h(px(18.)).border_1()
                .border_color(self.style.border()).rounded_sm().text_size(px(13.)).text_color(self.style.foreground())
                .child(if checked { "✓" } else { "" })
                .on_mouse_down(MouseButton::Left, |_, window, cx| { window.prevent_default(); cx.stop_propagation(); })
                .on_mouse_up(MouseButton::Left, |_, window, cx| { window.prevent_default(); cx.stop_propagation(); })
                .on_click(move |_, window, cx| {
                    window.prevent_default(); cx.stop_propagation();
                    editor.update(cx, |editor, cx| editor.toggle_task(range.clone(), checked, window, cx));
                }).into_any_element();
            state.task = Some(state.children.len());
            state.children.push(Child { element, source: 0..0, size: size(px(18.), px(18.)) });
        }
        if let Some(label) = &self.row.annotation {
            state.annotation = Some(state.children.len());
            state.children.push(Child { element: self.annotation(label, window.viewport_size().width - state.metrics.left),
                source: 0..0, size: size(px(0.), px(0.)) });
        }
        state
    }
}

impl Element for DocumentRow {
    type RequestLayoutState = Rc<RefCell<RowLayoutState>>;
    type PrepaintState = RowPrepaintState;
    fn id(&self) -> Option<ElementId> { Some(("document-row", self.index).into()) }
    fn source_location(&self) -> Option<&'static core::panic::Location<'static>> { None }
    fn request_layout(&mut self, _: Option<&GlobalElementId>, _: Option<&InspectorElementId>, window: &mut Window, cx: &mut App)
        -> (LayoutId, Self::RequestLayoutState) {
        let mut initial = self.make_state(window);
        // Measure children before the parent measure callback. GPUI temporarily takes
        // its layout engine while invoking that callback; nested child layout there
        // would attempt to borrow an absent engine. Chips use their intrinsic size.
        for child in &mut initial.children {
            child.size = child.element.layout_as_root(size(AvailableSpace::MaxContent, AvailableSpace::MaxContent), window, cx);
        }
        let state = Rc::new(RefCell::new(initial));
        let measured = state.clone();
        let mut style = Style::default();
        style.size.width = relative(1.).into();
        style.flex_shrink = 0.;
        let id = window.request_measured_layout(style, move |known, available, window, _cx| {
            let width = known.width.unwrap_or_else(|| match available.width {
                AvailableSpace::Definite(width) => width,
                AvailableSpace::MinContent => px(1.),
                AvailableSpace::MaxContent => window.viewport_size().width,
            }).max(px(1.));
            let mut state = measured.borrow_mut();
            layout_row(&mut state, width, window);
            size(width, known.height.unwrap_or(state.layout.height))
        });
        (id, state)
    }
    fn prepaint(&mut self, _: Option<&GlobalElementId>, _: Option<&InspectorElementId>, bounds: Bounds<Pixels>, state: &mut Self::RequestLayoutState,
        window: &mut Window, cx: &mut App) -> Self::PrepaintState {
        let mut state = state.borrow_mut();
        if state.layout.width != bounds.size.width { layout_row(&mut state, bounds.size.width, window); }
        let mut geometry = row_geometry(&state, &self.row, bounds.origin);
        let mut chip_origins = Vec::new();
        for line in &state.layout.lines {
            for item in &line.items {
                if let PlacedContent::Chip(index) = &item.content {
                    let child = &state.children[*index];
                    chip_origins.push((*index, bounds.origin + line.origin + point(item.x, (line.height - child.size.height) / 2.)));
                }
            }
        }
        for (index, origin) in chip_origins { state.children[index].element.prepaint_at(origin, window, cx); }
        if let Some(index) = state.task {
            let origin = bounds.origin + point(state.metrics.left - px(23.), state.metrics.padding + (state.metrics.line_height - px(18.)) / 2.);
            state.children[index].element.prepaint_at(origin, window, cx);
        }
        if let Some(index) = state.annotation {
            let origin = bounds.origin + point(state.metrics.left, state.metrics.padding);
            state.children[index].element.prepaint_at(origin, window, cx);
        }
        // Row edges represent concealed block syntax; ordinary navigation uses mapped graphemes.
        add_row_edges(&mut geometry, &self.row.source);
        self.geometry.borrow_mut().rows.insert(self.index, geometry.clone());
        let marker = if self.row.task.is_none() {
            self.row.marker.as_ref().map(|text| shape_plain(text, &state.metrics, window))
        } else { None };
        RowPrepaintState { geometry, marker }
    }
    fn paint(&mut self, _: Option<&GlobalElementId>, _: Option<&InspectorElementId>, bounds: Bounds<Pixels>, state: &mut Self::RequestLayoutState,
        prepaint: &mut Self::PrepaintState, window: &mut Window, cx: &mut App) {
        let mut state = state.borrow_mut();
        let content_height = state.layout.height - state.metrics.bottom;
        if let Some(kind) = &self.row.callout {
            let color = widget_layout::callout_color(kind);
            window.paint_quad(fill(Bounds::new(bounds.origin + point(self.gutter_width, px(0.)),
                size((bounds.size.width - self.gutter_width).max(px(0.)), content_height)), color.opacity(0.08)));
            window.paint_quad(fill(Bounds::new(bounds.origin + point(self.gutter_width, px(0.)), size(px(3.), content_height)), color));
        }
        if self.row.code || self.row.table_head {
            window.paint_quad(fill(Bounds::new(bounds.origin + point(self.gutter_width, px(0.)),
                size((bounds.size.width - self.gutter_width).max(px(0.)), content_height)), self.style.code_background()));
        }
        for quote in 0..self.row.quote {
            window.paint_quad(fill(Bounds::new(bounds.origin + point(self.gutter_width + px(quote as f32 * 14. + 3.), px(0.)), size(px(3.), content_height)), self.style.border()));
        }
        if self.row.rule {
            window.paint_quad(fill(Bounds::new(bounds.origin + point(state.metrics.left, content_height / 2.), size((bounds.size.width - state.metrics.left).max(px(1.)), px(1.))), self.style.border()));
        }
        if self.row.table {
            for cell in &state.layout.cells { paint_border(Bounds::new(bounds.origin + cell.origin, cell.size), self.style.border(), window); }
        }
        // Text backgrounds are below selection, not over it.
        for line in &state.layout.lines {
            for item in &line.items {
                if let PlacedContent::Text(fragment) = &item.content {
                    let _ = fragment.line.paint_background(bounds.origin + line.origin + point(item.x, fragment.baseline_offset), line.height, TextAlign::Left, None, window, cx);
                }
            }
        }
        for quad in selection_quads(&prepaint.geometry, &self.selection) { window.paint_quad(fill(quad, self.style.selection())); }
        for line in &state.layout.lines {
            for item in &line.items {
                if let PlacedContent::Text(fragment) = &item.content {
                    let _ = fragment.line.paint(bounds.origin + line.origin + point(item.x, fragment.baseline_offset), line.height, TextAlign::Left, None, window, cx);
                }
            }
        }
        if let Some(marked) = &self.marked {
            for quad in selection_quads(&prepaint.geometry, marked) {
                window.paint_quad(fill(Bounds::new(point(quad.left(), quad.bottom() - px(2.)), size(quad.size.width, px(1.))), self.style.foreground()));
            }
        }
        if let Some(marker) = &prepaint.marker {
            let _ = marker.paint(bounds.origin + point(state.metrics.left - px(23.), state.metrics.padding), state.metrics.line_height, TextAlign::Left, None, window, cx);
        }
        for child in &mut state.children { child.element.paint(window, cx); }
        // A chip's own surface otherwise covers the selection wash underneath it.
        for line in &state.layout.lines {
            for item in &line.items {
                if let PlacedContent::Chip(index) = &item.content {
                    let child = &state.children[*index];
                    if child.source.start < self.selection.end && child.source.end > self.selection.start {
                        window.paint_quad(fill(Bounds::new(bounds.origin + line.origin + point(item.x, px(0.)), size(child.size.width, line.height)), self.style.selection()));
                    }
                }
            }
        }
        if self.focused && self.caret_visible && self.selection.is_empty() {
            if let Some(caret) = self.geometry.borrow().caret(self.cursor) {
                if prepaint.geometry.lines.iter().any(|l| l.stops.iter().any(|s| s.position == caret.position && s.source == caret.source)) {
                    window.paint_quad(fill(Bounds::new(caret.position, size(px(1.5), caret.height)), self.style.foreground()));
                }
            }
        }
    }
}

fn make_run(len: usize, marks: Marks, metrics: &Metrics) -> TextRun {
    let mut font = metrics.font.clone();
    if marks.bold { font.weight = FontWeight::BOLD; }
    if marks.italic { font.style = FontStyle::Italic; }
    if marks.code || metrics.code { font.family = metrics.font.family.clone(); }
    let color = if marks.tone > 0 { match marks.tone {
        1 => rgb(0x7b8b8f).into(), 2 => rgb(0xb887d8).into(), 3 => rgb(0x86b982).into(),
        4 => rgb(0xd4a571).into(), 5 => rgb(0x6baecb).into(), 6 => rgb(0xd9bf76).into(),
        7 => rgb(0x91a5bc).into(), _ => metrics.style.foreground(),
    } } else if marks.syntax { metrics.style.muted_foreground() }
        else if marks.link { metrics.style.link() }
        else if marks.code { metrics.style.inline_code().color.unwrap_or(metrics.style.foreground()) }
        else { metrics.style.foreground() };
    TextRun { len, font, color,
        background_color: if marks.highlight { Some(rgb(0xd8b72b).opacity(0.25).into()) }
            else if marks.code && !metrics.code { Some(metrics.style.inline_code().background_color.unwrap_or(metrics.style.code_background())) } else { None },
        underline: marks.link.then_some(UnderlineStyle { color: Some(color), thickness: px(1.), wavy: false }),
        strikethrough: marks.strike.then_some(StrikethroughStyle { color: Some(color), thickness: px(1.) }),
    }
}
fn shape_plain(text: &str, metrics: &Metrics, window: &Window) -> ShapedLine {
    window.text_system().shape_line(text.to_owned().into(), metrics.size, &[make_run(text.len(), Marks::default(), metrics)], None)
}
fn shape_fragment(text: &MappedText, range: Range<usize>, metrics: &Metrics, window: &Window) -> TextFragment {
    let runs: Vec<_> = text.styles.iter().filter_map(|(span, marks)| {
        let start = span.start.max(range.start); let end = span.end.min(range.end);
        (start < end).then(|| make_run(end - start, *marks, metrics))
    }).collect();
    let value = &text.text[range.clone()];
    let script = text.styles.iter().find(|(span, _)| span.start <= range.start && span.end > range.start).map_or(0, |(_, marks)| marks.script);
    let line = window.text_system().shape_line(value.to_owned().into(), if script == 0 { metrics.size } else { metrics.size * 0.75 }, &runs, None);
    let boundaries: Vec<_> = value.grapheme_indices(true).map(|(index, _)| index).chain(std::iter::once(value.len())).collect();
    // Interpolate only *inside* a shaped glyph cluster (e.g. ffi ligatures).
    // Cluster endpoints come from the very same shaped line used for painting.
    let mut clusters: Vec<_> = line.runs.iter().flat_map(|run| run.glyphs.iter().map(|glyph| glyph.index))
        .chain(std::iter::once(value.len())).collect();
    clusters.sort_unstable(); clusters.dedup();
    let positions: Vec<_> = boundaries.iter().map(|&boundary| {
        let next = clusters.partition_point(|&index| index <= boundary);
        let start = next.checked_sub(1).map_or(0, |index| clusters[index]);
        let end = clusters.get(next).copied().unwrap_or(value.len());
        if start == boundary || end <= start { return line.x_for_index(boundary); }
        let first = boundaries.partition_point(|&index| index < start);
        let last = boundaries.partition_point(|&index| index < end);
        let current = boundaries.partition_point(|&index| index < boundary);
        let fraction = (current - first) as f32 / (last - first).max(1) as f32;
        line.x_for_index(start) + (line.x_for_index(end) - line.x_for_index(start)) * fraction
    }).collect();
    let mut graphemes: Vec<(Range<usize>, Pixels, Pixels)> = Vec::new();
    for (index, pair) in boundaries.windows(2).enumerate() {
        if let Some(source) = text.source_for(range.start + pair[0]..range.start + pair[1]) {
            if let Some(previous) = graphemes.last_mut().filter(|previous| previous.0 == source) {
                previous.2 = positions[index + 1];
            } else { graphemes.push((source, positions[index], positions[index + 1])); }
        }
    }
    TextFragment { line, graphemes, baseline_offset: if script > 0 { -metrics.size * 0.22 } else if script < 0 { metrics.size * 0.22 } else { px(0.) } }
}

fn new_line(cell: usize, x: Pixels, y: Pixels, width: Pixels, metrics: &Metrics, source: usize) -> FlowLine {
    FlowLine { cell, origin: point(x, y), width, height: metrics.line_height, used: px(0.), empty_source: source, items: Vec::new() }
}
fn finish_line(mut line: FlowLine, align: AlignKind, lines: &mut Vec<FlowLine>) -> Pixels {
    let slack = (line.width - line.used).max(px(0.));
    let offset = match align { AlignKind::Center => slack / 2., AlignKind::Right => slack, _ => px(0.) };
    for item in &mut line.items { item.x += offset; }
    // Empty cells also respect their column alignment.
    if line.items.is_empty() { line.origin.x += offset; line.width -= offset; }
    let next_y = line.origin.y + line.height;
    lines.push(line); next_y
}
fn layout_row(state: &mut RowLayoutState, width: Pixels, window: &Window) {
    let metrics = &state.metrics;
    let columns = if metrics.table { state.cells.len().max(1) } else { 1 };
    let column_width = ((width - metrics.left).max(px(1.))) / columns as f32;
    let inner_width = (column_width - metrics.padding * 2.).max(px(1.));
    let mut layout = FlowLayout { width, ..Default::default() };
    let mut maximum_bottom = metrics.padding + metrics.line_height;
    for (cell_index, cell) in state.cells.iter().enumerate() {
        let cell_x = metrics.left + column_width * if metrics.table { cell_index as f32 } else { 0. };
        let x = cell_x + metrics.padding;
        let content_y = metrics.padding + state.annotation.map_or(px(0.), |index| state.children[index].size.height + px(4.));
        let mut line = new_line(cell_index, x, content_y, inner_width, metrics, cell.source.start);
        let mut next_source = cell.source.start;
        for atom in &cell.atoms {
            match atom {
                Atom::Chip(index) => {
                    let child = &state.children[*index];
                    if line.used > px(0.) && line.used + child.size.width > inner_width {
                        let y = finish_line(line, cell.align, &mut layout.lines);
                        line = new_line(cell_index, x, y, inner_width, metrics, child.source.start);
                    }
                    line.items.push(Placed { x: line.used, content: PlacedContent::Chip(*index) });
                    line.used += child.size.width; line.height = line.height.max(child.size.height);
                    next_source = child.source.end;
                }
                Atom::Text(text) => {
                    let mut offset = 0;
                    while offset < text.text.len() {
                        if text.text[offset..].starts_with('\n') {
                            let source = text.source_for(offset..offset + 1).unwrap_or(next_source..next_source);
                            let fragment = TextFragment { line: shape_plain("", metrics, window), graphemes: vec![(source.clone(), px(0.), px(2.))], baseline_offset: px(0.) };
                            line.items.push(Placed { x: line.used, content: PlacedContent::Text(fragment) });
                            next_source = source.end;
                            let y = finish_line(line, cell.align, &mut layout.lines);
                            line = new_line(cell_index, x, y, inner_width, metrics, next_source);
                            offset += 1; continue;
                        }
                        let logical_end = text.text[offset..].find('\n').map_or(text.text.len(), |end| offset + end);
                        let remaining = (inner_width - line.used).max(px(0.));
                        let shaped = shape_fragment(text, offset..logical_end, metrics, window);
                        let mut end = logical_end;
                        if shaped.line.width() > remaining {
                            let boundaries: Vec<_> = text.text[offset..logical_end].grapheme_indices(true)
                                .map(|(index, grapheme)| offset + index + grapheme.len()).collect();
                            let fit = boundaries.iter().copied().take_while(|&boundary| shaped.line.x_for_index(boundary - offset) <= remaining).last();
                            if fit.is_none() && line.used > px(0.) {
                                let y = finish_line(line, cell.align, &mut layout.lines);
                                line = new_line(cell_index, x, y, inner_width, metrics, next_source);
                                continue;
                            }
                            end = fit.unwrap_or(boundaries[0]);
                            // Prefer a word boundary, falling back to an exact grapheme boundary
                            // for CJK and words wider than an entire cell.
                            if let Some(word_end) = text.text[offset..logical_end].split_word_bound_indices()
                                .map(|(index, word)| offset + index + word.len()).filter(|&boundary| boundary <= end).last() {
                                end = word_end;
                            } else if line.used > px(0.) {
                                let y = finish_line(line, cell.align, &mut layout.lines);
                                line = new_line(cell_index, x, y, inner_width, metrics, next_source);
                                continue;
                            }
                        }
                        let fragment = if end == logical_end { shaped } else { shape_fragment(text, offset..end, metrics, window) };
                        // Re-shaping a split can alter kerning. Reduce to a preceding grapheme
                        // until the *painted* shaped result fits, except for a single wide glyph.
                        let mut fragment = fragment;
                        while fragment.line.width() > remaining {
                            let previous = text.text[offset..end].grapheme_indices(true).last().map(|(index, _)| offset + index).unwrap_or(offset);
                            if previous == offset { break; }
                            end = previous; fragment = shape_fragment(text, offset..end, metrics, window);
                        }
                        if let Some((source, _, _)) = fragment.graphemes.last() { next_source = source.end; }
                        let advance = fragment.line.width();
                        line.items.push(Placed { x: line.used, content: PlacedContent::Text(fragment) });
                        line.used += advance; offset = end;
                        if offset < logical_end {
                            let y = finish_line(line, cell.align, &mut layout.lines);
                            line = new_line(cell_index, x, y, inner_width, metrics, next_source);
                        }
                    }
                }
            }
        }
        let bottom = finish_line(line, cell.align, &mut layout.lines) + metrics.padding;
        maximum_bottom = maximum_bottom.max(bottom);
        layout.cells.push(Bounds::new(point(cell_x, px(0.)), size(column_width, bottom)));
    }
    for cell in &mut layout.cells { cell.size.height = maximum_bottom; }
    layout.height = maximum_bottom + metrics.bottom;
    state.layout = layout;
}
fn row_geometry(state: &RowLayoutState, row: &Row, origin: Point<Pixels>) -> RowGeometry {
    let mut result = RowGeometry {
        source: row.source.clone(), bounds: Bounds::new(origin, size(state.layout.width, state.layout.height)),
        cells: state.layout.cells.iter().map(|cell| Bounds::new(origin + cell.origin, cell.size)).collect(),
        lines: Vec::new(),
    };
    for line in &state.layout.lines {
        let position = origin + line.origin;
        let mut geometry = VisualGeometry { bounds: Bounds::new(position, size(line.width, line.height)), cell: line.cell, stops: Vec::new(), spans: Vec::new() };
        for item in &line.items {
            let x = position.x + item.x;
            match &item.content {
                PlacedContent::Text(fragment) => {
                    let text_y = position.y + fragment.baseline_offset;
                    for (source, left, right) in &fragment.graphemes {
                        geometry.stops.push(CaretStop { source: source.start, position: point(x + *left, text_y), height: line.height });
                        geometry.stops.push(CaretStop { source: source.end, position: point(x + *right, text_y), height: line.height });
                        geometry.spans.push(SourceQuad { source: source.clone(), bounds: Bounds::new(point(x + (*left).min(*right), text_y), size((*right - *left).abs().max(px(1.)), line.height)) });
                    }
                }
                PlacedContent::Chip(index) => {
                    let child = &state.children[*index];
                    geometry.stops.push(CaretStop { source: child.source.start, position: point(x, position.y), height: line.height });
                    geometry.stops.push(CaretStop { source: child.source.end, position: point(x + child.size.width, position.y), height: line.height });
                    geometry.spans.push(SourceQuad { source: child.source.clone(), bounds: Bounds::new(point(x, position.y), size(child.size.width, line.height)) });
                }
            }
        }
        if geometry.stops.is_empty() { geometry.stops.push(CaretStop { source: line.empty_source, position, height: line.height }); }
        result.lines.push(geometry);
    }
    for (index, cell) in state.cells.iter().enumerate() {
        let first = state.layout.lines.iter().position(|l| l.cell == index);
        let last = state.layout.lines.iter().rposition(|l| l.cell == index);
        if let (Some(first), Some(last)) = (first, last) {
            if !result.lines[first].stops.iter().any(|s| s.source == cell.source.start) {
                let mut start = result.lines[first].stops[0]; start.source = cell.source.start;
                result.lines[first].stops.insert(0, start);
            }
            if !result.lines[last].stops.iter().any(|s| s.source == cell.source.end) {
                let mut end = *result.lines[last].stops.last().unwrap(); end.source = cell.source.end;
                result.lines[last].stops.push(end);
            }
        }
    }
    result
}
fn add_row_edges(geometry: &mut RowGeometry, source: &Range<usize>) {
    if let Some(line) = geometry.lines.first_mut() {
        if !line.stops.iter().any(|s| s.source == source.start) {
            if let Some(mut stop) = line.stops.first().copied() { stop.source = source.start; line.stops.insert(0, stop); }
        }
    }
    if let Some(line) = geometry.lines.last_mut() {
        if !line.stops.iter().any(|s| s.source == source.end) {
            if let Some(mut stop) = line.stops.last().copied() { stop.source = source.end; line.stops.push(stop); }
        }
    }
}
fn paint_border(bounds: Bounds<Pixels>, color: Hsla, window: &mut Window) {
    for quad in [
        Bounds::new(bounds.origin, size(bounds.size.width, px(1.))),
        Bounds::new(point(bounds.left(), bounds.bottom() - px(1.)), size(bounds.size.width, px(1.))),
        Bounds::new(bounds.origin, size(px(1.), bounds.size.height)),
        Bounds::new(point(bounds.right() - px(1.), bounds.top()), size(px(1.), bounds.size.height)),
    ] { window.paint_quad(fill(quad, color)); }
}

#[cfg(test)]
mod mapping_hit_tests {
    use super::*;
    #[test]
    fn rendered_atom_chooses_nearest_outer_boundary() {
        let stops = [CaretStop { source: 10, position: point(px(0.), px(0.)), height: px(20.) },
            CaretStop { source: 30, position: point(px(60.), px(0.)), height: px(20.) }];
        assert_eq!(closest_x(&stops, px(10.)).unwrap().source, 10);
        assert_eq!(closest_x(&stops, px(50.)).unwrap().source, 30);
    }
    #[test]
    fn hidden_aliases_have_deterministic_left_and_right_hits() {
        let stops = [CaretStop { source: 0, position: point(px(0.), px(0.)), height: px(20.) },
            CaretStop { source: 2, position: point(px(0.), px(0.)), height: px(20.) }];
        assert_eq!(closest_x(&stops, px(-1.)).unwrap().source, 0);
        assert_eq!(closest_x(&stops, px(1.)).unwrap().source, 2);
    }
}
