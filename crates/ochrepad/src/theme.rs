//! The look of ochrepad: warm neutral greys (charcoal in dark mode, paper in
//! light mode) with ochre (#CC7722) as the one accent colour, and a taller
//! tab bar.

use egui::epaint::Shadow;
use egui::{
    Color32, CornerRadius, Margin, Painter, Pos2, Rect, Stroke, Style, Theme, Vec2, Visuals,
};

/// Spacing, in points: a small set used for margins and gaps so panels
/// line up with each other.
pub mod space {
    pub const XS: f32 = 4.0;
    pub const S: f32 = 8.0;
    pub const M: f32 = 12.0;
    pub const L: f32 = 16.0;
}

/// Height of the tab bar, and of the sidebar's header beside it.
pub const BAR_H: f32 = 36.0;

/// The accent colour.
pub const OCHRE: Color32 = Color32::from_rgb(
    crate::logo::OCHRE[0],
    crate::logo::OCHRE[1],
    crate::logo::OCHRE[2],
);

/// The colours one theme is built from.
struct Palette {
    dark: bool,
    /// Editor background (egui's `extreme_bg_color`).
    editor: Color32,
    /// Menu bar, tab bar and status bar.
    panel: Color32,
    /// Current line, gutter of the compare view, code blocks.
    faint: Color32,
    window: Color32,
    border: Color32,
    widget: Color32,
    hovered: Color32,
    pressed: Color32,
    text: Color32,
    strong: Color32,
    /// Accent for strokes and text (caret, focus ring, links).
    accent: Color32,
    /// Text drawn on the selection.
    on_selection: Color32,
    selection_alpha: u8,
    shadow_alpha: u8,
}

const DARK: Palette = Palette {
    dark: true,
    editor: Color32::from_rgb(0x1B, 0x19, 0x17),
    panel: Color32::from_rgb(0x23, 0x20, 0x1D),
    faint: Color32::from_rgb(0x27, 0x24, 0x20),
    window: Color32::from_rgb(0x26, 0x23, 0x20),
    border: Color32::from_rgb(0x36, 0x32, 0x2D),
    widget: Color32::from_rgb(0x35, 0x31, 0x2C),
    hovered: Color32::from_rgb(0x41, 0x3B, 0x35),
    pressed: Color32::from_rgb(0x4D, 0x46, 0x3E),
    text: Color32::from_rgb(0xD9, 0xD2, 0xC7),
    strong: Color32::from_rgb(0xF5, 0xF0, 0xE8),
    accent: Color32::from_rgb(0xE0, 0x8A, 0x3A),
    on_selection: Color32::from_rgb(0xF4, 0xC4, 0x8C),
    selection_alpha: 92,
    shadow_alpha: 120,
};

const LIGHT: Palette = Palette {
    dark: false,
    editor: Color32::from_rgb(0xFF, 0xFD, 0xF9),
    panel: Color32::from_rgb(0xF2, 0xEE, 0xE8),
    faint: Color32::from_rgb(0xF6, 0xF1, 0xE9),
    window: Color32::from_rgb(0xFB, 0xF9, 0xF5),
    border: Color32::from_rgb(0xDD, 0xD5, 0xCA),
    widget: Color32::from_rgb(0xE8, 0xE2, 0xD9),
    hovered: Color32::from_rgb(0xDE, 0xD6, 0xCA),
    pressed: Color32::from_rgb(0xD1, 0xC7, 0xB8),
    text: Color32::from_rgb(0x3A, 0x34, 0x2E),
    strong: Color32::from_rgb(0x1E, 0x1A, 0x16),
    accent: Color32::from_rgb(0xB0, 0x5F, 0x16),
    on_selection: Color32::from_rgb(0x6B, 0x38, 0x08),
    selection_alpha: 72,
    shadow_alpha: 40,
};

impl Palette {
    fn visuals(&self) -> Visuals {
        let mut v = if self.dark {
            Visuals::dark()
        } else {
            Visuals::light()
        };
        let radius = CornerRadius::same(5);
        let w = &mut v.widgets;
        w.noninteractive.bg_fill = self.window;
        w.noninteractive.weak_bg_fill = self.window;
        w.noninteractive.bg_stroke = Stroke::new(1.0, self.border);
        w.noninteractive.fg_stroke = Stroke::new(1.0, self.text);
        w.noninteractive.corner_radius = radius;
        for (state, fill, fg) in [
            (&mut w.inactive, self.widget, self.text),
            (&mut w.hovered, self.hovered, self.strong),
            (&mut w.active, self.pressed, self.strong),
            (&mut w.open, self.hovered, self.strong),
        ] {
            state.bg_fill = fill;
            state.weak_bg_fill = fill;
            state.bg_stroke = Stroke::NONE;
            state.fg_stroke = Stroke::new(1.0, fg);
            state.corner_radius = radius;
            state.expansion = 0.0;
        }
        w.hovered.bg_stroke = Stroke::new(1.0, self.border);
        w.active.bg_stroke = Stroke::new(1.0, self.accent);

        v.selection.bg_fill =
            Color32::from_rgba_unmultiplied(OCHRE.r(), OCHRE.g(), OCHRE.b(), self.selection_alpha);
        v.selection.stroke = Stroke::new(1.0, self.on_selection);
        v.hyperlink_color = self.accent;
        v.text_cursor.stroke = Stroke::new(2.0, self.accent);
        v.extreme_bg_color = self.editor;
        v.text_edit_bg_color = Some(self.editor);
        v.faint_bg_color = self.faint;
        v.code_bg_color = self.faint;
        v.panel_fill = self.panel;
        v.window_fill = self.window;
        v.window_stroke = Stroke::new(1.0, self.border);
        v.window_corner_radius = CornerRadius::same(10);
        v.menu_corner_radius = CornerRadius::same(8);
        let shadow = Shadow {
            offset: [0, 6],
            blur: 22,
            spread: 0,
            color: Color32::from_black_alpha(self.shadow_alpha),
        };
        v.window_shadow = shadow;
        v.popup_shadow = Shadow {
            offset: [0, 4],
            blur: 14,
            ..shadow
        };
        v.window_highlight_topmost = false;
        v
    }
}

/// Installs the dark and light visuals and the spacing; egui picks the
/// visuals that match the theme (System, Dark or Light).
pub fn install(ctx: &egui::Context) {
    ctx.set_visuals_of(Theme::Dark, DARK.visuals());
    ctx.set_visuals_of(Theme::Light, LIGHT.visuals());
    ctx.all_styles_mut(|style| {
        let s = &mut style.spacing;
        s.item_spacing = Vec2::new(8.0, 5.0);
        s.button_padding = Vec2::new(8.0, 3.0);
        s.menu_margin = Margin::same(6);
        s.window_margin = Margin::same(14);
        s.interact_size.y = 22.0;
        s.menu_spacing = 4.0;
    });
}

/// Accent colour for strokes and text in the current theme (the caret colour).
pub fn accent(visuals: &Visuals) -> Color32 {
    visuals.text_cursor.stroke.color
}

/// A diagnostic's colour by rank (0 an error, 1 a warning, 2 information,
/// 3 a hint): red, yellow, blue, grey. Never the ochre accent, which means
/// the selection and the caret.
pub fn diagnostic_color(visuals: &Visuals, rank: u8) -> Color32 {
    let dark = visuals.dark_mode;
    match rank {
        0 => {
            if dark {
                Color32::from_rgb(0xF0, 0x5A, 0x50)
            } else {
                Color32::from_rgb(0xD0, 0x2E, 0x2E)
            }
        }
        1 => {
            if dark {
                Color32::from_rgb(0xE8, 0xD0, 0x40)
            } else {
                Color32::from_rgb(0xB0, 0x90, 0x00)
            }
        }
        2 => {
            if dark {
                Color32::from_rgb(0x5A, 0xA8, 0xF0)
            } else {
                Color32::from_rgb(0x1F, 0x6F, 0xB8)
            }
        }
        _ => visuals.weak_text_color(),
    }
}

/// Background of find-panel matches in the editor: teal, the complement of
/// ochre, so matches never look like the (ochre) selection.
pub fn match_bg(visuals: &Visuals) -> Color32 {
    if visuals.dark_mode {
        Color32::from_rgba_unmultiplied(70, 170, 175, 70)
    } else {
        Color32::from_rgba_unmultiplied(40, 160, 165, 75)
    }
}

/// Background of smart-highlighted text (other occurrences of the
/// selection): a faint ochre, so it reads as a lighter echo of the selection
/// and stays distinct from the teal find matches.
pub fn smart_highlight_bg(visuals: &Visuals) -> Color32 {
    let alpha = if visuals.dark_mode { 40 } else { 34 };
    Color32::from_rgba_unmultiplied(OCHRE.r(), OCHRE.g(), OCHRE.b(), alpha)
}

/// The colour of a kind of change, as straight RGB (tints are built from
/// it with an alpha): removed red, added green, and changed blue, so it
/// never looks like the ochre selection. The compare view and the change
/// history use it.
pub fn diff_rgb(kind: ochrepad_core::diff::RowKind) -> (u8, u8, u8) {
    match kind {
        ochrepad_core::diff::RowKind::Removed => (225, 70, 70),
        ochrepad_core::diff::RowKind::Added => (60, 185, 90),
        _ => (80, 145, 235),
    }
}

/// A bookmarked line's number sits on a tag in the accent colour
/// (Search > Bookmark)...
pub fn bookmark_bg(visuals: &Visuals) -> Color32 {
    accent(visuals)
}

/// ...drawn in the editor's background colour.
pub fn bookmark_fg(visuals: &Visuals) -> Color32 {
    visuals.extreme_bg_color
}

/// The terminal's 16 ANSI colours (black, red, green, yellow, blue,
/// magenta, cyan, white, then the bright ones), warmed to sit with the
/// charcoal and paper themes. The default text and background are the
/// editor's.
pub fn terminal_palette(dark: bool) -> [Color32; 16] {
    let c = Color32::from_rgb;
    if dark {
        [
            c(0x3B, 0x36, 0x31),
            c(0xE0, 0x6C, 0x5B),
            c(0xA3, 0xBE, 0x6E),
            c(0xE0, 0xB0, 0x4A),
            c(0x6F, 0xA3, 0xD6),
            c(0xC5, 0x8A, 0xC9),
            c(0x6F, 0xBF, 0xB3),
            c(0xD9, 0xD2, 0xC7),
            c(0x7A, 0x71, 0x6A),
            c(0xF0, 0x8A, 0x7A),
            c(0xBF, 0xD8, 0x8A),
            c(0xF2, 0xCC, 0x6E),
            c(0x92, 0xBE, 0xEA),
            c(0xDC, 0xA8, 0xE0),
            c(0x91, 0xD9, 0xCE),
            c(0xF5, 0xF0, 0xE8),
        ]
    } else {
        [
            c(0x3A, 0x34, 0x2E),
            c(0xB8, 0x3A, 0x2B),
            c(0x4E, 0x7A, 0x1E),
            c(0x9A, 0x6A, 0x00),
            c(0x2F, 0x64, 0xA8),
            c(0x8E, 0x3E, 0x96),
            c(0x1E, 0x7A, 0x70),
            c(0xC9, 0xC0, 0xB3),
            c(0x7A, 0x71, 0x6A),
            c(0xD2, 0x51, 0x3F),
            c(0x5F, 0x92, 0x27),
            c(0xB5, 0x82, 0x00),
            c(0x3F, 0x7B, 0xC4),
            c(0xA8, 0x52, 0xB0),
            c(0x2A, 0x94, 0x8A),
            c(0xE6, 0xDF, 0xD4),
        ]
    }
}

/// The dock (tab bar, split separators, drop overlay).
pub fn dock_style(style: &Style) -> egui_dock::Style {
    let v = &style.visuals;
    let mut d = egui_dock::Style::from_egui(style);
    let border = v.widgets.noninteractive.bg_stroke.color;
    let weak = v.weak_text_color();

    d.tab_bar.height = BAR_H;
    d.tab_bar.bg_fill = v.panel_fill;
    d.tab_bar.hline_color = border;
    d.tab_bar.corner_radius = CornerRadius::ZERO;

    let top = CornerRadius {
        nw: 6,
        ne: 6,
        sw: 0,
        se: 0,
    };
    let current = egui_dock::TabInteractionStyle {
        outline_color: border,
        corner_radius: top,
        bg_fill: v.extreme_bg_color,
        text_color: v.strong_text_color(),
    };
    let other = egui_dock::TabInteractionStyle {
        outline_color: Color32::TRANSPARENT,
        corner_radius: top,
        bg_fill: v.panel_fill,
        text_color: weak,
    };
    let t = &mut d.tab;
    t.focused = current.clone();
    t.focused_with_kb_focus = current.clone();
    t.active = egui_dock::TabInteractionStyle {
        text_color: v.text_color(),
        ..current
    };
    t.active_with_kb_focus = t.active.clone();
    t.inactive = other.clone();
    t.inactive_with_kb_focus = other.clone();
    t.hovered = egui_dock::TabInteractionStyle {
        bg_fill: v.faint_bg_color,
        text_color: v.text_color(),
        ..other
    };
    t.minimum_width = Some(104.0);
    t.spacing = 2.0;
    t.tab_body.bg_fill = v.extreme_bg_color;
    t.tab_body.stroke = Stroke::NONE;
    t.tab_body.inner_margin = Margin::ZERO;

    d.buttons.close_tab_color = weak;
    d.buttons.close_tab_active_color = v.strong_text_color();
    d.buttons.close_tab_bg_fill = v.widgets.hovered.bg_fill;

    d.separator.width = 1.0;
    d.separator.extra_interact_width = 4.0;
    d.separator.color_idle = border;
    d.separator.color_hovered = OCHRE;
    d.separator.color_dragged = OCHRE;

    d.overlay.selection_color = OCHRE.gamma_multiply(0.35);
    d.overlay.button_color = v.text_color();
    d
}

/// The ochre line on top of a pane's active tab (bright in the focused
/// pane, faint in the others).
pub fn paint_tab_accent(painter: &Painter, tab: Rect, focused_pane: bool) {
    let color = if focused_pane {
        OCHRE
    } else {
        OCHRE.gamma_multiply(0.4)
    };
    let line = Rect::from_min_max(
        tab.left_top() + Vec2::new(1.0, 0.0),
        Pos2::new(tab.right() - 1.0, tab.top() + 2.0),
    );
    painter.rect_filled(line, CornerRadius::same(1), color);
}

/// The logo as a window icon (`size` × `size` pixels).
pub fn icon(size: u32) -> egui::IconData {
    egui::IconData {
        rgba: crate::logo::rgba(size),
        width: size,
        height: size,
    }
}
