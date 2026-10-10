//! The look of ochrepad: warm neutral greys (charcoal in dark mode, paper in
//! light mode) with ochre (#CC7722) as the one accent colour, a taller tab
//! bar, and the logo (a pad with lines of text) for the menu bar and the
//! window icon.

use egui::epaint::Shadow;
use egui::{
    Color32, CornerRadius, Margin, Painter, Pos2, Rect, Stroke, Style, Theme, Vec2, Visuals,
};

/// The accent colour.
pub const OCHRE: Color32 = Color32::from_rgb(0xCC, 0x77, 0x22);

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

/// Background of find-panel matches in the editor: teal, the complement of
/// ochre, so matches never look like the (ochre) selection.
pub fn match_bg(visuals: &Visuals) -> Color32 {
    if visuals.dark_mode {
        Color32::from_rgba_unmultiplied(70, 170, 175, 70)
    } else {
        Color32::from_rgba_unmultiplied(40, 160, 165, 75)
    }
}

/// The dock (tab bar, split separators, drop overlay).
pub fn dock_style(style: &Style) -> egui_dock::Style {
    let v = &style.visuals;
    let mut d = egui_dock::Style::from_egui(style);
    let border = v.widgets.noninteractive.bg_stroke.color;
    let weak = v.weak_text_color();

    d.tab_bar.height = 32.0;
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
    t.minimum_width = Some(96.0);
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

// --- Logo ------------------------------------------------------------------------

/// Cream lines of text on the pad.
const PAPER: Color32 = Color32::from_rgb(0xFF, 0xF3, 0xE2);

/// The logo in the unit square: the pad (rounded square, corner radius) and
/// its lines of text (each a capsule: rect, radius).
fn logo_shapes() -> ((Rect, f32), [(Rect, f32); 3]) {
    let pad = (
        Rect::from_min_max(Pos2::new(0.03, 0.03), Pos2::new(0.97, 0.97)),
        0.24,
    );
    let h = 0.1;
    let line = |y: f32, len: f32| {
        (
            Rect::from_min_size(Pos2::new(0.24, y - h / 2.0), Vec2::new(len, h)),
            h / 2.0,
        )
    };
    (pad, [line(0.34, 0.52), line(0.5, 0.36), line(0.66, 0.46)])
}

/// Draws the logo into `rect` (a square).
pub fn paint_logo(painter: &Painter, rect: Rect) {
    let map = |r: Rect| {
        Rect::from_min_max(
            rect.min + r.min.to_vec2() * rect.size(),
            rect.min + r.max.to_vec2() * rect.size(),
        )
    };
    let s = rect.width();
    let (pad, lines) = logo_shapes();
    painter.rect_filled(map(pad.0), pad.1 * s, OCHRE);
    for (r, radius) in lines {
        painter.rect_filled(map(r), radius * s, PAPER);
    }
}

/// The logo as a window icon (`size` × `size` pixels, antialiased).
pub fn icon(size: u32) -> egui::IconData {
    fn inside((r, radius): (Rect, f32), p: Pos2) -> bool {
        let inner = r.shrink(radius);
        let dx = (inner.left() - p.x).max(p.x - inner.right()).max(0.0);
        let dy = (inner.top() - p.y).max(p.y - inner.bottom()).max(0.0);
        dx * dx + dy * dy <= radius * radius
    }
    const SS: u32 = 4; // samples per axis
    let (pad, lines) = logo_shapes();
    let mut rgba = Vec::with_capacity((size * size * 4) as usize);
    for y in 0..size {
        for x in 0..size {
            let (mut on_pad, mut on_line) = (0u32, 0u32);
            for sy in 0..SS {
                for sx in 0..SS {
                    let p = Pos2::new(
                        (x as f32 + (sx as f32 + 0.5) / SS as f32) / size as f32,
                        (y as f32 + (sy as f32 + 0.5) / SS as f32) / size as f32,
                    );
                    if inside(pad, p) {
                        on_pad += 1;
                        if lines.iter().any(|&l| inside(l, p)) {
                            on_line += 1;
                        }
                    }
                }
            }
            let n = (SS * SS) as f32;
            let t = if on_pad == 0 {
                0.0
            } else {
                on_line as f32 / on_pad as f32
            };
            let mix = |a: u8, b: u8| (f32::from(a) * (1.0 - t) + f32::from(b) * t).round() as u8;
            rgba.extend_from_slice(&[
                mix(OCHRE.r(), PAPER.r()),
                mix(OCHRE.g(), PAPER.g()),
                mix(OCHRE.b(), PAPER.b()),
                (on_pad as f32 / n * 255.0).round() as u8,
            ]);
        }
    }
    egui::IconData {
        rgba,
        width: size,
        height: size,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn icon_is_ochre_with_transparent_corners_and_cream_lines() {
        let icon = icon(64);
        assert_eq!(icon.rgba.len(), 64 * 64 * 4);
        let px = |x: usize, y: usize| &icon.rgba[(y * 64 + x) * 4..(y * 64 + x) * 4 + 4];
        assert_eq!(px(0, 0)[3], 0, "rounded corner is transparent");
        assert_eq!(px(58, 40), &[OCHRE.r(), OCHRE.g(), OCHRE.b(), 255]);
        assert_eq!(px(20, 22), &[PAPER.r(), PAPER.g(), PAPER.b(), 255]);
    }
}
