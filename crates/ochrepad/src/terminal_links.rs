//! Links in a terminal's output: web links, and files with the line (and
//! column) compilers and tracebacks give, found under the pointer on
//! Ctrl+hover. Files are looked for the way the editor's Open File looks
//! for them (`editor_menu::file_named`): on this machine only, never a
//! network path.

use std::ops::Range;
use std::path::{Path, PathBuf};

use crate::editor_menu::{file_named, path_run, url_in};

/// What a link opens.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LinkTarget {
    Url(String),
    /// A file, at a line and column when the text gave them.
    File(PathBuf, Option<(usize, usize)>),
}

/// A link on a row: the screen columns it covers, and what it opens.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Link {
    pub cols: Range<usize>,
    pub target: LinkTarget,
}

/// The link at column `col` of a row, given as its characters with the
/// column each starts at (a wide character covers two). Relative paths are
/// looked for in `dirs`.
pub fn link_at(cells: &[(usize, char)], col: usize, dirs: &[&Path]) -> Option<Link> {
    // The row as text, and where each character starts in it.
    let mut text = String::new();
    let mut starts = Vec::with_capacity(cells.len());
    for &(_, c) in cells {
        starts.push(text.len());
        text.push(c);
    }
    let i = cells.iter().rposition(|&(c, _)| c <= col)?;
    let at = starts[i];
    // Byte range → columns: from the first character's column to the end
    // of the last one.
    let cols = |bytes: Range<usize>| -> Range<usize> {
        let first = starts.partition_point(|&s| s < bytes.start);
        let last = starts.partition_point(|&s| s < bytes.end).saturating_sub(1);
        let end = cells
            .get(last + 1)
            .map_or(cells[last].0 + 1, |&(next, _)| next);
        cells[first].0..end
    };
    if let Some((bytes, url)) = url_in(&text, at) {
        return Some(Link {
            cols: cols(bytes),
            target: LinkTarget::Url(url),
        });
    }
    let run = path_run(&text, at);
    let word = &text[run.clone()];
    // What the path's own text leaves out: trailing punctuation, which is
    // not underlined either.
    let trimmed = word.trim_end_matches(['.', ',', ':']);
    if trimmed.is_empty() {
        return None;
    }
    let (path, mut place) = file_named(trimmed, dirs)?;
    if place.is_none() {
        place = place_after(&text[run.end..]);
    }
    Some(Link {
        cols: cols(run.start..run.start + trimmed.len()),
        target: LinkTarget::File(path, place),
    })
}

/// A line (and column) written after a path in the ways that do not use a
/// colon: Python's `File "x.py", line 3`, and MSVC's and .NET's
/// `x.cs(12,5)`.
fn place_after(rest: &str) -> Option<(usize, usize)> {
    let number = |s: &str| -> Option<(usize, usize)> {
        let digits = s.bytes().take_while(u8::is_ascii_digit).count();
        Some((s[..digits].parse().ok()?, digits))
    };
    if let Some(after) = rest.strip_prefix("\", line ") {
        let (line, _) = number(after)?;
        return Some((line, 1));
    }
    let after = rest.strip_prefix('(')?;
    let (line, n) = number(after)?;
    let after = &after[n..];
    if after.starts_with(')') {
        return Some((line, 1));
    }
    let (col, n) = number(after.strip_prefix(',')?)?;
    after[1 + n..].starts_with(')').then_some((line, col))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cells(s: &str) -> Vec<(usize, char)> {
        s.chars().enumerate().collect()
    }

    fn find(row: &str, col: usize, dir: &Path) -> Option<Link> {
        link_at(&cells(row), col, &[dir])
    }

    #[test]
    fn web_links_cover_their_columns() {
        let dir = std::env::temp_dir();
        let row = "see https://example.com/a(b). now";
        let link = find(row, 10, &dir).unwrap();
        assert_eq!(link.cols, 4..28);
        assert_eq!(
            link.target,
            LinkTarget::Url("https://example.com/a(b)".to_owned())
        );
        assert!(find(row, 31, &dir).is_none());
    }

    #[test]
    fn compiler_and_traceback_places() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir(dir.path().join("src")).unwrap();
        std::fs::write(dir.path().join("src").join("main.rs"), "fn main() {}\n").unwrap();
        std::fs::write(dir.path().join("x.py"), "pass\n").unwrap();
        std::fs::write(dir.path().join("Prog.cs"), "\n").unwrap();
        let main = dir.path().join("src").join("main.rs");

        // cargo: "  --> src/main.rs:3:5"
        let link = find("  --> src/main.rs:3:5", 9, dir.path()).unwrap();
        assert_eq!(link.cols, 6..21);
        assert_eq!(link.target, LinkTarget::File(main.clone(), Some((3, 5))));

        // Python: '  File "x.py", line 7, in <module>'
        let link = find("  File \"x.py\", line 7, in <module>", 9, dir.path()).unwrap();
        assert_eq!(link.cols, 8..12);
        assert_eq!(
            link.target,
            LinkTarget::File(dir.path().join("x.py"), Some((7, 1)))
        );

        // MSVC and .NET: "Prog.cs(12,5): error"
        let link = find("Prog.cs(12,5): error CS1002", 2, dir.path()).unwrap();
        assert_eq!(
            link.target,
            LinkTarget::File(dir.path().join("Prog.cs"), Some((12, 5)))
        );

        // A plain path, then punctuation.
        let link = find("wrote src/main.rs.", 8, dir.path()).unwrap();
        assert_eq!(link.cols, 6..17);
        assert_eq!(link.target, LinkTarget::File(main, None));

        // Words that are not files.
        assert!(find("error: could not compile", 3, dir.path()).is_none());
    }

    #[test]
    fn wide_characters_take_two_columns() {
        let dir = std::env::temp_dir();
        // "你 https://x.org": 你 covers columns 0 and 1.
        let row = vec![(0, '你'), (2, ' '), (3, 'h')]
            .into_iter()
            .chain("ttps://x.org".chars().enumerate().map(|(i, c)| (4 + i, c)))
            .collect::<Vec<_>>();
        let link = link_at(&row, 5, &[&dir]).unwrap();
        assert_eq!(link.cols, 3..16);
    }

    #[cfg(windows)]
    #[test]
    fn network_paths_are_never_looked_at() {
        let dir = std::env::temp_dir();
        assert!(find(r"\\server\share\x.rs:3", 5, &dir).is_none());
        assert!(find("//server/share/x.rs:3", 5, &dir).is_none());
    }
}
