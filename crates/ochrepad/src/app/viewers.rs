//! Files too large to edit, in the app (PLAN_v0.3.0 P7): opening them in a
//! viewer tab, the find panel and Go to Line on a viewer, Find All into the
//! Search Results panel, the status bar while a viewer has the focus, and
//! View > Open as Editable.

use std::path::Path;
use std::sync::atomic::AtomicBool;

use egui::{Id, Modal, Ui};
use egui_dock::NodePath;
use ochrepad_core::ByteOffset;
use ochrepad_core::search::ByteSource;
use ochrepad_fileio as fileio;
use ochrepad_fileio::bigfile::BigFile;

use super::{OchrepadApp, Tab};
use crate::search_results::{FileHits, GoTo, Hit, MAX_MATCHES, Source, shorten};
use crate::viewer::{ViewerEvent, ViewerId, human_bytes};

/// Bytes of context read around a match for its line in the results.
const CONTEXT: u64 = 400;

impl OchrepadApp {
    /// The size above which a file opens in a viewer.
    fn viewer_threshold(&self) -> u64 {
        (u64::from(self.settings.viewer_threshold_mb) * 1024 * 1024).min(fileio::MAX_LOAD_SIZE)
    }

    /// Whether `path` is too large to edit, so it opens in a viewer.
    pub(super) fn opens_in_viewer(&self, path: &Path) -> bool {
        std::fs::metadata(path).is_ok_and(|m| m.is_file() && m.len() > self.viewer_threshold())
    }

    /// Opens `path` read-only in a viewer tab, or switches to its viewer.
    pub fn open_viewer(&mut self, path: &Path) -> Option<ViewerId> {
        if let Some(id) = self.viewers.find(path) {
            self.activate_viewer(id);
            return Some(id);
        }
        match BigFile::open(path) {
            Ok(file) => {
                let id = self.viewers.add(file);
                self.dock.push_to_focused_leaf(Tab::Viewer(id));
                self.settings.add_recent(super::canonical(path));
                self.activate_viewer(id);
                Some(id)
            }
            Err(e) => {
                self.message = Some(format!("Could not open {}:\n{e}", path.display()));
                None
            }
        }
    }

    /// Makes viewer `id` the focused tab and gives it the keyboard.
    pub fn activate_viewer(&mut self, id: ViewerId) {
        if let Some(path) = self.dock.find_tab(&Tab::Viewer(id)) {
            let _ = self.dock.set_active_tab(path);
            self.dock.set_focused_node_and_surface(NodePath {
                surface: path.surface,
                node: path.node,
            });
        }
        if let Some(v) = self.viewers.get_mut(id) {
            v.state.request_focus();
        }
    }

    /// The viewer in the focused pane, if a viewer is what it shows.
    pub fn focused_viewer(&mut self) -> Option<ViewerId> {
        match self.dock.find_active_focused() {
            Some((_, Tab::Viewer(id))) => Some(*id),
            _ => None,
        }
    }

    /// Closes viewer `id` and its tab.
    pub fn close_viewer(&mut self, id: ViewerId) {
        if let Some(path) = self.dock.find_tab(&Tab::Viewer(id)) {
            self.dock.remove_tab(path);
        }
        if self.filter.is_open(id) {
            self.filter
                .close(&mut self.editor, &mut self.viewers, id.into());
        }
        self.viewers.remove(id);
        if self.confirm_editable == Some(id) {
            self.confirm_editable = None;
        }
    }

    /// The file name a viewer shows.
    pub fn viewer_name(&self, id: ViewerId) -> String {
        self.viewers
            .get(id)
            .and_then(|v| v.file.path().file_name())
            .map_or_else(|| "?".to_owned(), |n| n.to_string_lossy().into_owned())
    }

    /// Opens the find panel on a viewer, with its one-line selection as the
    /// pattern.
    pub(super) fn open_find_on_viewer(&mut self, id: ViewerId) {
        let selected = self
            .viewers
            .get(id)
            .and_then(|v| v.state.selected_line_text(&v.file))
            .filter(|t| t.len() <= crate::find_panel::MAX_PATTERN);
        self.find.open = true;
        self.find.show_replace = false;
        self.find.focus_field();
        self.find.clear_message();
        if let Some(text) = selected {
            self.find.pattern = text;
        }
    }

    /// Find Next/Previous in a viewer (in the background).
    pub(super) fn viewer_find(&mut self, id: ViewerId, backward: bool) {
        let query = self.find.query();
        if query.pattern.is_empty() {
            return;
        }
        let Some(v) = self.viewers.get_mut(id) else {
            return;
        };
        match v.state.start_find(&v.file, &query, backward) {
            Ok(()) => self.find.set_message("Searching…", false),
            Err(e) => self.find.set_message(format!("Invalid pattern: {e}"), true),
        }
    }

    /// Count in a viewer (in the background).
    pub(super) fn viewer_count(&mut self, id: ViewerId) {
        let query = self.find.query();
        let Some(v) = self.viewers.get_mut(id) else {
            return;
        };
        match v.state.start_count(&v.file, &query) {
            Ok(()) => self.find.set_message("Counting…", false),
            Err(e) => self.find.set_message(format!("Invalid pattern: {e}"), true),
        }
    }

    /// Find All in a viewer: every match (the first [`MAX_MATCHES`] of
    /// them) in the Search Results panel, found in the background.
    pub(super) fn viewer_find_all(&mut self, id: ViewerId) {
        let query = self.find.query();
        let name = self.viewer_name(id);
        let Some(v) = self.viewers.get(id) else {
            return;
        };
        let searcher = match ochrepad_core::Searcher::new_raw(&query) {
            Ok(s) => s,
            Err(e) => {
                self.find
                    .set_message(format!("Invalid pattern: {}", e.0), true);
                return;
            }
        };
        let lines = v.file.line_finder();
        let decoder = v.file.decoder();
        let scope = format!("in {name}");
        // The panel's Stop and Clear set this flag, and the file source
        // stops reading on it.
        let stop = std::sync::Arc::new(AtomicBool::new(false));
        let src = v.file.source(stop.clone());
        self.results
            .start_external(query, scope, stop, move |cancel| {
                let decode = |bytes: &[u8]| decoder.decode(bytes).into_owned();
                let mut hits: Vec<Hit> = Vec::new();
                // Where the window of the last hit starts and ends in the file.
                let mut window = 0..0;
                let (mut matches, mut truncated, mut hint) = (0usize, false, None);
                for m in searcher.matches_in(&src, 0..src.len()) {
                    if cancel.load(std::sync::atomic::Ordering::Relaxed) {
                        break;
                    }
                    if matches == MAX_MATCHES {
                        truncated = true;
                        break;
                    }
                    matches += 1;
                    let Some(line) = lines.line_of(m.start, hint, &cancel) else {
                        break;
                    };
                    hint = Some((m.start, line));
                    let line = line as usize;
                    if let Some(last) = hits.last_mut()
                        && last.line == line
                    {
                        if window.contains(&m.start) && m.end <= window.end {
                            let a = decode(&lines.read(window.start..m.start)).len();
                            let b = decode(&lines.read(window.start..m.end)).len();
                            last.ranges.push(a..b);
                        }
                        continue;
                    }
                    // The line around the match: from the break before it (or a
                    // window's worth before) to the break after it.
                    let from = m.start.saturating_sub(CONTEXT);
                    let bytes = lines.read(from..m.end + CONTEXT);
                    // Fewer bytes than asked for if the file got shorter since.
                    let at = ((m.start - from) as usize).min(bytes.len());
                    let begin = bytes[..at]
                        .iter()
                        .rposition(|&b| b == b'\n' || b == b'\r')
                        .map_or(0, |i| i + 1);
                    let after = ((m.end - from) as usize).min(bytes.len());
                    let end = bytes[after..]
                        .iter()
                        .position(|&b| b == b'\n' || b == b'\r')
                        .map_or(bytes.len(), |i| after + i);
                    window = from + begin as u64..from + end as u64;
                    let text = decode(&bytes[begin..end]);
                    let a = decode(&bytes[begin..at]).len();
                    let b = decode(&bytes[begin..after.max(at)]).len();
                    let first = a..b;
                    let mut hit = Hit {
                        line,
                        text,
                        ranges: vec![first],
                        start: ByteOffset(m.start as usize),
                        end: ByteOffset(m.end as usize),
                        column: a,
                    };
                    shorten(&mut hit);
                    hits.push(hit);
                }
                let files = if hits.is_empty() {
                    Vec::new()
                } else {
                    vec![FileHits {
                        source: Source::Viewer(id),
                        name,
                        revision: 0,
                        hits,
                        matches,
                        expanded: true,
                    }]
                };
                Ok((files, truncated))
            });
    }

    /// Goes to a result in a viewer.
    pub(super) fn go_to_viewer_result(&mut self, id: ViewerId, go: GoTo) {
        let Some(v) = self.viewers.get_mut(id) else {
            self.message = Some("That file has been closed.".to_owned());
            return;
        };
        v.state
            .select_range(go.hit.start.0 as u64..go.hit.end.0 as u64);
        self.activate_viewer(id);
    }

    /// Ctrl+G on a viewer.
    pub(super) fn viewer_go_to_line(&mut self, id: ViewerId, line: u64, column: usize) {
        if let Some(v) = self.viewers.get_mut(id) {
            v.state.go_to_line(line, column);
            v.state.request_focus();
        }
    }

    /// The number of lines for the Go to Line dialog: known, or estimated
    /// while the index is still being built.
    pub(super) fn viewer_line_hint(&self, id: ViewerId) -> String {
        match self.viewers.get(id) {
            Some(v) => {
                let p = v.file.progress();
                if p.done {
                    format!("1–{}", p.lines)
                } else {
                    format!("1–about {}", p.estimated_lines())
                }
            }
            None => String::new(),
        }
    }

    /// Reports what the viewers did: in the find panel, or the status bar.
    pub(super) fn viewer_events(&mut self) {
        let ids: Vec<ViewerId> = self.viewers.iter().map(|(id, _)| id).collect();
        for id in ids {
            let Some(v) = self.viewers.get_mut(id) else {
                continue;
            };
            for e in std::mem::take(&mut v.state.events) {
                match e {
                    ViewerEvent::Found { wrapped: true } => {
                        self.find.set_message("Wrapped around", false);
                    }
                    ViewerEvent::Found { wrapped: false } => self.find.clear_message(),
                    ViewerEvent::NotFound => self.find.set_message("Not found", true),
                    ViewerEvent::Count(n) => self.find.set_message(format!("{n} matches"), false),
                    ViewerEvent::Copied {
                        bytes,
                        truncated: true,
                    } => self.note_text(format!(
                        "Copied the first {} of the selection",
                        human_bytes(bytes as u64)
                    )),
                    ViewerEvent::Copied { .. } => {}
                    ViewerEvent::Reloaded => {
                        self.note_text("The file got shorter on disk, so it was read again");
                    }
                }
            }
        }
    }

    /// The status bar while viewer `id` has the focus: the caret, how far
    /// indexing has got, and the file's encoding and line endings.
    pub(super) fn viewer_status_bar(&mut self, ui: &mut Ui, id: ViewerId) {
        let Some(v) = self.viewers.get(id) else {
            return;
        };
        let p = v.file.progress();
        let (ending, mixed) = v.file.line_ending();
        let encoding = if v.file.has_bom() {
            format!("{} BOM", v.file.decoder().name())
        } else {
            v.file.decoder().name().to_owned()
        };
        let head = v.state.head;
        let caret = format!("Ln {}, Col {}", head.line + 1, head.col + 1);
        let searching = v.state.searching();
        let going = v.state.waiting_to_go();
        let size = human_bytes(v.file.len());
        let following = v.state.follow;
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.label(encoding);
            ui.separator();
            ui.label(match ending {
                ochrepad_core::LineEnding::CrLf => "Windows (CR LF)",
                ochrepad_core::LineEnding::Lf => "Unix (LF)",
                ochrepad_core::LineEnding::Cr => "Macintosh (CR)",
            })
            .on_hover_text(if mixed {
                "The most common line ending; the file has more than one kind"
            } else {
                "Line endings"
            });
            ui.separator();
            ui.label(format!("Read-only, {size}"))
                .on_hover_text("Files this large open in the viewer: View > Open as Editable");
            if following {
                ui.separator();
                if ui
                    .button("Following")
                    .on_hover_text("The end of the file stays in view as it grows. Click to stop.")
                    .clicked()
                    && let Some(v) = self.viewers.get_mut(id)
                {
                    v.state.follow = false;
                }
            }
            ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
                ui.label(caret);
                ui.separator();
                if p.done {
                    ui.label(format!("{} lines", p.lines));
                } else {
                    let pct = p.scanned * 100 / p.len.max(1);
                    ui.add(
                        egui::Label::new(format!("Indexing… {pct}%, {} lines so far", p.lines))
                            .truncate(),
                    );
                }
                if let Some(line) = going {
                    ui.separator();
                    ui.add(egui::Label::new(format!("Going to line {line}…")).truncate());
                }
                if searching {
                    ui.separator();
                    ui.label("Searching…");
                }
                if let Some((note, until)) = &self.status_note {
                    if self.now < *until {
                        ui.separator();
                        let color = ui.visuals().warn_fg_color;
                        ui.add(egui::Label::new(egui::RichText::new(note).color(color)).truncate());
                        ui.ctx()
                            .request_repaint_after(std::time::Duration::from_secs_f64(
                                (until - self.now).max(0.0),
                            ));
                    } else {
                        self.status_note = None;
                    }
                }
            });
        });
    }

    /// View > Open as Editable: asks first, since the file is large.
    pub(super) fn ask_open_as_editable(&mut self, id: ViewerId) {
        self.confirm_editable = Some(id);
    }

    pub(super) fn open_as_editable_dialog(&mut self, ui: &mut Ui) {
        let Some(id) = self.confirm_editable else {
            return;
        };
        let Some(v) = self.viewers.get(id) else {
            self.confirm_editable = None;
            return;
        };
        let size = v.file.len();
        let name = self.viewer_name(id);
        let fits = size <= fileio::MAX_LOAD_SIZE;
        let resp = Modal::new(Id::new("ochrepad-open-editable")).show(ui.ctx(), |ui| {
            if fits {
                ui.label(format!(
                    "{name} is {}. Editing it holds it in memory several times over (about {}) \
                     and may be slow.\nOpen it for editing?",
                    human_bytes(size),
                    human_bytes(size.saturating_mul(4)),
                ));
            } else {
                ui.label(format!(
                    "{name} is {}, more than ochrepad can edit ({}).",
                    human_bytes(size),
                    human_bytes(fileio::MAX_LOAD_SIZE)
                ));
            }
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                let open = fits && ui.button("Open for Editing").clicked();
                let cancel = ui.button(if fits { "Cancel" } else { "OK" }).clicked();
                (open, cancel)
            })
            .inner
        });
        let (open, cancel) = resp.inner;
        if cancel || resp.should_close() {
            self.confirm_editable = None;
        } else if open {
            self.confirm_editable = None;
            let path = v.file.path().to_path_buf();
            self.close_viewer(id);
            self.open_in_editor(&path);
        }
    }
}
