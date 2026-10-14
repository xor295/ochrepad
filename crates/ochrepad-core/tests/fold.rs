//! Code folding: region detection, fold actions, and folds following edits
//! and carets.

mod util;

use ochrepad_core::fold::{self, Region};
use ochrepad_core::{
    Action, ByteOffset, Document, FoldStrategy, Motion, SearchOptions, SearchQuery,
};
use util::{T, setup};

const RUST: &str = "\
fn main() {
    let x = 1;
    if x > 0 {
        println!(\"{x}\");
    }

}

fn other() {}
";

const PYTHON: &str = "\
class A:
    def f(self):
        return 1

    def g(self):
\t\treturn 2


x = A()
";

/// (header, last) of every region, from the one-pass scan.
fn regions(text: &str, strategy: FoldStrategy) -> Vec<(usize, usize)> {
    let doc = Document::new(text);
    let all = fold::regions(&doc, strategy, 4);
    // The single-region functions agree with the scan.
    for r in &all {
        assert_eq!(fold::region(&doc, r.header, strategy, 4), Some(r.last));
        assert!(fold::is_header(&doc, r.header, strategy, 4));
    }
    let headers: Vec<usize> = all.iter().map(|r| r.header).collect();
    for line in 0..doc.line_count() {
        if !headers.contains(&line) {
            assert_eq!(fold::region(&doc, line, strategy, 4), None, "line {line}");
            assert!(!fold::is_header(&doc, line, strategy, 4), "line {line}");
        }
    }
    all.iter().map(|r| (r.header, r.last)).collect()
}

fn folded(t: &T) -> Vec<usize> {
    t.ed.view(t.view).unwrap().folds().headers().collect()
}

fn markdown(text: &str) -> T {
    let mut t = setup(text);
    let b = t.ed.view(t.view).unwrap().buffer();
    t.ed.buffer_mut(b).unwrap().fold_strategy = FoldStrategy::Markdown;
    t
}

#[test]
fn indentation_regions_leave_closing_lines_and_trailing_blanks_visible() {
    // The closing `}` lines stay visible, and so does the blank line
    // before the last one.
    assert_eq!(regions(RUST, FoldStrategy::Indent), vec![(0, 4), (2, 3)]);
    // Tabs count to the next tab stop: two tabs are eight cells.
    assert_eq!(
        regions(PYTHON, FoldStrategy::Indent),
        vec![(0, 5), (1, 2), (4, 5)]
    );
}

#[test]
fn allman_braces_and_yaml_fold_by_indentation() {
    let allman = "void f()\n{\n    x();\n}\n";
    assert_eq!(regions(allman, FoldStrategy::Indent), vec![(1, 2)]);
    let yaml = "a:\n  b: 1\n  c:\n    - d\n";
    assert_eq!(regions(yaml, FoldStrategy::Indent), vec![(0, 3), (2, 3)]);
}

#[test]
fn levels_are_nesting_depths() {
    let doc = Document::new(PYTHON);
    let levels: Vec<(usize, usize)> = fold::regions(&doc, FoldStrategy::Indent, 4)
        .into_iter()
        .map(|Region { header, level, .. }| (header, level))
        .collect();
    assert_eq!(levels, vec![(0, 1), (1, 2), (4, 2)]);
}

#[test]
fn markdown_headings_fold_to_the_next_heading_of_their_level() {
    let md = "# A\ntext\n## B\n```\n# not a heading\n```\nmore\n\n# C\n## D\n";
    // C holds the level-2 heading D, which has no body of its own.
    assert_eq!(
        regions(md, FoldStrategy::Markdown),
        vec![(0, 6), (2, 6), (8, 9)]
    );
    let doc = Document::new(md);
    assert!(
        !fold::is_header(&doc, 4, FoldStrategy::Markdown, 4),
        "inside a fence"
    );
}

#[test]
fn folding_hides_lines_from_vertical_motions() {
    let mut t = setup(RUST);
    t.ed.fold_at(t.view, 2);
    assert_eq!(folded(&t), vec![2]);
    // From line 1 down past the folded `if` lands on its closing `}`.
    t.set_sels(vec![(16, 16)]);
    t.mv(Motion::Down).mv(Motion::Down);
    let head = t.sels().primary().head;
    assert_eq!(t.buffer().doc().line_of(head), 4);
    t.mv(Motion::Up);
    assert_eq!(t.buffer().doc().line_of(t.sels().primary().head), 2);
    assert_eq!(folded(&t), vec![2], "moving never opened the fold");
}

#[test]
fn folding_moves_carets_out_and_a_caret_moving_in_unfolds() {
    // The caret is on the hidden `println!` line.
    let mut t = setup(RUST);
    let doc = Document::new(RUST);
    let inside = doc.line_start(3).0 + 4;
    t.set_sels(vec![(inside, inside)]);
    t.apply(Action::Fold);
    assert_eq!(folded(&t), vec![2], "the caret's innermost region");
    assert_eq!(
        t.sels().primary().head,
        doc.line_end(2),
        "moved to the header"
    );

    // Right from the end of the header steps into the hidden text.
    t.mv(Motion::Right);
    assert!(folded(&t).is_empty());
}

#[test]
fn find_into_a_fold_unfolds_it() {
    let mut t = setup(RUST);
    t.ed.fold_at(t.view, 0);
    t.set_sels(vec![(0, 0)]);
    let query = SearchQuery::new("println", SearchOptions::default());
    t.apply(Action::Find {
        query,
        backward: false,
    });
    assert!(folded(&t).is_empty());
}

#[test]
fn edits_keep_untouched_folds_and_drop_touched_ones() {
    let mut t = setup(RUST);
    t.ed.fold_at(t.view, 2);
    // Typing at the end of the header line and above the fold keeps it.
    let header_end = t.buffer().doc().line_end(2).0;
    t.set_sels(vec![(header_end, header_end)]);
    t.typ(" // y");
    t.set_sels(vec![(0, 0)]);
    t.typ("// top\n");
    assert_eq!(folded(&t), vec![3], "moved down a line with the text");
    let doc = t.buffer().doc().clone();
    assert!(doc.line_text(3).ends_with("{ // y"));

    // Replacing text inside the fold drops it.
    let query = SearchQuery::new("println", SearchOptions::default());
    t.apply(Action::ReplaceAll {
        query,
        replacement: "print".into(),
    });
    assert!(folded(&t).is_empty());
}

#[test]
fn undo_maps_folds_too() {
    let mut t = setup(RUST);
    t.ed.fold_at(t.view, 2);
    t.set_sels(vec![(0, 0)]);
    t.typ("\n");
    assert_eq!(folded(&t), vec![3]);
    t.apply(Action::Undo);
    assert_eq!(folded(&t), vec![2]);
    assert_eq!(t.sels().primary().head, ByteOffset(0));
}

#[test]
fn fold_all_levels_and_unfold() {
    let mut t = setup(PYTHON);
    t.apply(Action::FoldAll);
    assert_eq!(folded(&t), vec![0, 1, 4]);
    t.apply(Action::UnfoldAll);
    assert!(folded(&t).is_empty());

    t.apply(Action::FoldLevel(2));
    assert_eq!(folded(&t), vec![1, 4]);
    t.apply(Action::FoldLevel(1));
    t.apply(Action::UnfoldLevel(2));
    assert_eq!(folded(&t), vec![0]);

    // Unfold on the caret's folded line; ToggleFold folds it again.
    t.set_sels(vec![(0, 0)]);
    t.apply(Action::Unfold);
    assert!(folded(&t).is_empty());
    t.apply(Action::ToggleFold).apply(Action::ToggleFold);
    assert!(folded(&t).is_empty());
    t.apply(Action::ToggleFold);
    assert_eq!(folded(&t), vec![0]);
}

#[test]
fn fold_actions_are_not_undo_steps_and_leave_the_buffer_clean() {
    let mut t = setup(PYTHON);
    t.apply(Action::FoldAll);
    assert!(!t.buffer().is_dirty());
    assert!(!t.buffer().history().can_undo());
}

#[test]
fn markdown_folds_by_heading() {
    let mut t = markdown("# A\ntext\n## B\nmore\n# C\n");
    t.apply(Action::FoldLevel(2));
    assert_eq!(folded(&t), vec![2]);
    t.apply(Action::FoldAll);
    assert_eq!(folded(&t), vec![0, 2]);
}

#[test]
fn removing_the_text_of_the_last_fold_shows_every_line() {
    let mut t = setup("a\n  b\nc");
    t.ed.fold_at(t.view, 0);
    t.set_sels(vec![(1, 5)]);
    t.apply(Action::DeleteBackward);
    assert_eq!(t.text(), "a\nc");
    let view = t.ed.view(t.view).unwrap();
    let rows = ochrepad_core::DisplayMap::new(t.buffer(), view, 4);
    assert!(view.folds().is_empty());
    assert_eq!(rows.visible_line_count(), 2);
}
