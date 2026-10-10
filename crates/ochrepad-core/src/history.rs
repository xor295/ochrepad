//! Undo/redo over [`Transaction`]s, with coalescing of consecutive typing.

use std::time::Duration;

use crate::changeset::ChangeSet;
use crate::selection::SelectionSet;

/// The concrete result of resolving an action: the edits plus the selections
/// before and after.
#[derive(Clone, Debug)]
pub struct Transaction {
    pub changes: ChangeSet,
    pub inverse: ChangeSet,
    pub before: SelectionSet,
    pub after: SelectionSet,
}

/// Which consecutive edits may merge into one undo step.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EditKind {
    /// A single grapheme typed at every caret (not a newline).
    Insert,
    DeleteBackward,
    DeleteForward,
    /// Never coalesces.
    Other,
}

/// Typing pauses longer than this start a new undo step.
pub const COALESCE_TIMEOUT: Duration = Duration::from_secs(1);

/// Identifies a document state. Undo/redo restore the state's revision, so
/// `revision == saved_revision` exactly when the text matches the saved file.
pub type Revision = u64;

#[derive(Clone, Debug)]
pub struct UndoStep {
    pub txns: Vec<Transaction>,
    pub kind: EditKind,
    pub rev_before: Revision,
    pub rev_after: Revision,
    last_edit_at: Duration,
}

#[derive(Clone, Debug, Default)]
pub struct History {
    undo: Vec<UndoStep>,
    redo: Vec<UndoStep>,
    /// False after cursor movement, undo/redo or save: the next edit starts a
    /// new step even if it would otherwise coalesce.
    coalesce_open: bool,
    /// Nesting depth of explicit groups (macro playback, replace all).
    group_depth: usize,
    /// Whether the open group already has a step on the undo stack.
    group_started: bool,
}

impl History {
    pub fn push(
        &mut self,
        txn: Transaction,
        kind: EditKind,
        rev_before: Revision,
        rev_after: Revision,
        now: Duration,
    ) {
        self.redo.clear();
        let extend_top = if self.group_depth > 0 {
            self.group_started
        } else {
            self.coalesce_open
                && kind != EditKind::Other
                && self.undo.last().is_some_and(|top| {
                    top.kind == kind
                        && top.rev_after == rev_before
                        && now.saturating_sub(top.last_edit_at) <= COALESCE_TIMEOUT
                })
        };
        match self.undo.last_mut() {
            Some(top) if extend_top => {
                top.txns.push(txn);
                top.rev_after = rev_after;
                top.last_edit_at = now;
            }
            _ => self.undo.push(UndoStep {
                txns: vec![txn],
                kind,
                rev_before,
                rev_after,
                last_edit_at: now,
            }),
        }
        self.group_started = self.group_depth > 0;
        self.coalesce_open = true;
    }

    pub fn break_coalescing(&mut self) {
        self.coalesce_open = false;
    }

    /// Everything pushed until the matching `end_group` becomes one undo step.
    pub fn begin_group(&mut self) {
        if self.group_depth == 0 {
            self.group_started = false;
        }
        self.group_depth += 1;
    }

    pub fn end_group(&mut self) {
        self.group_depth = self.group_depth.saturating_sub(1);
        if self.group_depth == 0 {
            self.group_started = false;
            self.coalesce_open = false;
        }
    }

    pub fn can_undo(&self) -> bool {
        !self.undo.is_empty()
    }

    pub fn can_redo(&self) -> bool {
        !self.redo.is_empty()
    }

    pub(crate) fn pop_undo(&mut self) -> Option<UndoStep> {
        self.coalesce_open = false;
        self.group_started = false;
        self.undo.pop()
    }

    pub(crate) fn push_redo(&mut self, step: UndoStep) {
        self.redo.push(step);
    }

    pub(crate) fn pop_redo(&mut self) -> Option<UndoStep> {
        self.coalesce_open = false;
        self.group_started = false;
        self.redo.pop()
    }

    pub(crate) fn push_undo(&mut self, step: UndoStep) {
        self.undo.push(step);
    }
}
