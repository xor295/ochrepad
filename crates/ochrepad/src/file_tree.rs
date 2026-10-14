//! The folder workspace's file tree, shown in the left sidebar
//! (PLAN_v0.2.0 N7). Folders are listed lazily, when expanded, by the
//! `fileio` lister thread; changes on disk relist a folder after a short
//! pause. The tree only asks for things to happen: opening, creating and
//! renaming go back to the app as [`TreeAction`]s.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use egui::{
    Align2, Color32, EventFilter, FontId, Id, Key, Pos2, Rect, Response, ScrollArea, Sense, Stroke,
    TextEdit, Ui, Vec2, WidgetInfo, WidgetType,
};
use ochrepad_fileio::tree::{Entry, Lister};

/// Height of one row.
const ROW_H: f32 = 22.0;
/// Indent per level.
const INDENT: f32 = 14.0;
/// Wait this long after a change on disk before relisting a folder.
const RELIST_DELAY: f64 = 0.2;

/// What the tree asks the app to do.
#[derive(Clone, Debug, PartialEq)]
pub enum TreeAction {
    Open(PathBuf),
    /// Ask for a name and create a file (or folder) in this folder.
    NewFile(PathBuf),
    NewFolder(PathBuf),
    Rename {
        from: PathBuf,
        to: PathBuf,
    },
    /// Put this text on the clipboard.
    Copy(String),
    RevealInFileManager(PathBuf),
    /// Find in Files in this folder.
    FindInFolder(PathBuf),
    CloseFolder,
    /// Give the keyboard back to the editor.
    FocusEditor,
}

enum Listing {
    Loading,
    Loaded(Arc<Vec<Entry>>),
    Failed(String),
}

/// What one visible row shows: an entry of a listing, or a note.
enum RowKind {
    Entry(Arc<Vec<Entry>>, usize),
    Note(String),
}

struct Row {
    depth: usize,
    kind: RowKind,
}

impl Row {
    fn entry(&self) -> Option<&Entry> {
        match &self.kind {
            RowKind::Entry(list, i) => Some(&list[*i]),
            RowKind::Note(_) => None,
        }
    }
}

pub struct FileTree {
    root: PathBuf,
    pub show_ignored: bool,
    listings: HashMap<PathBuf, Listing>,
    pub expanded: HashSet<PathBuf>,
    selected: Option<PathBuf>,
    /// Inline rename: the entry and the name being typed (and whether the
    /// field still needs the keyboard).
    rename: Option<(PathBuf, String, bool)>,
    lister: Option<Lister>,
    relist_due: HashMap<PathBuf, f64>,
    /// A file to show once its folders are listed.
    reveal: Option<PathBuf>,
    /// Scroll so that this row index is visible.
    scroll_to: Option<usize>,
    scroll_offset: f32,
}

fn tree_id() -> Id {
    Id::new("ochrepad-file-tree")
}

impl FileTree {
    pub fn new(root: PathBuf) -> Self {
        Self {
            root,
            show_ignored: false,
            listings: HashMap::new(),
            expanded: HashSet::new(),
            selected: None,
            rename: None,
            lister: None,
            relist_due: HashMap::new(),
            reveal: None,
            scroll_to: None,
            scroll_offset: 0.0,
        }
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Lists `dir` (again).
    fn request(&mut self, ctx: &egui::Context, dir: &Path) {
        let lister = self.lister.get_or_insert_with(|| {
            let ctx = ctx.clone();
            Lister::spawn(move || ctx.request_repaint())
        });
        lister.request(dir.to_path_buf(), self.show_ignored);
        self.listings
            .entry(dir.to_path_buf())
            .or_insert(Listing::Loading);
    }

    /// Relists every folder that has been listed (after toggling ignored
    /// files, or Refresh).
    pub fn refresh(&mut self, ctx: &egui::Context) {
        let dirs: Vec<PathBuf> = self.listings.keys().cloned().collect();
        for d in dirs {
            self.request(ctx, &d);
        }
    }

    /// The folders to watch for changes: the root and the expanded ones.
    pub fn watched_dirs(&self) -> Vec<PathBuf> {
        let mut out = vec![self.root.clone()];
        out.extend(
            self.expanded
                .iter()
                .filter(|d| matches!(self.listings.get(*d), Some(Listing::Loaded(_))))
                .cloned(),
        );
        out
    }

    /// Something changed in `dir` on disk: relist it shortly.
    pub fn changed(&mut self, dir: &Path, now: f64) {
        if self.listings.contains_key(dir) {
            self.relist_due
                .insert(dir.to_path_buf(), now + RELIST_DELAY);
        }
    }

    /// Expands the folders down to `file` and selects it once listed.
    pub fn reveal(&mut self, file: &Path) {
        if !file.starts_with(&self.root) {
            return;
        }
        let mut dir = file.parent();
        while let Some(d) = dir.filter(|d| d.starts_with(&self.root) && *d != self.root) {
            self.expanded.insert(d.to_path_buf());
            dir = d.parent();
        }
        self.reveal = Some(file.to_path_buf());
    }

    /// Starts renaming `path` in place.
    pub fn start_rename(&mut self, path: &Path) {
        let name = path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        self.selected = Some(path.to_path_buf());
        self.rename = Some((path.to_path_buf(), name, true));
    }

    /// The listing of `dir`, if loaded.
    pub fn entries(&self, dir: &Path) -> Option<&[Entry]> {
        match self.listings.get(dir) {
            Some(Listing::Loaded(e)) => Some(e.as_slice()),
            _ => None,
        }
    }

    /// Takes finished listings, asks for due relists and for expanded
    /// folders not listed yet. Returns whether listings are on the way.
    pub fn tick(&mut self, ctx: &egui::Context, now: f64) -> bool {
        if !self.listings.contains_key(&self.root) {
            let root = self.root.clone();
            self.request(ctx, &root);
        }
        if let Some(lister) = &mut self.lister {
            for (dir, result) in lister.poll() {
                let listing = match result {
                    Ok(entries) => Listing::Loaded(Arc::new(entries)),
                    Err(e) => Listing::Failed(e),
                };
                self.listings.insert(dir, listing);
            }
        }
        let due: Vec<PathBuf> = self
            .relist_due
            .iter()
            .filter(|(_, t)| **t <= now)
            .map(|(d, _)| d.clone())
            .collect();
        for d in due {
            self.relist_due.remove(&d);
            self.request(ctx, &d);
        }
        let missing: Vec<PathBuf> = self
            .expanded
            .iter()
            .filter(|d| !self.listings.contains_key(*d))
            .cloned()
            .collect();
        for d in missing {
            self.request(ctx, &d);
        }
        if let Some(next) = self.relist_due.values().copied().reduce(f64::min) {
            ctx.request_repaint_after(std::time::Duration::from_secs_f64((next - now).max(0.0)));
        }
        self.lister.as_ref().is_some_and(|l| l.pending() > 0)
    }

    /// The rows on screen, depth first through expanded folders.
    fn rows(&self) -> Vec<Row> {
        let mut out = Vec::new();
        self.push_rows(&self.root, 0, &mut out);
        out
    }

    fn push_rows(&self, dir: &Path, depth: usize, out: &mut Vec<Row>) {
        match self.listings.get(dir) {
            None | Some(Listing::Loading) => out.push(Row {
                depth,
                kind: RowKind::Note("Loading…".into()),
            }),
            Some(Listing::Failed(e)) => out.push(Row {
                depth,
                kind: RowKind::Note(e.clone()),
            }),
            Some(Listing::Loaded(entries)) => {
                if entries.is_empty() && depth > 0 {
                    out.push(Row {
                        depth,
                        kind: RowKind::Note("(empty)".into()),
                    });
                }
                for (i, e) in entries.iter().enumerate() {
                    out.push(Row {
                        depth,
                        kind: RowKind::Entry(entries.clone(), i),
                    });
                    if e.is_dir && self.expanded.contains(&e.path) {
                        self.push_rows(&e.path, depth + 1, out);
                    }
                }
            }
        }
    }

    /// The sidebar body. `active` is the active tab's file.
    pub fn ui(&mut self, ui: &mut Ui, active: Option<&Path>) -> Vec<TreeAction> {
        let mut actions = Vec::new();
        let ctx = ui.ctx().clone();
        let now = ui.input(|i| i.time);
        self.tick(&ctx, now);

        // Header: the folder's name, with the folder commands on a menu.
        let name = self
            .root
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| self.root.display().to_string());
        // As tall as the tab bar beside it, so their bottom lines meet.
        let header = ui
            .allocate_ui_with_layout(
                Vec2::new(ui.available_width(), crate::theme::BAR_H),
                egui::Layout::left_to_right(egui::Align::Center),
                |ui| {
                    ui.set_min_size(Vec2::new(ui.available_width(), crate::theme::BAR_H));
                    ui.add_space(crate::theme::space::M);
                    ui.label(egui::RichText::new(name.to_uppercase()).strong().size(12.0));
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.add_space(crate::theme::space::S);
                        ui.menu_button("Folder", |ui| self.folder_menu(ui, &mut actions))
                    });
                },
            )
            .response;
        let border = ui.visuals().widgets.noninteractive.bg_stroke.color;
        ui.painter().hline(
            header.rect.x_range(),
            header.rect.bottom() - 0.5,
            Stroke::new(1.0, border),
        );
        header.on_hover_text(self.root.display().to_string());
        ui.add_space(crate::theme::space::XS);

        // Failed root: say so and offer to close.
        if let Some(Listing::Failed(e)) = self.listings.get(&self.root) {
            ui.label(e.clone());
            if ui.button("Close Folder").clicked() {
                actions.push(TreeAction::CloseFolder);
            }
            return actions;
        }

        let focused = ui.memory(|m| m.has_focus(tree_id()));
        let area = ui.available_rect_before_wrap();
        // Behind the rows: takes the keyboard focus and the empty space's
        // context menu.
        let background = ui.interact(area, tree_id(), Sense::click());
        if background.clicked() {
            background.request_focus();
        }
        background.context_menu(|ui| self.folder_menu(ui, &mut actions));
        if focused {
            ui.memory_mut(|m| {
                m.set_focus_lock_filter(
                    tree_id(),
                    EventFilter {
                        tab: false,
                        horizontal_arrows: true,
                        vertical_arrows: true,
                        escape: true,
                    },
                )
            });
        }

        let rows = self.rows();
        let entry_index = |p: &Path| {
            rows.iter()
                .position(|r| r.entry().is_some_and(|e| e.path == p))
        };
        if let Some(target) = self.reveal.clone()
            && let Some(i) = entry_index(&target)
        {
            self.selected = Some(target);
            self.scroll_to = Some(i);
            self.reveal = None;
        }

        // Keyboard, while the tree has the focus.
        let mut select: Option<usize> = None;
        let selected_index = self.selected.as_deref().and_then(entry_index);
        let mut clicked: Vec<(PathBuf, bool, bool)> = Vec::new(); // (path, is_dir, double)
        if focused && self.rename.is_none() {
            let keys = ui.input(|i| {
                [
                    Key::ArrowUp,
                    Key::ArrowDown,
                    Key::ArrowLeft,
                    Key::ArrowRight,
                    Key::Enter,
                    Key::F2,
                    Key::Escape,
                ]
                .map(|k| i.key_pressed(k))
            });
            let entry_at = |i: usize| rows.get(i).and_then(Row::entry);
            let step = |from: Option<usize>, down: bool| -> Option<usize> {
                let mut i = from.map_or(if down { 0 } else { rows.len() }, |i| {
                    if down { i + 1 } else { i.wrapping_sub(1) }
                });
                while i < rows.len() {
                    if entry_at(i).is_some() {
                        return Some(i);
                    }
                    i = if down { i + 1 } else { i.wrapping_sub(1) };
                }
                from
            };
            if keys[0] {
                select = step(selected_index, false);
            }
            if keys[1] {
                select = step(selected_index, true);
            }
            if let Some(e) = selected_index.and_then(entry_at) {
                let open = self.expanded.contains(&e.path);
                if keys[2] {
                    if e.is_dir && open {
                        self.expanded.remove(&e.path);
                    } else if let Some(parent) = e.path.parent().and_then(entry_index) {
                        select = Some(parent);
                    }
                }
                if keys[3] && e.is_dir {
                    if open {
                        select = step(selected_index, true);
                    } else {
                        clicked.push((e.path.clone(), true, false));
                    }
                }
                if keys[4] {
                    clicked.push((e.path.clone(), e.is_dir, true));
                }
                if keys[5] {
                    self.rename = Some((e.path.clone(), e.name.clone(), true));
                }
            }
            if keys[6] {
                actions.push(TreeAction::FocusEditor);
            }
        }
        if let Some(i) = select
            && let Some(e) = rows.get(i).and_then(Row::entry)
        {
            self.selected = Some(e.path.clone());
            self.scroll_to = Some(i);
        }

        let mut scroll = ScrollArea::vertical().auto_shrink(false);
        if let Some(i) = self.scroll_to.take() {
            let y = i as f32 * ROW_H;
            let height = area.height();
            if y < self.scroll_offset {
                scroll = scroll.vertical_scroll_offset(y);
            } else if y + ROW_H > self.scroll_offset + height {
                scroll = scroll.vertical_scroll_offset(y + ROW_H - height);
            }
        }
        let font = egui::TextStyle::Body.resolve(ui.style());
        let mut rename_done: Option<Option<(PathBuf, String)>> = None;
        let output = scroll.show_rows(ui, ROW_H, rows.len(), |ui, range| {
            ui.spacing_mut().item_spacing.y = 0.0;
            for row in &rows[range] {
                let e = match &row.kind {
                    RowKind::Entry(list, i) => &list[*i],
                    RowKind::Note(text) => {
                        let (rect, _) = ui.allocate_exact_size(
                            Vec2::new(ui.available_width(), ROW_H),
                            Sense::hover(),
                        );
                        ui.painter().text(
                            Pos2::new(
                                rect.min.x + 8.0 + (row.depth as f32 + 1.0) * INDENT,
                                rect.center().y,
                            ),
                            Align2::LEFT_CENTER,
                            text,
                            font.clone(),
                            ui.visuals().weak_text_color(),
                        );
                        continue;
                    }
                };
                if let Some((path, name, needs_focus)) = &mut self.rename
                    && *path == e.path
                {
                    let (rect, _) = ui.allocate_exact_size(
                        Vec2::new(ui.available_width(), ROW_H),
                        Sense::hover(),
                    );
                    let x = rect.min.x + 8.0 + row.depth as f32 * INDENT + 16.0;
                    let field = Rect::from_min_max(
                        Pos2::new(x, rect.min.y + 1.0),
                        Pos2::new(rect.max.x - 4.0, rect.max.y - 1.0),
                    );
                    let resp = ui.put(
                        field,
                        TextEdit::singleline(name).id(Id::new("ochrepad-tree-rename")),
                    );
                    if std::mem::take(needs_focus) {
                        resp.request_focus();
                    }
                    if resp.lost_focus() {
                        let cancel = ui.input(|i| i.key_pressed(Key::Escape));
                        rename_done = Some((!cancel).then(|| (path.clone(), name.clone())));
                    }
                    continue;
                }
                let is_active = active.is_some_and(|a| same_file(a, &e.path));
                let resp = tree_row(
                    ui,
                    row.depth,
                    e,
                    self.expanded.contains(&e.path),
                    self.selected.as_deref() == Some(e.path.as_path()),
                    is_active,
                    focused,
                    &font,
                );
                if resp.clicked() || resp.double_clicked() {
                    clicked.push((e.path.clone(), e.is_dir, resp.double_clicked()));
                }
                resp.context_menu(|ui| {
                    self.selected = Some(e.path.clone());
                    entry_menu(ui, e, &self.root, &mut actions, &mut self.rename);
                });
            }
        });
        self.scroll_offset = output.state.offset.y;

        match rename_done {
            Some(Some((from, name))) => {
                self.rename = None;
                let name = name.trim();
                let unchanged = from
                    .file_name()
                    .is_some_and(|n| n.to_string_lossy() == name);
                if !name.is_empty()
                    && !unchanged
                    && let Some(parent) = from.parent()
                {
                    actions.push(TreeAction::Rename {
                        to: parent.join(name),
                        from,
                    });
                }
                ui.memory_mut(|m| m.request_focus(tree_id()));
            }
            Some(None) => {
                self.rename = None;
                ui.memory_mut(|m| m.request_focus(tree_id()));
            }
            None => {}
        }

        for (path, is_dir, double) in clicked {
            self.selected = Some(path.clone());
            ui.memory_mut(|m| m.request_focus(tree_id()));
            if is_dir {
                if !self.expanded.remove(&path) {
                    self.expanded.insert(path.clone());
                    if !self.listings.contains_key(&path) {
                        self.request(&ctx, &path);
                    }
                }
            } else if double {
                actions.push(TreeAction::Open(path));
            }
        }
        actions
    }

    /// Commands for the whole folder (header menu and empty space).
    fn folder_menu(&mut self, ui: &mut Ui, actions: &mut Vec<TreeAction>) {
        if ui.button("New File…").clicked() {
            actions.push(TreeAction::NewFile(self.root.clone()));
        }
        if ui.button("New Folder…").clicked() {
            actions.push(TreeAction::NewFolder(self.root.clone()));
        }
        ui.separator();
        if ui.button("Refresh").clicked() {
            self.refresh(ui.ctx());
        }
        if ui.button("Collapse All").clicked() {
            self.expanded.clear();
        }
        if ui
            .checkbox(&mut self.show_ignored, "Show Ignored Files")
            .changed()
        {
            self.refresh(ui.ctx());
        }
        if ui.button("Reveal in File Manager").clicked() {
            actions.push(TreeAction::RevealInFileManager(self.root.clone()));
        }
        if ui.button("Find in Folder…").clicked() {
            actions.push(TreeAction::FindInFolder(self.root.clone()));
        }
        ui.separator();
        if ui.button("Close Folder").clicked() {
            actions.push(TreeAction::CloseFolder);
        }
    }
}

/// The right-click menu of one entry.
fn entry_menu(
    ui: &mut Ui,
    e: &Entry,
    root: &Path,
    actions: &mut Vec<TreeAction>,
    rename: &mut Option<(PathBuf, String, bool)>,
) {
    let dir = if e.is_dir {
        e.path.clone()
    } else {
        e.path
            .parent()
            .map_or_else(|| root.to_path_buf(), Path::to_path_buf)
    };
    if !e.is_dir && ui.button("Open").clicked() {
        actions.push(TreeAction::Open(e.path.clone()));
    }
    if ui.button("New File…").clicked() {
        actions.push(TreeAction::NewFile(dir.clone()));
    }
    if ui.button("New Folder…").clicked() {
        actions.push(TreeAction::NewFolder(dir.clone()));
    }
    if ui.button("Find in Folder…").clicked() {
        actions.push(TreeAction::FindInFolder(dir));
    }
    ui.separator();
    if ui.button("Rename").clicked() {
        *rename = Some((e.path.clone(), e.name.clone(), true));
    }
    if ui.button("Copy Path").clicked() {
        actions.push(TreeAction::Copy(e.path.display().to_string()));
    }
    if ui.button("Copy Relative Path").clicked() {
        let rel = e.path.strip_prefix(root).unwrap_or(&e.path);
        actions.push(TreeAction::Copy(rel.display().to_string()));
    }
    if ui.button("Reveal in File Manager").clicked() {
        actions.push(TreeAction::RevealInFileManager(e.path.clone()));
    }
}

/// Whether two paths name the same file (cheap unless the names match).
fn same_file(a: &Path, b: &Path) -> bool {
    a == b
        || (a.file_name() == b.file_name()
            && std::fs::canonicalize(a)
                .ok()
                .is_some_and(|ca| std::fs::canonicalize(b).ok() == Some(ca)))
}

/// One entry's row: indent, a chevron for folders, a glyph, the name.
#[allow(clippy::too_many_arguments)]
fn tree_row(
    ui: &mut Ui,
    depth: usize,
    e: &Entry,
    open: bool,
    selected: bool,
    active: bool,
    focused: bool,
    font: &FontId,
) -> Response {
    let (rect, response) =
        ui.allocate_exact_size(Vec2::new(ui.available_width(), ROW_H), Sense::click());
    response.widget_info(|| WidgetInfo::labeled(WidgetType::Button, true, &e.name));
    let painter = ui.painter();
    let v = ui.visuals();
    if selected {
        let fill = if focused {
            v.selection.bg_fill
        } else {
            v.widgets.hovered.weak_bg_fill
        };
        painter.rect_filled(rect, 3.0, fill);
    } else if response.hovered() {
        painter.rect_filled(
            rect,
            3.0,
            v.widgets.hovered.weak_bg_fill.gamma_multiply(0.6),
        );
    }
    let x = rect.min.x + crate::theme::space::S + depth as f32 * INDENT;
    let cy = rect.center().y;
    let weak = v.weak_text_color();
    if e.is_dir {
        let c = Pos2::new(x + 4.0, cy);
        let s = 3.5;
        let points = if open {
            vec![
                c + Vec2::new(-s, -s / 2.0),
                c + Vec2::new(0.0, s / 2.0 + 1.0),
                c + Vec2::new(s, -s / 2.0),
            ]
        } else {
            vec![
                c + Vec2::new(-s / 2.0, -s),
                c + Vec2::new(s / 2.0 + 1.0, 0.0),
                c + Vec2::new(-s / 2.0, s),
            ]
        };
        painter.line(points, Stroke::new(1.5, weak));
    }
    // Glyph: a folder tab or a page.
    let g = Rect::from_center_size(Pos2::new(x + 18.0, cy), Vec2::new(11.0, 9.0));
    let glyph_color = if e.is_dir {
        crate::theme::accent(v).gamma_multiply(0.8)
    } else {
        weak
    };
    if e.is_dir {
        painter.rect_stroke(
            g,
            1.5,
            Stroke::new(1.2, glyph_color),
            egui::StrokeKind::Inside,
        );
        painter.line_segment(
            [
                g.left_top() + Vec2::new(1.0, 0.0),
                g.left_top() + Vec2::new(4.5, 0.0),
            ],
            Stroke::new(2.0, glyph_color),
        );
    } else {
        let page = Rect::from_center_size(Pos2::new(x + 18.0, cy), Vec2::new(8.0, 10.5));
        painter.rect_stroke(
            page,
            1.0,
            Stroke::new(1.2, glyph_color),
            egui::StrokeKind::Inside,
        );
    }
    let color: Color32 = if e.ignored {
        weak
    } else if active {
        v.strong_text_color()
    } else {
        v.text_color()
    };
    let galley = painter.layout_no_wrap(e.name.clone(), font.clone(), color);
    painter.with_clip_rect(rect).galley(
        Pos2::new(x + 28.0, cy - galley.size().y / 2.0),
        galley,
        color,
    );
    if active {
        // The active tab's file: an ochre mark at the left edge.
        painter.line_segment(
            [
                rect.left_top() + Vec2::new(1.0, 4.0),
                rect.left_bottom() + Vec2::new(1.0, -4.0),
            ],
            Stroke::new(2.0, crate::theme::accent(v)),
        );
    }
    response
}
