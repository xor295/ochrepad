//! Find and replace actions.

mod util;

use ochrepad_core::{Action, ActionError, SearchOptions, SearchQuery};
use util::setup;

fn query(pattern: &str, f: impl FnOnce(&mut SearchOptions)) -> SearchQuery {
    let mut o = SearchOptions::default();
    f(&mut o);
    SearchQuery::new(pattern, o)
}

fn find(q: &SearchQuery) -> Action {
    Action::Find {
        query: q.clone(),
        backward: false,
    }
}

fn find_back(q: &SearchQuery) -> Action {
    Action::Find {
        query: q.clone(),
        backward: true,
    }
}

#[test]
fn find_next_and_previous_with_and_without_wrap() {
    let mut t = setup("|foo bar foo baz foo");
    let q = query("foo", |_| {});
    assert_eq!(t.apply(find(&q)).show(), "^foo| bar foo baz foo");
    assert_eq!(t.apply(find(&q)).show(), "foo bar ^foo| baz foo");
    assert_eq!(t.apply(find(&q)).show(), "foo bar foo baz ^foo|");
    assert_eq!(t.ed.apply(t.view, find(&q)), Err(ActionError::NotFound));
    assert_eq!(t.apply(find_back(&q)).show(), "foo bar ^foo| baz foo");

    let wrap = query("foo", |o| o.wrap = true);
    t.apply(find(&wrap));
    assert_eq!(t.apply(find(&wrap)).show(), "^foo| bar foo baz foo");
    assert_eq!(t.apply(find_back(&wrap)).show(), "foo bar foo baz ^foo|");
}

#[test]
fn empty_matches_step_from_line_to_line() {
    let mut t = setup("a\nb\nc|");
    let q = query("^", |o| {
        o.regex = true;
        o.wrap = true;
    });
    assert_eq!(t.apply(find(&q)).show(), "|a\nb\nc");
    assert_eq!(t.apply(find(&q)).show(), "a\n|b\nc");
    assert_eq!(t.apply(find(&q)).show(), "a\nb\n|c");
}

#[test]
fn replace_replaces_the_current_match_then_moves_on() {
    let mut t = setup("|cat dog cat");
    let q = query("cat", |_| {});
    let replace = Action::Replace {
        query: q.clone(),
        replacement: "bird".into(),
    };
    // The caret is not on a match: Replace only finds the first.
    assert_eq!(t.apply(replace.clone()).show(), "^cat| dog cat");
    assert_eq!(t.apply(replace.clone()).show(), "bird dog ^cat|");
    assert_eq!(t.apply(replace.clone()).show(), "bird dog bird|");
    assert_eq!(t.ed.apply(t.view, replace), Err(ActionError::NotFound));
    t.apply(Action::Undo);
    assert_eq!(t.text(), "bird dog cat");
}

#[test]
fn replace_all_is_one_undo_step_and_expands_captures() {
    let mut t = setup("|John Smith, Jane Doe");
    let q = query(r"(\w+) (?<last>\w+)", |o| o.regex = true);
    t.apply(Action::ReplaceAll {
        query: q,
        replacement: "${last} $1".into(),
    });
    assert_eq!(t.text(), "Smith John, Doe Jane");
    t.apply(Action::Undo);
    assert_eq!(t.text(), "John Smith, Jane Doe");
}

#[test]
fn replace_all_with_empty_matches_prefixes_lines() {
    let mut t = setup("|a\nb\nc");
    t.apply(Action::ReplaceAll {
        query: query("^", |o| o.regex = true),
        replacement: "// ".into(),
    });
    assert_eq!(t.text(), "// a\n// b\n// c");
}

#[test]
fn replace_all_in_selection_only_touches_the_selection() {
    let mut t = setup("x x x x");
    t.set_sels(vec![(2, 5)]);
    t.apply(Action::ReplaceAll {
        query: query("x", |o| o.in_selection = true),
        replacement: "yy".into(),
    });
    assert_eq!(t.text(), "x yy yy x");
    // The selection still covers the replaced text.
    assert_eq!(t.show(), "x ^yy yy| x");
}

#[test]
fn replace_all_with_no_matches_changes_nothing() {
    let mut t = setup("abc|");
    t.apply(Action::ReplaceAll {
        query: query("zzz", |_| {}),
        replacement: "y".into(),
    });
    assert_eq!(t.show(), "abc|");
    assert!(!t.buffer().history().can_undo());
}

#[test]
fn select_all_matches_makes_one_selection_each() {
    let mut t = setup("|one two one three one");
    t.apply(Action::SelectAllMatches(query("one", |o| {
        o.whole_word = true
    })));
    assert_eq!(t.show(), "^one| two ^one| three ^one|");
    t.typ("1");
    assert_eq!(t.text(), "1 two 1 three 1");
}

#[test]
fn invalid_patterns_are_errors_and_are_not_recorded() {
    let mut t = setup("abc|");
    t.ed.start_recording();
    let r = t.ed.apply(t.view, find(&query("(", |o| o.regex = true)));
    assert!(matches!(r, Err(ActionError::InvalidPattern(_))));
    assert!(t.ed.stop_recording().is_empty());
}

#[test]
fn a_recorded_find_replays_from_wherever_the_caret_is() {
    // "Find 'TODO', type 'x' over it" replayed until there is no match.
    let mut t = setup("|a TODO b TODO c TODO");
    let q = query("TODO", |_| {});
    t.ed.start_recording();
    t.apply(find(&q)).apply(Action::InsertText("DONE".into()));
    let macro_ = t.ed.stop_recording();
    let mut runs = 1;
    'outer: loop {
        for a in &macro_ {
            if t.ed.apply(t.view, a.clone()).is_err() {
                break 'outer;
            }
        }
        runs += 1;
    }
    assert_eq!(t.text(), "a DONE b DONE c DONE");
    assert_eq!(runs, 3);
}
