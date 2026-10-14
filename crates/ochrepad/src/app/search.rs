//! Find All and Replace All across open documents (PLAN_v0.2.0 N6): starting
//! searches for the Search Results panel, going to a result, and the
//! confirmation before replacing in every open document.

use egui::{Id, Modal, Ui};
use ochrepad_core::{Action, BufferId, ByteOffset, SearchQuery, Searcher, Selection, SelectionSet};

use super::OchrepadApp;
use crate::search_results::{GoTo, Job, OpenDoc, Source};

/// A Replace All in All Open Documents waiting for the user to confirm.
pub struct ReplaceAllPlan {
    query: SearchQuery,
    replacement: String,
    /// Documents with matches, and how many.
    counts: Vec<(BufferId, usize)>,
}

impl OchrepadApp {
    /// The open documents in tab order, each once.
    fn open_buffers(&self) -> Vec<BufferId> {
        let mut out = Vec::new();
        for v in self.tabs() {
            if let Some(b) = self.editor.view(v).map(|v| v.buffer())
                && !out.contains(&b)
            {
                out.push(b);
            }
        }
        out
    }

    /// Lists every match of the find panel's pattern in the Search Results
    /// panel: in the current document, or in every open one ("in
    /// selection" applies only to the current document).
    pub fn find_all(&mut self, all_documents: bool) {
        let mut query = self.find.query();
        let active = self.buffer_id();
        let buffers = if all_documents {
            query.options.in_selection = false;
            self.open_buffers()
        } else {
            vec![active]
        };
        let selections = (!all_documents && query.options.in_selection)
            .then(|| self.editor.view(self.active).map(|v| v.selections.clone()))
            .flatten();
        let jobs: Vec<Job> = buffers
            .iter()
            .filter_map(|&b| {
                let buf = self.editor.buffer(b)?;
                Some(Job {
                    source: Source::Buffer(b),
                    name: self.buffer_name(b),
                    doc: buf.doc().clone(),
                    revision: buf.revision(),
                    selections: selections.clone(),
                })
            })
            .collect();
        let scope = if all_documents {
            format!("in {} open documents", jobs.len())
        } else {
            format!("in {}", self.buffer_name(active))
        };
        self.results.start(query, scope, jobs);
    }

    /// Selects a search result. If its document changed since the search,
    /// the match nearest the old column on the same line is selected
    /// instead (or the line's start). A file that was not open is opened
    /// and looked up the same way.
    pub fn go_to_result(&mut self, go: GoTo) {
        if let Source::Viewer(id) = &go.source {
            let id = *id;
            self.go_to_viewer_result(id, go);
            return;
        }
        let (buffer, exact) = match &go.source {
            Source::Viewer(_) => return,
            Source::Buffer(b) => (*b, true),
            Source::Path(path) => {
                self.open_path(path);
                let opened = self
                    .buffer()
                    .meta
                    .path
                    .as_deref()
                    .is_some_and(|p| super::canonical(p) == super::canonical(path));
                if !opened {
                    return;
                }
                (self.buffer_id(), false)
            }
        };
        let Some(buf) = self.editor.buffer(buffer) else {
            self.message = Some("That document has been closed.".to_owned());
            return;
        };
        let doc = buf.doc();
        let range = if exact && buf.revision() == go.revision {
            go.hit.start..go.hit.end
        } else if go.hit.line < doc.line_count() {
            let start = doc.line_start(go.hit.line);
            let column = start.0 + go.hit.column;
            Searcher::new(&go.query)
                .ok()
                .and_then(|s| {
                    s.find_all(doc, start..doc.line_end(go.hit.line))
                        .into_iter()
                        .min_by_key(|m| m.start.0.abs_diff(column))
                })
                .unwrap_or(start..start)
        } else {
            doc.len()..doc.len()
        };
        let in_active = self.editor.view(self.active).map(|v| v.buffer()) == Some(buffer);
        let view = if in_active {
            Some(self.active)
        } else {
            self.tabs()
                .into_iter()
                .find(|&v| self.editor.view(v).map(|v| v.buffer()) == Some(buffer))
        };
        let Some(view) = view else {
            return;
        };
        self.activate(view);
        let sel = Selection::new(range.start, range.end);
        let _ = self
            .editor
            .apply(view, Action::SetSelections(SelectionSet::single(sel)));
        let state = self.view_state_mut();
        state.reveal_caret();
        state.request_focus();
    }

    /// Opens the find panel's Find in Files row, on `folder` or else (when
    /// the folder field is empty) the workspace folder or the active file's
    /// folder.
    pub fn open_find_in_files(&mut self, folder: Option<&std::path::Path>) {
        let fallback = self
            .tree
            .as_ref()
            .map(|t| t.root().to_path_buf())
            .or_else(|| {
                self.buffer()
                    .meta
                    .path
                    .as_deref()
                    .and_then(|p| p.parent())
                    .map(|p| p.to_path_buf())
            });
        self.find
            .open(&self.editor, self.active, self.find.show_replace);
        self.find.open_files(folder, fallback.as_deref());
    }

    /// Find in Files with the find panel's pattern, folder and filters.
    /// Open documents are searched as they are in the editor.
    pub fn find_in_files(&mut self) {
        let folder = std::path::PathBuf::from(self.find.folder.trim());
        if !folder.is_dir() {
            self.find
                .set_message(format!("{} is not a folder", folder.display()), true);
            return;
        }
        let mut query = self.find.query();
        query.options.in_selection = false;
        let options = ochrepad_fileio::tree::WalkOptions {
            subfolders: self.find.subfolders,
            include_ignored: self.find.include_ignored,
            filters: ochrepad_fileio::tree::parse_filters(&self.find.filters),
        };
        let open = self
            .editor
            .buffers()
            .filter_map(|b| {
                Some(OpenDoc {
                    path: b.meta.path.clone()?,
                    source: Source::Buffer(b.id()),
                    doc: b.doc().clone(),
                    revision: b.revision(),
                })
            })
            .collect();
        self.results.start_in_files(query, folder, options, open);
    }

    /// Counts the matches in every open document and asks before replacing.
    pub fn plan_replace_all_in_all(&mut self) {
        let mut query = self.find.query();
        query.options.in_selection = false;
        let Ok(searcher) = Searcher::new(&query) else {
            return;
        };
        let counts: Vec<(BufferId, usize)> = self
            .open_buffers()
            .into_iter()
            .filter_map(|b| {
                let doc = self.editor.buffer(b)?.doc();
                let n = searcher.matches(doc, ByteOffset(0)..doc.len()).count();
                (n > 0).then_some((b, n))
            })
            .collect();
        if counts.is_empty() {
            self.find.set_message("Not found", true);
            return;
        }
        self.replace_all_plan = Some(ReplaceAllPlan {
            query,
            replacement: self.find.replacement.clone(),
            counts,
        });
    }

    /// The confirmation for Replace All in All Open Documents.
    pub(super) fn replace_all_dialog(&mut self, ui: &mut Ui) {
        let Some(plan) = &self.replace_all_plan else {
            return;
        };
        let total: usize = plan.counts.iter().map(|c| c.1).sum();
        let docs = plan.counts.len();
        let resp = Modal::new(Id::new("ochrepad-replace-all")).show(ui.ctx(), |ui| {
            ui.label(format!(
                "Replace {total} matches in {docs} open documents?\n\
                 Each document can be undone on its own; nothing is saved."
            ));
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                if ui.button("Replace All Matches").clicked() {
                    Some(true)
                } else if ui.button("Cancel").clicked() {
                    Some(false)
                } else {
                    None
                }
            })
            .inner
        });
        let choice = if resp.should_close() {
            Some(false)
        } else {
            resp.inner
        };
        let Some(confirmed) = choice else {
            return;
        };
        let Some(plan) = self.replace_all_plan.take() else {
            return;
        };
        if confirmed {
            for (buffer, _) in &plan.counts {
                // Any view of the document; one undo step in it. Not recorded:
                // a macro plays in one view.
                let view = self.editor.views_of(*buffer).next().map(|v| v.id());
                if let Some(view) = view {
                    let _ = self.editor.apply_unrecorded(
                        view,
                        Action::ReplaceAll {
                            query: plan.query.clone(),
                            replacement: plan.replacement.clone(),
                        },
                    );
                }
            }
            self.find.set_message(
                format!("Replaced {total} occurrences in {docs} documents"),
                false,
            );
        }
        self.view_state_mut().request_focus();
    }
}
