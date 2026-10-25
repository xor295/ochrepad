//! The folder index's speed and size (PLAN_completion C3): indexes a folder
//! from scratch, then again from its cache, and prints the times, the
//! counts and the cache's size.
//!
//! ```sh
//! cargo run --release -p ochrepad --example index_bench -- path/to/folder
//! ```

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use ochrepad::folder_index::{FolderIndex, Progress};
use ochrepad_core::complete::{self, Candidate, Kind, Source};
use ochrepad_syntax::Registry;

/// Indexes `root` until ready; returns the index and the times to words and
/// to ready.
fn index(
    root: &Path,
    cache: Option<PathBuf>,
    reg: &Arc<Registry>,
) -> (FolderIndex, Duration, Duration) {
    let start = Instant::now();
    let mut index = FolderIndex::open(
        root.to_path_buf(),
        cache,
        reg.clone(),
        egui::Context::default(),
    );
    let mut words = None;
    loop {
        index.poll();
        if words.is_none() && index.snapshot.is_some() {
            words = Some(start.elapsed());
        }
        if index.progress == Progress::Ready && index.snapshot.is_some() {
            let ready = start.elapsed();
            return (index, words.unwrap_or(ready), ready);
        }
        std::thread::sleep(Duration::from_millis(5));
    }
}

fn main() {
    let Some(root) = std::env::args().nth(1).map(PathBuf::from) else {
        eprintln!("usage: index_bench <folder>");
        return;
    };
    let reg = Arc::new(Registry::load());
    let data = std::env::temp_dir().join("ochrepad-index-bench");
    let _ = std::fs::create_dir_all(&data);
    let cache = data.join("index.json");
    let _ = std::fs::remove_file(&cache);

    let (mut first, words, ready) = index(&root, Some(cache.clone()), &reg);
    let s = first.snapshot.clone().unwrap_or_default();
    println!(
        "from scratch: words after {:.2} s, ready after {:.2} s",
        words.as_secs_f64(),
        ready.as_secs_f64()
    );
    println!(
        "  {} files{}, {} distinct words, {} defined names",
        s.files,
        if s.truncated { " (truncated)" } else { "" },
        s.words.len(),
        s.definitions.len()
    );
    let start = Instant::now();
    let bytes = loop {
        first.poll();
        if let Some((_, bytes)) = first.take_cache() {
            break bytes;
        }
        if start.elapsed() > Duration::from_secs(60) {
            eprintln!("no cache came");
            return;
        }
        std::thread::sleep(Duration::from_millis(5));
    };
    println!("  cache: {:.1} MB", bytes.len() as f64 / 1e6);
    // What a keystroke costs with this many words: the folder's best
    // words as candidates, ranked (as `completion::Sources` does), the
    // slowest of a few runs.
    for prefix in ["", "s", "ge", "to", "gtl", "parse", "new_with"] {
        let mut worst = Duration::ZERO;
        let mut kept = 0;
        for _ in 0..5 {
            let start = Instant::now();
            let pattern = complete::Pattern::new(prefix);
            let candidates: Vec<Candidate> = s
                .words
                .best(&pattern, 200)
                .into_iter()
                .map(|(w, _)| Candidate::new(w, Kind::Word, Source::Word))
                .collect();
            kept = candidates.len();
            let ranked = complete::rank(prefix, candidates, 50);
            std::hint::black_box(ranked);
            worst = worst.max(start.elapsed());
        }
        println!(
            "  rank {prefix:>10?}: {:>7} candidates, {:.1} ms",
            kept,
            worst.as_secs_f64() * 1e3
        );
    }
    let _ = std::fs::write(&cache, bytes);
    drop(first);

    let (_, words, ready) = index(&root, Some(cache.clone()), &reg);
    println!(
        "from the cache: words after {:.2} s, ready after {:.2} s",
        words.as_secs_f64(),
        ready.as_secs_f64()
    );
    let _ = std::fs::remove_file(&cache);
}
