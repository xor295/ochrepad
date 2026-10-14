//! Smart indentation: Enter after an opening bracket, and closing brackets
//! lining up with their partners.

mod util;

use ochrepad_core::{Action, Indent};
use util::setup;

fn smart(text: &str) -> util::T {
    let mut t = setup(text);
    t.ed.settings.smart_indent = true;
    let b = t.ed.view(t.view).unwrap().buffer();
    t.ed.buffer_mut(b).unwrap().indent = Indent {
        use_spaces: true,
        width: 4,
    };
    t
}

#[test]
fn enter_after_an_opening_bracket_indents() {
    let mut t = smart("fn main() {|");
    t.apply(Action::InsertNewline);
    assert_eq!(t.show(), "fn main() {\n    |");
    // Inside a bracket pair, the closing one gets its own line.
    let mut t = smart("    let v = [|];");
    t.apply(Action::InsertNewline);
    assert_eq!(t.show(), "    let v = [\n        |\n    ];");
    // Blanks after the caret before the closer go.
    let mut t = smart("call(|  )");
    t.apply(Action::InsertNewline);
    assert_eq!(t.show(), "call(\n    |\n)");
    // Other lines keep their indentation, as without it.
    let mut t = smart("    x = 1;|");
    t.apply(Action::InsertNewline);
    assert_eq!(t.show(), "    x = 1;\n    |");
}

#[test]
fn a_colon_indents_in_python_only() {
    let mut t = smart("def f():|");
    t.apply(Action::InsertNewline);
    assert_eq!(t.show(), "def f():\n|");
    let mut t = smart("def f():|");
    let b = t.ed.view(t.view).unwrap().buffer();
    t.ed.buffer_mut(b).unwrap().language = Some("Python".into());
    t.apply(Action::InsertNewline);
    assert_eq!(t.show(), "def f():\n    |");
}

#[test]
fn a_closing_bracket_lines_up_with_its_partner() {
    let mut t = smart("if x {\n    y();\n        |");
    t.apply(Action::InsertText("}".into()));
    assert_eq!(t.show(), "if x {\n    y();\n}|");
    // After text on the line it is typed where it is.
    let mut t = smart("    a = (b|");
    t.apply(Action::InsertText(")".into()));
    assert_eq!(t.show(), "    a = (b)|");
    // One undo step takes it back.
    let mut t = smart("{\n    |");
    t.apply(Action::InsertText("}".into()));
    assert_eq!(t.show(), "{\n}|");
    t.apply(Action::Undo);
    assert_eq!(t.show(), "{\n    |");
}

#[test]
fn off_by_default_and_out_of_the_way_with_carets_on_one_line() {
    let mut t = setup("fn main() {|");
    t.apply(Action::InsertNewline);
    assert_eq!(t.show(), "fn main() {\n|");
    let mut t = smart("{|{|");
    t.apply(Action::InsertNewline);
    assert_eq!(t.text(), "{\n{\n");
}

#[test]
fn with_auto_close_enter_opens_the_pair() {
    let mut t = smart("fn main() |");
    t.ed.settings.auto_close = true;
    t.apply(Action::InsertText("{".into()));
    t.apply(Action::InsertNewline);
    assert_eq!(t.show(), "fn main() {\n    |\n}");
}
