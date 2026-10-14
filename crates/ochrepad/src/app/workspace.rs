//! The folder workspace (PLAN_v0.2.0 N7): opening a folder, the sidebar
//! with its file tree, and the file operations the tree asks for. Every
//! change on disk goes through the `fileio` worker.

use std::path::{Path, PathBuf};

use egui::{Id, Key, Modal, Panel, TextEdit, Ui};
use ochrepad_fileio::worker::Job;
use serde::{Deserialize, Serialize};

use super::OchrepadApp;
use super::autosave::PendingJob;
use crate::file_tree::{FileTree, TreeAction};

/// The workspace part of the session.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct SessionWorkspace {
    pub root: PathBuf,
    #[serde(default)]
    pub expanded: Vec<PathBuf>,
    #[serde(default)]
    pub show_ignored: bool,
}

/// A file operation the worker is doing for the tree.
pub enum TreeJob {
    Create { path: PathBuf, is_dir: bool },
    Rename { from: PathBuf, to: PathBuf },
}

/// The New File / New Folder name prompt.
pub struct NewEntry {
    dir: PathBuf,
    is_dir: bool,
    name: String,
    error: Option<String>,
}

/// How many recent folders to remember.
const MAX_RECENT_FOLDERS: usize = 10;

impl OchrepadApp {
    /// Opens `dir` as the workspace, showing its tree in the sidebar.
    pub fn open_folder(&mut self, dir: &Path) {
        let dir = std::path::absolute(dir).unwrap_or_else(|_| dir.to_path_buf());
        if !dir.is_dir() {
            self.message = Some(format!("{} is not a folder.", dir.display()));
            return;
        }
        self.tree = Some(FileTree::new(dir.clone()));
        self.settings.show_sidebar = true;
        let recent = &mut self.settings.recent_folders;
        recent.retain(|p| *p != dir);
        recent.insert(0, dir);
        recent.truncate(MAX_RECENT_FOLDERS);
    }

    pub fn close_folder(&mut self) {
        self.tree = None;
    }

    /// The workspace's file tree, if a folder is open.
    pub fn file_tree(&self) -> Option<&FileTree> {
        self.tree.as_ref()
    }

    pub(super) fn open_folder_dialog(&mut self) {
        if let Some(dir) = rfd::FileDialog::new().pick_folder() {
            self.open_folder(&dir);
        }
    }

    /// Shows the sidebar's file tree in the selected folder.
    pub fn reveal_active_file(&mut self) {
        let path = self.buffer().meta.path.clone();
        if let (Some(tree), Some(path)) = (&mut self.tree, path) {
            self.settings.show_sidebar = true;
            tree.reveal(&path);
        }
    }

    /// The left sidebar, when a folder is open and it is not hidden.
    pub(super) fn sidebar(&mut self, ui: &mut Ui) {
        if !self.settings.show_sidebar {
            return;
        }
        let active = self.buffer().meta.path.clone();
        let Some(tree) = &mut self.tree else {
            return;
        };
        let actions = Panel::left("sidebar")
            .resizable(true)
            .default_size(self.settings.sidebar_width)
            .size_range(120.0..=600.0)
            .show(ui, |ui| tree.ui(ui, active.as_deref()));
        let width = actions.response.rect.width();
        if (width - self.settings.sidebar_width).abs() > 0.5 {
            self.settings.sidebar_width = width;
        }
        for action in actions.inner {
            self.tree_action(ui, action);
        }
    }

    fn tree_action(&mut self, ui: &Ui, action: TreeAction) {
        let is_dir = matches!(action, TreeAction::NewFolder(_));
        match action {
            TreeAction::Open(path) => {
                self.open_path(&path);
                self.view_state_mut().request_focus();
            }
            TreeAction::NewFile(dir) | TreeAction::NewFolder(dir) if self.new_entry.is_none() => {
                self.new_entry = Some(NewEntry {
                    dir,
                    is_dir,
                    name: String::new(),
                    error: None,
                });
            }
            TreeAction::NewFile(_) | TreeAction::NewFolder(_) => {}
            TreeAction::Rename { from, to } => {
                let tag = self.persist.worker.submit(Job::Rename {
                    from: from.clone(),
                    to: to.clone(),
                });
                self.persist
                    .pending
                    .insert(tag, PendingJob::Tree(TreeJob::Rename { from, to }));
            }
            TreeAction::Copy(text) => ui.ctx().copy_text(text),
            TreeAction::RevealInFileManager(path) => {
                if let Err(e) = reveal_in_file_manager(&path) {
                    self.message = Some(format!("Could not open the file manager:\n{e}"));
                }
            }
            TreeAction::FindInFolder(dir) => self.open_find_in_files(Some(&dir)),
            TreeAction::CloseFolder => self.close_folder(),
            TreeAction::FocusEditor => self.view_state_mut().request_focus(),
        }
    }

    /// The name prompt for New File / New Folder.
    pub(super) fn new_entry_dialog(&mut self, ui: &mut Ui) {
        let Some(entry) = &mut self.new_entry else {
            return;
        };
        let title = if entry.is_dir {
            "New Folder"
        } else {
            "New File"
        };
        let resp = Modal::new(Id::new("ochrepad-new-entry")).show(ui.ctx(), |ui| {
            ui.strong(title);
            ui.label(format!("In {}", entry.dir.display()));
            let field = ui.add(
                TextEdit::singleline(&mut entry.name)
                    .id(Id::new("ochrepad-new-entry-name"))
                    .hint_text("name"),
            );
            if !field.has_focus() && !field.lost_focus() {
                field.request_focus();
            }
            let enter = field.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter));
            if let Some(e) = &entry.error {
                ui.colored_label(ui.visuals().error_fg_color, e);
            }
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                if ui.button("Create").clicked() || enter {
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
        match choice {
            Some(true) => {
                let name = entry.name.trim().to_owned();
                if name.is_empty() || name.contains(['/', '\\']) || name == "." || name == ".." {
                    entry.error = Some("Type a name without slashes.".to_owned());
                    return;
                }
                let path = entry.dir.join(&name);
                if path.exists() {
                    entry.error = Some(format!("{name} already exists."));
                    return;
                }
                let is_dir = entry.is_dir;
                self.new_entry = None;
                let job = if is_dir {
                    Job::CreateDir(path.clone())
                } else {
                    Job::CreateFile(path.clone())
                };
                let tag = self.persist.worker.submit(job);
                self.persist
                    .pending
                    .insert(tag, PendingJob::Tree(TreeJob::Create { path, is_dir }));
            }
            Some(false) => self.new_entry = None,
            None => {}
        }
    }

    /// A tree job finished.
    pub(super) fn tree_job_done(&mut self, job: TreeJob, result: Result<(), String>) {
        if let Err(e) = result {
            self.message = Some(format!("Could not do that:\n{e}"));
            return;
        }
        let now = self.now;
        match job {
            TreeJob::Create { path, is_dir } => {
                if let Some(tree) = &mut self.tree
                    && let Some(dir) = path.parent()
                {
                    tree.changed(dir, now);
                    if dir != tree.root() {
                        tree.expanded.insert(dir.to_path_buf());
                    }
                    tree.reveal(&path);
                }
                if !is_dir {
                    self.open_path(&path);
                }
            }
            TreeJob::Rename { from, to } => {
                self.renamed_on_disk(&from, &to);
                if let Some(tree) = &mut self.tree {
                    for p in [&from, &to] {
                        if let Some(dir) = p.parent() {
                            tree.changed(dir, now);
                        }
                    }
                    if tree.expanded.remove(&from) {
                        tree.expanded.insert(to.clone());
                    }
                    tree.reveal(&to);
                }
            }
        }
    }

    /// After `from` was renamed to `to`: open documents at or under `from`
    /// take their new paths (their tabs, the watcher and the session
    /// follow), and may change language with their name.
    fn renamed_on_disk(&mut self, from: &Path, to: &Path) {
        let moved: Vec<(ochrepad_core::BufferId, PathBuf)> = self
            .editor
            .buffers()
            .filter_map(|b| {
                let p = b.meta.path.as_deref()?;
                let rest = p.strip_prefix(from).ok()?;
                let new = if rest.as_os_str().is_empty() {
                    to.to_path_buf()
                } else {
                    to.join(rest)
                };
                Some((b.id(), new))
            })
            .collect();
        for (buffer, path) in moved {
            if let Some(b) = self.editor.buffer_mut(buffer) {
                b.meta.path = Some(path);
            }
            self.attach_highlighting(buffer);
            self.record_disk_stamp(buffer);
        }
    }

    /// The workspace for the session file.
    pub(super) fn session_workspace(&self) -> Option<SessionWorkspace> {
        let tree = self.tree.as_ref()?;
        let mut expanded: Vec<PathBuf> = tree.expanded.iter().cloned().collect();
        expanded.sort();
        Some(SessionWorkspace {
            root: tree.root().to_path_buf(),
            expanded,
            show_ignored: tree.show_ignored,
        })
    }

    pub(super) fn restore_workspace(&mut self, w: &SessionWorkspace) {
        if !w.root.is_dir() {
            return;
        }
        let show_sidebar = self.settings.show_sidebar;
        self.open_folder(&w.root);
        self.settings.show_sidebar = show_sidebar;
        if let Some(tree) = &mut self.tree {
            tree.expanded = w.expanded.iter().filter(|p| p.is_dir()).cloned().collect();
            tree.show_ignored = w.show_ignored;
        }
    }
}

/// Shows `path` in the system's file manager: selected in Explorer on
/// Windows; its folder (or itself, for a folder) elsewhere.
fn reveal_in_file_manager(path: &Path) -> std::io::Result<()> {
    if cfg!(windows) {
        let mut arg = std::ffi::OsString::from("/select,");
        arg.push(path);
        std::process::Command::new("explorer").arg(arg).spawn()?;
    } else {
        let dir = if path.is_dir() {
            path
        } else {
            path.parent().unwrap_or(path)
        };
        std::process::Command::new("xdg-open").arg(dir).spawn()?;
    }
    Ok(())
}
