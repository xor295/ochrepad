//! Macro playback. Recording happens in [`Editor::apply`]: while recording,
//! every successful recordable action is appended.

use std::fmt;

use serde::{Deserialize, Serialize};

use crate::action::{Action, ActionError};
use crate::editor::Editor;
use crate::view::ViewId;

/// A saved macro.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct NamedMacro {
    pub name: String,
    /// An optional key binding, e.g. "Ctrl+Shift+1" (the app parses it).
    #[serde(default)]
    pub shortcut: Option<String>,
    pub actions: Vec<Action>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Repeat {
    Times(usize),
    /// Until an action fails (typically a Find with no more matches), a run
    /// starts and ends with the primary caret at the end of the document
    /// (so a "process this line, move down" macro handles the last line
    /// once), or a run changes nothing.
    UntilEnd,
}

/// Safety limit for [`Repeat::UntilEnd`].
pub const MAX_UNTIL_END_RUNS: usize = 1_000_000;

/// Where playback stopped.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MacroError {
    /// 1-based run number.
    pub run: usize,
    /// 1-based step within the run.
    pub step: usize,
    pub action: Action,
    pub error: ActionError,
}

impl fmt::Display for MacroError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "the macro stopped at step {} of run {}: {}",
            self.step, self.run, self.error
        )
    }
}

impl std::error::Error for MacroError {}

impl Action {
    /// Whether the macro recorder keeps this action. Mouse-made selections
    /// (absolute positions) and undo/redo are left out, as in Notepad++.
    pub fn is_recordable(&self) -> bool {
        !matches!(
            self,
            Action::SetSelections(_)
                | Action::ColumnSelect { .. }
                | Action::Undo
                | Action::Redo
                | Action::Fold
                | Action::Unfold
                | Action::ToggleFold
                | Action::FoldAll
                | Action::UnfoldAll
                | Action::FoldLevel(_)
                | Action::UnfoldLevel(_)
        )
    }
}

/// (revision, selections, caret at end of document).
type MacroState = (u64, crate::selection::SelectionSet, bool);

impl Editor {
    /// Plays `actions` on `view`. The whole playback is one undo step and
    /// is not itself recorded. Returns the number of complete runs.
    ///
    /// With [`Repeat::Times`] the first failing action aborts playback and is
    /// reported. With [`Repeat::UntilEnd`] a failure is the normal way to
    /// stop.
    pub fn play_macro(
        &mut self,
        view: ViewId,
        actions: &[Action],
        repeat: Repeat,
    ) -> Result<usize, MacroError> {
        let fail = |error| MacroError {
            run: 1,
            step: 1,
            action: actions.first().cloned().unwrap_or(Action::Undo),
            error,
        };
        self.begin_undo_group(view).map_err(fail)?;
        let recording = self.take_recording();
        let result = self.run_macro(view, actions, repeat);
        self.restore_recording(recording);
        self.end_undo_group(view).map_err(fail)?;
        result
    }

    fn run_macro(
        &mut self,
        view: ViewId,
        actions: &[Action],
        repeat: Repeat,
    ) -> Result<usize, MacroError> {
        if actions.is_empty() {
            return Ok(0);
        }
        let limit = match repeat {
            Repeat::Times(n) => n,
            Repeat::UntilEnd => MAX_UNTIL_END_RUNS,
        };
        let until_end = repeat == Repeat::UntilEnd;
        let mut runs = 0;
        while runs < limit {
            let before = self.macro_state(view);
            for (i, action) in actions.iter().enumerate() {
                if let Err(error) = self.apply(view, action.clone()) {
                    if until_end {
                        return Ok(runs);
                    }
                    return Err(MacroError {
                        run: runs + 1,
                        step: i + 1,
                        action: action.clone(),
                        error,
                    });
                }
            }
            runs += 1;
            if until_end {
                let after = self.macro_state(view);
                let at_end = |s: &Option<MacroState>| s.as_ref().is_some_and(|s| s.2);
                if after == before || (at_end(&before) && at_end(&after)) {
                    break;
                }
            }
        }
        Ok(runs)
    }

    /// (revision, selections, caret at end) — to detect runs that change
    /// nothing and the end of the document.
    fn macro_state(&self, view: ViewId) -> Option<MacroState> {
        let v = self.view(view)?;
        let b = self.buffer(v.buffer())?;
        let at_end = v.selections.primary().head >= b.doc().len();
        Some((b.revision(), v.selections.clone(), at_end))
    }
}
