//! Completion ranking speed (PLAN_completion C1): 50,000 identifiers ranked
//! for a few patterns, the slowest of several runs.
//!
//! ```sh
//! cargo run --release -p ochrepad --example complete_bench
//! cargo run --release -p ochrepad --example complete_bench -- path/to/file   # its words
//! ```

use std::time::{Duration, Instant};

use ochrepad_core::complete::{self, Candidate, Kind, Source};

/// Made-up identifiers in the usual styles, deterministic.
fn synthetic(n: usize) -> Vec<String> {
    const PARTS: [&str; 24] = [
        "get", "set", "text", "length", "buffer", "view", "line", "caret", "select", "range",
        "find", "next", "prev", "word", "map", "row", "col", "fold", "paint", "layout", "doc",
        "edit", "undo", "index",
    ];
    let mut seed: u64 = 0x9E37_79B9_7F4A_7C15;
    let mut next = || {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        seed
    };
    (0..n)
        .map(|_| {
            let k = 1 + next() % 4;
            let parts: Vec<&str> = (0..k).map(|_| PARTS[(next() % 24) as usize]).collect();
            match next() % 3 {
                0 => parts.join("_"),
                1 => {
                    let mut s = parts[0].to_owned();
                    for p in &parts[1..] {
                        let mut c = p.chars();
                        if let Some(f) = c.next() {
                            s.extend(f.to_uppercase());
                            s.push_str(c.as_str());
                        }
                    }
                    s
                }
                _ => parts.concat(),
            }
        })
        .collect()
}

fn main() {
    let words: Vec<String> = match std::env::args().nth(1) {
        Some(path) => {
            let text = std::fs::read_to_string(&path).expect("readable UTF-8 file");
            let mut w: Vec<String> = complete::words(&text, &[], 3).map(str::to_owned).collect();
            w.sort();
            w.dedup();
            w
        }
        None => {
            let mut w = synthetic(80_000);
            w.sort();
            w.dedup();
            w.truncate(50_000);
            w
        }
    };
    println!("{} candidates", words.len());
    let index = complete::WordIndex::new(words.iter().map(|w| (w.clone(), 1)));
    println!("{:<8} {:>8} {:>10}", "pattern", "matches", "slowest");
    for pattern in ["g", "ge", "get", "gtl", "setLine", "xq", "bufview"] {
        let mut slowest = Duration::ZERO;
        let mut matches = 0;
        for _ in 0..5 {
            // As the app does: the source keeps the words that match, then
            // they are ranked.
            let t = Instant::now();
            let prepared = complete::Pattern::new(pattern);
            let candidates: Vec<Candidate> = index
                .matching(&prepared)
                .map(|(w, _)| Candidate::new(w, Kind::Word, Source::Word))
                .collect();
            matches = complete::rank(pattern, candidates, 100).len();
            slowest = slowest.max(t.elapsed());
        }
        println!(
            "{pattern:<8} {matches:>8} {:>8.2}ms",
            slowest.as_secs_f64() * 1000.0
        );
    }
}
