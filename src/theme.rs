use crate::{config::{Config, MdColors}, theme_palette::Palette};
use gpui_kit::component::{Theme, ThemeMode as KitThemeMode};
use gpui_kit::{px, App, Window, Styled, StyleRefinement, HighlightStyle};

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize, Default)]
pub enum ThemeMode {
    System,
    Light,
    #[default]
    Dark,
}

/// Read-only presentation settings; never an editor document or save snapshot.
pub struct EditorTypography {
    pub line_height: f32,
    pub paragraph_spacing: u16,
    pub colors: MdColors,
}
impl gpui_kit::Global for EditorTypography {}

/// Compact UI spacing scales with UI text; document spacing scales with body text.
#[derive(Clone, Copy)]
pub struct LayoutMetrics {
    ui_scale: f32,
    body_scale: f32,
}
impl LayoutMetrics {
    pub fn from_app(cx: &App) -> Self {
        let theme = Theme::global(cx);
        Self {
            ui_scale: f32::from(theme.font_size) / 16.,
            body_scale: f32::from(theme.mono_font_size) / 16.,
        }
    }
    pub fn ui(self, base: f32) -> gpui_kit::Pixels { px(base * self.ui_scale) }
    pub fn body(self, base: f32) -> gpui_kit::Pixels { px(base * self.body_scale) }
    pub fn control_height(self) -> gpui_kit::Pixels { self.ui(24.).max(px(22.)) }
    pub fn tab_height(self) -> gpui_kit::Pixels { self.ui(28.).max(px(24.)) }
}

pub fn apply(cfg: &Config, window: Option<&mut Window>, cx: &mut App) {
    match cfg.theme_mode {
        ThemeMode::System => Theme::sync_system_appearance(window, cx),
        ThemeMode::Light => Theme::change(KitThemeMode::Light, window, cx),
        ThemeMode::Dark => Theme::change(KitThemeMode::Dark, window, cx),
    }
    let ui = cfg.ui_fonts.first().map(|p| crate::fonts::font_display_name(std::path::Path::new(p)));
    let editor = cfg.editor_fonts.first().map(|p| crate::fonts::font_display_name(std::path::Path::new(p)));
    Theme::update(cx, |theme| {
        let palette = Palette::for_mode(theme.is_dark());
        palette.apply(theme);
        theme.font_family = ui.unwrap_or_else(|| "Microsoft YaHei UI".into()).into();
        theme.font_size = px(cfg.ui_font_size.clamp(10, 24) as f32);
        theme.mono_font_family = editor.unwrap_or_else(|| "Consolas".into()).into();
        theme.mono_font_size = px(cfg.font_size.round().clamp(10., 24.));
        let colors = if theme.is_dark() { &cfg.md_dark } else { &cfg.md_light };
        let mut highlight = (*theme.highlight_theme).clone();
        highlight.style.editor_background = Some(gpui_kit::rgb(palette.canvas).into());
        highlight.style.editor_foreground = Some(color(colors.text));
        highlight.style.editor_active_line = Some(gpui_kit::rgb(palette.chrome).into());
        highlight.style.editor_line_number = Some(gpui_kit::rgb(palette.muted).into());
        highlight.style.editor_active_line_number = Some(gpui_kit::rgb(palette.primary).into());
        highlight.style.editor_gutter_background = Some(gpui_kit::rgb(palette.chrome).into());
        let syntax = &mut highlight.style.syntax;
        let style = |rgb: [u8; 3], weight: u16, italic: bool| {
            serde_json::from_value(serde_json::json!({"color": color(rgb), "font_weight": weight, "font_style": if italic { "italic" } else { "normal" }})).ok()
        };
        syntax.title = style(colors.heading, 700, false);
        syntax.emphasis_strong = style(colors.bold, 700, false);
        syntax.emphasis = style(colors.italic, 400, true);
        syntax.text_code_span = style(colors.code_text, 400, false);
        syntax.text_literal = style(colors.code_text, 400, false);
        syntax.link_text = style(colors.link, 400, false);
        syntax.link_uri = style(colors.link, 400, false);
        syntax.punctuation_list_marker = style(colors.list_marker, 400, false);
        syntax.punctuation = style(colors.syntax, 400, false);
        syntax.punctuation_special = style(colors.quote_bar, 400, false);
        syntax.comment = style(colors.quote_text, 400, false);
        theme.highlight_theme = std::sync::Arc::new(highlight);
    });
    let dark = Theme::global(cx).is_dark();
    cx.set_global(EditorTypography {
        line_height: crate::config::normalize_line_height(cfg.line_height),
        paragraph_spacing: cfg.paragraph_spacing.min(crate::config::MAX_PARAGRAPH_SPACING),
        colors: if dark { cfg.md_dark.clone() } else { cfg.md_light.clone() },
    });
}

/// Shared renderer style, so the visible document uses the same palette as UI.
pub fn document_style(cx: &App) -> gpui_kit::base::TextViewStyle {
    let theme = Theme::global(cx);
    let colors = cx.try_global::<EditorTypography>().map(|t| t.colors.clone())
        .unwrap_or_else(|| if theme.is_dark() { MdColors::default_dark() } else { MdColors::default_light() });
    let heading = color(colors.heading);
    let metrics = LayoutMetrics::from_app(cx);
    let paragraph_spacing = cx.try_global::<EditorTypography>().map_or(0, |t| t.paragraph_spacing);
    gpui_kit::base::TextViewStyle::from_theme(&gpui_kit::base::Theme::global(cx))
        .with_foreground(color(colors.text))
        .with_muted_foreground(color(colors.quote_text))
        .with_link(color(colors.link))
        .with_selection(Palette::for_mode(theme.is_dark()).selection())
        .with_border(color(colors.quote_bar))
        .with_code_background(color(colors.code_bg))
        // Rich paragraphs have their own spacing, not another line-height multiplier.
        .with_paragraph_gap(gpui_kit::rems(paragraph_spacing as f32 / f32::from(theme.font_size).max(1.)))
        .with_heading(move |_| StyleRefinement::default().text_color(heading))
        .with_code_block(StyleRefinement::default().text_size(theme.mono_font_size)
            .px(metrics.body(12.)).pb(metrics.body(12.)).border_0()
            .text_color(color(colors.code_text)))
        .with_inline_code(HighlightStyle { color: Some(color(colors.code_text)), background_color: Some(color(colors.code_bg)), ..Default::default() })
        .with_table_head(StyleRefinement::default().bg(theme.colors.title_bar).text_color(theme.colors.foreground))
}

/// Explicit native caret/selection colors, refreshed only when theme changes.
pub fn editor_style(cx: &App) -> markdown_editor::BlockEditorStyle {
    let theme = Theme::global(cx);
    markdown_editor::BlockEditorStyle::default()
        .with_foreground(theme.colors.foreground)
        .with_muted_foreground(theme.colors.muted_foreground)
        .with_caret(theme.colors.caret)
        .with_composition_underline(theme.colors.primary)
        .with_selection(Palette::for_mode(theme.is_dark()).selection())
}

pub fn color([r, g, b]: [u8; 3]) -> gpui_kit::Hsla {
    gpui_kit::rgb(((r as u32) << 16) | ((g as u32) << 8) | b as u32).into()
}
