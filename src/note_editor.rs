//! NxNote adapter for the selected upstream GPUI Kit Markdown BlockEditor.
use crate::attachments::{is_image_file, Attachments};
use anyhow::Context as _;
use gpui_kit::component::{
    button::{Button, ButtonVariants}, dialog::{DialogAction, DialogClose, DialogFooter},
    input::{Input, InputEvent, InputState},
    notification::{Notification, NotificationType},
    ActiveTheme, Sizable, StyledExt, WindowExt,
};
use gpui_kit::*;
use markdown_editor::{BlockEditor, BlockEditorEvent, BlockEditorState, BlockKind,
    ClipboardPolicy, EditorCommand, MarkKind, PasteRequest};

pub fn init(cx: &mut App) { markdown_editor::init(cx); }

pub struct NoteEditor {
    state: Entity<BlockEditorState>,
    scroll: ScrollHandle,
    committed: SharedString,
    baseline: SharedString,
    original: SharedString,
    revision: u64,
    attachments: Attachments,
    attachment_error: Option<String>,
    paste_task: Option<Task<()>>,
    paste_pending: bool,
    _subscriptions: Vec<Subscription>,
}
impl EventEmitter<InputEvent> for NoteEditor {}
impl Focusable for NoteEditor {
    fn focus_handle(&self, cx: &App) -> FocusHandle { self.state.focus_handle(cx) }
}
impl NoteEditor {
    pub fn new(text: &str, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let scroll = ScrollHandle::new();
        let state = cx.new(|cx| {
            let mut state = BlockEditorState::new(window, cx).with_clipboard_policy(ClipboardPolicy::Host);
            state.set_editor_style(crate::theme::editor_style(cx), cx);
            // set_source reports failures; with_source silently discards them.
            if let Err(error) = state.set_source(text, cx) {
                state.set_readonly(true, cx);
                window.push_notification(Notification::new().message(format!("无法导入 Markdown：{error}；原文未修改")).with_type(NotificationType::Error).autohide(false), cx);
            }
            state.set_scroll_handle(scroll.clone());
            state
        });
        let baseline = state.read(cx).source_snapshot().text().clone();
        let revision = state.read(cx).revision();
        let events = cx.subscribe_in(&state, window, |this, _, event: &BlockEditorEvent, window, cx| {
            match event {
                BlockEditorEvent::Changed { .. } => this.sync(cx),
                BlockEditorEvent::Blur => { this.sync(cx); cx.emit(InputEvent::Blur); }
                BlockEditorEvent::Focus => cx.emit(InputEvent::Focus),
                BlockEditorEvent::PasteRequested { request, item } => this.paste(request.clone(), item.clone(), window, cx),
                BlockEditorEvent::EditLinkRequested { url } => this.link_dialog(url.clone(), window, cx),
                BlockEditorEvent::LinkClicked { .. } => Self::report("链接仅供编辑；当前不自动打开外部地址", window, cx),
                _ => {}
            }
            cx.notify();
        });
        let observer = cx.observe(&state, |this, _, cx| { this.sync(cx); cx.notify(); });
        let typography_observer = cx.observe_global::<crate::theme::EditorTypography>(|this, cx| {
            let style = crate::theme::editor_style(cx);
            this.state.update(cx, |state, cx| state.set_editor_style(style, cx));
            cx.notify();
        });
        let (attachments, attachment_error) = match Attachments::load() {
            Ok(value) => (value, None), Err(error) => (Attachments::default(), Some(error.to_string())),
        };
        Self { state, scroll, committed: text.to_string().into(), original: text.to_string().into(),
            baseline, revision, attachments, attachment_error, paste_task: None, paste_pending: false,
            _subscriptions: vec![events, observer, typography_observer] }
    }
    fn sync(&mut self, cx: &mut Context<Self>) {
        let state = self.state.read(cx);
        if state.is_composing() || state.revision() == self.revision { return; }
        self.revision = state.revision();
        let canonical = state.source_snapshot().text().clone();
        // Focus/render and undo back to the initial model never normalize disk text.
        let value = if canonical == self.baseline { self.original.clone() } else { canonical };
        if value != self.committed { self.committed = value; cx.emit(InputEvent::Change); }
    }
    pub fn value(&self) -> SharedString { self.committed.clone() }
    pub fn focus(&self, window: &mut Window, cx: &mut Context<Self>) { self.state.focus_handle(cx).focus(window, cx); }
    pub fn select_end(&mut self, cx: &mut Context<Self>) {
        self.state.update(cx, |state, cx| { let _ = state.select_end(cx); });
    }
    pub fn set_value(&mut self, text: String, window: &mut Window, cx: &mut Context<Self>) {
        let result = self.state.update(cx, |state, cx| state.set_source(&text, cx));
        if let Err(error) = result { Self::report(&format!("无法载入：{error}；原文未修改"), window, cx); return; }
        self.original = text.clone().into(); self.committed = text.into();
        self.baseline = self.state.read(cx).source_snapshot().text().clone();
        self.revision = self.state.read(cx).revision(); cx.notify();
    }
    pub fn move_block(&mut self, down: bool, window: &mut Window, cx: &mut Context<Self>) {
        self.command(EditorCommand::MoveBlocks { down }, window, cx);
    }
    fn command(&mut self, command: EditorCommand, window: &mut Window, cx: &mut Context<Self>) {
        if self.state.read(cx).is_composing() { return; }
        let result = self.state.update(cx, |state, cx| { let result = state.run(command, cx); state.focus(window, cx); result });
        if let Err(error) = result { Self::report(&format!("当前选区无法执行：{error:?}"), window, cx); }
        self.sync(cx); cx.notify();
    }
    fn report(message: &str, window: &mut Window, cx: &mut Context<Self>) {
        window.push_notification(Notification::new().message(message.to_string()).with_type(NotificationType::Error), cx);
    }
    fn link_dialog(&self, url: Option<SharedString>, window: &mut Window, cx: &mut Context<Self>) {
        let input = cx.new(|cx| InputState::new(window, cx).default_value(url.unwrap_or_default()));
        let state = self.state.clone();
        let revision = state.read(cx).revision(); let selection = state.read(cx).selection().clone();
        window.open_dialog(cx, move |dialog, _, _| {
            let input = input.clone(); let state = state.clone(); let selection = selection.clone();
            dialog.title("编辑链接（留空移除）").child(Input::new(&input))
                .footer(DialogFooter::new()
                    .child(DialogClose::new().trigger(|button| button.label("取消")))
                    .child(DialogAction::new().child(Button::new("apply-link").primary().label("应用"))))
                .on_ok(move |_, window, cx| {
                    let url = input.read(cx).value().to_string();
                    state.update(cx, |state, cx| {
                        if state.revision() != revision || state.selection() != &selection || state.is_composing() { return false; }
                        state.set_link(if url.trim().is_empty() { None } else { Some(url.trim()) }, cx);
                        state.focus(window, cx); true
                    })
                })
        });
    }
    fn paste(&mut self, request: PasteRequest, item: ClipboardItem, window: &mut Window, cx: &mut Context<Self>) {
        let mut paths = item.entries().iter().flat_map(|entry| match entry {
            ClipboardEntry::ExternalPaths(paths) => paths.paths().to_vec(), _ => Vec::new(),
        }).filter(|path| is_image_file(path)).collect::<Vec<_>>();
        paths.extend(crate::platform::clipboard_files().into_iter().filter(|path| is_image_file(path)));
        let mut seen = std::collections::HashSet::new(); paths.retain(|p| seen.insert(p.clone()));
        let image = if paths.is_empty() { item.entries().iter().find_map(|entry| match entry {
            ClipboardEntry::Image(image) => Some(image.bytes().to_vec()), _ => None,
        }) } else { None };
        if paths.is_empty() && image.is_none() {
            let text = item.text();
            self.state.update(cx, |state, cx| { state.apply_paste(&request, text.as_deref(), cx); });
            return;
        }
        if self.paste_pending || self.attachment_error.is_some() {
            Self::report(self.attachment_error.as_deref().unwrap_or("图片正在保存，请稍后重试"), window, cx);
            self.state.update(cx, |state, cx| { state.apply_paste(&request, None, cx); }); return;
        }
        if !self.state.read(cx).is_paste_current(&request) || self.state.read(cx).is_composing() { return; }
        let mut attachments = self.attachments.clone();
        let work = cx.background_executor().spawn(async move {
            let mut values = Vec::new();
            if let Some(bytes) = image { values.push(attachments.save_image(&bytes)?); }
            for path in paths {
                image::image_dimensions(&path).with_context(|| format!("无法读取图片 {}", path.display()))?;
                let label = path.file_name().and_then(|s| s.to_str()).unwrap_or("image.png").to_string();
                values.push(attachments.register(&path, label)?);
            }
            Ok::<_, anyhow::Error>((attachments, values))
        });
        self.paste_pending = true;
        self.paste_task = Some(cx.spawn_in(window, async move |view, cx| {
            let result = work.await;
            let _ = cx.update(|window, cx| { if let Some(view) = view.upgrade() { view.update(cx, |this, cx| {
                this.paste_pending = false;
                match result {
                    Ok((attachments, paths)) => {
                        this.attachments = attachments;
                        if !this.state.read(cx).is_paste_current(&request) || !this.state.focus_handle(cx).is_focused(window) || this.state.read(cx).is_composing() {
                            Self::report("图片已保存，但正文或光标已变化，请重新粘贴", window, cx); return;
                        }
                        let markdown = paths.into_iter().map(|path| {
                            let path = path.replace('\\', "/").replace('%', "%25").replace(' ', "%20").replace('<', "%3C").replace('>', "%3E").replace('#', "%23");
                            format!("![image](<{path}>)")
                        }).collect::<Vec<_>>().join("\n\n");
                        this.state.update(cx, |state, cx| {
                            state.apply_markdown_paste(&request, &markdown, cx);
                        });
                    }
                    Err(error) => { this.state.update(cx, |state, cx| { state.apply_paste(&request, None, cx); }); Self::report(&format!("图片粘贴失败：{error:#}"), window, cx); }
                }
                cx.notify();
            }); } });
        }));
    }
    fn tool(&self, id: &'static str, label: &'static str, command: EditorCommand, cx: &mut Context<Self>) -> Button {
        Button::new(id).ghost().small().label(label)
            .on_mouse_down(MouseButton::Left, |_, window, cx| { window.prevent_default(); cx.stop_propagation(); })
            .on_click(cx.listener(move |this, _, window, cx| this.command(command.clone(), window, cx)))
    }
}
impl Render for NoteEditor {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let line_height = cx.try_global::<crate::theme::EditorTypography>()
            .map_or(1.5, |typography| typography.line_height);
        let text_style = crate::theme::document_style(cx);
        let foreground = text_style.foreground();
        div().v_flex().size_full().min_h_0().key_context("NxNoteEditor")
            .child(div().flex().flex_wrap().gap_1().p_1().bg(cx.theme().title_bar).border_b_1().border_color(cx.theme().border)
                .child(self.tool("b", "B", EditorCommand::ToggleMark(MarkKind::Bold), cx))
                .child(self.tool("i", "I", EditorCommand::ToggleMark(MarkKind::Italic), cx))
                .child(self.tool("p", "正文", EditorCommand::SetBlockKind(BlockKind::Paragraph), cx))
                .child(self.tool("h1", "H1", EditorCommand::SetBlockKind(BlockKind::Heading { level: 1 }), cx))
                .child(self.tool("h2", "H2", EditorCommand::SetBlockKind(BlockKind::Heading { level: 2 }), cx))
                .child(self.tool("ul", "列表", EditorCommand::ToggleList { ordered: false }, cx))
                .child(self.tool("task", "待办", EditorCommand::ToggleTask, cx))
                .child(self.tool("code", "代码", EditorCommand::SetCodeBlock { lang: None }, cx))
                .child(self.tool("table", "表格", EditorCommand::InsertTable { rows: 3, cols: 3 }, cx)))
            .child(div().id("block-editor-scroll").flex_1().min_h_0().w_full().overflow_y_scroll().track_scroll(&self.scroll)
                .font_family(cx.theme().mono_font_family.clone()).text_size(cx.theme().mono_font_size)
                .text_color(foreground)
                .child(BlockEditor::new(&self.state).text_style(text_style).line_height(relative(line_height))
                    .line_numbers(true).min_h_full().py_3().pr_3().pl_0()))
    }
}
