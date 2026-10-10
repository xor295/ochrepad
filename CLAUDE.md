Follow PLAN.md; current milestone is M8 (done on branch `m8-transforms`, awaiting review; next is M9)

## Decisions made after M0 (apply when building the editor widget, M3)

- CJK text: load a CJK fallback font from the system font directory at startup (egui's built-in fonts have no CJK glyphs). If none is found, keep rendering (tofu boxes stay on the grid).
- Emoji: clip each grapheme to its grid slot (`VisualCol` width × cell width). egui does not compose modifier/ZWJ sequences, so e.g. 👨‍👩‍👧 draws as several glyphs and would otherwise overflow.
- Scrolling: do not use egui `ScrollArea` for the editor. Keep scroll as first visible line + fractional offset (f32 pixel offsets lose precision for large files).
- The core crate lives in `crates/core` but the package is `ochrepad-core`, since a crate named `core` clashes with Rust's built-in `core`.
