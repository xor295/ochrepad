//! Undo/redo over [`Transaction`]s, with coalescing of consecutive typing,
//! and the history kept between sessions ([`SavedHistory`], PLAN_v0.3.0
//! 3.3).

use std::time::Duration;

use serde::{Deserialize, Serialize};

use crate::changeset::ChangeSet;
use crate::selection::SelectionSet;
use crate::text::Document;

/// The concrete result of resolving an action: the edits plus the selections
/// before and after.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Transaction {
    pub changes: ChangeSet,
    pub inverse: ChangeSet,
    pub before: SelectionSet,
    pub after: SelectionSet,
}

/// Which consecutive edits may merge into one undo step.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
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

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct UndoStep {
    pub txns: Vec<Transaction>,
    pub kind: EditKind,
    pub rev_before: Revision,
    pub rev_after: Revision,
    /// For coalescing; not kept between sessions.
    #[serde(skip)]
    last_edit_at: Duration,
}

impl UndoStep {
    /// Roughly how many bytes the step takes saved: its text, and a little
    /// for each edit and selection.
    pub fn approx_size(&self) -> usize {
        self.txns
            .iter()
            .map(|t| {
                let edits = t.changes.edits().iter().chain(t.inverse.edits());
                let sels = t.before.len() + t.after.len();
                edits.map(|e| e.text.len() + 48).sum::<usize>() + sels * 40 + 64
            })
            .sum()
    }
}

/// An undo history as kept between sessions: the steps, and the revision of
/// the text they end at. It belongs to one exact text; whoever stores it
/// also stores that text's [`sha256_hex`] and restores it only onto the
/// same text.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct SavedHistory {
    /// Oldest first, like [`History`]'s stacks.
    pub undo: Vec<UndoStep>,
    /// The next step to redo last.
    pub redo: Vec<UndoStep>,
    /// The revision of the text the history ends at.
    pub revision: Revision,
    /// Greater than every revision in the history.
    pub next_revision: Revision,
}

impl SavedHistory {
    pub fn is_empty(&self) -> bool {
        self.undo.is_empty() && self.redo.is_empty()
    }

    /// Checks the history against `doc`, the text it should end at: the
    /// revisions connect, and every step's edits and selections fit the
    /// text they apply to (the earlier texts are rebuilt by undoing, the
    /// later ones by redoing). A damaged or mismatched file fails here
    /// instead of corrupting the document or crashing later.
    pub fn check(&self, doc: &Document) -> Result<(), String> {
        let all = self.undo.iter().chain(&self.redo);
        if all
            .clone()
            .any(|s| s.rev_before >= self.next_revision || s.rev_after >= self.next_revision)
        {
            return Err("a revision is out of range".into());
        }
        if self
            .undo
            .last()
            .is_some_and(|s| s.rev_after != self.revision)
            || self
                .redo
                .last()
                .is_some_and(|s| s.rev_before != self.revision)
            || self
                .undo
                .windows(2)
                .any(|w| w[0].rev_after != w[1].rev_before)
            || self
                .redo
                .windows(2)
                .any(|w| w[1].rev_after != w[0].rev_before)
        {
            return Err("the steps do not connect".into());
        }
        if all.clone().any(|s| s.txns.is_empty()) {
            return Err("a step is empty".into());
        }
        let bad = |what: &str| Err(format!("{what} does not fit the text"));
        let mut d = doc.clone();
        for t in self.undo.iter().rev().flat_map(|s| s.txns.iter().rev()) {
            if !t.after.fits(&d) || !t.inverse.fits(&d) {
                return bad("an undo step");
            }
            t.inverse.apply(&mut d);
            if !t.changes.fits(&d) || !t.before.fits(&d) {
                return bad("an undo step");
            }
        }
        let mut d = doc.clone();
        for t in self.redo.iter().rev().flat_map(|s| s.txns.iter()) {
            if !t.before.fits(&d) || !t.changes.fits(&d) {
                return bad("a redo step");
            }
            t.changes.apply(&mut d);
            if !t.inverse.fits(&d) || !t.after.fits(&d) {
                return bad("a redo step");
            }
        }
        Ok(())
    }
}

/// The SHA-256 of a document's text, as lowercase hex: what a saved history
/// is checked against.
pub fn sha256_hex(doc: &Document) -> String {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    for chunk in doc.chunks() {
        hasher.update(chunk.as_bytes());
    }
    hasher
        .finalize()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
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

    /// The newest steps whose [`approx_size`](UndoStep::approx_size) adds
    /// up to at most `max_bytes`: the redo steps nearest the current text
    /// first, then undo steps from the most recent back. Returns (undo,
    /// redo), oldest first.
    pub(crate) fn newest(&self, max_bytes: usize) -> (Vec<UndoStep>, Vec<UndoStep>) {
        let mut budget = max_bytes;
        let mut take = |steps: &[UndoStep]| {
            let mut kept: Vec<UndoStep> = Vec::new();
            for s in steps.iter().rev() {
                let size = s.approx_size();
                if size > budget {
                    break;
                }
                budget -= size;
                kept.push(s.clone());
            }
            kept.reverse();
            kept
        };
        let redo = take(&self.redo);
        let undo = take(&self.undo);
        (undo, redo)
    }

    /// Replaces the stacks with ones from an earlier session. Coalescing
    /// starts afresh: the next edit is a new step.
    pub(crate) fn replace(&mut self, undo: Vec<UndoStep>, redo: Vec<UndoStep>) {
        *self = History {
            undo,
            redo,
            ..History::default()
        };
    }

    /// Forgets every step.
    pub fn clear(&mut self) {
        *self = History::default();
    }
}
