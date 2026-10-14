use std::sync::LazyLock;
use std::time::{Duration, Instant};

use ochrepad_core::{Action, ByteOffset, Document, Editor, FileMeta, SelectionSet, ViewId};
use ochrepad_syntax::{BufferHighlighter, LineSpans, PLAIN_TEXT, Registry};
use proptest::prelude::*;

static REG: LazyLock<Registry> = LazyLock::new(Registry::load);

fn theme() -> &'static str {
    Registry::default_theme(true)
}

fn full(doc: &Document, syntax: &str) -> Vec<Option<LineSpans>> {
    let mut h = BufferHighlighter::new(&REG, syntax, theme());
    while h.work_lines(&REG, doc, 0..0, 10_000) {}
    h.line_spans(&REG, doc, 0..doc.line_count())
}

fn rust_file(lines: usize) -> String {
    let unit = [
        "/// Docs with `code`.",
        "pub fn f(x: u32) -> String {",
        "    let s = \"a string /* not a comment */\";",
        "    /* block",
        "       comment */ let y = x * 2; // trailing",
        "    format!(\"{s}{y}\")",
        "}",
        "",
    ];
    (0..lines)
        .map(|i| unit[i % unit.len()])
        .collect::<Vec<_>>()
        .join("\n")
}

#[test]
fn detects_by_extension_name_and_first_line() {
    assert_eq!(REG.detect(Some("main.rs"), ""), "Rust");
    assert_eq!(REG.detect(Some("a.PY"), ""), "Python");
    assert_eq!(
        REG.detect(Some("script"), "#!/usr/bin/env python3"),
        "Python"
    );
    assert_eq!(REG.detect(None, "#!/bin/bash"), "Bourne Again Shell (bash)");
    assert_eq!(REG.detect(Some("Makefile"), ""), "Makefile");
    assert_eq!(REG.detect(Some("notes.unknownext"), "hello"), PLAIN_TEXT);
    assert!(REG.language_names().len() > 100);
    assert!(Registry::theme_names().contains(&theme()));
}

#[test]
fn spans_cover_tokens_and_exclude_the_newline() {
    let doc = Document::new("fn main() {}\n// hi");
    let spans = full(&doc, "Rust");
    let line0 = spans[0].as_ref().unwrap();
    assert_eq!(line0.first().unwrap().start, 0);
    assert_eq!(line0.last().unwrap().end, "fn main() {}".len());
    // `fn` and the comment get different colors from plain identifiers.
    assert_ne!(line0[0].color, spans[1].as_ref().unwrap()[0].color);
}

#[test]
fn very_long_lines_are_left_plain_and_skipped_over() {
    // A huge minified line between two ordinary ones: it is not parsed (that
    // would take seconds) and the next line is highlighted from the state
    // before it.
    let long = format!("let a = [{}];", "1, ".repeat(10_000));
    assert!(long.len() > ochrepad_syntax::MAX_HIGHLIGHT_LINE);
    let doc = Document::new(&format!("// before\n{long}\nfn after() {{}}"));
    let t = Instant::now();
    let spans = full(&doc, "Rust");
    assert!(spans[1].as_ref().unwrap().is_empty());
    let alone = full(&Document::new("// before\nfn after() {}"), "Rust");
    assert_eq!(spans[2], alone[1]);
    if !cfg!(debug_assertions) {
        assert!(
            t.elapsed() < Duration::from_millis(200),
            "{:?}",
            t.elapsed()
        );
    }
}

#[test]
fn edit_at_the_top_of_a_large_file_converges_quickly() {
    let text = rust_file(10_000);
    let mut ed = Editor::new();
    let buf = ed.add_buffer(&text, FileMeta::default());
    let view = ed.add_view(buf).unwrap();
    let mut h = BufferHighlighter::new(&REG, "Rust", theme());
    let t = Instant::now();
    while h.work(
        &REG,
        ed.buffer(buf).unwrap().doc(),
        0..50,
        Duration::from_millis(50),
    ) {}
    let full_pass = t.elapsed();
    let _ = ed.buffer_mut(buf).unwrap().take_line_edits();

    for _ in 0..20 {
        ed.apply(view, Action::InsertText("x".into())).unwrap();
        let edits = ed.buffer_mut(buf).unwrap().take_line_edits();
        h.apply_edits(&edits);
        let before = h.lines_processed;
        while h.work(
            &REG,
            ed.buffer(buf).unwrap().doc(),
            0..50,
            Duration::from_millis(4),
        ) {}
        let lines = h.lines_processed - before;
        assert!(
            lines <= 2 * 128,
            "re-highlighted {lines} lines after one keystroke"
        );
    }
    println!("full pass over 10k lines: {full_pass:?}");
    // And the result matches a fresh highlight.
    let doc = ed.buffer(buf).unwrap().doc();
    assert_eq!(
        h.line_spans(&REG, doc, 0..300),
        full(doc, "Rust")[0..300].to_vec()
    );
}

#[test]
fn opening_a_block_comment_rehighlights_to_the_end() {
    let text = rust_file(2_000);
    let mut ed = Editor::new();
    let buf = ed.add_buffer(&text, FileMeta::default());
    let view = ed.add_view(buf).unwrap();
    let mut h = BufferHighlighter::new(&REG, "Rust", theme());
    while h.work_lines(&REG, ed.buffer(buf).unwrap().doc(), 0..0, 100_000) {}
    ed.apply(view, Action::InsertText("/*".into())).unwrap();
    h.apply_edits(&ed.buffer_mut(buf).unwrap().take_line_edits());
    while h.work_lines(&REG, ed.buffer(buf).unwrap().doc(), 0..0, 100_000) {}
    let doc = ed.buffer(buf).unwrap().doc();
    let n = doc.line_count();
    assert_eq!(h.line_spans(&REG, doc, 0..n), full(doc, "Rust"));
}

#[test]
fn far_viewport_gets_spans_before_the_pass_arrives() {
    let doc = Document::new(&rust_file(50_000));
    let mut h = BufferHighlighter::new(&REG, "Rust", theme());
    let t = Instant::now();
    let spans = h.line_spans(&REG, &doc, 40_000..40_060);
    // Only the visible lines are parsed (from a fresh state), not the 40,000
    // before them. Timing is only meaningful in an optimized build.
    if !cfg!(debug_assertions) {
        assert!(
            t.elapsed() < Duration::from_millis(200),
            "{:?}",
            t.elapsed()
        );
    }
    assert_eq!(h.lines_processed, 0, "the background pass did not run");
    assert!(spans.iter().all(Option::is_some));
    assert!(h.is_busy());
}

#[test]
fn changing_theme_or_language_restarts() {
    let doc = Document::new("fn x() {}");
    let mut h = BufferHighlighter::new(&REG, "Rust", theme());
    while h.work_lines(&REG, &doc, 0..1, 100) {}
    let dark = h.line_spans(&REG, &doc, 0..1);
    h.set_theme(&REG, Registry::default_theme(false));
    assert!(h.is_busy());
    while h.work_lines(&REG, &doc, 0..1, 100) {}
    assert_ne!(h.line_spans(&REG, &doc, 0..1), dark);
    h.set_syntax(&REG, PLAIN_TEXT);
    while h.work_lines(&REG, &doc, 0..1, 100) {}
    assert_eq!(h.line_spans(&REG, &doc, 0..1)[0].as_ref().unwrap().len(), 1);
}

#[test]
fn a_whole_document_is_highlighted_as_the_editor_does() {
    // File > Export highlights everything at once, on a thread.
    let doc = Document::new(&rust_file(300));
    let all = REG.highlight_all("Rust", theme(), &doc);
    let editor: Vec<LineSpans> = full(&doc, "Rust").into_iter().flatten().collect();
    assert_eq!(all, editor);
    // Text without a scope of its own is in the theme's default colour.
    let colors = REG.theme_colors(theme());
    let plain = REG.highlight_all(PLAIN_TEXT, theme(), &Document::new("just text"));
    assert_eq!(plain[0][0].color, colors.foreground);
}

#[test]
fn themes_say_their_text_and_background_colors() {
    let luma = |[r, g, b, _]: [u8; 4]| u32::from(r) * 3 + u32::from(g) * 6 + u32::from(b);
    let dark = REG.theme_colors(Registry::default_theme(true));
    assert!(luma(dark.background) < luma(dark.foreground), "{dark:?}");
    let light = REG.theme_colors(Registry::default_theme(false));
    assert!(luma(light.background) > luma(light.foreground), "{light:?}");
}

// ---------------------------------------------------------------------------

const PIECES: &[&str] = &[
    "fn ", "x", "y", " ", "\n", "\n", "/*", "*/", "\"", "//", "{", "}", "1", "'", "r#\"", "\"#",
    "\t", "漢",
];

#[derive(Debug, Clone)]
enum Step {
    Insert(usize, String),
    Delete(usize, usize),
    Undo,
    Work(usize),
    Look(usize),
}

fn step() -> impl Strategy<Value = Step> {
    prop_oneof![
        4 => (any::<usize>(), prop::collection::vec(prop::sample::select(PIECES), 1..4))
            .prop_map(|(p, v)| Step::Insert(p, v.concat())),
        2 => (any::<usize>(), 1usize..12).prop_map(|(p, n)| Step::Delete(p, n)),
        1 => Just(Step::Undo),
        3 => (0usize..40).prop_map(Step::Work),
        2 => any::<usize>().prop_map(Step::Look),
    ]
}

fn caret_at(ed: &mut Editor, view: ViewId, pos: usize) {
    let len = ed
        .buffer(ed.view(view).unwrap().buffer())
        .unwrap()
        .doc()
        .len()
        .0;
    let set = SelectionSet::caret(ByteOffset(pos % (len + 1)));
    ed.apply(view, Action::SetSelections(set)).unwrap();
}

proptest! {
    #![proptest_config(ProptestConfig { cases: 1000, ..ProptestConfig::default() })]

    /// Whatever the interleaving of edits, partial passes and viewport
    /// queries, the incremental result equals a fresh full highlight.
    #[test]
    fn incremental_equals_full(
        initial in prop::collection::vec(prop::sample::select(PIECES), 0..200).prop_map(|v| v.concat()),
        steps in prop::collection::vec(step(), 0..30),
        interval in 1usize..6,
        catch_up in 0usize..4,
    ) {
        let mut ed = Editor::new();
        let buf = ed.add_buffer(&initial, FileMeta::default());
        let view = ed.add_view(buf).unwrap();
        let mut h = BufferHighlighter::new(&REG, "Rust", theme());
        h.set_limits(interval, catch_up);
        for s in steps {
            match s {
                Step::Insert(p, text) => {
                    caret_at(&mut ed, view, p);
                    ed.apply(view, Action::InsertText(text)).unwrap();
                }
                Step::Delete(p, n) => {
                    caret_at(&mut ed, view, p);
                    for _ in 0..n {
                        ed.apply(view, Action::DeleteForward).unwrap();
                    }
                }
                Step::Undo => ed.apply(view, Action::Undo).unwrap(),
                Step::Work(n) => {
                    let doc = ed.buffer(buf).unwrap().doc();
                    h.work_lines(&REG, doc, 0..doc.line_count(), n);
                }
                Step::Look(first) => {
                    let doc = ed.buffer(buf).unwrap().doc();
                    let first = first % doc.line_count();
                    h.line_spans(&REG, doc, first..first + 5);
                }
            }
            h.apply_edits(&ed.buffer_mut(buf).unwrap().take_line_edits());
        }
        let doc = ed.buffer(buf).unwrap().doc();
        while h.work_lines(&REG, doc, 0..doc.line_count(), 7) {}
        let n = doc.line_count();
        prop_assert_eq!(h.line_spans(&REG, doc, 0..n), full(doc, "Rust"));
    }
}
