use egui::{
    pos2, vec2, Align2, Color32, CursorIcon, FontId, Id, Rect, Sense, Stroke, Ui, ViewportCommand,
};

use crate::icons;
use crate::theme::{palette, ThemeMode};

pub const TITLE_BAR_HEIGHT: f32 = 24.0;
const RESIZE_THICK: f32 = 4.0;

pub struct TitleBarConfig<'a> {
    pub title: &'a str,
    pub show_min_max: bool,
    pub mode: ThemeMode,
    /// 显示"始终置顶"切换按钮；Some(state)=显示并反映状态，None=不显示
    pub on_top: Option<bool>,
}

#[derive(Default)]
pub struct TitleBarOutput {
    /// 用户点击了"置顶"图标，外部应翻转 cfg.always_on_top 并下发 WindowLevel
    pub on_top_toggled: bool,
}

/// 自绘标题栏。在调用方分配出来的 rect 内绘制。
pub fn draw_title_bar(
    ctx: &egui::Context,
    ui: &mut Ui,
    rect: Rect,
    cfg: TitleBarConfig<'_>,
) -> TitleBarOutput {
    let p = palette(cfg.mode);
    let painter = ui.painter_at(rect);
    let mut out = TitleBarOutput::default();

    // 背景
    painter.rect_filled(rect, 0.0, p.bg);
    // 底边分隔线
    painter.line_segment(
        [
            pos2(rect.left(), rect.bottom() - 0.5),
            pos2(rect.right(), rect.bottom() - 0.5),
        ],
        Stroke::new(1.0, p.stroke),
    );

    // 标题文字（居左）
    painter.text(
        pos2(rect.left() + 8.0, rect.center().y),
        Align2::LEFT_CENTER,
        cfg.title,
        FontId::proportional(11.5),
        p.text_strong,
    );

    let btn_w = 32.0;
    let mut right = rect.right();

    // 关闭按钮
    let close_rect = Rect::from_min_max(pos2(right - btn_w, rect.top()), pos2(right, rect.bottom()));
    if nav_button(
        ui,
        close_rect,
        icons::CLOSE,
        Id::new("nx_tb_close"),
        p.danger,
        Color32::WHITE,
        p.text,
    ) {
        ctx.send_viewport_cmd(ViewportCommand::Close);
    }
    right -= btn_w;

    if cfg.show_min_max {
        // 最大化按钮
        let max_rect = Rect::from_min_max(pos2(right - btn_w, rect.top()), pos2(right, rect.bottom()));
        if nav_button(
            ui,
            max_rect,
            icons::MAXIMIZE,
            Id::new("nx_tb_max"),
            p.hover,
            p.text_strong,
            p.text,
        ) {
            let maximized = ctx.input(|i| i.viewport().maximized.unwrap_or(false));
            ctx.send_viewport_cmd(ViewportCommand::Maximized(!maximized));
        }
        right -= btn_w;

        // 最小化按钮
        let min_rect = Rect::from_min_max(pos2(right - btn_w, rect.top()), pos2(right, rect.bottom()));
        if nav_button(
            ui,
            min_rect,
            icons::MINIMIZE,
            Id::new("nx_tb_min"),
            p.hover,
            p.text_strong,
            p.text,
        ) {
            ctx.send_viewport_cmd(ViewportCommand::Minimized(true));
        }
        right -= btn_w;
    }

    // 置顶按钮（可选）
    if let Some(active) = cfg.on_top {
        let pin_rect = Rect::from_min_max(pos2(right - btn_w, rect.top()), pos2(right, rect.bottom()));
        let normal_text = if active { p.accent } else { p.text_weak };
        if nav_button(
            ui,
            pin_rect,
            icons::STAY_ON_TOP,
            Id::new("nx_tb_on_top"),
            p.hover,
            if active { p.accent } else { p.text_strong },
            normal_text,
        ) {
            out.on_top_toggled = true;
        }
        right -= btn_w;
    }

    // 拖拽区域：标题文字右侧到按钮左侧
    let drag_rect = Rect::from_min_max(
        pos2(rect.left() + 6.0, rect.top()),
        pos2(right, rect.bottom()),
    );
    let drag_resp = ui.interact(drag_rect, Id::new("nx_titlebar_drag"), Sense::click_and_drag());
    if drag_resp.is_pointer_button_down_on() {
        ctx.send_viewport_cmd(ViewportCommand::StartDrag);
    }
    if cfg.show_min_max && drag_resp.double_clicked() {
        let maximized = ctx.input(|i| i.viewport().maximized.unwrap_or(false));
        ctx.send_viewport_cmd(ViewportCommand::Maximized(!maximized));
    }

    out
}

pub fn nav_button(
    ui: &mut Ui,
    rect: Rect,
    glyph: &str,
    id: Id,
    hover_bg: Color32,
    hover_text: Color32,
    text: Color32,
) -> bool {
    let resp = ui.interact(rect, id, Sense::click());
    let (bg, fg) = if resp.is_pointer_button_down_on() {
        (hover_bg, hover_text)
    } else if resp.hovered() {
        (hover_bg, hover_text)
    } else {
        (Color32::TRANSPARENT, text)
    };
    let p = ui.painter_at(rect);
    if bg != Color32::TRANSPARENT {
        p.rect_filled(rect, 0.0, bg);
    }
    p.text(
        rect.center(),
        Align2::CENTER_CENTER,
        glyph,
        icons::font(12.0),
        fg,
    );
    resp.clicked()
}

/// 在窗口四边/四角放置缩放交互区。需要在所有内容绘制之后调用，以覆盖边缘控件。
pub fn draw_resize_handles(ctx: &egui::Context, ui: &mut Ui) {
    let screen = ctx.screen_rect();
    let t = RESIZE_THICK;

    let handle = |rect: Rect,
                      suffix: &'static str,
                      cmd: ViewportCommand,
                      cursor: CursorIcon| {
        if rect.width() <= 0.0 || rect.height() <= 0.0 {
            return;
        }
        let resp = ui.interact(rect, Id::new(("nx_resize", suffix)), Sense::click_and_drag());
        if resp.hovered() || resp.is_pointer_button_down_on() {
            ctx.set_cursor_icon(cursor);
        }
        if resp.drag_started() {
            ctx.send_viewport_cmd(cmd.clone());
        }
    };

    // 四角优先
    handle(
        Rect::from_min_size(screen.left_top(), vec2(t, t)),
        "nw",
        ViewportCommand::BeginResize(egui::ResizeDirection::NorthWest),
        CursorIcon::ResizeNwSe,
    );
    handle(
        Rect::from_min_size(pos2(screen.right() - t, screen.top()), vec2(t, t)),
        "ne",
        ViewportCommand::BeginResize(egui::ResizeDirection::NorthEast),
        CursorIcon::ResizeNeSw,
    );
    handle(
        Rect::from_min_size(pos2(screen.left(), screen.bottom() - t), vec2(t, t)),
        "sw",
        ViewportCommand::BeginResize(egui::ResizeDirection::SouthWest),
        CursorIcon::ResizeNeSw,
    );
    handle(
        Rect::from_min_size(pos2(screen.right() - t, screen.bottom() - t), vec2(t, t)),
        "se",
        ViewportCommand::BeginResize(egui::ResizeDirection::SouthEast),
        CursorIcon::ResizeNwSe,
    );

    // 四边
    handle(
        Rect::from_min_max(
            pos2(screen.left() + t, screen.top()),
            pos2(screen.right() - t, screen.top() + t),
        ),
        "n",
        ViewportCommand::BeginResize(egui::ResizeDirection::North),
        CursorIcon::ResizeVertical,
    );
    handle(
        Rect::from_min_max(
            pos2(screen.left() + t, screen.bottom() - t),
            pos2(screen.right() - t, screen.bottom()),
        ),
        "s",
        ViewportCommand::BeginResize(egui::ResizeDirection::South),
        CursorIcon::ResizeVertical,
    );
    handle(
        Rect::from_min_max(
            pos2(screen.left(), screen.top() + t),
            pos2(screen.left() + t, screen.bottom() - t),
        ),
        "w",
        ViewportCommand::BeginResize(egui::ResizeDirection::West),
        CursorIcon::ResizeHorizontal,
    );
    handle(
        Rect::from_min_max(
            pos2(screen.right() - t, screen.top() + t),
            pos2(screen.right(), screen.bottom() - t),
        ),
        "e",
        ViewportCommand::BeginResize(egui::ResizeDirection::East),
        CursorIcon::ResizeHorizontal,
    );
}
