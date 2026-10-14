//! Markdown preview tabs (PLAN M11), rendered with `egui_commonmark`.
//!
//! A preview is bound to a buffer. It re-renders 200 ms after the buffer
//! stops changing, and resolves relative image paths against the file's
//! directory. Images load only from this machine (see `safe_images`).
//! Scroll syncing with the editor is out of scope for v1.
//!
//! Clicked links: `http`, `https` and `mailto` go to the browser; a link to
//! a file on this machine (relative links included) opens it in ochrepad;
//! anything else (`file:` to a network path, `ms-settings:`, custom
//! protocol handlers) is refused, because the operating system would run
//! whatever handles it. Hovering a link shows where it goes.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

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

/// A link clicked in a preview that the app handles itself.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LinkClick {
    /// A file on this machine, to open in an editor.
    Open(PathBuf),
    /// A link that is not followed; the text says why.
    Refused(String),
}

/// What happens to a link clicked in a document in `doc_dir`: `None` lets
/// egui hand it to the browser.
pub fn classify_link(url: &str, doc_dir: Option<&Path>) -> Option<LinkClick> {
    let network = || LinkClick::Refused("Links to network paths are not opened.".to_owned());
    let without_fragment = url.split(['#', '?']).next().unwrap_or(url);
    // A scheme has two letters or more (`C:` is a drive).
    let scheme = url
        .split_once(':')
        .map(|(s, _)| s.to_ascii_lowercase())
        .filter(|s| {
            s.len() > 1
                && s.chars()
                    .all(|c| c.is_ascii_alphanumeric() || "+-.".contains(c))
        });
    match scheme.as_deref() {
        Some("http" | "https" | "mailto") => None,
        Some("file") => Some(
            crate::safe_images::local_file(&percent_decode(without_fragment))
                .map_or_else(network, LinkClick::Open),
        ),
        Some(s) => Some(LinkClick::Refused(format!(
            "Links to \"{s}:\" are not opened, only web and mail links."
        ))),
        None if without_fragment.is_empty() => Some(LinkClick::Refused(
            "Links within the document are not followed.".to_owned(),
        )),
        None => {
            // A path: relative ones are relative to the document.
            let path = Path::new(&percent_decode(without_fragment)).to_path_buf();
            let path = match doc_dir {
                _ if path.is_absolute() => path,
                Some(dir) => dir.join(path),
                None => {
                    return Some(LinkClick::Refused(
                        "Save the document first: its links are relative to its folder.".to_owned(),
                    ));
                }
            };
            Some(if crate::safe_images::is_local_path(&path) {
                LinkClick::Open(path)
            } else {
                network()
            })
        }
    }
}

/// `%XX` escapes decoded (a malformed one is kept as it is). Non-UTF-8
/// results keep the escapes.
fn percent_decode(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        let hex = |c: u8| (c as char).to_digit(16);
        match (
            b[i],
            b.get(i + 1).copied().and_then(hex),
            b.get(i + 2).copied().and_then(hex),
        ) {
            (b'%', Some(h), Some(l)) => {
                out.push((h * 16 + l) as u8);
                i += 3;
            }
            (c, _, _) => {
                out.push(c);
                i += 1;
            }
        }
    }
    String::from_utf8(out).unwrap_or_else(|_| s.to_owned())
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

    /// Draws the preview of `buffer` filling `ui`. Returns a clicked link
    /// that the app should handle (see [`classify_link`]).
    pub fn show(&mut self, ui: &mut Ui, buffer: &Buffer) -> Option<LinkClick> {
        crate::safe_images::install(ui);
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
        let doc_dir = buffer.meta.path.as_deref().and_then(Path::parent);
        let commands_before = ui.ctx().output(|o| o.commands.len());
        egui::ScrollArea::vertical()
            .id_salt(("ochrepad-preview", buffer.id().0))
            .auto_shrink(false)
            .show(ui, |ui| {
                // Show where a link goes before it is clicked.
                ui.style_mut().url_in_tooltip = true;
                egui::Frame::new().inner_margin(12.0).show(ui, |ui| {
                    let width = ui.available_width().max(100.0) as usize;
                    CommonMarkViewer::new()
                        .default_implicit_uri_scheme(base)
                        .max_image_width(Some(width))
                        .show(ui, &mut p.cache, &p.text);
                });
            });
        // Links clicked in this preview: keep web and mail links for egui to
        // open; take the rest.
        ui.ctx().output_mut(|o| {
            let mut click = None;
            let mut i = commands_before.min(o.commands.len());
            while i < o.commands.len() {
                if let egui::OutputCommand::OpenUrl(open) = &o.commands[i]
                    && let Some(c) = classify_link(&open.url, doc_dir)
                {
                    click.get_or_insert(c);
                    o.commands.remove(i);
                } else {
                    i += 1;
                }
            }
            click
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_web_and_mail_links_reach_the_browser() {
        let dir = if cfg!(windows) { r"C:\docs" } else { "/docs" };
        let doc = |url| classify_link(url, Some(Path::new(dir)));
        for url in [
            "https://example.com/a",
            "http://x.org",
            "mailto:me@x.org",
            "HTTPS://X",
        ] {
            assert_eq!(doc(url), None, "{url}");
        }
        for url in [
            "ms-settings:privacy",
            "search-ms:query=x",
            "javascript:alert(1)",
            "vscode://file/x",
            "#heading",
        ] {
            assert!(matches!(doc(url), Some(LinkClick::Refused(_))), "{url}");
        }
        // Paths are relative to the document's folder.
        assert_eq!(
            doc("notes/a%20b.md#part"),
            Some(LinkClick::Open(Path::new(dir).join("notes/a b.md")))
        );
        assert!(matches!(
            classify_link("other.md", None),
            Some(LinkClick::Refused(_))
        ));
        if cfg!(windows) {
            assert_eq!(
                doc("file:///C:/docs/other%20file.md#part"),
                Some(LinkClick::Open(PathBuf::from("C:/docs/other file.md")))
            );
            assert_eq!(
                doc("C:/elsewhere/x.md"),
                Some(LinkClick::Open(PathBuf::from("C:/elsewhere/x.md")))
            );
            for url in [
                "file://host/share/x.md",
                r"\\host\share\x.md",
                "//host/share/x.md",
            ] {
                assert!(matches!(doc(url), Some(LinkClick::Refused(_))), "{url}");
            }
        } else {
            assert_eq!(
                doc("file:///home/me/other.md"),
                Some(LinkClick::Open(PathBuf::from("/home/me/other.md")))
            );
        }
    }

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
