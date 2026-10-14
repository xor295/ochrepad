//! The documentation's key-binding examples must work, and every action
//! must be documented in docs/keymap.md.

use std::collections::BTreeSet;
use std::path::PathBuf;

use ochrepad::keymap::{EditorCommand, Keymap};
use ochrepad_core::Action;

fn read(rel: &str) -> String {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join(rel);
    std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
}

/// The contents of every ```json block.
fn json_blocks(md: &str) -> Vec<String> {
    let mut blocks = Vec::new();
    let mut current: Option<String> = None;
    for line in md.lines() {
        let trimmed = line.trim_start();
        match &mut current {
            None if trimmed.starts_with("```json") => current = Some(String::new()),
            Some(block) if trimmed.starts_with("```") => {
                blocks.push(std::mem::take(block));
                current = None;
            }
            Some(block) => {
                block.push_str(line);
                block.push('\n');
            }
            None => {}
        }
    }
    blocks
}

/// Inline code spans in table rows that hold a command (`{"Action": …}`,
/// `{"PageUp": …}`).
fn table_commands(md: &str) -> Vec<String> {
    md.lines()
        .filter(|l| l.starts_with('|'))
        .flat_map(|l| {
            l.split('`')
                .skip(1)
                .step_by(2)
                .map(str::to_owned)
                .collect::<Vec<_>>()
        })
        .filter(|s| {
            // Not the motions table's `{"PageUp": 20}`, which is a motion.
            s.starts_with("{\"Action\"")
                || s.starts_with("{\"PageUp\": {")
                || s.starts_with("{\"PageDown\": {")
        })
        .collect()
}

/// Every action variant's name. Adding a variant breaks this match: add the
/// name here and document the action in docs/keymap.md.
fn name(action: &Action) -> &'static str {
    match action {
        Action::InsertText(_) => "InsertText",
        Action::Paste(_) => "Paste",
        Action::InsertTab => "InsertTab",
        Action::InsertNewline => "InsertNewline",
        Action::DeleteBackward => "DeleteBackward",
        Action::DeleteForward => "DeleteForward",
        Action::DeleteWordBackward => "DeleteWordBackward",
        Action::DeleteWordForward => "DeleteWordForward",
        Action::MoveCursor { .. } => "MoveCursor",
        Action::SelectAll => "SelectAll",
        Action::SelectWord => "SelectWord",
        Action::SelectLine => "SelectLine",
        Action::AddCursorAbove => "AddCursorAbove",
        Action::AddCursorBelow => "AddCursorBelow",
        Action::CollapseSelections => "CollapseSelections",
        Action::SetSelections(_) => "SetSelections",
        Action::ColumnSelect { .. } => "ColumnSelect",
        Action::ColumnExtend(_) => "ColumnExtend",
        Action::PasteBlock(_) => "PasteBlock",
        Action::Find { .. } => "Find",
        Action::Replace { .. } => "Replace",
        Action::ReplaceAll { .. } => "ReplaceAll",
        Action::SelectAllMatches(_) => "SelectAllMatches",
        Action::ConvertCase(_) => "ConvertCase",
        Action::SortLines(_) => "SortLines",
        Action::TrimTrailingWhitespace => "TrimTrailingWhitespace",
        Action::DuplicateLine => "DuplicateLine",
        Action::MoveLinesUp => "MoveLinesUp",
        Action::MoveLinesDown => "MoveLinesDown",
        Action::JoinLines => "JoinLines",
        Action::RemoveEmptyLines { .. } => "RemoveEmptyLines",
        Action::RemoveDuplicateLines { .. } => "RemoveDuplicateLines",
        Action::Fold => "Fold",
        Action::Unfold => "Unfold",
        Action::ToggleFold => "ToggleFold",
        Action::FoldAll => "FoldAll",
        Action::UnfoldAll => "UnfoldAll",
        Action::FoldLevel(_) => "FoldLevel",
        Action::UnfoldLevel(_) => "UnfoldLevel",
        Action::GoToLine { .. } => "GoToLine",
        Action::Indent => "Indent",
        Action::Unindent => "Unindent",
        Action::ConvertIndentation { .. } => "ConvertIndentation",
        Action::ToggleLineComment => "ToggleLineComment",
        Action::ToggleBlockComment => "ToggleBlockComment",
        Action::GoToMatchingBracket => "GoToMatchingBracket",
        Action::SelectToMatchingBracket => "SelectToMatchingBracket",
        Action::FormatJson { .. } => "FormatJson",
        Action::MinifyJson => "MinifyJson",
        Action::Encode(_) => "Encode",
        Action::Decode(_) => "Decode",
        Action::Undo => "Undo",
        Action::Redo => "Redo",
    }
}

const ALL: [&str; 53] = [
    "InsertText",
    "Paste",
    "InsertTab",
    "InsertNewline",
    "DeleteBackward",
    "DeleteForward",
    "DeleteWordBackward",
    "DeleteWordForward",
    "MoveCursor",
    "SelectAll",
    "SelectWord",
    "SelectLine",
    "AddCursorAbove",
    "AddCursorBelow",
    "CollapseSelections",
    "SetSelections",
    "ColumnSelect",
    "ColumnExtend",
    "PasteBlock",
    "Find",
    "Replace",
    "ReplaceAll",
    "SelectAllMatches",
    "ConvertCase",
    "SortLines",
    "TrimTrailingWhitespace",
    "DuplicateLine",
    "MoveLinesUp",
    "MoveLinesDown",
    "JoinLines",
    "RemoveEmptyLines",
    "RemoveDuplicateLines",
    "Fold",
    "Unfold",
    "ToggleFold",
    "FoldAll",
    "UnfoldAll",
    "FoldLevel",
    "UnfoldLevel",
    "GoToLine",
    "Indent",
    "Unindent",
    "ConvertIndentation",
    "ToggleLineComment",
    "ToggleBlockComment",
    "GoToMatchingBracket",
    "SelectToMatchingBracket",
    "FormatJson",
    "MinifyJson",
    "Encode",
    "Decode",
    "Undo",
    "Redo",
];

/// Mouse-only actions: named in the docs, without an example.
const MOUSE_ONLY: [&str; 2] = ["SetSelections", "ColumnSelect"];

#[test]
fn keymap_examples_in_the_docs_are_valid_keymaps() {
    for file in ["docs/keymap.md", "README.md"] {
        let blocks = json_blocks(&read(file));
        assert!(!blocks.is_empty(), "{file} has no JSON examples");
        for block in blocks {
            if let Err(e) = Keymap::with_file(&block) {
                panic!("{file}: {e}\n{block}");
            }
        }
    }
}

#[test]
fn every_action_is_documented_with_a_working_example() {
    let md = read("docs/keymap.md");
    let mut documented = BTreeSet::new();
    for json in table_commands(&md) {
        match serde_json::from_str::<EditorCommand>(&json) {
            Ok(EditorCommand::Action(a)) => {
                documented.insert(name(&a));
            }
            Ok(_) => {}
            Err(e) => panic!("docs/keymap.md: {e}\n{json}"),
        }
    }
    for action in ALL {
        if MOUSE_ONLY.contains(&action) {
            assert!(
                md.contains(&format!("`{action}`")),
                "{action} is not mentioned"
            );
        } else {
            assert!(documented.contains(action), "no example for {action}");
        }
    }
}
