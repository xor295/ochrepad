//! `Editor`: owns buffers and views. [`Editor::apply`] is the only path that
//! mutates a document (PLAN 3.5).

use std::collections::BTreeMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use crate::action::{Action, ActionError};
use crate::buffer::{Buffer, BufferId, FileMeta, Indent, LineEdit};
use crate::changeset::{Bias, ChangeSet};
use crate::display::DisplayMap;
use crate::fold::{self, FoldSet};
use crate::history::Transaction;
use crate::position::{ByteOffset, byte_to_line_col, line_col_to_byte};
use crate::resolve::{Resolved, resolve, resolve_search};
use crate::selection::{Selection, SelectionSet};
use crate::view::{View, ViewId};

/// Monotonic time source for undo coalescing; replaceable in tests.
pub trait Clock: Send {
    fn now(&self) -> Duration;
}

pub struct SystemClock {
    origin: Instant,
}

impl Default for SystemClock {
    fn default() -> Self {
        Self {
            origin: Instant::now(),
        }
    }
}

impl Clock for SystemClock {
    fn now(&self) -> Duration {
        self.origin.elapsed()
    }
}

/// A clock that only moves when told to. Clones share the same time.
#[derive(Clone, Default)]
pub struct ManualClock(Arc<AtomicU64>);

impl ManualClock {
    pub fn advance(&self, d: Duration) {
        self.0.fetch_add(d.as_millis() as u64, Ordering::Relaxed);
    }
}

impl Clock for ManualClock {
    fn now(&self) -> Duration {
        Duration::from_millis(self.0.load(Ordering::Relaxed))
    }
}

/// Defaults for new buffers (each buffer has its own [`Indent`]).
#[derive(Clone, Debug)]
pub struct Settings {
    pub tab_width: usize,
    /// The Tab key inserts spaces instead of a tab character.
    pub use_spaces: bool,
    /// Typing an opening bracket or quote inserts its partner (Notepad++'s
    /// Auto-Insert; see `autoclose`).
    pub auto_close: bool,
}

impl Settings {
    /// The indentation a new buffer starts with.
    pub fn indent(&self) -> Indent {
        Indent {
            use_spaces: self.use_spaces,
            width: self.tab_width,
        }
    }
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            tab_width: 4,
            use_spaces: false,
            auto_close: false,
        }
    }
}

pub struct Editor {
    buffers: BTreeMap<BufferId, Buffer>,
    views: BTreeMap<ViewId, View>,
    next_id: u64,
    pub settings: Settings,
    clock: Box<dyn Clock>,
    recording: Option<Vec<Action>>,
}

impl Default for Editor {
    fn default() -> Self {
        Self::with_clock(Box::new(SystemClock::default()))
    }
}

impl Editor {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_clock(clock: Box<dyn Clock>) -> Self {
        Self {
            buffers: BTreeMap::new(),
            views: BTreeMap::new(),
            next_id: 1,
            settings: Settings::default(),
            clock,
            recording: None,
        }
    }

    fn fresh_id(&mut self) -> u64 {
        let id = self.next_id;
        self.next_id += 1;
        id
    }

    pub fn add_buffer(&mut self, text: &str, meta: FileMeta) -> BufferId {
        let id = BufferId(self.fresh_id());
        let mut buffer = Buffer::new(id, text, meta);
        buffer.indent = self.settings.indent();
        self.buffers.insert(id, buffer);
        id
    }

    pub fn add_view(&mut self, buffer: BufferId) -> Result<ViewId, ActionError> {
        if !self.buffers.contains_key(&buffer) {
            return Err(ActionError::UnknownBuffer(buffer));
        }
        let id = ViewId(self.fresh_id());
        self.views.insert(id, View::new(id, buffer));
        Ok(id)
    }

    pub fn remove_view(&mut self, view: ViewId) -> Option<View> {
        self.views.remove(&view)
    }

    /// Removes the buffer and every view of it.
    pub fn remove_buffer(&mut self, buffer: BufferId) -> Option<Buffer> {
        self.views.retain(|_, v| v.buffer() != buffer);
        self.buffers.remove(&buffer)
    }

    pub fn buffer(&self, id: BufferId) -> Option<&Buffer> {
        self.buffers.get(&id)
    }

    pub fn buffer_mut(&mut self, id: BufferId) -> Option<&mut Buffer> {
        self.buffers.get_mut(&id)
    }

    pub fn view(&self, id: ViewId) -> Option<&View> {
        self.views.get(&id)
    }

    pub fn view_mut(&mut self, id: ViewId) -> Option<&mut View> {
        self.views.get_mut(&id)
    }

    pub fn buffers(&self) -> impl Iterator<Item = &Buffer> {
        self.buffers.values()
    }

    pub fn buffers_mut(&mut self) -> impl Iterator<Item = &mut Buffer> {
        self.buffers.values_mut()
    }

    pub fn views(&self) -> impl Iterator<Item = &View> {
        self.views.values()
    }

    /// The views of `buffer` that are tabs (not auxiliary ones).
    pub fn views_of(&self, buffer: BufferId) -> impl Iterator<Item = &View> {
        self.views
            .values()
            .filter(move |v| v.buffer() == buffer && !v.is_aux())
    }

    /// A view that is not a tab of its own (one side of a compare view). It
    /// follows edits like any view, is left out of
    /// [`views_of`](Self::views_of), and goes when its buffer is removed.
    pub fn add_aux_view(&mut self, buffer: BufferId) -> Result<ViewId, ActionError> {
        let id = self.add_view(buffer)?;
        if let Some(v) = self.views.get_mut(&id) {
            v.set_aux();
        }
        Ok(id)
    }

    pub fn start_recording(&mut self) {
        self.recording = Some(Vec::new());
    }

    pub fn is_recording(&self) -> bool {
        self.recording.is_some()
    }

    pub fn stop_recording(&mut self) -> Vec<Action> {
        self.recording.take().unwrap_or_default()
    }

    /// The actions recorded so far, while recording.
    pub fn recorded(&self) -> Option<&[Action]> {
        self.recording.as_deref()
    }

    pub(crate) fn take_recording(&mut self) -> Option<Vec<Action>> {
        self.recording.take()
    }

    pub(crate) fn restore_recording(&mut self, recording: Option<Vec<Action>>) {
        self.recording = recording;
    }

    /// Everything applied to `view`'s buffer until the matching
    /// [`Self::end_undo_group`] undoes as one step.
    pub fn begin_undo_group(&mut self, view: ViewId) -> Result<(), ActionError> {
        let (buffer, _) = self.lookup(view)?;
        self.buffers
            .get_mut(&buffer)
            .ok_or(ActionError::UnknownBuffer(buffer))?
            .history
            .begin_group();
        Ok(())
    }

    pub fn end_undo_group(&mut self, view: ViewId) -> Result<(), ActionError> {
        let (buffer, _) = self.lookup(view)?;
        self.buffers
            .get_mut(&buffer)
            .ok_or(ActionError::UnknownBuffer(buffer))?
            .history
            .end_group();
        Ok(())
    }

    fn lookup(&self, view: ViewId) -> Result<(BufferId, &View), ActionError> {
        let v = self
            .views
            .get(&view)
            .ok_or(ActionError::UnknownView(view))?;
        if !self.buffers.contains_key(&v.buffer()) {
            return Err(ActionError::UnknownBuffer(v.buffer()));
        }
        Ok((v.buffer(), v))
    }

    /// Resolves `action` against `view` and applies it: edits the rope, pushes
    /// undo, maps other views' selections, marks dirty lines for the
    /// highlighter, bumps the revision and records the action if recording.
    pub fn apply(&mut self, view: ViewId, action: Action) -> Result<(), ActionError> {
        let (buffer_id, _) = self.lookup(view)?;
        let read_only = self.buffers.get(&buffer_id).is_some_and(|b| b.read_only);
        match &action {
            Action::Undo | Action::Redo if read_only => return Err(ActionError::ReadOnly),
            Action::Undo => self.undo(view, buffer_id),
            Action::Redo => self.redo(view, buffer_id),
            Action::Fold
            | Action::Unfold
            | Action::ToggleFold
            | Action::FoldAll
            | Action::UnfoldAll
            | Action::FoldLevel(_)
            | Action::UnfoldLevel(_) => self.fold_action(view, buffer_id, &action),
            _ => {
                let resolved = self.resolve_in(view, buffer_id, &action)??;
                self.commit(view, buffer_id, resolved)?;
                if let Action::ConvertIndentation { to_spaces } = action
                    && let Some(b) = self.buffers.get_mut(&buffer_id)
                {
                    b.indent.use_spaces = to_spaces;
                    b.indent_set = true;
                    b.indent_mixed = false;
                }
            }
        }
        self.sync_folds(buffer_id);
        if let Some(rec) = &mut self.recording
            && action.is_recordable()
        {
            rec.push(action);
        }
        Ok(())
    }

    /// Like [`apply`](Self::apply), but never recorded in a macro being
    /// recorded: for commands that act outside the view the macro plays in
    /// (Replace All in all open documents).
    pub fn apply_unrecorded(&mut self, view: ViewId, action: Action) -> Result<(), ActionError> {
        let recording = self.recording.take();
        let result = self.apply(view, action);
        self.recording = recording;
        result
    }

    /// Resolves `action` against the view's current state. The outer error
    /// is a missing view/buffer; the inner one the action's own failure.
    fn resolve_in(
        &self,
        view: ViewId,
        buffer_id: BufferId,
        action: &Action,
    ) -> Result<Result<Resolved, ActionError>, ActionError> {
        let buffer = self
            .buffers
            .get(&buffer_id)
            .ok_or(ActionError::UnknownBuffer(buffer_id))?;
        let v = self
            .views
            .get(&view)
            .ok_or(ActionError::UnknownView(view))?;
        let sels = &v.selections;
        Ok(match action {
            Action::Find { .. }
            | Action::Replace { .. }
            | Action::ReplaceAll { .. }
            | Action::SelectAllMatches(_) => resolve_search(action, &buffer.doc, sels),
            Action::ToggleLineComment | Action::ToggleBlockComment => {
                let syntax = buffer.comments.as_ref();
                let resolved = match (action, syntax) {
                    (Action::ToggleLineComment, Some(c)) => {
                        crate::transform::toggle_comment_lines(&buffer.doc, sels, c)
                    }
                    (_, Some(c)) => c.block.as_ref().map(|(open, close)| {
                        crate::transform::toggle_block_comment(&buffer.doc, sels, open, close)
                    }),
                    _ => None,
                };
                resolved.ok_or(ActionError::NoCommentSyntax)
            }
            Action::FormatJson { .. }
            | Action::MinifyJson
            | Action::Encode(_)
            | Action::Decode(_) => crate::text_tools::resolve(
                action,
                &buffer.doc,
                sels,
                buffer.indent,
                buffer.meta.line_ending,
            ),
            _ => {
                if self.settings.auto_close
                    && let Some(r) = crate::autoclose::resolve(action, &buffer.doc, sels)
                {
                    return Ok(Ok(r));
                }
                let rows = DisplayMap::new(buffer, v);
                Ok(resolve(action, &buffer.doc, sels, buffer.indent, &rows))
            }
        })
    }

    /// Applies a resolved action: edits the rope, pushes undo, maps other
    /// views' selections, bumps the revision.
    fn commit(
        &mut self,
        view: ViewId,
        buffer_id: BufferId,
        resolved: Resolved,
    ) -> Result<(), ActionError> {
        match resolved {
            Resolved::Selections(sels) => {
                if let Some(b) = self.buffers.get_mut(&buffer_id) {
                    b.history.break_coalescing();
                }
                self.set_selections(view, sels);
            }
            Resolved::Edit { .. } if self.buffers.get(&buffer_id).is_some_and(|b| b.read_only) => {
                return Err(ActionError::ReadOnly);
            }
            Resolved::Edit {
                changes,
                after,
                kind,
            } => {
                let now = self.clock.now();
                let before = self.views[&view].selections.clone();
                let Some(buffer) = self.buffers.get_mut(&buffer_id) else {
                    return Err(ActionError::UnknownBuffer(buffer_id));
                };
                let inverse = changes.invert(&buffer.doc);
                let line_edits = apply_changes(buffer, &changes);
                let rev_before = buffer.revision;
                let rev_after = buffer.next_revision;
                buffer.next_revision += 1;
                buffer.revision = rev_after;
                buffer.history.push(
                    Transaction {
                        changes: changes.clone(),
                        inverse,
                        before,
                        after: after.clone(),
                    },
                    kind,
                    rev_before,
                    rev_after,
                    now,
                );
                self.set_selections(view, after);
                self.map_other_views(buffer_id, view, &changes);
                self.map_folds(buffer_id, &changes, &line_edits);
            }
        }
        Ok(())
    }

    /// Replaces a buffer's text with `text` read from disk (an external
    /// change), as one undoable step; carets keep their line and column.
    /// The buffer takes `meta` and is marked saved.
    pub fn reload_buffer(
        &mut self,
        buffer: BufferId,
        text: &str,
        meta: FileMeta,
    ) -> Result<(), ActionError> {
        let view = self
            .views_of(buffer)
            .next()
            .map(View::id)
            .ok_or(ActionError::UnknownBuffer(buffer))?;
        let b = self
            .buffers
            .get(&buffer)
            .ok_or(ActionError::UnknownBuffer(buffer))?;
        if b.doc.to_string() != text {
            let new_doc = crate::text::Document::new(text);
            let remap = |p: ByteOffset| {
                let lc = byte_to_line_col(&b.doc, p);
                let mut q = line_col_to_byte(&new_doc, lc);
                while !new_doc.is_char_boundary(q) {
                    q.0 -= 1;
                }
                q
            };
            let after = self.views[&view]
                .selections
                .map(|s| Selection::new(remap(s.anchor), remap(s.head)));
            let changes = ChangeSet::new(
                b.doc.len(),
                vec![crate::changeset::Edit::new(
                    ByteOffset(0),
                    b.doc.len(),
                    text,
                )],
            );
            self.commit(
                view,
                buffer,
                Resolved::Edit {
                    changes,
                    after,
                    kind: crate::history::EditKind::Other,
                },
            )?;
        }
        if let Some(b) = self.buffers.get_mut(&buffer) {
            b.meta = meta;
            b.mark_saved();
        }
        self.sync_folds(buffer);
        Ok(())
    }

    fn undo(&mut self, view: ViewId, buffer_id: BufferId) {
        let Some(buffer) = self.buffers.get_mut(&buffer_id) else {
            return;
        };
        let Some(step) = buffer.history.pop_undo() else {
            return;
        };
        let inverses: Vec<ChangeSet> = step.txns.iter().rev().map(|t| t.inverse.clone()).collect();
        let line_edits: Vec<Vec<LineEdit>> = inverses
            .iter()
            .map(|inv| apply_changes(buffer, inv))
            .collect();
        buffer.revision = step.rev_before;
        let sels = step.txns[0].before.clone();
        buffer.history.push_redo(step);
        self.set_selections(view, sels);
        for (inv, edits) in inverses.iter().zip(&line_edits) {
            self.map_other_views(buffer_id, view, inv);
            self.map_folds(buffer_id, inv, edits);
        }
    }

    fn redo(&mut self, view: ViewId, buffer_id: BufferId) {
        let Some(buffer) = self.buffers.get_mut(&buffer_id) else {
            return;
        };
        let Some(step) = buffer.history.pop_redo() else {
            return;
        };
        let changes: Vec<ChangeSet> = step.txns.iter().map(|t| t.changes.clone()).collect();
        let line_edits: Vec<Vec<LineEdit>> =
            changes.iter().map(|cs| apply_changes(buffer, cs)).collect();
        buffer.revision = step.rev_after;
        let sels = step.txns[step.txns.len() - 1].after.clone();
        buffer.history.push_undo(step);
        self.set_selections(view, sels);
        for (cs, edits) in changes.iter().zip(&line_edits) {
            self.map_other_views(buffer_id, view, cs);
            self.map_folds(buffer_id, cs, edits);
        }
    }

    fn set_selections(&mut self, view: ViewId, sels: SelectionSet) {
        if let Some(v) = self.views.get_mut(&view) {
            v.selections = sels;
        }
    }

    /// Maps every view's folds through `changes`, dropping the folds it
    /// touches (PLAN_v0.2.0 3.6). [`sync_folds`](Self::sync_folds) then
    /// recomputes their lines. A filter moves by `line_edits` (the same
    /// change by line), showing the lines it touched.
    fn map_folds(&mut self, buffer: BufferId, changes: &ChangeSet, line_edits: &[LineEdit]) {
        for v in self.views.values_mut() {
            if v.buffer() != buffer {
                continue;
            }
            if !v.folds.is_empty() {
                v.folds.map(changes);
            }
            v.folds.filter_line_edits(line_edits);
        }
    }

    /// Appends `text` that the file on disk gained (follow mode, PLAN_v0.3.0
    /// P6). It is not an undo step: undoing earlier steps keeps it. A clean
    /// buffer stays clean. Carets at the end move along to the new end.
    pub fn append_unrecorded(
        &mut self,
        buffer_id: BufferId,
        text: &str,
    ) -> Result<(), ActionError> {
        let buffer = self
            .buffers
            .get_mut(&buffer_id)
            .ok_or(ActionError::UnknownBuffer(buffer_id))?;
        if text.is_empty() {
            return Ok(());
        }
        let end = buffer.doc.len();
        let changes = ChangeSet::new(end, vec![crate::changeset::Edit::new(end, end, text)]);
        let clean = !buffer.is_dirty();
        let line_edits = apply_changes(buffer, &changes);
        let (old, new) = (buffer.revision, buffer.next_revision);
        buffer.history.extend_base(text.len(), old, new);
        buffer.revision = new;
        buffer.next_revision += 1;
        if clean {
            buffer.mark_saved();
        }
        for v in self.views.values_mut() {
            if v.buffer() == buffer_id {
                v.selections = v.selections.map(|s| Selection {
                    anchor: changes.map_position(s.anchor, Bias::After),
                    head: changes.map_position(s.head, Bias::After),
                    goal: None,
                });
            }
        }
        self.map_folds(buffer_id, &changes, &line_edits);
        self.sync_folds(buffer_id);
        Ok(())
    }

    /// Shows only the lines `shown` (sorted line numbers) of `view`, or
    /// every line again with `None`: the filter view (PLAN_v0.3.0 P6).
    /// Carets on lines it hides move to the start of the nearest shown
    /// line. With no line to show, the primary caret's line stays.
    pub fn set_filter(&mut self, view: ViewId, shown: Option<&[usize]>) {
        let Some(v) = self.views.get_mut(&view) else {
            return;
        };
        let Some(b) = self.buffers.get(&v.buffer()) else {
            return;
        };
        let doc = &b.doc;
        let count = doc.line_count();
        let Some(shown) = shown else {
            v.folds.set_filter(None, count);
            return;
        };
        let caret_line = [doc.line_of(v.selections.primary().head)];
        let in_doc = shown.partition_point(|&l| l < count);
        let shown = if in_doc == 0 {
            &caret_line[..]
        } else {
            &shown[..in_doc]
        };
        v.folds.set_filter(Some(shown), count);
        let folds = &v.folds;
        let moved = v.selections.map(|s| {
            let line = doc.line_of(s.head);
            if !folds.filter_hides(line) {
                return s.clone();
            }
            let after = shown.partition_point(|&l| l < line);
            let target = shown
                .get(after)
                .or_else(|| after.checked_sub(1).and_then(|i| shown.get(i)))
                .copied()
                .unwrap_or(0)
                .min(count - 1);
            Selection::caret(doc.line_start(target))
        });
        v.selections = moved;
    }

    /// After text was appended to a filtered `view` (follow mode): hides
    /// the lines from `from` on but `shown` (sorted) and the lines holding
    /// a caret, leaving the lines before `from` as they were.
    pub fn filter_appended(&mut self, view: ViewId, from: usize, shown: &[usize]) {
        let Some(v) = self.views.get_mut(&view) else {
            return;
        };
        let Some(b) = self.buffers.get(&v.buffer()) else {
            return;
        };
        let doc = &b.doc;
        let mut lines = shown.to_vec();
        lines.extend(
            v.selections
                .ranges()
                .iter()
                .map(|s| doc.line_of(s.head))
                .filter(|&l| l >= from),
        );
        lines.sort_unstable();
        lines.dedup();
        v.folds.set_filter_from(from, &lines, doc.line_count());
    }

    /// The lines of `view` holding a selection's head, in order.
    pub fn caret_lines(&self, view: ViewId) -> Vec<usize> {
        let Some(v) = self.views.get(&view) else {
            return Vec::new();
        };
        let Some(b) = self.buffers.get(&v.buffer()) else {
            return Vec::new();
        };
        let mut lines: Vec<usize> = v
            .selections
            .ranges()
            .iter()
            .map(|s| b.doc.line_of(s.head))
            .collect();
        lines.dedup();
        lines
    }

    /// After any action: brings the folds' line numbers up to date and
    /// unfolds every fold hiding a caret, in every view of `buffer`.
    fn sync_folds(&mut self, buffer: BufferId) {
        let Some(doc) = self.buffers.get(&buffer).map(|b| &b.doc) else {
            return;
        };
        for v in self.views.values_mut() {
            if v.buffer() != buffer {
                continue;
            }
            // Also when an edit dropped the last fold: the hidden lines need
            // rebuilding. Cheap when nothing was mapped.
            v.folds.relines(doc);
            // A caret the filter hides (Go to Line, a match on a hidden
            // line, an edit elsewhere) brings its line back.
            if v.folds.has_filter() {
                let lines: Vec<usize> = v
                    .selections
                    .ranges()
                    .iter()
                    .map(|s| doc.line_of(s.head))
                    .filter(|&l| v.folds.filter_hides(l))
                    .collect();
                v.folds.unfilter(&lines);
            }
            if v.folds.is_empty() {
                continue;
            }
            let hidden: Vec<usize> = v
                .selections
                .ranges()
                .iter()
                .map(|s| doc.line_of(s.head))
                .filter(|&l| v.folds.is_hidden(l))
                .collect();
            if !hidden.is_empty() {
                v.folds
                    .retain(|header, last| !hidden.iter().any(|&l| header < l && l <= last));
            }
        }
    }

    /// Folds the region `line` heads in `view`; returns whether it did.
    /// Carets inside move to the end of the header line.
    pub fn fold_at(&mut self, view: ViewId, line: usize) -> bool {
        let Some(v) = self.views.get_mut(&view) else {
            return false;
        };
        let Some(b) = self.buffers.get(&v.buffer()) else {
            return false;
        };
        let tab_width = b.indent.width;
        let Some(last) = fold::region(&b.doc, line, b.fold_strategy, tab_width) else {
            return false;
        };
        v.folds.insert(&b.doc, [(line, last)]);
        reveal_carets(&b.doc, v);
        true
    }

    /// Unfolds the fold at `line` in `view`; returns whether there was one.
    pub fn unfold_at(&mut self, view: ViewId, line: usize) -> bool {
        self.views
            .get_mut(&view)
            .is_some_and(|v| v.folds.remove(line))
    }

    /// Unfolds `line` if it is folded, else folds the region it heads.
    pub fn toggle_fold_at(&mut self, view: ViewId, line: usize) -> bool {
        self.unfold_at(view, line) || self.fold_at(view, line)
    }

    fn fold_action(&mut self, view: ViewId, buffer: BufferId, action: &Action) {
        let (Some(v), Some(b)) = (self.views.get_mut(&view), self.buffers.get(&buffer)) else {
            return;
        };
        let tab_width = b.indent.width;
        let (doc, strategy) = (&b.doc, b.fold_strategy);
        let caret = doc.line_of(v.selections.primary().head);
        let around = || fold::enclosing(doc, caret, strategy, tab_width);
        match action {
            Action::Fold => {
                if let Some(r) = around() {
                    v.folds.insert(doc, [r]);
                }
            }
            Action::ToggleFold => {
                if !v.folds.remove(caret)
                    && let Some(r) = around()
                {
                    v.folds.insert(doc, [r]);
                }
            }
            Action::Unfold => {
                if !v.folds.remove(caret)
                    && let Some((header, last)) = around()
                {
                    v.folds.retain(|h, _| h < header || h > last);
                }
            }
            Action::FoldAll => {
                let all = fold::regions(doc, strategy, tab_width);
                v.folds.insert(doc, all.iter().map(|r| (r.header, r.last)));
            }
            Action::UnfoldAll => v.folds = FoldSet::default(),
            Action::FoldLevel(n) => {
                let all = fold::regions(doc, strategy, tab_width);
                let level = all.iter().filter(|r| r.level == *n);
                v.folds.insert(doc, level.map(|r| (r.header, r.last)));
            }
            Action::UnfoldLevel(n) => {
                let level: std::collections::HashSet<usize> =
                    fold::regions(doc, strategy, tab_width)
                        .into_iter()
                        .filter(|r| r.level == *n)
                        .map(|r| r.header)
                        .collect();
                v.folds.retain(|h, _| !level.contains(&h));
            }
            _ => {}
        }
        reveal_carets(doc, v);
    }

    /// Shifts the selections of every other view of `buffer` through `changes`
    /// so their carets stay put relative to the text.
    fn map_other_views(&mut self, buffer: BufferId, except: ViewId, changes: &ChangeSet) {
        for v in self.views.values_mut() {
            if v.buffer() == buffer && v.id() != except {
                v.selections = v.selections.map(|s| Selection {
                    anchor: changes.map_position(s.anchor, Bias::Before),
                    head: changes.map_position(s.head, Bias::Before),
                    goal: s.goal,
                });
            }
        }
    }
}

/// Moves every selection head that a fold hides to the end of the fold's
/// header line, so folding does not immediately unfold again.
fn reveal_carets(doc: &crate::text::Document, v: &mut View) {
    let folds = &v.folds;
    if folds.is_empty() {
        return;
    }
    let moved = v.selections.map(|s| {
        let Some((start, _)) = folds.fold_hidden_range(doc.line_of(s.head)) else {
            return s.clone();
        };
        let head = doc.line_end(start - 1);
        if s.is_empty() {
            Selection::caret(head)
        } else {
            Selection::new(s.anchor, head)
        }
    });
    v.selections = moved;
}

/// Applies `changes` to the buffer's text and returns how the lines changed,
/// in the order the edits were applied.
fn apply_changes(buffer: &mut Buffer, changes: &ChangeSet) -> Vec<LineEdit> {
    // `ChangeSet::apply` goes back to front, so each line edit is recorded in
    // the coordinates of the document at the moment it is applied.
    let mut edits = Vec::with_capacity(changes.edits().len());
    for e in changes.edits().iter().rev() {
        let start = buffer.doc.line_of(e.start);
        let edit = LineEdit {
            start,
            old_end: buffer.doc.line_of(e.end),
            new_end: start + e.text.bytes().filter(|&b| b == b'\n').count(),
        };
        buffer.record_line_edit(edit);
        edits.push(edit);
    }
    changes.apply(&mut buffer.doc);
    edits
}
