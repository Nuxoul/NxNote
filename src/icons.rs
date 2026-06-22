//! Material Icons 字形常量。codepoint 取自 Google Material Icons (legacy) 字体。

#![allow(dead_code)]

pub const ICON_FAMILY: &str = "material_icons";

pub fn font(size: f32) -> egui::FontId {
    egui::FontId::new(size, egui::FontFamily::Name(ICON_FAMILY.into()))
}

pub fn rich(glyph: &'static str, size: f32) -> egui::RichText {
    egui::RichText::new(glyph)
        .family(egui::FontFamily::Name(ICON_FAMILY.into()))
        .size(size)
}

// 通用
pub const SETTINGS: &str = "\u{E8B8}";        // settings (齿轮)
pub const CLOSE: &str = "\u{E5CD}";           // close (X)
pub const MINIMIZE: &str = "\u{E15B}";        // remove (−)
pub const MAXIMIZE: &str = "\u{E3C6}";        // crop_square (□)
pub const VISIBILITY: &str = "\u{E8F4}";      // visibility (眼)
pub const EDIT: &str = "\u{E3C9}";            // edit (笔)
pub const ADD: &str = "\u{E145}";             // add (+)
pub const DELETE: &str = "\u{E872}";          // delete (垃圾桶)
pub const FOLDER: &str = "\u{E2C7}";          // folder
pub const DESCRIPTION: &str = "\u{E873}";     // description (文件)
pub const PIN: &str = "\u{E866}";             // bookmark (实心)
pub const PIN_OFF: &str = "\u{E867}";         // bookmark_border (空心)
pub const TARGET: &str = "\u{E55C}";          // my_location (准星)
pub const ARROW_DROP_DOWN: &str = "\u{E5C5}"; // arrow_drop_down
pub const CHECK: &str = "\u{E5CA}";           // check (√)
pub const SEARCH: &str = "\u{E8B6}";          // search
pub const HISTORY: &str = "\u{E889}";         // history
pub const APPS: &str = "\u{E5C3}";            // apps (九宫格)
pub const COMPUTER: &str = "\u{E30A}";        // computer
pub const KEYBOARD: &str = "\u{E312}";        // keyboard
