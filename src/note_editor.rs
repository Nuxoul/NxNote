//! One native editable Markdown document. Paint, pointer hits, keyboard navigation
//! and IME bounds share the same document geometry; there are no paragraph inputs.
use crate::{
    attachments::{is_image_file, Attachments},
    document_layout::{DocumentGeometry, DocumentRow},
    markdown_projection::Document,
};
use anyhow::Context as _;
use gpui_kit::base::input as input_action;
use gpui_kit::component::{
    button::{Button, ButtonVariants},
    input::{Input, InputEvent, InputState},
    ActiveTheme, IconName, Sizable, StyledExt,
};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::*;
use std::{cell::RefCell, ops::Range, path::PathBuf, rc::Rc, time::{Duration, Instant}};
use unicode_segmentation::UnicodeSegmentation;

pub struct CursorPosition { pub line: usize, pub character: usize }
#[derive(Clone)]
struct Snapshot { text: SharedString, selection: Range<usize>, reversed: bool }

pub struct NoteEditor {
    focus: FocusHandle,
    search: Entity<InputState>,
    search_open: bool,
    text: SharedString,
    selection: Range<usize>,
    reversed: bool,
    marked: Option<Range<usize>>,
    composition_before: Option<Snapshot>,
    document: Rc<Document>,
    geometry: Rc<RefCell<DocumentGeometry>>,
    scroll: ScrollHandle,
    style: gpui_kit::base::TextViewStyle,
    undo: Vec<Snapshot>,
    redo: Vec<Snapshot>,
    last_edit: Option<(Instant, usize)>,
    selecting: bool,
    drag_anchor: usize,
    pointer: Option<Point<Pixels>>,
    drag_task: Option<Task<()>>,
    preferred_x: Option<Pixels>,
    reveal_cursor: bool,
    caret_visible: bool,
    blink_task: Option<Task<()>>,
    attachments: Attachments,
    attachment_error: Option<String>,
    _subscriptions: Vec<Subscription>,
    paste_task: Option<Task<()>>,
    paste_pending: bool,
    revision: u64,
}
impl EventEmitter<InputEvent> for NoteEditor {}
impl Focusable for NoteEditor {
    fn focus_handle(&self, _: &App) -> FocusHandle { self.focus.clone() }
}
impl NoteEditor {
    pub fn new(text: &str, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let (attachments, attachment_error) = match Attachments::load() {
            Ok(attachments) => (attachments, None),
            Err(error) => (Attachments::default(), Some(error.to_string())),
        };
        let focus = cx.focus_handle();
        let search = cx.new(|cx| InputState::new(window, cx).placeholder("查找全文…"));
        let search_events = cx.subscribe_in(&search, window, |this, _, event: &InputEvent, window, cx| {
            if let InputEvent::PressEnter { shift, .. } = event { this.find(*shift, window, cx); }
        });
        let focus_event = cx.on_focus(&focus, window, |this, window, cx| {
            this.restart_blink(window, cx); cx.emit(InputEvent::Focus); cx.notify();
        });
        let blur_event = cx.on_blur(&focus, window, |this, _, cx| {
            let composing = this.composition_before.is_some();
            this.blink_task = None; this.stop_drag(); this.commit_composition(); this.rebuild();
            if composing { cx.emit(InputEvent::Change); }
            cx.emit(InputEvent::Blur); cx.notify();
        });
        Self {
            focus, search, search_open: false, text: text.to_string().into(), selection: 0..0,
            reversed: false, marked: None, composition_before: None,
            document: Rc::new(Document::new(text, 0, false, &attachments)),
            geometry: Rc::new(RefCell::new(DocumentGeometry::default())), scroll: ScrollHandle::new(),
            style: gpui_kit::base::TextViewStyle::from_theme(&gpui_kit::base::Theme::global(cx)),
            undo: Vec::new(), redo: Vec::new(), last_edit: None,
            selecting: false, drag_anchor: 0, pointer: None, drag_task: None,
            preferred_x: None, reveal_cursor: false, caret_visible: true, blink_task: None,
            attachments, attachment_error, _subscriptions: vec![search_events, focus_event, blur_event],
            paste_task: None, paste_pending: false, revision: 0,
        }
    }
    pub fn set_style(&mut self, style: gpui_kit::base::TextViewStyle) { self.style = style; }
    pub fn value(&self) -> SharedString {
        self.composition_before.as_ref().map_or_else(|| self.text.clone(), |before| before.text.clone())
    }
    pub fn selected_range(&self) -> Range<usize> { self.selection.clone() }
    pub fn cursor_position(&self) -> CursorPosition {
        let before = &self.text[..self.cursor()];
        CursorPosition { line: before.bytes().filter(|b| *b == b'\n').count(),
            character: before.rsplit('\n').next().unwrap_or("").chars().count() }
    }
    pub fn focus(&self, window: &mut Window, cx: &mut Context<Self>) { self.focus.focus(window, cx); }
    fn cursor(&self) -> usize { if self.reversed { self.selection.start } else { self.selection.end } }
    fn anchor(&self) -> usize { if self.reversed { self.selection.end } else { self.selection.start } }
    fn snapshot(&self) -> Snapshot { Snapshot { text: self.text.clone(), selection: self.selection.clone(), reversed: self.reversed } }
    fn rebuild(&mut self) {
        self.document = Rc::new(Document::new(&self.text, self.cursor(), self.marked.is_some(), &self.attachments));
    }
    pub fn set_selected_range(&mut self, range: Range<usize>, cx: &mut Context<Self>) {
        let start = boundary(&self.text, range.start);
        let end = boundary(&self.text, range.end).max(start);
        self.selection = start..end; self.reversed = false; self.last_edit = None;
        self.preferred_x = None; self.reveal_cursor = true;
        if !self.selecting { self.rebuild(); }
        cx.notify();
    }
    fn select_to(&mut self, offset: usize, extend: bool, window: &mut Window, cx: &mut Context<Self>) {
        let offset = boundary(&self.text, offset);
        let anchor = if extend { self.anchor() } else { offset };
        self.selection = anchor.min(offset)..anchor.max(offset);
        self.reversed = offset < anchor;
        self.last_edit = None; self.reveal_cursor = !self.selecting;
        if !self.selecting { self.rebuild(); }
        self.restart_blink(window, cx); cx.notify();
    }
    pub fn set_value(&mut self, text: String, _: &mut Window, cx: &mut Context<Self>) {
        self.text = text.into(); self.selection = 0..0; self.reversed = false;
        self.marked = None; self.composition_before = None; self.undo.clear(); self.redo.clear();
        self.last_edit = None; self.preferred_x = None; self.stop_drag(); self.revision += 1;
        self.scroll.set_offset(point(px(0.), px(0.))); self.rebuild(); cx.notify();
    }
    pub fn replace(&mut self, text: impl Into<SharedString>, window: &mut Window, cx: &mut Context<Self>) {
        self.commit_composition();
        self.edit(self.selection.clone(), &text.into(), false, window, cx);
    }
    pub fn replace_all(&mut self, text: impl Into<SharedString>, window: &mut Window, cx: &mut Context<Self>) {
        self.commit_composition();
        self.edit(0..self.text.len(), &text.into(), false, window, cx);
    }
    fn remember(&mut self, coalesce: bool, range: &Range<usize>) {
        let now = Instant::now();
        let grouped = coalesce && range.is_empty() && self.redo.is_empty()
            && self.last_edit.is_some_and(|(last, end)| end == range.start && now.duration_since(last) < Duration::from_millis(700));
        if !grouped { self.undo.push(self.snapshot()); }
        if self.undo.len() > 200 { self.undo.remove(0); }
        self.redo.clear(); self.last_edit = if coalesce { Some((now, range.start)) } else { None };
    }
    fn normalize_image_range(&self, mut range: Range<usize>) -> Range<usize> {
        for image in self.document.images() {
            if range.is_empty() {
                if range.start > image.start && range.start < image.end { range = image.end..image.end; }
            } else if range.start < image.end && range.end > image.start {
                range.start = range.start.min(image.start); range.end = range.end.max(image.end);
            }
        }
        range
    }
    fn edit(&mut self, range: Range<usize>, value: &str, coalesce: bool, window: &mut Window, cx: &mut Context<Self>) {
        let start = boundary(&self.text, range.start);
        let end = boundary(&self.text, range.end).max(start);
        let range = self.normalize_image_range(start..end);
        if self.text.get(range.clone()) == Some(value) { return; }
        let empty_code = range.is_empty() && !value.is_empty() && self.document.rows.iter().any(|row| {
            row.code && row.source.is_empty() && row.source.start == range.start
        }) && self.text[range.start..].trim_start().starts_with(|c: char| c == '`' || c == '~');
        if empty_code {
            self.remember(false, &range);
            self.apply(range.clone(), &format!("{value}\n"));
            let cursor = range.start + value.len(); self.selection = cursor..cursor;
            self.changed(window, cx); return;
        }
        self.remember(coalesce, &range);
        self.apply(range.clone(), value);
        if coalesce { self.last_edit = Some((Instant::now(), range.start + value.len())); }
        self.changed(window, cx);
    }
    fn apply(&mut self, range: Range<usize>, value: &str) {
        let mut text = self.text.to_string(); text.replace_range(range.clone(), value); self.text = text.into();
        let end = range.start + value.len(); self.selection = end..end; self.reversed = false;
        self.preferred_x = None; self.revision += 1;
    }
    fn changed(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.rebuild(); self.reveal_cursor = true; self.restart_blink(window, cx);
        cx.emit(InputEvent::Change); cx.notify();
    }
    fn commit_composition(&mut self) {
        if let Some(before) = self.composition_before.take() {
            if before.text != self.text { self.undo.push(before); self.redo.clear(); }
        }
        self.marked = None; self.last_edit = None;
    }
    fn history(&mut self, redo: bool, window: &mut Window, cx: &mut Context<Self>) {
        self.commit_composition(); self.stop_drag();
        let next = if redo { self.redo.pop() } else { self.undo.pop() };
        if let Some(next) = next {
            let current = self.snapshot(); if redo { self.undo.push(current); } else { self.redo.push(current); }
            self.text = next.text; self.selection = next.selection; self.reversed = next.reversed; self.revision += 1;
            self.changed(window, cx);
        }
    }
    fn restart_blink(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.caret_visible = true;
        if !self.focus.is_focused(window) { return; }
        self.blink_task = Some(cx.spawn_in(window, async move |view, cx| {
            loop {
                cx.background_executor().timer(Duration::from_millis(530)).await;
                let keep = cx.update(|window, cx| view.upgrade().map(|view| view.update(cx, |this, cx| {
                    if !this.focus.is_focused(window) { return false; }
                    if !this.selecting && this.marked.is_none() { this.caret_visible = !this.caret_visible; cx.notify(); }
                    true
                })).unwrap_or(false)).unwrap_or(false);
                if !keep { break; }
            }
        }));
    }
    fn step(&self, backwards: bool, word: bool) -> usize {
        let cursor = self.cursor();
        let stops = self.geometry.borrow().source_steps();
        let next = if backwards { stops.iter().copied().rev().find(|s| *s < cursor) }
            else { stops.iter().copied().find(|s| *s > cursor) };
        if word {
            let target = if backwards {
                self.text[..cursor].unicode_word_indices().next_back().map_or(0, |(start, _)| start)
            } else {
                self.text[cursor..].unicode_word_indices().next().map_or(self.text.len(), |(start, word)| cursor + start + word.len())
            };
            return if backwards { stops.iter().copied().rev().find(|s| *s <= target).unwrap_or(target) }
                else { stops.iter().copied().find(|s| *s >= target).unwrap_or(target) };
        }
        next.unwrap_or_else(|| if backwards {
            self.text[..cursor].grapheme_indices(true).next_back().map_or(0, |(start, _)| start)
        } else {
            self.text[cursor..].graphemes(true).next().map_or(cursor, |g| cursor + g.len())
        })
    }
    fn horizontal(&mut self, backwards: bool, extend: bool, word: bool, window: &mut Window, cx: &mut Context<Self>) {
        if self.marked.is_some() { return; }
        let target = if !extend && !self.selection.is_empty() {
            if backwards { self.selection.start } else { self.selection.end }
        } else { self.step(backwards, word) };
        self.preferred_x = None; self.select_to(target, extend, window, cx);
    }
    fn vertical(&mut self, direction: i32, extend: bool, window: &mut Window, cx: &mut Context<Self>) {
        if self.marked.is_some() { return; }
        let cursor = self.cursor();
        let geometry = self.geometry.borrow();
        let x = self.preferred_x.or_else(|| geometry.caret(cursor).map(|c| c.position.x)).unwrap_or(px(0.));
        let target = geometry.vertical(cursor, x, direction).map(|c| c.source)
            .unwrap_or(if direction < 0 { 0 } else { self.text.len() });
        drop(geometry); self.select_to(target, extend, window, cx); self.preferred_x = Some(x);
    }
    fn edge(&mut self, end: bool, extend: bool, document: bool, window: &mut Window, cx: &mut Context<Self>) {
        if self.marked.is_some() { return; }
        let target = if document { if end { self.text.len() } else { 0 } }
            else { self.geometry.borrow().line_edge(self.cursor(), end).unwrap_or(self.cursor()) };
        self.preferred_x = None; self.select_to(target, extend, window, cx);
    }
    fn page(&mut self, direction: i32, window: &mut Window, cx: &mut Context<Self>) {
        if self.marked.is_some() { return; }
        let geometry = self.geometry.borrow();
        let Some(caret) = geometry.caret(self.cursor()) else { return; };
        let position = point(caret.position.x, caret.position.y + self.scroll.bounds().size.height * direction as f32);
        let target = geometry.hit(position).unwrap_or(self.cursor());
        drop(geometry); self.select_to(target, false, window, cx);
    }
    fn delete(&mut self, backwards: bool, word: bool, window: &mut Window, cx: &mut Context<Self>) {
        if self.marked.is_some() { return; }
        if !self.selection.is_empty() { self.edit(self.selection.clone(), "", false, window, cx); return; }
        let cursor = self.cursor();
        if let Some((row, column)) = self.document.cell_at(cursor) {
            let cell = &self.document.rows[row].cells[column];
            if (backwards && cursor == cell.source.start) || (!backwards && cursor == cell.source.end) { return; }
            if word {
                let target = self.step(backwards, true).clamp(cell.source.start, cell.source.end);
                self.edit(cursor.min(target)..cursor.max(target), "", false, window, cx); return;
            }
        }
        if let Some(row) = self.document.rows.iter().find(|row| row.code && cursor >= row.source.start && cursor <= row.source.end) {
            let edge = if backwards { row.source.start } else { row.source.end };
            if cursor == edge {
                let adjacent = if backwards { self.text[..cursor].chars().next_back() } else { self.text[cursor..].chars().next() };
                if adjacent != Some('\n') && adjacent != Some('\r') { return; }
                let target = if backwards { cursor.saturating_sub(1) } else { (cursor + 1).min(self.text.len()) };
                let other_code = self.document.rows.iter().any(|candidate| candidate.code && (candidate.source.start == target || candidate.source.end == target));
                if !other_code { self.select_to(self.step(backwards, false), false, window, cx); return; }
                self.edit(cursor.min(target)..cursor.max(target), "", false, window, cx); return;
            }
        }
        let target = self.step(backwards, word);
        if target == cursor { return; }
        let range = cursor.min(target)..cursor.max(target);
        // At a reading block's start, remove its structural prefix first, then join
        // paragraphs on the next Backspace; do not eat the last glyph of the prior block.
        if backwards && self.document.rows.iter().any(|row| row.cells.iter().any(|cell| cell.source.start == cursor)) {
            let line_start = self.text[..cursor].rfind('\n').map_or(0, |i| i + 1);
            let prefix = &self.text[line_start..cursor];
            if !prefix.is_empty() && (prefix.trim_start().starts_with('#') || prefix.trim_start().starts_with('>')
                || prefix.trim_start().starts_with(|c: char| matches!(c, '-' | '*' | '+')) || prefix.trim_start().chars().next().is_some_and(|c| c.is_ascii_digit())) {
                self.edit(line_start..cursor, "", false, window, cx); return;
            }
            if line_start == cursor && cursor > 0 {
                self.edit(cursor - 1..cursor, "", false, window, cx); return;
            }
        }
        if !backwards && self.text[cursor..].starts_with('\n') {
            self.edit(cursor..cursor + 1, "", false, window, cx); return;
        }
        let mapped = if word { None } else { self.geometry.borrow().deletion_range(cursor, backwards) };
        self.edit(mapped.unwrap_or(range), "", false, window, cx);
    }
    fn enter(&mut self, shift: bool, secondary: bool, window: &mut Window, cx: &mut Context<Self>) {
        if self.marked.is_some() { return; }
        let cursor = self.cursor();
        if self.document.cell_at(cursor).is_some() && !shift {
            self.table_move(1, true, window, cx); return;
        }
        let code = self.document.code_at(cursor);
        let line_start = self.text[..cursor].rfind('\n').map_or(0, |i| i + 1);
        let line = self.text[line_start..cursor].to_string();
        let indent = if code { line.chars().take_while(|c| *c == ' ' || *c == '\t').collect::<String>() } else { String::new() };
        self.remember(false, &self.selection.clone());
        let replacement = if self.document.cell_at(cursor).is_some() { "<br>".to_string() }
            else if shift && !code { "  \n".to_string() } else { format!("\n{indent}") };
        self.apply(self.selection.clone(), &replacement);
        if !shift && !code {
            if let Some((range, replacement)) = crate::editor_support::continuation_edit(&self.text, self.cursor()) {
                self.apply(range, &replacement);
            } else {
                let quote = line.trim_start();
                if quote.starts_with('>') {
                    let prefix = quote.chars().take_while(|c| *c == '>' || *c == ' ').collect::<String>();
                    if quote[prefix.len()..].trim().is_empty() {
                        let end = self.cursor(); self.apply(line_start..end, "");
                    } else { let cursor = self.cursor(); self.apply(cursor..cursor, &prefix); }
                }
            }
        }
        self.changed(window, cx); cx.emit(InputEvent::PressEnter { secondary, shift });
    }
    fn table_move(&mut self, direction: i32, vertical: bool, window: &mut Window, cx: &mut Context<Self>) {
        let Some((row_index, column)) = self.document.cell_at(self.cursor()) else { return; };
        let mut table_start = row_index;
        while table_start > 0 && self.document.rows[table_start - 1].table { table_start -= 1; }
        let mut table_end = row_index + 1;
        while table_end < self.document.rows.len() && self.document.rows[table_end].table { table_end += 1; }
        let columns = self.document.rows[row_index].cells.len().max(1);
        let offset = (row_index - table_start) * columns + column;
        let target = offset as i32 + direction * (if vertical { columns as i32 } else { 1 });
        if target < 0 { self.select_to(self.document.rows[table_start].source.start, false, window, cx); return; }
        let target_row = table_start + target as usize / columns;
        let target_col = target as usize % columns;
        if target_row < table_end {
            if let Some(start) = self.document.rows[target_row].cells.get(target_col).map(|cell| cell.source.start) {
                self.select_to(start, false, window, cx);
            }
        } else {
            let end = self.document.rows[table_end - 1].source.end;
            let replacement = format!("\n|{}", "  |".repeat(columns));
            self.edit(end..end, &replacement, false, window, cx);
            let start = end + 3 + target_col * 3;
            self.select_to(start.min(self.text.len()), false, window, cx);
        }
    }
    fn indent(&mut self, backwards: bool, window: &mut Window, cx: &mut Context<Self>) {
        if self.marked.is_some() { return; }
        if self.document.cell_at(self.cursor()).is_some() { self.table_move(if backwards { -1 } else { 1 }, false, window, cx); return; }
        if self.document.code_at(self.cursor()) && self.selection.is_empty() && !backwards {
            self.edit(self.selection.clone(), "    ", false, window, cx); return;
        }
        let start = self.text[..self.selection.start].rfind('\n').map_or(0, |i| i + 1);
        let end = self.text[self.selection.end..].find('\n').map_or(self.text.len(), |i| self.selection.end + i);
        let source = &self.text[start..end];
        let replacement = source.split('\n').map(|line| {
            if backwards { line.strip_prefix("    ").or_else(|| line.strip_prefix('\t')).or_else(|| line.strip_prefix("  ")).unwrap_or(line).to_string() }
            else { format!("    {line}") }
        }).collect::<Vec<_>>().join("\n");
        self.edit(start..end, &replacement, false, window, cx);
    }
    fn copy(&self, cx: &mut Context<Self>) {
        if !self.selection.is_empty() { cx.write_to_clipboard(ClipboardItem::new_string(self.text[self.selection.clone()].to_string())); }
    }
    fn cut(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.selection.is_empty() { self.copy(cx); self.edit(self.selection.clone(), "", false, window, cx); }
    }
    fn find(&mut self, backwards: bool, window: &mut Window, cx: &mut Context<Self>) {
        let query = self.search.read(cx).value(); if query.is_empty() { return; }
        let found = if backwards {
            let end = self.selection.start;
            self.text[..end].rfind(query.as_ref()).or_else(|| self.text[end..].rfind(query.as_ref()).map(|i| end + i))
        } else {
            let start = self.selection.end;
            self.text[start..].find(query.as_ref()).map(|i| start + i).or_else(|| self.text[..start].find(query.as_ref()))
        };
        if let Some(start) = found { self.set_selected_range(start..start + query.len(), cx); self.restart_blink(window, cx); }
    }
    fn mouse_down(&mut self, event: &MouseDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        if window.default_prevented() { return; }
        self.commit_composition(); self.focus.focus(window, cx);
        let hit = { self.geometry.borrow().hit(event.position) };
        let Some(offset) = hit else { return; };
        self.selecting = true; self.pointer = Some(event.position); self.preferred_x = None;
        self.drag_anchor = if event.modifiers.shift { self.anchor() } else { offset };
        if event.click_count >= 3 {
            let start = self.text[..offset].rfind('\n').map_or(0, |i| i + 1);
            let end = self.text[offset..].find('\n').map_or(self.text.len(), |i| offset + i);
            self.selection = start..end; self.reversed = false; self.drag_anchor = start;
        } else if event.click_count == 2 {
            let word = self.text.unicode_word_indices().find(|(start, word)| offset >= *start && offset < *start + word.len());
            if let Some((start, word)) = word { self.selection = start..start + word.len(); self.reversed = false; self.drag_anchor = start; }
            else { self.selection = offset..offset; self.reversed = false; }
        } else {
            let anchor = self.drag_anchor; self.selection = anchor.min(offset)..anchor.max(offset); self.reversed = offset < anchor;
        }
        self.last_edit = None; self.caret_visible = true; self.start_drag_scroll(window, cx); cx.notify();
    }
    fn mouse_move(&mut self, event: &MouseMoveEvent, _: &mut Window, cx: &mut Context<Self>) {
        if !self.selecting { return; }
        if event.pressed_button != Some(MouseButton::Left) { self.stop_drag(); self.rebuild(); cx.notify(); return; }
        self.pointer = Some(event.position);
        if let Some(offset) = self.geometry.borrow().hit(event.position) {
            let anchor = self.drag_anchor; self.selection = anchor.min(offset)..anchor.max(offset); self.reversed = offset < anchor; cx.notify();
        }
    }
    fn mouse_up(&mut self, _: &MouseUpEvent, window: &mut Window, cx: &mut Context<Self>) {
        if self.selecting { self.stop_drag(); self.rebuild(); self.restart_blink(window, cx); cx.notify(); }
    }
    fn stop_drag(&mut self) { self.selecting = false; self.pointer = None; self.drag_task = None; }
    fn start_drag_scroll(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.drag_task = Some(cx.spawn_in(window, async move |view, cx| {
            loop {
                cx.background_executor().timer(Duration::from_millis(30)).await;
                let keep = cx.update(|_, cx| view.upgrade().map(|view| view.update(cx, |this, cx| {
                    if !this.selecting { return false; }
                    let Some(pointer) = this.pointer else { return true; };
                    let bounds = this.scroll.bounds();
                    let delta = if pointer.y < bounds.top() + px(20.) { px(12.) }
                        else if pointer.y > bounds.bottom() - px(20.) { px(-12.) } else { px(0.) };
                    if delta != px(0.) {
                        let mut offset = this.scroll.offset();
                        offset.y = (offset.y + delta).min(px(0.)).max(-this.scroll.max_offset().y);
                        this.scroll.set_offset(offset);
                        if let Some(source) = this.geometry.borrow().hit(pointer) {
                            this.selection = this.drag_anchor.min(source)..this.drag_anchor.max(source); this.reversed = source < this.drag_anchor;
                        }
                        cx.notify();
                    }
                    true
                })).unwrap_or(false)).unwrap_or(false);
                if !keep { break; }
            }
        }));
    }
    pub(crate) fn remove_image(&mut self, range: Range<usize>, window: &mut Window, cx: &mut Context<Self>) {
        self.edit(range, "", false, window, cx); self.focus.focus(window, cx);
    }
    pub(crate) fn toggle_task(&mut self, range: Range<usize>, checked: bool, window: &mut Window, cx: &mut Context<Self>) {
        self.edit(range, if checked { " " } else { "x" }, false, window, cx); self.focus.focus(window, cx);
    }
    fn paste_item(&mut self, mut item: ClipboardItem, window: &mut Window, cx: &mut Context<Self>) {
        let files: Vec<PathBuf> = crate::platform::clipboard_files().into_iter().filter(|path| is_image_file(path)).collect();
        if !files.is_empty() { item.entries.push(ClipboardEntry::ExternalPaths(ExternalPaths(files.into()))); }
        if item.entries().iter().any(|entry| match entry {
            ClipboardEntry::Image(_) => true,
            ClipboardEntry::ExternalPaths(paths) => paths.paths().iter().any(|path| is_image_file(path)), _ => false,
        }) { self.paste_images(item, window, cx); }
        else if let Some(text) = item.text() {
            if crate::editor_support::image_url(&text) { self.replace(format!("![图片]({})", text.trim()), window, cx); }
            else {
                let text = if self.document.cell_at(self.cursor()).is_some() {
                    text.replace('|', "\\|").replace('\n', "<br>")
                } else { text };
                self.replace(text, window, cx);
            }
        }
    }
    fn paste_images(&mut self, item: ClipboardItem, window: &mut Window, cx: &mut Context<Self>) {
        if self.paste_pending { self.report("图片正在保存，请稍后再粘贴".into(), window, cx); return; }
        if let Some(error) = &self.attachment_error { self.report(error.clone(), window, cx); return; }
        let mut paths: Vec<PathBuf> = item.entries().iter().flat_map(|entry| match entry {
            ClipboardEntry::ExternalPaths(paths) => paths.paths().to_vec(), _ => Vec::new(),
        }).filter(|path| is_image_file(path)).collect();
        let mut seen = std::collections::HashSet::new(); paths.retain(|path| seen.insert(path.clone()));
        let image = if paths.is_empty() { item.entries().iter().find_map(|entry| match entry {
            ClipboardEntry::Image(image) => Some(image.bytes().to_vec()), _ => None,
        }) } else { None };
        let mut attachments = self.attachments.clone();
        let selection = self.selection.clone(); let revision = self.revision;
        let load = cx.background_executor().spawn(async move {
            let mut values = Vec::new();
            if let Some(bytes) = image { values.push(attachments.save_image(&bytes)?); }
            for path in paths {
                image::image_dimensions(&path).with_context(|| format!("无法读取图片 {}", path.display()))?;
                let label = path.file_name().and_then(|n| n.to_str()).unwrap_or("image.png").to_string();
                values.push(attachments.register(&path, label)?);
            }
            Ok::<_, anyhow::Error>((attachments, values))
        });
        self.paste_pending = true;
        self.paste_task = Some(cx.spawn_in(window, async move |view, cx| {
            let result = load.await;
            let _ = cx.update(|window, cx| {
                if let Some(view) = view.upgrade() { view.update(cx, |this, cx| {
                    this.paste_pending = false;
                    match result {
                        Ok((attachments, paths)) => {
                            this.attachments = attachments;
                            if this.revision != revision || this.selection != selection {
                                this.report("图片已保存，但正文或光标已变化，请重新粘贴".into(), window, cx); return;
                            }
                            this.replace(paths.join(" "), window, cx);
                        }
                        Err(error) => this.report(format!("粘贴图片失败: {error:#}"), window, cx),
                    }
                }); }
            });
        }));
    }
    fn report(&self, message: String, window: &mut Window, cx: &mut Context<Self>) {
        use gpui_kit::component::{notification::{Notification, NotificationType}, WindowExt};
        window.push_notification(Notification::new().message(message).with_type(NotificationType::Error), cx);
    }
    fn utf16(&self, byte: usize) -> usize { self.text[..boundary(&self.text, byte)].encode_utf16().count() }
    fn utf8(&self, units: usize) -> usize { byte_for_utf16(&self.text, units) }
    fn from_utf16(&self, range: Range<usize>) -> Range<usize> {
        let start = self.utf8(range.start); let end = self.utf8(range.end).max(start); start..end
    }
    fn to_utf16(&self, range: Range<usize>) -> Range<usize> { self.utf16(range.start)..self.utf16(range.end) }
}
impl EntityInputHandler for NoteEditor {
    fn text_for_range(&mut self, range: Range<usize>, adjusted: &mut Option<Range<usize>>, _: &mut Window, _: &mut Context<Self>) -> Option<String> {
        let range = self.from_utf16(range); *adjusted = Some(self.to_utf16(range.clone())); Some(self.text[range].to_string())
    }
    fn selected_text_range(&mut self, _: bool, _: &mut Window, _: &mut Context<Self>) -> Option<UTF16Selection> {
        Some(UTF16Selection { range: self.to_utf16(self.selection.clone()), reversed: self.reversed })
    }
    fn marked_text_range(&self, _: &mut Window, _: &mut Context<Self>) -> Option<Range<usize>> { self.marked.clone().map(|r| self.to_utf16(r)) }
    fn unmark_text(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let composing = self.composition_before.is_some();
        self.commit_composition(); self.rebuild(); self.restart_blink(window, cx);
        if composing { cx.emit(InputEvent::Change); }
        cx.notify();
    }
    fn replace_text_in_range(&mut self, range: Option<Range<usize>>, text: &str, window: &mut Window, cx: &mut Context<Self>) {
        let range = range.map(|r| self.from_utf16(r)).or_else(|| self.marked.clone()).unwrap_or(self.selection.clone());
        let value = if self.document.cell_at(range.start).is_some() {
            text.replace('|', "\\|").replace('\n', "<br>")
        } else { text.to_string() };
        if self.composition_before.is_some() {
            self.apply(range, &value); self.commit_composition(); self.changed(window, cx);
        } else { self.edit(range, &value, !text.contains('\n'), window, cx); }
    }
    fn replace_and_mark_text_in_range(&mut self, range: Option<Range<usize>>, text: &str, selected: Option<Range<usize>>, window: &mut Window, cx: &mut Context<Self>) {
        let range = range.map(|r| self.from_utf16(r)).or_else(|| self.marked.clone()).unwrap_or(self.selection.clone());
        if self.composition_before.is_none() { self.composition_before = Some(self.snapshot()); }
        let empty_code = range.is_empty() && !text.is_empty() && self.document.rows.iter().any(|row| {
            row.code && row.source.is_empty() && row.source.start == range.start
        }) && self.text[range.start..].trim_start().starts_with(|c: char| c == '`' || c == '~');
        let start = range.start;
        if empty_code { self.apply(range, &format!("{text}\n")); }
        else { self.apply(range, text); }
        let end = start + text.len(); self.selection = end..end;
        self.marked = (!text.is_empty()).then_some(start..end);
        if let Some(selected) = selected {
            let a = start + byte_for_utf16(text, selected.start); let b = start + byte_for_utf16(text, selected.end);
            self.selection = a..b.max(a); self.reversed = false;
        }
        self.rebuild(); self.reveal_cursor = true; self.caret_visible = true; cx.notify();
    }
    fn bounds_for_range(&mut self, range: Range<usize>, _: Bounds<Pixels>, _: &mut Window, _: &mut Context<Self>) -> Option<Bounds<Pixels>> {
        let range = self.from_utf16(range);
        let geometry = self.geometry.borrow();
        let start = geometry.caret(range.start)?; let end = geometry.caret(range.end).unwrap_or(start);
        let right = if end.position.y == start.position.y { end.position.x.max(start.position.x + px(1.)) } else { start.position.x + px(1.) };
        Some(Bounds::from_corners(start.position, point(right, start.position.y + start.height)))
    }
    fn character_index_for_point(&mut self, position: Point<Pixels>, _: &mut Window, _: &mut Context<Self>) -> Option<usize> {
        self.geometry.borrow().hit(position).map(|offset| self.utf16(offset))
    }
}
impl Render for NoteEditor {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let focused = self.focus.is_focused(window);
        let caret_visible = self.caret_visible || self.marked.is_some();
        let cursor = self.cursor();
        let geometry = self.geometry.clone();
        let font = font(cx.theme().mono_font_family.clone()); let font_size = cx.theme().mono_font_size;
        let rows = self.document.rows.iter().cloned().enumerate().map(|(index, row)| DocumentRow {
            editor: cx.entity(), row, index, geometry: geometry.clone(), style: self.style.clone(),
            selection: self.selection.clone(), cursor, marked: self.marked.clone(), focused, caret_visible,
            font: font.clone(), font_size,
        }).collect::<Vec<_>>();
        let clear = geometry.clone();
        let scroll = self.scroll.clone();
        let reveal = self.reveal_cursor; self.reveal_cursor = false;
        let input = cx.entity(); let focus = self.focus.clone();
        let document = div().id("markdown-document").key_context("MarkdownDocument").track_focus(&self.focus)
            .v_flex().size_full().min_h_0().cursor(CursorStyle::IBeam)
            .on_action(cx.listener(|this, _: &input_action::SelectAll, window, cx| { this.selection = 0..this.text.len(); this.reversed = false; this.rebuild(); this.restart_blink(window, cx); cx.notify(); }))
            .on_action(cx.listener(|this, _: &input_action::Copy, _, cx| this.copy(cx)))
            .on_action(cx.listener(|this, _: &input_action::Cut, window, cx| this.cut(window, cx)))
            .on_action(cx.listener(|this, _: &input_action::Paste, window, cx| { if let Some(item) = cx.read_from_clipboard() { this.paste_item(item, window, cx); } }))
            .on_action(cx.listener(|this, _: &input_action::Undo, window, cx| this.history(false, window, cx)))
            .on_action(cx.listener(|this, _: &input_action::Redo, window, cx| this.history(true, window, cx)))
            .on_action(cx.listener(|this, _: &input_action::Backspace, window, cx| this.delete(true, false, window, cx)))
            .on_action(cx.listener(|this, _: &input_action::Delete, window, cx| this.delete(false, false, window, cx)))
            .on_action(cx.listener(|this, _: &input_action::DeleteToPreviousWordStart, window, cx| this.delete(true, true, window, cx)))
            .on_action(cx.listener(|this, _: &input_action::DeleteToNextWordEnd, window, cx| this.delete(false, true, window, cx)))
            .on_action(cx.listener(|this, _: &input_action::MoveLeft, window, cx| this.horizontal(true, false, false, window, cx)))
            .on_action(cx.listener(|this, _: &input_action::MoveRight, window, cx| this.horizontal(false, false, false, window, cx)))
            .on_action(cx.listener(|this, _: &input_action::MoveToPreviousWord, window, cx| this.horizontal(true, false, true, window, cx)))
            .on_action(cx.listener(|this, _: &input_action::MoveToNextWord, window, cx| this.horizontal(false, false, true, window, cx)))
            .on_action(cx.listener(|this, _: &gpui_kit::base::actions::SelectLeft, window, cx| this.horizontal(true, true, false, window, cx)))
            .on_action(cx.listener(|this, _: &gpui_kit::base::actions::SelectRight, window, cx| this.horizontal(false, true, false, window, cx)))
            .on_action(cx.listener(|this, _: &input_action::SelectToPreviousWordStart, window, cx| this.horizontal(true, true, true, window, cx)))
            .on_action(cx.listener(|this, _: &input_action::SelectToNextWordEnd, window, cx| this.horizontal(false, true, true, window, cx)))
            .on_action(cx.listener(|this, _: &input_action::MoveUp, window, cx| this.vertical(-1, false, window, cx)))
            .on_action(cx.listener(|this, _: &input_action::MoveDown, window, cx| this.vertical(1, false, window, cx)))
            .on_action(cx.listener(|this, _: &input_action::MovePageUp, window, cx| this.page(-1, window, cx)))
            .on_action(cx.listener(|this, _: &input_action::MovePageDown, window, cx| this.page(1, window, cx)))
            .on_action(cx.listener(|this, _: &gpui_kit::base::actions::SelectUp, window, cx| this.vertical(-1, true, window, cx)))
            .on_action(cx.listener(|this, _: &gpui_kit::base::actions::SelectDown, window, cx| this.vertical(1, true, window, cx)))
            .on_action(cx.listener(|this, _: &input_action::MoveHome, window, cx| this.edge(false, false, false, window, cx)))
            .on_action(cx.listener(|this, _: &input_action::MoveEnd, window, cx| this.edge(true, false, false, window, cx)))
            .on_action(cx.listener(|this, _: &input_action::SelectToStartOfLine, window, cx| this.edge(false, true, false, window, cx)))
            .on_action(cx.listener(|this, _: &input_action::SelectToEndOfLine, window, cx| this.edge(true, true, false, window, cx)))
            .on_action(cx.listener(|this, _: &input_action::MoveToStart, window, cx| this.edge(false, false, true, window, cx)))
            .on_action(cx.listener(|this, _: &input_action::MoveToEnd, window, cx| this.edge(true, false, true, window, cx)))
            .on_action(cx.listener(|this, _: &input_action::SelectToStart, window, cx| this.edge(false, true, true, window, cx)))
            .on_action(cx.listener(|this, _: &input_action::SelectToEnd, window, cx| this.edge(true, true, true, window, cx)))
            .on_action(cx.listener(|this, event: &input_action::Enter, window, cx| this.enter(event.shift, event.secondary, window, cx)))
            .on_action(cx.listener(|this, _: &input_action::Indent, window, cx| this.indent(false, window, cx)))
            .on_action(cx.listener(|this, _: &input_action::Outdent, window, cx| this.indent(true, window, cx)))
            .on_action(cx.listener(|this, _: &input_action::Escape, window, cx| { this.search_open = false; this.commit_composition(); this.focus.focus(window, cx); cx.notify(); }))
            .on_mouse_down(MouseButton::Left, cx.listener(Self::mouse_down))
            .on_mouse_move(cx.listener(Self::mouse_move))
            .on_mouse_up(MouseButton::Left, cx.listener(Self::mouse_up))
            .on_mouse_up_out(MouseButton::Left, cx.listener(Self::mouse_up))
            .child(div().id("document-scroll").relative().v_flex().flex_1().min_h_0().overflow_y_scroll().track_scroll(&self.scroll).px_3().py_2()
                .child(canvas(move |_, _, _| { clear.borrow_mut().clear(); }, |_, _, _, _| {}).h(px(0.)).w_full())
                .children(rows)
                .child(div().id("document-end").h(px(80.)).w_full().on_click(cx.listener(|this, _, window, cx| {
                    let end = this.text.len(); this.select_to(end, false, window, cx);
                })))
                .child(canvas(|_, _, _| {}, move |bounds, _, window, cx| {
                    window.handle_input(&focus, ElementInputHandler::new(bounds, input.clone()), cx);
                    let move_input = input.clone();
                    window.on_mouse_event(move |event: &MouseMoveEvent, phase, window, cx| {
                        if phase == DispatchPhase::Bubble {
                            move_input.update(cx, |this, cx| { if this.selecting { this.mouse_move(event, window, cx); } });
                        }
                    });
                    let up_input = input.clone();
                    window.on_mouse_event(move |event: &MouseUpEvent, phase, window, cx| {
                        if phase == DispatchPhase::Bubble && event.button == MouseButton::Left {
                            up_input.update(cx, |this, cx| { if this.selecting { this.mouse_up(event, window, cx); } });
                        }
                    });
                    if reveal {
                        if let Some(caret) = geometry.borrow().caret(cursor) {
                            let viewport = scroll.bounds(); let mut offset = scroll.offset();
                            if caret.position.y < viewport.top() + px(8.) { offset.y += viewport.top() + px(8.) - caret.position.y; }
                            else if caret.position.y + caret.height > viewport.bottom() - px(8.) { offset.y -= caret.position.y + caret.height - viewport.bottom() + px(8.); }
                            offset.y = offset.y.min(px(0.)).max(-scroll.max_offset().y);
                            if offset != scroll.offset() { scroll.set_offset(offset); window.refresh(); }
                        }
                    }
                }).absolute().size_full()));
        let search_bar = self.search_open.then(|| div().flex().items_center().gap_1().p_2()
            .child(div().flex_1().child(Input::new(&self.search).small()))
            .child(Button::new("find-previous").ghost().xsmall().label("上一个").on_click(cx.listener(|this, _, window, cx| this.find(true, window, cx))))
            .child(Button::new("find-next").ghost().xsmall().label("下一个").on_click(cx.listener(|this, _, window, cx| this.find(false, window, cx))))
            .child(Button::new("close-find").ghost().xsmall().icon(IconName::Close).on_click(cx.listener(|this, _, window, cx| {
                this.search_open = false; this.focus.focus(window, cx); cx.notify();
            }))));
        div().v_flex().size_full().min_h_0()
            .on_action(cx.listener(|this, _: &input_action::Search, window, cx| {
                this.search_open = true; this.search.focus_handle(cx).focus(window, cx); cx.notify();
            }))
            .children(search_bar).child(div().flex_1().min_h_0().child(document))
    }
}
fn boundary(text: &str, offset: usize) -> usize {
    let mut offset = offset.min(text.len()); while !text.is_char_boundary(offset) { offset -= 1; } offset
}
fn byte_for_utf16(text: &str, units: usize) -> usize {
    let mut count = 0;
    for (offset, c) in text.char_indices() { if count >= units { return offset; } count += c.len_utf16(); }
    text.len()
}
