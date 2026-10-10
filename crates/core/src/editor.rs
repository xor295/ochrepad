//! `Editor`: owns buffers and views. [`Editor::apply`] is the only path that
//! mutates a document (PLAN 3.5).

use std::collections::BTreeMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use crate::action::{Action, ActionError};
use crate::buffer::{Buffer, BufferId, FileMeta, LineEdit};
use crate::changeset::{Bias, ChangeSet};
use crate::history::Transaction;
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

#[derive(Clone, Debug)]
pub struct Settings {
    pub tab_width: usize,
}

impl Default for Settings {
    fn default() -> Self {
        Self { tab_width: 4 }
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
        self.buffers.insert(id, Buffer::new(id, text, meta));
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

    pub fn views_of(&self, buffer: BufferId) -> impl Iterator<Item = &View> {
        self.views.values().filter(move |v| v.buffer() == buffer)
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
        match &action {
            Action::Undo => self.undo(view, buffer_id),
            Action::Redo => self.redo(view, buffer_id),
            _ => {
                let resolved = self.resolve_in(view, buffer_id, &action)??;
                self.commit(view, buffer_id, resolved)?;
            }
        }
        if let Some(rec) = &mut self.recording
            && action.is_recordable()
        {
            rec.push(action);
        }
        Ok(())
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
        let sels = &self
            .views
            .get(&view)
            .ok_or(ActionError::UnknownView(view))?
            .selections;
        Ok(match action {
            Action::Find { .. }
            | Action::Replace { .. }
            | Action::ReplaceAll { .. }
            | Action::SelectAllMatches(_) => resolve_search(action, &buffer.doc, sels),
            _ => Ok(resolve(action, &buffer.doc, sels, self.settings.tab_width)),
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
                apply_changes(buffer, &changes);
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
            }
        }
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
        for inv in &inverses {
            apply_changes(buffer, inv);
        }
        buffer.revision = step.rev_before;
        let sels = step.txns[0].before.clone();
        buffer.history.push_redo(step);
        self.set_selections(view, sels);
        for inv in &inverses {
            self.map_other_views(buffer_id, view, inv);
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
        for cs in &changes {
            apply_changes(buffer, cs);
        }
        buffer.revision = step.rev_after;
        let sels = step.txns[step.txns.len() - 1].after.clone();
        buffer.history.push_undo(step);
        self.set_selections(view, sels);
        for cs in &changes {
            self.map_other_views(buffer_id, view, cs);
        }
    }

    fn set_selections(&mut self, view: ViewId, sels: SelectionSet) {
        if let Some(v) = self.views.get_mut(&view) {
            v.selections = sels;
        }
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

fn apply_changes(buffer: &mut Buffer, changes: &ChangeSet) {
    // `ChangeSet::apply` goes back to front, so each line edit is recorded in
    // the coordinates of the document at the moment it is applied.
    for e in changes.edits().iter().rev() {
        let start = buffer.doc.line_of(e.start);
        let edit = LineEdit {
            start,
            old_end: buffer.doc.line_of(e.end),
            new_end: start + e.text.bytes().filter(|&b| b == b'\n').count(),
        };
        buffer.record_line_edit(edit);
    }
    changes.apply(&mut buffer.doc);
}
