//! Times the compare view's line diff on a perf fixture (PLAN M12).
//!
//!     cargo run --release -p ochrepad --example diff_bench -- <file>

use std::time::Instant;

use ochrepad_core::Document;
use ochrepad_core::diff::{diff_lines, diff_words};

fn main() {
    let Some(path) = std::env::args().nth(1) else {
        eprintln!("usage: diff_bench <file>");
        return;
    };
    let text = match ochrepad_fileio::load(std::path::Path::new(&path)) {
        Ok(d) => d.text,
        Err(e) => {
            eprintln!("{e}");
            return;
        }
    };
    // Edit every 100th line, delete every 997th, insert after every 1009th.
    let mut edited = Vec::new();
    for (i, line) in text.lines().enumerate() {
        if i % 997 == 0 {
            continue;
        }
        if i % 100 == 0 {
            edited.push(format!("{line} // edited"));
        } else {
            edited.push(line.to_owned());
        }
        if i % 1009 == 0 {
            edited.push("// inserted".to_owned());
        }
    }
    let (a, b) = (Document::new(&text), Document::new(&edited.join("\n")));
    let t = Instant::now();
    let d = diff_lines(&a, &b);
    let lines = t.elapsed();
    let t = Instant::now();
    let mut words = 0;
    for r in d
        .rows
        .iter()
        .filter(|r| r.left.is_some() && r.right.is_some())
        .take(60)
    {
        let (l, rr) = (r.left.unwrap_or(0), r.right.unwrap_or(0));
        words += diff_words(&a.line_text(l), &b.line_text(rr)).0.len();
    }
    println!(
        "{} lines: line diff {lines:?} ({} rows, {} hunks, {:?}); 60 word diffs {:?} ({words} ranges)",
        a.line_count(),
        d.rows.len(),
        d.hunks.len(),
        d.stats,
        t.elapsed()
    );
}
