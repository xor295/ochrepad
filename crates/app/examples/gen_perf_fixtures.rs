//! Generates the performance fixtures (PLAN 6), which are not committed:
//! a 50,000-line Rust source file and a ~100 MB plain-text file.
//!
//!     cargo run --release -p ochrepad --example gen_perf_fixtures [out_dir]
//!
//! Default output directory: target/perf-fixtures.

use std::io::{BufWriter, Write};
use std::path::PathBuf;

fn main() -> std::io::Result<()> {
    let dir = std::env::args()
        .nth(1)
        .map_or_else(|| PathBuf::from("target/perf-fixtures"), PathBuf::from);
    std::fs::create_dir_all(&dir)?;

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
