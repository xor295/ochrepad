//! Times find-all on a perf fixture (PLAN 6, M7).
//!
//!     cargo run --release -p ochrepad --example find_bench -- <file>

use std::time::Instant;

use ochrepad_core::{ByteOffset, Document, SearchOptions, SearchQuery, Searcher};

fn main() {
    let Some(path) = std::env::args().nth(1) else {
        eprintln!("usage: find_bench <file>");
        return;
    };
    let decoded = match ochrepad_fileio::load(std::path::Path::new(&path)) {
        Ok(d) => d,
        Err(e) => {
            eprintln!("{e}");
            return;
        }
    };
    let doc = Document::new(&decoded.text);
    println!("{path}: {} bytes, {} lines", doc.len().0, doc.line_count());
    let cases: [(&str, SearchOptions); 5] = [
        ("fox", SearchOptions::default()),
        (
            "value",
            SearchOptions {
                match_case: true,
                whole_word: true,
                ..Default::default()
            },
        ),
        (
            r"\d+ The",
            SearchOptions {
                regex: true,
                ..Default::default()
            },
        ),
        (
            r"^\s+let",
            SearchOptions {
                regex: true,
                ..Default::default()
            },
        ),
        ("zzz-not-there", SearchOptions::default()),
    ];
    for (pattern, options) in cases {
        let s = Searcher::new(&SearchQuery::new(pattern, options)).expect("valid");
        let t = Instant::now();
        let n = s.find_all(&doc, ByteOffset(0)..doc.len()).len();
        let all = t.elapsed();
        let t = Instant::now();
        let prev = s.find_prev(&doc, doc.len());
        let back = t.elapsed();
        println!(
            "{pattern:>14}: {n:>8} matches, find all {all:>10.2?}, find previous from end {back:>8.2?} ({})",
            if prev.is_some() { "found" } else { "none" }
        );
    }
}
