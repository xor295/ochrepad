//! Box-drawing characters and block elements drawn as shapes filling their
//! cell, as terminals do: a font's glyphs for them rarely span the whole
//! line height, which leaves gaps in frames and bars drawn from them.

use egui::{Color32, Painter, Pos2, Rect, Shape, Stroke, Vec2};

/// A line's weight on one side of a box-drawing character.
#[derive(Clone, Copy, PartialEq, Eq)]
enum W {
    No,
    Light,
    Heavy,
    Double,
}

use W::{Double as D, Heavy as H, Light as L, No as N};

/// The lines from the cell's centre to its left, right, top and bottom
/// edges.
fn arms(c: char) -> Option<[W; 4]> {
    Some(match c {
        '─' | '┄' | '┈' | '╌' => [L, L, N, N],
        '━' | '┅' | '┉' | '╍' => [H, H, N, N],
        '│' | '┆' | '┊' | '╎' => [N, N, L, L],
        '┃' | '┇' | '┋' | '╏' => [N, N, H, H],
        '┌' => [N, L, N, L],
        '┐' => [L, N, N, L],
        '└' => [N, L, L, N],
        '┘' => [L, N, L, N],
        '├' => [N, L, L, L],
        '┤' => [L, N, L, L],
        '┬' => [L, L, N, L],
        '┴' => [L, L, L, N],
        '┼' => [L, L, L, L],
        '┏' => [N, H, N, H],
        '┓' => [H, N, N, H],
        '┗' => [N, H, H, N],
        '┛' => [H, N, H, N],
        '┣' => [N, H, H, H],
        '┫' => [H, N, H, H],
        '┳' => [H, H, N, H],
        '┻' => [H, H, H, N],
        '╋' => [H, H, H, H],
        '═' => [D, D, N, N],
        '║' => [N, N, D, D],
        '╔' => [N, D, N, D],
        '╗' => [D, N, N, D],
        '╚' => [N, D, D, N],
        '╝' => [D, N, D, N],
        '╠' => [N, D, D, D],
        '╣' => [D, N, D, D],
        '╦' => [D, D, N, D],
        '╩' => [D, D, D, N],
        '╬' => [D, D, D, D],
        '╴' => [L, N, N, N],
        '╵' => [N, N, L, N],
        '╶' => [N, L, N, N],
        '╷' => [N, N, N, L],
        '╸' => [H, N, N, N],
        '╹' => [N, N, H, N],
        '╺' => [N, H, N, N],
        '╻' => [N, N, N, H],
        _ => return None,
    })
}

/// Draws `c` in `cell` if it is a box-drawing character or a block element
/// this knows; returns whether it did.
pub fn draw(painter: &Painter, cell: Rect, c: char, color: Color32) -> bool {
    // Whole physical pixels wide, centred on a pixel, so the lines are crisp
    // and the same in every cell.
    let ppp = painter.pixels_per_point();
    let light = ((cell.height() * ppp / 14.0).round().max(1.0)) / ppp;
    let snap = |v: f32| ((v * ppp).floor() + 0.5) / ppp;
    let center = Pos2::new(snap(cell.center().x), snap(cell.center().y));
    // Arms reach a little past the cell's edges, so a line drawn across many
    // cells shows no seams where they meet.
    let reach = 0.5 / ppp;
    if let Some([left, right, up, down]) = arms(c) {
        let ends = [
            (left, Pos2::new(cell.min.x - reach, center.y)),
            (right, Pos2::new(cell.max.x + reach, center.y)),
            (up, Pos2::new(center.x, cell.min.y - reach)),
            (down, Pos2::new(center.x, cell.max.y + reach)),
        ];
        for (i, (w, end)) in ends.into_iter().enumerate() {
            let horizontal = i < 2;
            // A line from the centre, overlapping it by half a line so the
            // arms join without a notch.
            let along = if horizontal {
                Vec2::new((end.x - center.x).signum(), 0.0)
            } else {
                Vec2::new(0.0, (end.y - center.y).signum())
            };
            let start = |width: f32| center - along * width / 2.0;
            match w {
                N => {}
                L | H => {
                    let width = if w == H { light * 2.0 } else { light };
                    painter.line_segment([start(width), end], Stroke::new(width, color));
                }
                D => {
                    let gap = light * 1.5;
                    let side = Vec2::new(along.y, along.x) * gap;
                    for s in [side, -side] {
                        painter.line_segment(
                            [start(gap * 2.0 + light) + s, end + s],
                            Stroke::new(light, color),
                        );
                    }
                }
            }
        }
        return true;
    }
    // Rounded corners: a quarter circle between two edge midpoints.
    let corner = match c {
        '╭' => Some((
            Pos2::new(cell.max.x, center.y),
            Pos2::new(center.x, cell.max.y),
        )),
        '╮' => Some((
            Pos2::new(cell.min.x, center.y),
            Pos2::new(center.x, cell.max.y),
        )),
        '╯' => Some((
            Pos2::new(cell.min.x, center.y),
            Pos2::new(center.x, cell.min.y),
        )),
        '╰' => Some((
            Pos2::new(cell.max.x, center.y),
            Pos2::new(center.x, cell.min.y),
        )),
        _ => None,
    };
    if let Some((a, b)) = corner {
        // `a` is on the left or right edge, `b` on the top or bottom one.
        // The lines run straight in from them, then turn on a circle that
        // touches both, centred off the cell's centre toward the corner.
        let (sx, sy) = ((a.x - center.x).signum(), (b.y - center.y).signum());
        let r = (cell.width() / 2.0).min(cell.height() / 2.0);
        let pivot = center + Vec2::new(sx * r, sy * r);
        let a_in = Pos2::new(center.x + sx * r, center.y);
        let b_in = Pos2::new(center.x, center.y + sy * r);
        let start = (a_in - pivot).angle();
        let end = (b_in - pivot).angle();
        let mut sweep = end - start;
        if sweep > std::f32::consts::PI {
            sweep -= std::f32::consts::TAU;
        } else if sweep < -std::f32::consts::PI {
            sweep += std::f32::consts::TAU;
        }
        let mut points = vec![a];
        for i in 0..=12 {
            let t = start + sweep * i as f32 / 12.0;
            points.push(pivot + r * Vec2::angled(t));
        }
        points.push(b);
        painter.add(Shape::line(points, Stroke::new(light, color)));
        return true;
    }
    block(painter, cell, c, color)
}

/// Block elements (U+2580–U+259F): parts of the cell filled, and shades.
fn block(painter: &Painter, cell: Rect, c: char, color: Color32) -> bool {
    let (w, h) = (cell.width(), cell.height());
    let part = |x0: f32, y0: f32, x1: f32, y1: f32| {
        Rect::from_min_max(
            cell.min + Vec2::new(w * x0, h * y0),
            cell.min + Vec2::new(w * x1, h * y1),
        )
    };
    let code = c as u32;
    let rects: Vec<Rect> = match c {
        '▀' => vec![part(0.0, 0.0, 1.0, 0.5)],
        // ▁▂▃▄▅▆▇█: the lower eighths.
        '▁'..='█' => {
            let n = (code - 0x2580) as f32;
            vec![part(0.0, 1.0 - n / 8.0, 1.0, 1.0)]
        }
        // ▉▊▋▌▍▎▏: the left eighths, from seven down to one.
        '▉'..='▏' => {
            let n = (0x2590 - code) as f32;
            vec![part(0.0, 0.0, n / 8.0, 1.0)]
        }
        '▐' => vec![part(0.5, 0.0, 1.0, 1.0)],
        '▔' => vec![part(0.0, 0.0, 1.0, 0.125)],
        '▕' => vec![part(0.875, 0.0, 1.0, 1.0)],
        '░' | '▒' | '▓' => {
            let alpha = match c {
                '░' => 0.25,
                '▒' => 0.5,
                _ => 0.75,
            };
            painter.rect_filled(cell, 0.0, color.gamma_multiply(alpha));
            return true;
        }
        _ => {
            // Quadrants ▖▗▘▙▚▛▜▝▞▟: which of the four are filled.
            let [tl, tr, bl, br] = match c {
                '▖' => [false, false, true, false],
                '▗' => [false, false, false, true],
                '▘' => [true, false, false, false],
                '▙' => [true, false, true, true],
                '▚' => [true, false, false, true],
                '▛' => [true, true, true, false],
                '▜' => [true, true, false, true],
                '▝' => [false, true, false, false],
                '▞' => [false, true, true, false],
                '▟' => [false, true, true, true],
                _ => return false,
            };
            [
                (tl, part(0.0, 0.0, 0.5, 0.5)),
                (tr, part(0.5, 0.0, 1.0, 0.5)),
                (bl, part(0.0, 0.5, 0.5, 1.0)),
                (br, part(0.5, 0.5, 1.0, 1.0)),
            ]
            .into_iter()
            .filter_map(|(on, r)| on.then_some(r))
            .collect()
        }
    };
    for r in rects {
        painter.rect_filled(r, 0.0, color);
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn known_characters_are_drawn_and_others_left_to_the_font() {
        let ctx = egui::Context::default();
        let painter = Painter::new(ctx, egui::LayerId::background(), Rect::EVERYTHING);
        let cell = Rect::from_min_size(Pos2::ZERO, Vec2::new(8.0, 16.0));
        for c in "─│┌┐└┘├┤┬┴┼━┃╋═║╬╭╮╯╰▀▄█▌▐░▒▓▖▟▁▉".chars()
        {
            assert!(draw(&painter, cell, c, Color32::WHITE), "{c}");
        }
        for c in "a✔你".chars() {
            assert!(!draw(&painter, cell, c, Color32::WHITE), "{c}");
        }
    }
}
