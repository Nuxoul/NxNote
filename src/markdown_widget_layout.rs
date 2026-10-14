//! Native GPUI cards; no HTML or WebView execution.
use super::*;
use crate::markdown_projection::{LinkTarget, WidgetKind};
use crate::note_editor::WidgetView;
use gpui_kit::component::StyledExt;

impl DocumentRow {
    pub(super) fn widget(&self, kind: &WidgetKind, source: Range<usize>, maximum_width: Pixels) -> AnyElement {
        let view = self.widgets.get(kind);
        let editor = self.editor.clone();
        let source_start = source.start;
        let click_source = source.clone();
        let mut card = div().id(("markdown-widget", source_start)).v_flex().gap_1().max_w(maximum_width)
            .rounded_md().p_2().bg(self.style.code_background()).border_1().border_color(self.style.border())
            .on_mouse_down(MouseButton::Left, |_, window, cx| { window.prevent_default(); cx.stop_propagation(); })
            .on_mouse_up(MouseButton::Left, |_, window, cx| { window.prevent_default(); cx.stop_propagation(); });
        match kind {
            WidgetKind::Metadata { text } => {
                let summary = text.lines().filter(|line| !line.trim().is_empty()).take(3).collect::<Vec<_>>().join(" · ");
                card = card.child(div().text_size(px(12.)).text_color(self.style.muted_foreground()).child("▸ Front Matter · 点击展开"))
                    .child(div().text_size(px(12.)).max_w(maximum_width).truncate().child(summary))
                    .on_click(move |_, window, cx| editor.update(cx, |editor, cx| editor.expand_metadata(true, window, cx)));
            }
            WidgetKind::NoteEmbed { target } => {
                let target = target.clone();
                card = card.child(div().text_size(px(12.)).text_color(self.style.link()).child(format!("↗ {target}")));
                card = match view {
                    WidgetView::Preview(preview) => card.child(div().text_size(px(13.)).max_w(maximum_width).child(preview.text)),
                    WidgetView::Error(error) => card.child(div().text_size(px(12.)).text_color(rgb(0xe07070)).max_w(maximum_width).child(error)),
                    _ => card.child("正在读取笔记…"),
                };
                card = card.child(div().text_size(px(11.)).text_color(self.style.muted_foreground()).child("只读嵌入 · Ctrl+点击打开 · 点击编辑引用"))
                    .on_click(move |event, window, cx| editor.update(cx, |editor, cx| {
                        if event.modifiers().control { editor.open_target(LinkTarget::Note(target.clone()), window, cx); }
                        else { editor.edit_widget(click_source.clone(), window, cx); }
                    }));
            }
            WidgetKind::Math { .. } | WidgetKind::Mermaid { .. } => {
                let display = matches!(kind, WidgetKind::Mermaid { .. }) || matches!(kind, WidgetKind::Math { display: true, .. });
                card = match view {
                    WidgetView::Asset(asset) => {
                        let width = px(asset.width).min(maximum_width).max(px(1.));
                        let height = px(asset.height * (f32::from(width) / asset.width.max(1.))).max(px(1.));
                        card.child(img(asset.path).w(width).h(height).object_fit(ObjectFit::Contain))
                    }
                    WidgetView::Error(error) => card.child(div().text_size(px(12.)).text_color(rgb(0xe07070)).max_w(maximum_width).child(format!("渲染失败：{error}"))),
                    _ => card.child(if matches!(kind, WidgetKind::Mermaid { .. }) { "正在绘制图表…" } else { "正在排版公式…" }),
                };
                if display { card = card.child(div().text_size(px(11.)).text_color(self.style.muted_foreground()).child("点击编辑源码")); }
                card = card.on_click(move |_, window, cx| editor.update(cx, |editor, cx| editor.edit_widget(click_source.clone(), window, cx)));
            }
        }
        card.into_any_element()
    }
    pub(super) fn annotation(&self, label: &str, width: Pixels) -> AnyElement {
        let editor = self.editor.clone(); let metadata = label.starts_with("Front Matter");
        let source = self.row.source.start;
        div().id(("markdown-annotation", source)).max_w(width).text_size(px(12.)).text_color(self.style.muted_foreground())
            .child(label.to_string())
            .on_mouse_down(MouseButton::Left, |_, window, cx| { window.prevent_default(); cx.stop_propagation(); })
            .on_click(move |_, window, cx| { if metadata { editor.update(cx, |editor, cx| editor.expand_metadata(false, window, cx)); } })
            .into_any_element()
    }
}
pub(super) fn callout_color(kind: &str) -> Hsla {
    match kind.to_ascii_uppercase().as_str() {
        "TIP" => rgb(0x4aa878).into(),
        "IMPORTANT" => rgb(0xa082dc).into(),
        "WARNING" => rgb(0xd0a044).into(),
        "CAUTION" => rgb(0xd96a6a).into(),
        _ => rgb(0x6099d4).into(),
    }
}
