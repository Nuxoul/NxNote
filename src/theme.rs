use crate::config::Config;
use gpui_kit::component::{Theme, ThemeMode as KitThemeMode};
use gpui_kit::{px, App, Window};

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize, Default)]
pub enum ThemeMode {
    System,
    Light,
    #[default]
    Dark,
}

pub fn apply(cfg: &Config, window: Option<&mut Window>, cx: &mut App) {
    match cfg.theme_mode {
        ThemeMode::System => Theme::sync_system_appearance(window, cx),
        ThemeMode::Light => Theme::change(KitThemeMode::Light, window, cx),
        ThemeMode::Dark => Theme::change(KitThemeMode::Dark, window, cx),
    }
    let ui = cfg
        .ui_fonts
        .first()
        .map(|p| crate::fonts::font_display_name(std::path::Path::new(p)));
    let editor = cfg
        .editor_fonts
        .first()
        .map(|p| crate::fonts::font_display_name(std::path::Path::new(p)));
    Theme::update(cx, |theme| {
        theme.font_family = ui.unwrap_or_else(|| "Microsoft YaHei UI".into()).into();
        theme.font_size = px(cfg.ui_font_size.clamp(10, 24) as f32);
        theme.mono_font_family = editor.unwrap_or_else(|| "Consolas".into()).into();
        theme.mono_font_size = px(cfg.font_size.round().clamp(10., 24.));
        let colors = if theme.is_dark() {
            &cfg.md_dark
        } else {
            &cfg.md_light
        };
        let mut highlight = (*theme.highlight_theme).clone();
        highlight.style.editor_foreground = Some(color(colors.text));
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
}

pub fn color([r, g, b]: [u8; 3]) -> gpui_kit::Hsla {
    gpui_kit::rgb(((r as u32) << 16) | ((g as u32) << 8) | b as u32).into()
}

/// Same body-relative heading scales and line heights as Kit Markdown.
pub fn markdown_metrics(level: u8) -> (f32, f32) {
    match level {
        1 => (1.8, 1.25),
        2 => (4. / 3., 1.35),
        3 => (17. / 15., 1.35),
        4 => (1., 1.35),
        5 => (14. / 15., 1.35),
        6 => (13. / 15., 1.35),
        _ => (1., 1.5),
    }
}
pub fn document_style(cfg: &Config, cx: &App) -> gpui_kit::base::TextViewStyle {
    use gpui_kit::component::ActiveTheme;
    use gpui_kit::{HighlightStyle, StyleRefinement, Styled};
    let colors = if cx.theme().is_dark() {
        &cfg.md_dark
    } else {
        &cfg.md_light
    };
    let heading = color(colors.heading);
    let body_size = cfg.font_size.round().clamp(10., 24.);
    gpui_kit::base::TextViewStyle::from_theme(&gpui_kit::base::Theme::global(cx))
        .with_foreground(color(colors.text))
        .with_muted_foreground(color(colors.quote_text))
        .with_link(color(colors.link))
        .with_border(color(colors.quote_bar))
        .with_code_background(color(colors.code_bg))
        .with_heading(move |level| StyleRefinement::default()
            .text_color(heading)
            .text_size(px(body_size * markdown_metrics(level).0))
            .font_weight(gpui_kit::FontWeight::SEMIBOLD))
        .with_inline_code(HighlightStyle {
            color: Some(color(colors.code_text)),
            background_color: Some(color(colors.code_bg)),
            ..Default::default()
        })
        .with_code_block(StyleRefinement::default().text_color(color(colors.code_text)))
        .with_paragraph_gap(gpui_kit::rems(0.))
}
