//! Font setup. egui's built-in fonts have no CJK glyphs, so a CJK font is
//! loaded from the system font directory and appended as a fallback.

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
    } else if cfg!(target_os = "macos") {
        for p in [
            "/System/Library/Fonts/PingFang.ttc",
            "/System/Library/Fonts/Hiragino Sans GB.ttc",
            "/System/Library/Fonts/AppleSDGothicNeo.ttc",
            "/Library/Fonts/Arial Unicode.ttf",
        ] {
            out.push(PathBuf::from(p));
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

/// Installs egui's default fonts plus the first system CJK font found.
/// Returns the CJK font's path, or `None` if there is none (CJK text then
/// renders as boxes, still on the grid).
pub fn install(ctx: &egui::Context) -> Option<PathBuf> {
    let mut defs = FontDefinitions::default();
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
    ctx.set_fonts(defs);
    path
}
