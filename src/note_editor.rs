//! Component-first editor: image chips in Textarea, source tools in Editor.
use crate::attachments::{is_image_file, Attachments};
use anyhow::Context as _;
use gpui_kit::component::{
    button::{Button, ButtonVariants},
    hover_card::HoverCard,
    input::{Editor, EditorState, InputEvent, InputToken, Textarea, TextareaState},
    ActiveTheme, IconName, Selectable, Sizable, StyledExt,
};
use gpui_kit::*;
use std::{ops::Range, path::PathBuf, time::Duration};

pub struct CursorPosition {
    pub line: usize,
    pub character: usize,
}
pub struct NoteEditor {
    rich: Entity<TextareaState>,
    source: Entity<EditorState>,
    source_mode: bool,
    text: SharedString,
    selection: Range<usize>,
    attachments: Attachments,
    attachment_error: Option<String>,
    _subscriptions: Vec<Subscription>,
    paste_task: Option<Task<()>>,
    paste_pending: bool,
}
impl EventEmitter<InputEvent> for NoteEditor {}
impl Focusable for NoteEditor {
    fn focus_handle(&self, cx: &App) -> FocusHandle {
        if self.source_mode {
            self.source.focus_handle(cx)
        } else {
            self.rich.focus_handle(cx)
        }
    }
}
impl NoteEditor {
    pub fn new(text: &str, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let (attachments, attachment_error) = match Attachments::load() {
            Ok(attachments) => (attachments, None),
            Err(e) => (Attachments::default(), Some(e.to_string())),
        };
        let content = attachments.content(text);
        let rich = cx.new(|cx| {
            let mut state = TextareaState::new(window, cx)
                .soft_wrap(true)
                .searchable(true);
            state.set_value(content, window, cx);
            state
        });
        let source = cx.new(|cx| {
            EditorState::new(window, cx)
                .language("markdown")
                .line_number(true)
                .soft_wrap(true)
                .auto_close(false)
                .smart_indent(false)
                .default_value(text.to_string())
        });
        let mut subscriptions = Vec::new();
        subscriptions.push(cx.subscribe(&rich, |this, state, event: &InputEvent, cx| {
            if !this.source_mode {
                this.text = state.read(cx).value();
                this.selection = state.read(cx).selected_range();
                cx.emit(event.clone());
                cx.notify();
            }
        }));
        subscriptions.push(
            cx.subscribe(&source, |this, state, event: &InputEvent, cx| {
                if this.source_mode {
                    this.text = state.read(cx).value();
                    this.selection = state.read(cx).selected_range();
                    cx.emit(event.clone());
                    cx.notify();
                }
            }),
        );
        subscriptions.push(cx.observe(&rich, |this, state, cx| {
            if !this.source_mode {
                this.text = state.read(cx).value();
                this.selection = state.read(cx).selected_range();
                cx.notify();
            }
        }));
        subscriptions.push(cx.observe(&source, |this, state, cx| {
            if this.source_mode {
                this.text = state.read(cx).value();
                this.selection = state.read(cx).selected_range();
                cx.notify();
            }
        }));
        Self {
            rich,
            source,
            source_mode: false,
            text: text.to_string().into(),
            selection: 0..0,
            attachments,
            attachment_error,
            _subscriptions: subscriptions,
            paste_task: None,
            paste_pending: false,
        }
    }
    pub fn value(&self) -> SharedString {
        self.text.clone()
    }
    pub fn preview_markdown(&self) -> String {
        self.attachments.preview_markdown(&self.text)
    }
    pub fn selected_range(&self) -> Range<usize> {
        self.selection.clone()
    }
    pub fn cursor_position(&self) -> CursorPosition {
        let end = self.selection.end.min(self.text.len());
        let before = &self.text[..end];
        CursorPosition {
            line: before.bytes().filter(|b| *b == b'\n').count(),
            character: before.rsplit('\n').next().unwrap_or("").chars().count(),
        }
    }
    pub fn focus(&self, window: &mut Window, cx: &mut Context<Self>) {
        self.focus_handle(cx).focus(window, cx);
    }
    pub fn set_selected_range(&mut self, range: Range<usize>, cx: &mut Context<Self>) {
        if self.source_mode {
            self.source
                .update(cx, |state, cx| state.set_selected_range(range.clone(), cx));
        } else {
            self.rich
                .update(cx, |state, cx| state.set_selected_range(range.clone(), cx));
        }
        self.selection = range;
        cx.notify();
    }
    pub fn set_value(&mut self, text: String, window: &mut Window, cx: &mut Context<Self>) {
        self.rich.update(cx, |state, cx| {
            state.set_value(self.attachments.content(&text), window, cx)
        });
        self.source
            .update(cx, |state, cx| state.set_value(text.clone(), window, cx));
        self.text = text.into();
        self.selection = 0..0;
        cx.notify();
    }
    pub fn replace(
        &mut self,
        text: impl Into<SharedString>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let text = text.into();
        if self.source_mode {
            self.source
                .update(cx, |state, cx| state.replace(text, window, cx));
        } else {
            self.rich
                .update(cx, |state, cx| state.replace(text, window, cx));
        }
        self.refresh(cx);
    }
    pub fn replace_all(
        &mut self,
        text: impl Into<SharedString>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let text = text.into();
        if self.source_mode {
            self.source
                .update(cx, |state, cx| state.replace_all(text, window, cx));
        } else {
            self.rich
                .update(cx, |state, cx| state.replace_all(text, window, cx));
        }
        self.refresh(cx);
    }
    fn refresh(&mut self, cx: &mut Context<Self>) {
        if self.source_mode {
            self.text = self.source.read(cx).value();
            self.selection = self.source.read(cx).selected_range();
        } else {
            self.text = self.rich.read(cx).value();
            self.selection = self.rich.read(cx).selected_range();
        }
        cx.notify();
    }
    fn toggle_source(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.refresh(cx);
        let range = self.selection.clone();
        self.source_mode = !self.source_mode;
        if self.source_mode {
            self.source.update(cx, |s, cx| {
                if s.value() != self.text {
                    s.set_value(self.text.clone(), window, cx);
                }
                s.set_selected_range(range, cx);
            });
        } else {
            self.rich.update(cx, |s, cx| {
                if s.value() != self.text {
                    s.set_value(self.attachments.content(&self.text), window, cx);
                }
                s.set_selected_range(range, cx);
            });
        }
        self.focus(window, cx);
        cx.notify();
    }
    fn paste(&mut self, item: ClipboardItem, window: &mut Window, cx: &mut Context<Self>) {
        if self.paste_pending {
            self.report("图片正在保存，请稍后再粘贴".into(), window, cx);
            return;
        }
        if let Some(error) = &self.attachment_error {
            self.report(error.clone(), window, cx);
            return;
        }
        let mut paths: Vec<PathBuf> = item
            .entries()
            .iter()
            .flat_map(|entry| match entry {
                ClipboardEntry::ExternalPaths(paths) => paths.paths().to_vec(),
                _ => Vec::new(),
            })
            .filter(|path| is_image_file(path))
            .collect();
        let mut seen = std::collections::HashSet::new();
        paths.retain(|path| seen.insert(path.clone()));
        let image = if paths.is_empty() {
            item.entries().iter().find_map(|entry| match entry {
                ClipboardEntry::Image(image) => Some(image.bytes().to_vec()),
                _ => None,
            })
        } else {
            None
        };
        if image.is_none() && paths.is_empty() {
            return;
        }
        let mut attachments = self.attachments.clone();
        let selection = self.selected_range();
        let original = self.value();
        let load = cx.background_executor().spawn(async move {
            let mut values = Vec::new();
            if let Some(bytes) = image {
                values.push(attachments.save_image(&bytes)?);
            }
            for path in paths {
                image::image_dimensions(&path)
                    .with_context(|| format!("无法读取图片 {}", path.display()))?;
                let label = path
                    .file_name()
                    .and_then(|n| n.to_str())
                    .unwrap_or("image.png")
                    .to_string();
                values.push(attachments.register(&path, label)?);
            }
            Ok::<_, anyhow::Error>((attachments, values))
        });
        self.paste_pending = true;
        self.paste_task = Some(cx.spawn_in(window, async move |view, cx| {
            let result = load.await;
            let _ = cx.update(|window, cx| {
                if let Some(view) = view.upgrade() {
                    view.update(cx, |this, cx| {
                        this.paste_pending = false;
                        match result {
                            Ok((attachments, paths)) => {
                                this.attachments = attachments;
                                if this.value() != original || this.selected_range() != selection {
                                    this.report(
                                        "图片已保存，但正文或光标已变化，请重新粘贴".into(),
                                        window,
                                        cx,
                                    );
                                    return;
                                }
                                if this.source_mode {
                                    this.source
                                        .update(cx, |s, cx| s.insert(paths.join(" "), window, cx));
                                } else {
                                    for (i, path) in paths.iter().enumerate() {
                                        if i > 0 {
                                            this.rich.update(cx, |s, cx| s.insert(" ", window, cx));
                                        }
                                        let token = this.attachments.token(path);
                                        if let Err(e) = this.rich.update(cx, |s, cx| {
                                            s.replace_with_token(token, window, cx)
                                        }) {
                                            this.report(format!("插入图片失败: {e:?}"), window, cx);
                                            break;
                                        }
                                    }
                                }
                                this.refresh(cx);
                                cx.emit(InputEvent::Change);
                            }
                            Err(e) => this.report(format!("粘贴图片失败: {e:#}"), window, cx),
                        }
                    });
                }
            });
        }));
    }
    fn report(&self, message: String, window: &mut Window, cx: &mut Context<Self>) {
        use gpui_kit::component::{
            notification::{Notification, NotificationType},
            WindowExt,
        };
        window.push_notification(
            Notification::new()
                .message(message)
                .with_type(NotificationType::Error),
            cx,
        );
    }
}

impl Render for NoteEditor {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let weak = cx.weak_entity();
        let paste = move |item: &ClipboardItem, window: &mut Window, cx: &mut App| {
            let mut item = item.clone();
            let files: Vec<PathBuf> = crate::platform::clipboard_files()
                .into_iter()
                .filter(|path| is_image_file(path))
                .collect();
            if !files.is_empty() {
                item.entries
                    .push(ClipboardEntry::ExternalPaths(ExternalPaths(files.into())));
            }
            let has_image = item.entries().iter().any(|e| match e {
                ClipboardEntry::Image(_) => true,
                ClipboardEntry::ExternalPaths(p) => p.paths().iter().any(|p| is_image_file(p)),
                _ => false,
            });
            if has_image {
                if let Some(view) = weak.upgrade() {
                    view.update(cx, |view, cx| view.paste(item.clone(), window, cx));
                }
                true
            } else if let Some(text) = item
                .text()
                .filter(|text| crate::editor_support::image_url(text))
            {
                if let Some(view) = weak.upgrade() {
                    view.update(cx, |view, cx| {
                        view.replace(format!("![图片]({})\n", text.trim()), window, cx)
                    });
                }
                true
            } else {
                false
            }
        };
        let body = if self.source_mode {
            Editor::new(&self.source)
                .h_full()
                .appearance(false)
                .bordered(false)
                .aria_label("笔记源码编辑器")
                .on_paste(paste)
                .into_any_element()
        } else {
            let state = self.rich.downgrade();
            let click_state = self.rich.downgrade();
            Textarea::new(&self.rich)
                .h_full()
                .appearance(false)
                .bordered(false)
                .aria_label("笔记编辑器")
                .text_size(cx.theme().mono_font_size)
                .on_paste(paste)
                .on_token_click(move |event, window, cx| {
                    if event
                        .click()
                        .mouse_position()
                        .is_some_and(|position| position.x >= event.bounds().right() - px(22.))
                    {
                        if let Some(state) = click_state.upgrade() {
                            state.update(cx, |state, cx| {
                                state.set_selected_range(event.range(), cx);
                                state.replace("", window, cx);
                            });
                        }
                    }
                })
                .token(move |context, _, _| {
                    let path = PathBuf::from(context.token().text().as_ref());
                    let label = context.token().label().clone();
                    let range = context.range();
                    let start = range.start;
                    let state = state.clone();
                    HoverCard::new(("image-preview", start))
                        .open_delay(Duration::from_millis(180))
                        .close_delay(Duration::from_millis(100))
                        .trigger(
                            div()
                                .flex()
                                .items_center()
                                .gap_0()
                                .h(context.line_height())
                                .child(InputToken::new(context).icon(
                                    gpui_kit::component::Icon::default().path("icons/image.svg"),
                                ))
                                .child(
                                    Button::new(("remove-image", start))
                                        .ghost()
                                        .xsmall()
                                        .icon(IconName::Close)
                                        .tooltip("移除图片引用")
                                        .on_click(move |_, window, cx| {
                                            cx.stop_propagation();
                                            if let Some(state) = state.upgrade() {
                                                state.update(cx, |s, cx| {
                                                    s.set_selected_range(range.clone(), cx);
                                                    s.replace("", window, cx);
                                                });
                                            }
                                        }),
                                ),
                        )
                        .content(move |_, _, _| {
                            div()
                                .id("image-hover-preview")
                                .v_flex()
                                .gap_2()
                                .w(px(280.))
                                .p_2()
                                .child(
                                    img(path.clone())
                                        .w_full()
                                        .h(px(180.))
                                        .object_fit(ObjectFit::Contain),
                                )
                                .child(div().text_xs().child(label.clone()))
                        })
                })
                .into_any_element()
        };
        div()
            .v_flex()
            .size_full()
            .min_h_0()
            .child(
                div().flex().justify_end().px_2().flex_shrink_0().child(
                    Button::new("source-mode")
                        .ghost()
                        .xsmall()
                        .selected(self.source_mode)
                        .label(if self.source_mode {
                            "返回图片编辑"
                        } else {
                            "源码编辑"
                        })
                        .tooltip("源码模式提供 Markdown 高亮、行号与搜索")
                        .on_click(
                            cx.listener(|this, _, window, cx| this.toggle_source(window, cx)),
                        ),
                ),
            )
            .child(div().flex_1().min_h_0().child(body))
    }
}
