//! Font setup. egui's built-in fonts have no CJK glyphs and few symbols
//! (no ✓ or ✗), so a CJK font and then a symbol font are loaded from the
//! system font directory and appended as fallbacks. egui picks a font per
//! character, so the fallbacks only draw characters the fonts before them
//! lack, and row height comes from the first font only. The menus and panels
//! use the system's UI font where there is a known one.

use std::path::PathBuf;
use std::sync::Arc;

use egui::{FontData, FontDefinitions, FontFamily};

fn cjk_candidates() -> Vec<PathBuf> {
    let mut out = Vec::new();
    if cfg!(windows) {
        let root = std::env::var_os("SystemRoot")
            .map_or_else(|| PathBuf::from(r"C:\Windows"), PathBuf::from);
        for name in [
            "msyh.ttc",
            "msgothic.ttc",
            "YuGothM.ttc",
            "meiryo.ttc",
            "simsun.ttc",
            "malgun.ttf",
        ] {
            out.push(root.join("Fonts").join(name));
        }
    } else {
        for p in [
            "/usr/share/fonts/opentype/noto/NotoSansCJK-Regular.ttc",
            "/usr/share/fonts/noto-cjk/NotoSansCJK-Regular.ttc",
            "/usr/share/fonts/google-noto-cjk/NotoSansCJK-Regular.ttc",
            "/usr/share/fonts/truetype/noto/NotoSansCJK-Regular.ttc",
            "/usr/share/fonts/truetype/wqy/wqy-microhei.ttc",
            "/usr/share/fonts/wenquanyi/wqy-microhei/wqy-microhei.ttc",
        ] {
            out.push(PathBuf::from(p));
        }
    }
    out
}

/// A font with broad symbol coverage (dingbats, arrows, math), tried after
/// the CJK font.
fn symbol_candidates() -> Vec<PathBuf> {
    if cfg!(windows) {
        let root = std::env::var_os("SystemRoot")
            .map_or_else(|| PathBuf::from(r"C:\Windows"), PathBuf::from);
        vec![root.join("Fonts").join("seguisym.ttf")]
    } else {
        [
            "/usr/share/fonts/truetype/dejavu/DejaVuSansMono.ttf",
            "/usr/share/fonts/dejavu/DejaVuSansMono.ttf",
            "/usr/share/fonts/TTF/DejaVuSansMono.ttf",
            "/usr/share/fonts/truetype/noto/NotoSansSymbols2-Regular.ttf",
            "/usr/share/fonts/noto/NotoSansSymbols2-Regular.ttf",
        ]
        .into_iter()
        .map(PathBuf::from)
        .collect()
    }
}

/// The system UI font (static fonts only), used ahead of egui's built-in
/// proportional font.
fn ui_font_candidates() -> Vec<PathBuf> {
    if cfg!(windows) {
        let root = std::env::var_os("SystemRoot")
            .map_or_else(|| PathBuf::from(r"C:\Windows"), PathBuf::from);
        vec![root.join("Fonts").join("segoeui.ttf")]
    } else {
        [
            "/usr/share/fonts/truetype/inter/Inter-Regular.ttf",
            "/usr/share/fonts/opentype/inter/Inter-Regular.otf",
            "/usr/share/fonts/cantarell/Cantarell-Regular.otf",
            "/usr/share/fonts/truetype/noto/NotoSans-Regular.ttf",
            "/usr/share/fonts/noto/NotoSans-Regular.ttf",
            "/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf",
        ]
        .into_iter()
        .map(PathBuf::from)
        .collect()
    }
}

/// Monospace fonts installed on this system, as (name, path).
pub fn monospace_choices() -> Vec<(String, PathBuf)> {
    let candidates: Vec<(&str, PathBuf)> = if cfg!(windows) {
        let root = std::env::var_os("SystemRoot")
            .map_or_else(|| PathBuf::from(r"C:\Windows"), PathBuf::from)
            .join("Fonts");
        [
            ("Consolas", "consola.ttf"),
            ("Cascadia Mono", "CascadiaMono.ttf"),
            ("Cascadia Code", "CascadiaCode.ttf"),
            ("Courier New", "cour.ttf"),
            ("Lucida Console", "lucon.ttf"),
        ]
        .into_iter()
        .map(|(n, f)| (n, root.join(f)))
        .collect()
    } else {
        [
            (
                "DejaVu Sans Mono",
                "/usr/share/fonts/truetype/dejavu/DejaVuSansMono.ttf",
            ),
            (
                "Liberation Mono",
                "/usr/share/fonts/truetype/liberation/LiberationMono-Regular.ttf",
            ),
            (
                "Noto Sans Mono",
                "/usr/share/fonts/truetype/noto/NotoSansMono-Regular.ttf",
            ),
        ]
        .into_iter()
        .map(|(n, f)| (n, PathBuf::from(f)))
        .collect()
    };
    candidates
        .into_iter()
        .filter(|(_, p)| p.exists())
        .map(|(n, p)| (n.to_owned(), p))
        .collect()
}

/// Installs egui's default fonts plus the first system CJK font found.
/// Returns the CJK font's path, or `None` if there is none (CJK text then
/// renders as boxes, still on the grid).
pub fn install(ctx: &egui::Context) -> Option<PathBuf> {
    install_with(ctx, None)
}

/// Like [`install`], with `monospace` (a font file) used for the editor
/// ahead of the built-in monospace font.
pub fn install_with(ctx: &egui::Context, monospace: Option<&std::path::Path>) -> Option<PathBuf> {
    let mut defs = FontDefinitions::default();
    if let Some(bytes) = monospace.and_then(|p| std::fs::read(p).ok()) {
        defs.font_data.insert(
            "user-mono".to_owned(),
            Arc::new(FontData::from_owned(bytes)),
        );
        defs.families
            .entry(FontFamily::Monospace)
            .or_default()
            .insert(0, "user-mono".to_owned());
    }
    if let Some(bytes) = ui_font_candidates()
        .into_iter()
        .find_map(|p| std::fs::read(p).ok())
    {
        defs.font_data.insert(
            "system-ui".to_owned(),
            Arc::new(FontData::from_owned(bytes)),
        );
        defs.families
            .entry(FontFamily::Proportional)
            .or_default()
            .insert(0, "system-ui".to_owned());
    }
    let found = cjk_candidates()
        .into_iter()
        .find_map(|p| std::fs::read(&p).ok().map(|bytes| (p, bytes)));
    let path = found.map(|(path, bytes)| {
        defs.font_data.insert(
            "system-cjk".to_owned(),
            Arc::new(FontData::from_owned(bytes)),
        );
        for family in [FontFamily::Monospace, FontFamily::Proportional] {
            defs.families
                .entry(family)
                .or_default()
                .push("system-cjk".to_owned());
        }
        path
    });
    // Skip the symbol font if it is the editor font already.
    if let Some(bytes) = symbol_candidates()
        .into_iter()
        .filter(|p| monospace != Some(p.as_path()))
        .find_map(|p| std::fs::read(p).ok())
    {
        defs.font_data.insert(
            "system-symbols".to_owned(),
            Arc::new(FontData::from_owned(bytes)),
        );
        for family in [FontFamily::Monospace, FontFamily::Proportional] {
            defs.families
                .entry(family)
                .or_default()
                .push("system-symbols".to_owned());
        }
    }
    ctx.set_fonts(defs);
    path
}
