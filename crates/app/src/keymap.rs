//! Key bindings for the editor widget: key + modifiers -> command. Defaults
//! follow Notepad++ where it has a binding.

use egui::{Key, Modifiers};
use ochrepad_core::{Action, Motion};

#[derive(Clone, Debug, PartialEq)]
pub enum EditorCommand {
    Action(Action),
    /// Page moves; the widget fills in its page height.
    PageUp {
        extend: bool,
    },
    PageDown {
        extend: bool,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Chord {
    pub key: Key,
    /// Ctrl on Windows/Linux, Cmd on macOS.
    pub command: bool,
    pub shift: bool,
    pub alt: bool,
}

impl Chord {
    const fn new(key: Key) -> Self {
        Self {
            key,
            command: false,
            shift: false,
            alt: false,
        }
    }
    const fn cmd(mut self) -> Self {
        self.command = true;
        self
    }
    const fn shift(mut self) -> Self {
        self.shift = true;
        self
    }
    const fn alt(mut self) -> Self {
        self.alt = true;
        self
    }

    fn matches(&self, key: Key, m: Modifiers) -> bool {
        self.key == key && self.command == m.command && self.shift == m.shift && self.alt == m.alt
    }
}

pub struct Keymap {
    bindings: Vec<(Chord, EditorCommand)>,
}

fn mv(motion: Motion, extend: bool) -> EditorCommand {
    EditorCommand::Action(Action::MoveCursor { motion, extend })
}

fn act(a: Action) -> EditorCommand {
    EditorCommand::Action(a)
}

impl Default for Keymap {
    fn default() -> Self {
        use Key::*;
        let mut b: Vec<(Chord, EditorCommand)> = Vec::new();
        // Motions, each with a Shift (extend) variant.
        let motions = [
            (Chord::new(ArrowLeft), Motion::Left),
            (Chord::new(ArrowRight), Motion::Right),
            (Chord::new(ArrowLeft).cmd(), Motion::WordLeft),
            (Chord::new(ArrowRight).cmd(), Motion::WordRight),
            (Chord::new(ArrowUp), Motion::Up),
            (Chord::new(ArrowDown), Motion::Down),
            (Chord::new(Home), Motion::Home),
            (Chord::new(End), Motion::LineEnd),
            (Chord::new(Home).cmd(), Motion::DocStart),
            (Chord::new(End).cmd(), Motion::DocEnd),
        ];
        for (chord, motion) in motions {
            b.push((chord, mv(motion, false)));
            b.push((chord.shift(), mv(motion, true)));
        }
        for extend in [false, true] {
            let up = Chord::new(PageUp);
            let down = Chord::new(PageDown);
            let (up, down) = if extend {
                (up.shift(), down.shift())
            } else {
                (up, down)
            };
            b.push((up, EditorCommand::PageUp { extend }));
            b.push((down, EditorCommand::PageDown { extend }));
        }
        b.extend([
            (Chord::new(Backspace), act(Action::DeleteBackward)),
            (Chord::new(Backspace).shift(), act(Action::DeleteBackward)),
            (Chord::new(Delete), act(Action::DeleteForward)),
            (Chord::new(Backspace).cmd(), act(Action::DeleteWordBackward)),
            (Chord::new(Delete).cmd(), act(Action::DeleteWordForward)),
            (Chord::new(Enter), act(Action::InsertNewline)),
            (Chord::new(Enter).shift(), act(Action::InsertNewline)),
            (Chord::new(Tab), act(Action::InsertText("\t".to_owned()))),
            (Chord::new(Escape), act(Action::CollapseSelections)),
            (Chord::new(A).cmd(), act(Action::SelectAll)),
            (Chord::new(Z).cmd(), act(Action::Undo)),
            (Chord::new(Y).cmd(), act(Action::Redo)),
            (Chord::new(Z).cmd().shift(), act(Action::Redo)),
            (Chord::new(ArrowUp).cmd().alt(), act(Action::AddCursorAbove)),
            (
                Chord::new(ArrowDown).cmd().alt(),
                act(Action::AddCursorBelow),
            ),
        ]);
        Self { bindings: b }
    }
}

impl Keymap {
    pub fn lookup(&self, key: Key, modifiers: Modifiers) -> Option<&EditorCommand> {
        self.bindings
            .iter()
            .find(|(c, _)| c.matches(key, modifiers))
            .map(|(_, cmd)| cmd)
    }
}
