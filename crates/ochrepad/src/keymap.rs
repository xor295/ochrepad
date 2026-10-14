//! Key bindings for the editor widget: key + modifiers -> command. Defaults
//! follow Notepad++ where it has a binding. A `keymap.json` in the config
//! directory adds, replaces or removes bindings (see [`Keymap::with_file`]).

use egui::{Key, Modifiers};
use ochrepad_core::{Action, CaseMode, Direction, Motion};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum EditorCommand {
    Action(Action),
    /// Page moves; the widget fills in its page height.
    PageUp {
        extend: bool,
    },
    PageDown {
        extend: bool,
    },
    /// Switches between inserting and typing over (Insert).
    ToggleOvertype,
    /// Edit > Begin/End Select: marks where a selection begins, or selects
    /// from there to the caret.
    BeginEndSelect,
    /// The same, ending in a column selection.
    BeginEndSelectColumn,
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

    /// Parses "Ctrl+Shift+Up" (Ctrl and Cmd both mean the command key).
    pub fn parse(text: &str) -> Option<Chord> {
        let parts: Vec<&str> = text.split('+').map(str::trim).collect();
        let (key, mods) = parts.split_last()?;
        let key = Key::from_name(key).or_else(|| Key::from_name(&key.to_ascii_uppercase()))?;
        let mut chord = Chord::new(key);
        for m in mods {
            match m.to_ascii_lowercase().as_str() {
                "ctrl" | "cmd" | "command" => chord.command = true,
                "shift" => chord.shift = true,
                "alt" => chord.alt = true,
                _ => return None,
            }
        }
        Some(chord)
    }
}

impl std::fmt::Display for Chord {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if self.command {
            f.write_str("Ctrl+")?;
        }
        if self.alt {
            f.write_str("Alt+")?;
        }
        if self.shift {
            f.write_str("Shift+")?;
        }
        f.write_str(self.key.name())
    }
}

/// One entry of `keymap.json`. `command: null` removes the binding.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct KeymapEntry {
    pub keys: String,
    pub command: Option<EditorCommand>,
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
            // Row-aware: the same as Home and LineEnd unless lines wrap.
            (Chord::new(Home), Motion::RowHome),
            (Chord::new(End), Motion::RowEnd),
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
            (Chord::new(Tab), act(Action::InsertTab)),
            (Chord::new(Escape), act(Action::CollapseSelections)),
            (Chord::new(Insert), EditorCommand::ToggleOvertype),
            (Chord::new(A).cmd(), act(Action::SelectAll)),
            (Chord::new(Z).cmd(), act(Action::Undo)),
            (Chord::new(Y).cmd(), act(Action::Redo)),
            (Chord::new(Z).cmd().shift(), act(Action::Redo)),
            (
                Chord::new(ArrowUp).alt().shift(),
                act(Action::ColumnExtend(Direction::Up)),
            ),
            (
                Chord::new(ArrowDown).alt().shift(),
                act(Action::ColumnExtend(Direction::Down)),
            ),
            (
                Chord::new(ArrowLeft).alt().shift(),
                act(Action::ColumnExtend(Direction::Left)),
            ),
            (
                Chord::new(ArrowRight).alt().shift(),
                act(Action::ColumnExtend(Direction::Right)),
            ),
            // Text transforms (Notepad++ bindings where it has them).
            (
                Chord::new(U).cmd().shift(),
                act(Action::ConvertCase(CaseMode::Upper)),
            ),
            (
                Chord::new(U).cmd(),
                act(Action::ConvertCase(CaseMode::Lower)),
            ),
            (
                Chord::new(U).alt(),
                act(Action::ConvertCase(CaseMode::Title)),
            ),
            (
                Chord::new(U).alt().shift(),
                act(Action::ConvertCase(CaseMode::TitleBlend)),
            ),
            (
                Chord::new(U).cmd().alt(),
                act(Action::ConvertCase(CaseMode::Sentence)),
            ),
            (
                Chord::new(U).cmd().alt().shift(),
                act(Action::ConvertCase(CaseMode::SentenceBlend)),
            ),
            (
                Chord::new(I).cmd().alt(),
                act(Action::ConvertCase(CaseMode::Invert)),
            ),
            (Chord::new(D).cmd(), act(Action::DuplicateLine)),
            (Chord::new(ArrowUp).cmd().shift(), act(Action::MoveLinesUp)),
            (
                Chord::new(ArrowDown).cmd().shift(),
                act(Action::MoveLinesDown),
            ),
            (Chord::new(J).cmd(), act(Action::JoinLines)),
            // Notepad++'s (Scintilla's) line commands.
            (Chord::new(L).cmd().shift(), act(Action::DeleteLine)),
            (Chord::new(T).cmd(), act(Action::TransposeLines)),
            (Chord::new(Enter).cmd().alt(), act(Action::InsertLineAbove)),
            (
                Chord::new(Enter).cmd().alt().shift(),
                act(Action::InsertLineBelow),
            ),
            (Chord::new(ArrowUp).cmd().alt(), act(Action::AddCursorAbove)),
            (
                Chord::new(ArrowDown).cmd().alt(),
                act(Action::AddCursorBelow),
            ),
            (Chord::new(Tab).shift(), act(Action::Unindent)),
            (Chord::new(Slash).cmd(), act(Action::ToggleLineComment)),
            // Notepad++'s keys for the same.
            (Chord::new(Q).cmd(), act(Action::ToggleLineComment)),
            (Chord::new(K).cmd(), act(Action::CommentLines)),
            (Chord::new(K).cmd().shift(), act(Action::UncommentLines)),
            (Chord::new(Q).cmd().shift(), act(Action::ToggleBlockComment)),
            (
                Chord::new(Slash).cmd().shift(),
                act(Action::ToggleBlockComment),
            ),
            (Chord::new(B).cmd(), act(Action::GoToMatchingBracket)),
            (
                Chord::new(B).cmd().alt(),
                act(Action::SelectToMatchingBracket),
            ),
            // Bookmarks (Notepad++'s keys).
            (Chord::new(F2).cmd(), act(Action::ToggleBookmark)),
            (Chord::new(F2), act(Action::NextBookmark)),
            (Chord::new(F2).shift(), act(Action::PrevBookmark)),
            // Folding (Notepad++'s View > Fold/Collapse shortcuts).
            (Chord::new(F).cmd().alt(), act(Action::Fold)),
            (Chord::new(F).cmd().alt().shift(), act(Action::Unfold)),
            (Chord::new(Num0).alt(), act(Action::FoldAll)),
            (Chord::new(Num0).alt().shift(), act(Action::UnfoldAll)),
        ]);
        let digits = [Num1, Num2, Num3, Num4, Num5, Num6, Num7, Num8];
        for (level, key) in (1..).zip(digits) {
            b.push((Chord::new(key).alt(), act(Action::FoldLevel(level))));
            b.push((
                Chord::new(key).alt().shift(),
                act(Action::UnfoldLevel(level)),
            ));
        }
        Self { bindings: b }
    }
}

impl Keymap {
    /// The defaults with the entries of a `keymap.json` applied in order:
    /// an entry replaces any binding of its keys, or removes it when its
    /// command is `null`.
    pub fn with_file(json: &str) -> Result<Keymap, String> {
        let entries: Vec<KeymapEntry> =
            serde_json::from_str(json).map_err(|e| format!("keymap.json: {e}"))?;
        let mut keymap = Keymap::default();
        for (i, e) in entries.into_iter().enumerate() {
            let chord = Chord::parse(&e.keys).ok_or_else(|| {
                format!(
                    "keymap.json entry {}: \"{}\" is not a key combination",
                    i + 1,
                    e.keys
                )
            })?;
            keymap.bindings.retain(|(c, _)| *c != chord);
            if let Some(command) = e.command {
                keymap.bindings.push((chord, command));
            }
        }
        Ok(keymap)
    }

    /// Every default binding as `keymap.json` entries, to start editing from.
    pub fn default_entries() -> Vec<KeymapEntry> {
        Keymap::default()
            .bindings
            .into_iter()
            .map(|(c, command)| KeymapEntry {
                keys: c.to_string(),
                command: Some(command),
            })
            .collect()
    }

    pub fn lookup(&self, key: Key, modifiers: Modifiers) -> Option<&EditorCommand> {
        self.bindings
            .iter()
            .find(|(c, _)| c.matches(key, modifiers))
            .map(|(_, cmd)| cmd)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chords_round_trip_through_text() {
        for (text, chord) in [
            ("Ctrl+Shift+U", Chord::new(Key::U).cmd().shift()),
            ("Ctrl+Alt+Down", Chord::new(Key::ArrowDown).cmd().alt()),
            ("F3", Chord::new(Key::F3)),
        ] {
            assert_eq!(Chord::parse(text), Some(chord));
            assert_eq!(chord.to_string(), text);
        }
        assert_eq!(Chord::parse("ctrl + d"), Some(Chord::new(Key::D).cmd()));
        assert_eq!(
            Chord::parse("Alt+ArrowDown"),
            Some(Chord::new(Key::ArrowDown).alt())
        );
        assert_eq!(Chord::parse("Hyper+D"), None);
    }

    #[test]
    fn the_file_adds_replaces_and_removes_bindings() {
        let json = r#"[
            {"keys": "Ctrl+D", "command": {"Action": "SelectLine"}},
            {"keys": "Ctrl+J", "command": null},
            {"keys": "Ctrl+Alt+K", "command": {"Action": {"ConvertCase": "Upper"}}}
        ]"#;
        let k = Keymap::with_file(json).unwrap();
        let ctrl = Modifiers::COMMAND;
        assert_eq!(k.lookup(Key::D, ctrl), Some(&act(Action::SelectLine)));
        assert_eq!(k.lookup(Key::J, ctrl), None);
        assert_eq!(
            k.lookup(Key::K, ctrl.plus(Modifiers::ALT)),
            Some(&act(Action::ConvertCase(CaseMode::Upper)))
        );
        // Untouched defaults remain.
        assert_eq!(k.lookup(Key::A, ctrl), Some(&act(Action::SelectAll)));
        assert!(Keymap::with_file("[{\"keys\": \"Nope+Q\", \"command\": null}]").is_err());
        assert!(Keymap::with_file("not json").is_err());
    }

    #[test]
    fn the_defaults_written_out_load_back_identically() {
        let json = serde_json::to_string(&Keymap::default_entries()).unwrap();
        let k = Keymap::with_file(&json).unwrap();
        assert_eq!(k.bindings, Keymap::default().bindings);
    }
}
