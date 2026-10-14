//! 0.3.0 P1: Go to Line, indenting with Tab/Shift+Tab, toggling comments and
//! matching brackets.

mod util;

use ochrepad_core::{Action, ActionError, CommentSyntax};
use util::{T, setup};

fn with_comments(marked: &str, line: Option<&str>, block: Option<(&str, &str)>) -> T {
    let mut t = setup(marked);
    let b = t.ed.view(t.view).unwrap().buffer();
    t.ed.buffer_mut(b).unwrap().comments = Some(CommentSyntax {
        line: line.map(str::to_owned),
        block: block.map(|(a, b)| (a.to_owned(), b.to_owned())),
    });
    t
}

fn rust(marked: &str) -> T {
    with_comments(marked, Some("//"), Some(("/*", "*/")))
}

#[test]
fn go_to_line_and_column_count_from_one() {
    let mut t = setup("|ab\n\tcd\nef");
    t.apply(Action::GoToLine { line: 2, column: 5 });
    assert_eq!(t.show(), "ab\n\t|cd\nef", "column 5 is after the tab");
    t.apply(Action::GoToLine {
        line: 99,
        column: 1,
    });
    assert_eq!(t.show(), "ab\n\tcd\n|ef", "clamped to the last line");
    t.apply(Action::GoToLine {
        line: 1,
        column: 99,
    });
    assert_eq!(t.show(), "ab|\n\tcd\nef", "clamped to the line end");
}

#[test]
fn tab_indents_a_selection_over_lines_and_shift_tab_unindents() {
    let mut t = setup("a\n\nb\nc");
    t.set_sels(vec![(0, 5)]);
    t.apply(Action::InsertTab);
    assert_eq!(t.text(), "\ta\n\n\tb\nc", "empty lines stay empty");
    assert_eq!(t.show(), "^\ta\n\n\tb\n|c", "still up to the start of c");
    t.apply(Action::Unindent);
    assert_eq!(t.text(), "a\n\nb\nc");
    t.apply(Action::Undo);
    assert_eq!(t.text(), "\ta\n\n\tb\nc", "one undo step each");

    // Within one line Tab still replaces the selection.
    let mut t = setup("abc");
    t.set_sels(vec![(1, 2)]);
    t.apply(Action::InsertTab);
    assert_eq!(t.text(), "a\tc");

    // Shift+Tab on a caret line removes up to one indent of spaces.
    let mut t = setup("      x|");
    t.apply(Action::Unindent);
    assert_eq!(t.show(), "  x|");
}

#[test]
fn line_comments_toggle_at_the_common_indent() {
    let mut t = rust("fn f() {\n    a();\n\n        b();\n}");
    t.set_sels(vec![(9, 32)]);
    t.apply(Action::ToggleLineComment);
    assert_eq!(t.text(), "fn f() {\n    // a();\n\n    //     b();\n}");
    t.apply(Action::ToggleLineComment);
    assert_eq!(t.text(), "fn f() {\n    a();\n\n        b();\n}");

    // A mix of commented and plain lines gets commented.
    let mut t = rust("// a\nb|");
    t.set_sels(vec![(0, 6)]);
    t.apply(Action::ToggleLineComment);
    assert_eq!(t.text(), "// // a\n// b");

    // The caret's line, with the caret kept in place in the text.
    let mut t = rust("x = 1|");
    t.apply(Action::ToggleLineComment);
    assert_eq!(t.show(), "// x = 1|");
}

#[test]
fn comment_and_uncomment_add_and_remove_one_level() {
    // Comment adds a level even to lines that have one.
    let mut t = rust("  // a\n  b|");
    t.set_sels(vec![(0, 10)]);
    t.apply(Action::CommentLines);
    assert_eq!(t.text(), "  // // a\n  // b");
    // Uncomment takes one level off the lines that have one.
    t.apply(Action::UncommentLines);
    assert_eq!(t.text(), "  // a\n  b");
    t.apply(Action::UncommentLines);
    assert_eq!(t.text(), "  a\n  b");
    // Nothing left to take off: nothing changes, not even the revision.
    let revision = t.buffer().revision();
    t.apply(Action::UncommentLines);
    assert_eq!(t.text(), "  a\n  b");
    assert_eq!(t.buffer().revision(), revision);

    // Only blocks: the lines are wrapped once, and unwrapped.
    let mut t = with_comments("<a>\n<b>|", None, Some(("<!--", "-->")));
    t.set_sels(vec![(0, 7)]);
    t.apply(Action::CommentLines);
    assert_eq!(t.text(), "<!-- <a>\n<b> -->");
    t.apply(Action::CommentLines);
    assert_eq!(t.text(), "<!-- <a>\n<b> -->");
    t.apply(Action::UncommentLines);
    assert_eq!(t.text(), "<a>\n<b>");
    t.apply(Action::UncommentLines);
    assert_eq!(t.text(), "<a>\n<b>");
}

#[test]
fn block_only_comments_work_per_run_of_lines() {
    let css = |text: &str| with_comments(text, None, Some(("/*", "*/")));
    // Two lines commented each: uncommenting both takes both off, and
    // leaves no stray marker.
    let text = "/* color: red; */\n/* margin: 0; */";
    let mut t = css(text);
    t.set_sels(vec![(0, text.len())]);
    t.apply(Action::UncommentLines);
    assert_eq!(t.text(), "color: red;\nmargin: 0;");
    // The toggle does the same, and puts one block back.
    let mut t = css(text);
    t.set_sels(vec![(0, text.len())]);
    t.apply(Action::ToggleLineComment);
    assert_eq!(t.text(), "color: red;\nmargin: 0;");
    t.set_sels(vec![(0, 22)]);
    t.apply(Action::ToggleLineComment);
    assert_eq!(t.text(), "/* color: red;\nmargin: 0; */");

    // Carets on lines 1 and 3: two blocks, line 2 left alone.
    let mut t = css("|a\nb\n|c");
    t.apply(Action::CommentLines);
    assert_eq!(t.text(), "/* a */\nb\n/* c */");
    t.apply(Action::UncommentLines);
    assert_eq!(t.text(), "a\nb\nc");

    // Two comments on one line are not one comment around it.
    let mut t = css("/* a */ b /* c */|");
    t.apply(Action::ToggleLineComment);
    assert_eq!(t.text(), "/* /* a */ b /* c */ */");
}

#[test]
fn block_comments_wrap_and_unwrap() {
    let mut t = rust("let x = a + b;");
    t.set_sels(vec![(8, 13)]);
    t.apply(Action::ToggleBlockComment);
    assert_eq!(t.text(), "let x = /* a + b */;");
    t.set_sels(vec![(8, 19)]);
    t.apply(Action::ToggleBlockComment);
    assert_eq!(t.text(), "let x = a + b;");

    // HTML has only block comments: the line toggle wraps the lines.
    let mut t = with_comments("  <a>\n  <b>|", None, Some(("<!--", "-->")));
    t.set_sels(vec![(0, 11)]);
    t.apply(Action::ToggleLineComment);
    assert_eq!(t.text(), "  <!-- <a>\n  <b> -->");
}

#[test]
fn unknown_comment_syntax_is_an_error() {
    let mut t = setup("x|");
    let r = t.ed.apply(t.view, Action::ToggleLineComment);
    assert_eq!(r, Err(ActionError::NoCommentSyntax));
    assert_eq!(t.text(), "x");
}

#[test]
fn matching_brackets_jump_and_select() {
    let mut t = setup("f(|a, [b])");
    t.apply(Action::GoToMatchingBracket);
    assert_eq!(t.show(), "f(a, [b])|", "after the ( stays after the )");
    t.apply(Action::GoToMatchingBracket);
    assert_eq!(t.show(), "f(|a, [b])");
    t.set_sels(vec![(5, 5)]);
    t.apply(Action::SelectToMatchingBracket);
    assert_eq!(t.show(), "f(a, ^[b]|)");
}

#[test]
fn converting_indentation_keeps_widths_and_sets_the_buffer() {
    // Width 4: a tab plus two spaces is 6 cells.
    let mut t = setup("a\n\t  b\n    c|\n");
    t.apply(Action::ConvertIndentation { to_spaces: true });
    assert_eq!(t.text(), "a\n      b\n    c\n");
    assert!(t.buffer().indent.use_spaces && t.buffer().indent_set);
    t.apply(Action::ConvertIndentation { to_spaces: false });
    assert_eq!(t.text(), "a\n\t  b\n\tc\n");
    assert!(!t.buffer().indent.use_spaces);
    t.apply(Action::Undo);
    assert_eq!(t.text(), "a\n      b\n    c\n", "one undo step");
}

#[test]
fn begin_select_follows_the_edits_before_it() {
    use ochrepad_core::{ByteOffset, SelectionSet};
    let mut t = setup("abcd");
    let begin = |t: &T| t.ed.view(t.view).unwrap().begin_select;
    let caret = |t: &mut T, at: usize| {
        t.apply(Action::SetSelections(SelectionSet::caret(ByteOffset(at))));
    };
    t.ed.view_mut(t.view).unwrap().begin_select = Some(ByteOffset(2));
    caret(&mut t, 0);
    t.apply(Action::InsertText("XYZ".into()));
    assert_eq!(begin(&t), Some(ByteOffset(5)));
    // Typed where it began, the text goes before it, as in Notepad++.
    caret(&mut t, 5);
    t.apply(Action::InsertText("!".into()));
    assert_eq!(begin(&t), Some(ByteOffset(6)));
    // Undo and edits after it follow too; text after it leaves it alone.
    t.apply(Action::Undo);
    assert_eq!(begin(&t), Some(ByteOffset(5)));
    caret(&mut t, 7);
    t.apply(Action::InsertText("?".into()));
    assert_eq!(begin(&t), Some(ByteOffset(5)));
}
