//! Generates the performance fixtures (PLAN 6), which are not committed:
//! a 50,000-line Rust source file and a ~100 MB plain-text file, and on
//! request logs of any size for the viewer for large files (PLAN_v0.3.0 P7).
//!
//!     cargo run --release -p ochrepad --example gen_perf_fixtures [out_dir] [--log MB]...
//!
//! Default output directory: target/perf-fixtures. `--log 1024` writes
//! `log_1024mb.log`, a log of 1 GB; give it once per size. A 4 GB log needs
//! 4 GB of disk, so check the free space first.

use std::io::{BufWriter, Write};
use std::path::{Path, PathBuf};

fn main() -> std::io::Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let mut logs = Vec::new();
    let mut dir = PathBuf::from("target/perf-fixtures");
    let mut i = 0;
    while i < args.len() {
        if args[i] == "--log" {
            match args.get(i + 1).and_then(|n| n.parse::<u64>().ok()) {
                Some(mb) => logs.push(mb),
                None => {
                    eprintln!("--log needs a size in MB");
                    std::process::exit(2);
                }
            }
            i += 2;
        } else {
            dir = PathBuf::from(&args[i]);
            i += 1;
        }
    }
    std::fs::create_dir_all(&dir)?;
    if !logs.is_empty() {
        for mb in logs {
            write_log(&dir, mb)?;
        }
        return Ok(());
    }

    let src = dir.join("source_50k.rs");
    let mut w = BufWriter::new(std::fs::File::create(&src)?);
    let mut n = 0;
    while n < 50_000 {
        let i = n / 10;
        for line in [
            format!("/// Doc comment for item {i} — with “quotes” and 漢字."),
            format!("pub fn function_{i}(x: u32, name: &str) -> Result<String, Error> {{"),
            format!("    let value = compute(x * {i}, \"literal {i}\")?;"),
            "    if value > 42 {".to_owned(),
            "        return Err(Error::new(\"too big\"));".to_owned(),
            "    }".to_owned(),
            format!("    let s = format!(\"{{name}}: {{value}} \\t {i}\"); // trailing comment"),
            "    Ok(s)".to_owned(),
            "}".to_owned(),
            String::new(),
        ] {
            writeln!(w, "{line}")?;
            n += 1;
        }
    }
    w.flush()?;
    println!("wrote {} ({n} lines)", src.display());

    let big = dir.join("plain_100mb.txt");
    let mut w = BufWriter::new(std::fs::File::create(&big)?);
    let mut bytes = 0usize;
    let mut i = 0usize;
    while bytes < 100 * 1024 * 1024 {
        let line = format!(
            "{i:>9} The quick brown fox jumps over the lazy dog; pack my box with five dozen liquor jugs.\n"
        );
        w.write_all(line.as_bytes())?;
        bytes += line.len();
        i += 1;
    }
    w.flush()?;
    println!("wrote {} ({bytes} bytes, {i} lines)", big.display());
    Ok(())
}

/// A server log of `mb` megabytes: timestamps, levels (mostly INFO, some
/// DEBUG, WARN and ERROR), components and messages of varying length.
fn write_log(dir: &Path, mb: u64) -> std::io::Result<()> {
    const LEVELS: [&str; 8] = [
        "INFO", "INFO", "INFO", "DEBUG", "INFO", "DEBUG", "WARN", "INFO",
    ];
    const PARTS: [&str; 6] = ["http", "db.pool", "auth", "cache", "scheduler", "worker-7"];
    const WORDS: [&str; 12] = [
        "request",
        "completed",
        "user",
        "session",
        "retrying",
        "connection",
        "timeout",
        "queued",
        "flushed",
        "rows",
        "token",
        "refreshed",
    ];
    let path = dir.join(format!("log_{mb}mb.log"));
    let mut w = BufWriter::with_capacity(1 << 20, std::fs::File::create(&path)?);
    let target = mb * 1024 * 1024;
    let (mut bytes, mut i) = (0u64, 0u64);
    let mut rng: u64 = 0x9E37_79B9_7F4A_7C15;
    let mut line = String::with_capacity(256);
    while bytes < target {
        rng ^= rng << 13;
        rng ^= rng >> 7;
        rng ^= rng << 17;
        let level = if i % 997 == 0 {
            "ERROR"
        } else {
            LEVELS[(rng % 8) as usize]
        };
        let secs = i / 50;
        line.clear();
        use std::fmt::Write as _;
        let _ = write!(
            line,
            "2026-10-{:02} {:02}:{:02}:{:02}.{:03} {level:<5} [{}] ",
            1 + (secs / 86_400) % 28,
            (secs / 3600) % 24,
            (secs / 60) % 60,
            secs % 60,
            (i * 7) % 1000,
            PARTS[(rng >> 8) as usize % PARTS.len()],
        );
        for k in 0..(4 + (rng >> 16) % 14) {
            line.push_str(WORDS[((rng >> (k + 20)) as usize) % WORDS.len()]);
            line.push(' ');
        }
        let _ = writeln!(line, "id={i}");
        w.write_all(line.as_bytes())?;
        bytes += line.len() as u64;
        i += 1;
    }
    w.flush()?;
    println!("wrote {} ({bytes} bytes, {i} lines)", path.display());
    Ok(())
}
