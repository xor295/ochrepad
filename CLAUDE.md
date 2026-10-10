Follow PLAN.md; current milestone is M13 (done on branch `m13-polish`, awaiting review; this was the last milestone in the plan)

## Decisions made after M0 (apply when building the editor widget, M3)

- CJK text: load a CJK fallback font from the system font directory at startup (egui's built-in fonts have no CJK glyphs). If none is found, keep rendering (tofu boxes stay on the grid).
- Emoji: clip each grapheme to its grid slot (`VisualCol` width × cell width). egui does not compose modifier/ZWJ sequences, so e.g. 👨‍👩‍👧 draws as several glyphs and would otherwise overflow.
- Scrolling: do not use egui `ScrollArea` for the editor. Keep scroll as first visible line + fractional offset (f32 pixel offsets lose precision for large files).
- The core crate lives in `crates/core` but the package is `ochrepad-core`, since a crate named `core` clashes with Rust's built-in `core`.
- Windows exe icon: `winresource` (build-dependency, Windows hosts only, approved after M13). `build.rs` generates the .ico from `crates/app/src/logo.rs`, the same code that draws the window icon.
- Markdown preview images load only from this machine (`crates/app/src/safe_images.rs`): a guard loader refuses `file://` network/UNC paths (they would send NTLM credentials), and SVGs render with our own loader whose `<image>` resolver reads only files inside the SVG's folder. `resvg` (=0.45.1, no default features, the version egui_extras already uses) is a direct dependency for that (approved).
