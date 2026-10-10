//! Markdown preview tabs (PLAN M11), rendered with `egui_commonmark`.
//!
//! A preview is bound to a buffer. It re-renders 200 ms after the buffer
//! stops changing, and resolves relative image paths against the file's
//! directory. Scroll syncing with the editor is out of scope for v1.

use std::collections::HashMap;
use std::path::Path;

use egui::Ui;
use egui_commonmark::{CommonMarkCache, CommonMarkViewer};
use ochrepad_core::{Buffer, BufferId};

/// Quiet time after the last edit before the preview re-renders.
pub const DEBOUNCE_SECS: f64 = 0.2;

#[derive(Default)]
struct Preview {
    cache: CommonMarkCache,
    text: String,
    rendered: Option<u64>,
    /// The newest revision seen, and when it was first seen.
    seen: Option<(u64, f64)>,
}

#[derive(Default)]
pub struct Previews {
    map: HashMap<BufferId, Preview>,
}

/// The `file://` prefix for relative links in a document at `path`.
pub fn base_uri(path: Option<&Path>) -> String {
    match path.and_then(Path::parent) {
        Some(dir) if !dir.as_os_str().is_empty() => {
            // Standard file URIs have an empty host: `file:///C:/dir/` on
            // Windows (with two slashes `C:` would be read as a host name).
            let dir = dir.display().to_string().replace('\\', "/");
            let sep = if dir.starts_with('/') { "" } else { "/" };
            format!("file://{sep}{dir}/")
        }
        _ => "file://".to_owned(),
    }
}

impl Previews {
    /// The text the preview of `buffer` currently shows.
    pub fn rendered_text(&self, buffer: BufferId) -> Option<&str> {
        self.map
            .get(&buffer)
            .filter(|p| p.rendered.is_some())
            .map(|p| p.text.as_str())
    }

    pub fn remove(&mut self, buffer: BufferId) {
        self.map.remove(&buffer);
    }

    /// Draws the preview of `buffer` filling `ui`.
    pub fn show(&mut self, ui: &mut Ui, buffer: &Buffer) {
        let p = self.map.entry(buffer.id()).or_default();
        let now = ui.input(|i| i.time);
        let revision = buffer.revision();
        if p.seen.is_none_or(|(r, _)| r != revision) {
            p.seen = Some((revision, now));
        }
        if p.rendered != Some(revision) {
            let since = p.seen.map_or(0.0, |(_, t)| now - t);
            if p.rendered.is_none() || since >= DEBOUNCE_SECS {
                p.text = buffer.doc().to_string();
                p.rendered = Some(revision);
            } else {
                ui.ctx()
                    .request_repaint_after(std::time::Duration::from_secs_f64(
                        DEBOUNCE_SECS - since,
                    ));
            }
        }
        let base = base_uri(buffer.meta.path.as_deref());
        egui::ScrollArea::vertical()
            .id_salt(("ochrepad-preview", buffer.id().0))
            .auto_shrink(false)
            .show(ui, |ui| {
                egui::Frame::new().inner_margin(12.0).show(ui, |ui| {
                    let width = ui.available_width().max(100.0) as usize;
                    CommonMarkViewer::new()
                        .default_implicit_uri_scheme(base)
                        .max_image_width(Some(width))
                        .show(ui, &mut p.cache, &p.text);
                });
            });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn relative_links_resolve_against_the_file() {
        assert_eq!(base_uri(None), "file://");
        let p = Path::new(r"C:\docs\notes\readme.md");
        if cfg!(windows) {
            assert_eq!(base_uri(Some(p)), "file:///C:/docs/notes/");
        }
        assert_eq!(
            base_uri(Some(Path::new("/home/me/readme.md"))),
            "file:///home/me/"
        );
    }
}
