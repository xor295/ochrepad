//! Test helpers: build an editor from text with `|` caret markers, and render
//! the state back with `|` at each head and `^` at the anchor of each
//! non-empty selection.

#![allow(dead_code)]

use ochrepad_core::{
    Action, ByteOffset, Editor, FileMeta, ManualClock, Motion, Selection, SelectionSet, ViewId,
};

pub struct T {
    pub ed: Editor,
    pub view: ViewId,
    pub clock: ManualClock,
}

pub fn setup(marked: &str) -> T {
    let mut text = String::new();
    let mut carets = Vec::new();
    for c in marked.chars() {
        if c == '|' {
            carets.push(Selection::caret(ByteOffset(text.len())));
        } else {
            text.push(c);
        }
    }
    let clock = ManualClock::default();
    let mut ed = Editor::with_clock(Box::new(clock.clone()));
    let buffer = ed.add_buffer(&text, FileMeta::default());
    let view = ed.add_view(buffer).expect("buffer exists");
    if !carets.is_empty() {
        ed.view_mut(view).expect("view exists").selections = SelectionSet::new(carets, 0);
    }
    T { ed, view, clock }
}

impl T {
    pub fn text(&self) -> String {
        self.buffer().doc().to_string()
    }

    pub fn buffer(&self) -> &ochrepad_core::Buffer {
        let b = self.ed.view(self.view).expect("view").buffer();
        self.ed.buffer(b).expect("buffer")
    }

    pub fn sels(&self) -> SelectionSet {
        self.ed.view(self.view).expect("view").selections.clone()
    }

    pub fn set_sels(&mut self, ranges: Vec<(usize, usize)>) {
        let ranges = ranges
            .into_iter()
            .map(|(a, h)| Selection::new(ByteOffset(a), ByteOffset(h)))
            .collect();
        self.ed.view_mut(self.view).expect("view").selections = SelectionSet::new(ranges, 0);
    }

    /// The text with `|` at heads and `^` at anchors of non-empty selections.
    pub fn show(&self) -> String {
        render(&self.text(), &self.sels())
    }

    pub fn apply(&mut self, action: Action) -> &mut Self {
        self.ed.apply(self.view, action).expect("apply");
        self
    }

    pub fn mv(&mut self, motion: Motion) -> &mut Self {
        self.apply(Action::MoveCursor {
            motion,
            extend: false,
        })
    }

    pub fn ext(&mut self, motion: Motion) -> &mut Self {
        self.apply(Action::MoveCursor {
            motion,
            extend: true,
        })
    }

    pub fn typ(&mut self, s: &str) -> &mut Self {
        for c in s.chars() {
            if c == '\n' {
                self.apply(Action::InsertNewline);
            } else {
                self.apply(Action::InsertText(c.to_string()));
            }
        }
        self
    }
}

pub fn render(text: &str, sels: &SelectionSet) -> String {
    let mut marks: Vec<(usize, char)> = Vec::new();
    for s in sels.ranges() {
        if !s.is_empty() {
            marks.push((s.anchor.0, '^'));
        }
        marks.push((s.head.0, '|'));
    }
    marks.sort();
    let mut out = String::new();
    let mut last = 0;
    for (pos, m) in marks {
        out.push_str(&text[last..pos]);
        out.push(m);
        last = pos;
    }
    out.push_str(&text[last..]);
    out
}
