//! Background assets and navigation for the single Markdown input owner.
use super::*;
use crate::markdown_projection::{LinkTarget, WidgetKind};
use crate::markdown_renderers::{RenderKind, RenderedAsset};
use crate::note_links::{NoteLinkContext, NotePreview, NoteTarget};
use std::collections::BTreeMap;

#[derive(Clone)]
pub(crate) enum WidgetView {
    Pending,
    Asset(RenderedAsset),
    Preview(NotePreview),
    Error(String),
}
#[derive(Clone, Default)]
pub(crate) struct WidgetViews(pub BTreeMap<String, WidgetView>);
impl WidgetViews {
    pub fn get(&self, kind: &WidgetKind) -> WidgetView {
        self.0.get(&kind.key()).cloned().unwrap_or(WidgetView::Pending)
    }
}
#[derive(Clone)]
pub struct OpenNoteLink(pub NoteTarget);
impl EventEmitter<OpenNoteLink> for NoteEditor {}

impl NoteEditor {
    pub fn set_note_context(&mut self, context: NoteLinkContext, cx: &mut Context<Self>) {
        self.note_context = Some(context);
        self.widget_views = WidgetViews::default(); self.widget_task = None; self.widget_generation += 1;
        self.rebuild(); cx.notify();
    }
    pub(crate) fn expand_metadata(&mut self, expanded: bool, window: &mut Window, cx: &mut Context<Self>) {
        self.metadata_expanded = expanded;
        if !expanded {
            if let Some(block) = crate::markdown_extensions::Extensions::parse(&self.text).blocks.iter()
                .find(|block| matches!(&block.kind, crate::markdown_extensions::BlockKind::FrontMatter)) {
                self.selection = block.source.end..block.source.end; self.reversed = false;
            }
        }
        self.rebuild(); self.focus.focus(window, cx); cx.notify();
    }
    pub(crate) fn edit_widget(&mut self, source: Range<usize>, window: &mut Window, cx: &mut Context<Self>) {
        self.select_to(source.start, false, window, cx);
    }
    pub(crate) fn open_target(&mut self, target: LinkTarget, window: &mut Window, cx: &mut Context<Self>) {
        match target {
            LinkTarget::External(target) if target.starts_with('#') => self.jump_to_heading(&target[1..], window, cx),
            LinkTarget::External(target) => match crate::note_links::validate_external(&target) {
                Ok(target) => cx.open_url(&target),
                Err(error) => self.report(error.to_string(), window, cx),
            },
            LinkTarget::Note(target) => {
                let result = self.note_context.as_ref().ok_or_else(|| anyhow::anyhow!("笔记上下文尚未加载"))
                    .and_then(|context| context.resolve(&target));
                match result {
                    Ok(target) => { self.commit_composition(); cx.emit(OpenNoteLink(target)); }
                    Err(error) => self.report(error.to_string(), window, cx),
                }
            }
            LinkTarget::Footnote(id) => {
                if let Some(range) = self.document.footnotes.get(&id).cloned() {
                    self.footnote_return = Some(self.cursor());
                    self.select_to(range.start, false, window, cx);
                } else { self.report(format!("找不到脚注定义：{id}"), window, cx); }
            }
        }
    }
    pub(crate) fn return_from_footnote(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(source) = self.footnote_return.take() { self.select_to(source, false, window, cx); }
    }
    pub fn jump_to_heading(&mut self, heading: &str, window: &mut Window, cx: &mut Context<Self>) {
        let normalized = |text: &str| text.trim().to_lowercase().chars().filter(|ch| ch.is_alphanumeric() || *ch == '-' || ch.is_whitespace())
            .map(|ch| if ch.is_whitespace() { '-' } else { ch }).collect::<String>();
        let wanted = normalized(heading);
        let mut offset = 0;
        let mut found = None;
        for line in self.text.split_inclusive('\n') {
            let trimmed = line.trim(); let hashes = trimmed.bytes().take_while(|byte| *byte == b'#').count();
            if (1..=6).contains(&hashes) && trimmed[hashes..].starts_with(' ') {
                if normalized(trimmed[hashes..].trim().trim_end_matches('#')) == wanted { found = Some(offset); break; }
            }
            offset += line.len();
        }
        if let Some(source) = found { self.select_to(source, false, window, cx); }
        else { self.report(format!("找不到标题：{heading}"), window, cx); }
    }
    pub(crate) fn ensure_widget_assets(&mut self, window: &Window, cx: &mut Context<Self>) {
        if self.widget_task.is_some() { return; }
        let requests = self.document.widgets().map(|(kind, _)| kind.clone())
            .filter(|kind| !matches!(kind, WidgetKind::Metadata { .. }) && !self.widget_views.0.contains_key(&kind.key()))
            .take(16).collect::<Vec<_>>();
        if requests.is_empty() { return; }
        for kind in &requests { self.widget_views.0.insert(kind.key(), WidgetView::Pending); }
        let context = self.note_context.clone();
        let text = self.value().to_string();
        let generation = self.widget_generation;
        let work = cx.background_spawn(async move {
            requests.into_iter().map(|kind| {
                let key = kind.key();
                let value = match &kind {
                    WidgetKind::Math { formula, display } => crate::markdown_renderers::render(&RenderKind::Math { display: *display }, formula)
                        .map(WidgetView::Asset).unwrap_or_else(|error| WidgetView::Error(error.to_string())),
                    WidgetKind::Mermaid { diagram } => crate::markdown_renderers::render(&RenderKind::Mermaid, diagram)
                        .map(WidgetView::Asset).unwrap_or_else(|error| WidgetView::Error(error.to_string())),
                    WidgetKind::NoteEmbed { target } => context.as_ref().ok_or_else(|| anyhow::anyhow!("笔记上下文尚未加载"))
                        .and_then(|context| crate::note_links::preview(context, target, &text)).map(WidgetView::Preview)
                        .unwrap_or_else(|error| WidgetView::Error(error.to_string())),
                    WidgetKind::Metadata { .. } => WidgetView::Pending,
                };
                (key, value)
            }).collect::<Vec<_>>()
        });
        self.widget_task = Some(cx.spawn_in(window, async move |view, cx| {
            let result = work.await;
            let _ = cx.update(|_, cx| {
                if let Some(view) = view.upgrade() {
                    view.update(cx, |this, cx| {
                        if this.widget_generation == generation {
                            this.widget_views.0.extend(result); this.widget_task = None; cx.notify();
                        }
                    });
                }
            });
        }));
    }
}
