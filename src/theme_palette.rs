//! NxNote's two coherent UI palettes. Semantic roles, no document state.
use gpui_kit::component::Theme;
use gpui_kit::Hsla;

fn rgb(value: u32) -> Hsla { gpui_kit::rgb(value).into() }

#[derive(Clone, Copy)]
pub struct Palette {
    pub canvas: u32,
    pub chrome: u32,
    pub surface: u32,
    pub raised: u32,
    pub hover: u32,
    pub pressed: u32,
    pub text: u32,
    pub muted: u32,
    pub border: u32,
    pub input: u32,
    pub primary: u32,
    pub primary_hover: u32,
    pub primary_active: u32,
    pub primary_text: u32,
    pub selection: u32,
    pub active: u32,
    pub link: u32,
    pub danger: u32,
    pub success: u32,
    pub warning: u32,
}
impl Palette {
    pub fn for_mode(dark: bool) -> Self {
        if dark {
            Self {
                canvas: 0x181E28, chrome: 0x202835, surface: 0x252F3E,
                raised: 0x2B3748, hover: 0x303E52, pressed: 0x3A4B62,
                text: 0xE5EAF2, muted: 0xCADCF3, border: 0x3D4D63, input: 0x687C96,
                primary: 0xA6CAFA, primary_hover: 0xBDD8FC, primary_active: 0x8BB8F4,
                primary_text: 0x152339, selection: 0x365F8D, active: 0x2C4464,
                link: 0xD2E6FF, danger: 0xF5A8B0, success: 0x93D8B2, warning: 0xF3CF8B,
            }
        } else {
            Self {
                canvas: 0xFCFDFE, chrome: 0xF0F3F8, surface: 0xFFFFFF,
                raised: 0xE8EDF5, hover: 0xE1E9F4, pressed: 0xD3DFF0,
                text: 0x202B3C, muted: 0x43536B, border: 0xC6D0DE, input: 0x8797AC,
                primary: 0x245DA8, primary_hover: 0x1B4F94, primary_active: 0x143F7A,
                primary_text: 0xFFFFFF, selection: 0xA6CAFA, active: 0xD8E6FA,
                link: 0x174F96, danger: 0xA82F43, success: 0x246647, warning: 0x825511,
            }
        }
    }
    pub fn selection(self) -> Hsla { rgb(self.selection) }
    pub fn apply(self, theme: &mut Theme) {
        // Every component role used by the application has an explicit solid
        // value; Theme::update projects these to both legacy tokens and Base.
        macro_rules! set { ($value:expr; $($field:ident),+ $(,)?) => { $(theme.colors.$field = rgb($value);)+ }; }
        set!(self.canvas; background, list, table, tab_active);
        set!(self.chrome; title_bar, status_bar, tab_bar, tab, sidebar, table_head, table_foot, list_head, group_box, accordion, description_list_label);
        set!(self.surface; popover, button);
        set!(self.raised; muted, secondary, button_secondary, list_even, table_even, skeleton, switch, slider_bar, progress_bar, tab_bar_segmented);
        set!(self.hover; accent, button_hover, secondary_hover, button_secondary_hover, list_hover, table_hover, sidebar_accent);
        set!(self.pressed; button_active, secondary_active, button_secondary_active);
        set!(self.text; foreground, button_foreground, popover_foreground, accent_foreground, secondary_foreground, button_secondary_foreground, tab_active_foreground, sidebar_foreground, sidebar_accent_foreground, table_head_foreground, table_foot_foreground, group_box_foreground, description_list_label_foreground);
        set!(self.muted; muted_foreground, tab_foreground, scrollbar_thumb);
        set!(self.input; input, scrollbar_thumb_hover);
        set!(self.border; border, title_bar_border, status_bar_border, sidebar_border, table_row_border, window_border);
        set!(self.primary; primary, button_primary, ring, drag_border, list_active_border, table_active_border, sidebar_primary, slider_thumb, caret);
        set!(self.primary_hover; primary_hover, button_primary_hover);
        set!(self.primary_active; primary_active, button_primary_active);
        set!(self.primary_text; primary_foreground, button_primary_foreground, sidebar_primary_foreground);
        set!(self.selection; selection);
        set!(self.active; list_active, table_active, drop_target);
        set!(self.link; link, link_hover, link_active);
        set!(self.danger; danger, danger_hover, danger_active, button_danger, button_danger_hover, button_danger_active);
        set!(self.success; success, success_hover, success_active, button_success, button_success_hover, button_success_active);
        set!(self.warning; warning, warning_hover, warning_active, button_warning, button_warning_hover, button_warning_active);
        set!(self.primary; info, info_hover, info_active, button_info, button_info_hover, button_info_active);
        set!(self.primary_text; danger_foreground, button_danger_foreground, success_foreground, button_success_foreground, warning_foreground, button_warning_foreground, info_foreground, button_info_foreground);
        theme.colors.scrollbar = rgb(self.chrome).opacity(0.6);
        theme.colors.switch_thumb = rgb(self.primary_text);
        theme.colors.overlay = rgb(0x0B1220).opacity(0.55);
    }
}
