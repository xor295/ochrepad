//! Listing folders for the file tree (PLAN_v0.2.0 N7). The only module that
//! uses `ignore`: `.gitignore`, `.ignore` and git's global and repository
//! excludes are honoured, also outside a git repository, and `.git` itself
//! counts as ignored. Symbolic links are listed but never followed.
//!
//! [`Lister`] lists on a background thread, so a slow or unreachable folder
//! never holds up a frame.

use std::cmp::Ordering;
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{Receiver, Sender, channel};

use ignore::WalkBuilder;

/// One file or folder in a listing.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entry {
    pub name: String,
    pub path: PathBuf,
    pub is_dir: bool,
    /// Excluded by an ignore file (shown only on request).
    pub ignored: bool,
}

/// The files and folders directly in `dir`: folders first, then files, each
/// in natural order ("file2" before "file10", case ignored). Ignored entries
/// are left out unless `show_ignored`.
pub fn list_dir(dir: &Path, show_ignored: bool) -> Result<Vec<Entry>, String> {
    let shown: HashSet<PathBuf> = WalkBuilder::new(dir)
        .max_depth(Some(1))
        .hidden(false)
        .parents(true)
        .ignore(true)
        .git_ignore(true)
        .git_global(true)
        .git_exclude(true)
        .require_git(false)
        .follow_links(false)
        .build()
        .filter_map(Result::ok)
        .filter(|e| e.depth() == 1)
        .map(|e| e.into_path())
        .collect();
    let read = std::fs::read_dir(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    let mut out: Vec<Entry> = read
        .filter_map(Result::ok)
        .filter_map(|e| {
            let file_type = e.file_type().ok()?;
            let name = e.file_name().to_string_lossy().into_owned();
            let path = e.path();
            let ignored = name == ".git" || !shown.contains(&path);
            (show_ignored || !ignored).then_some(Entry {
                is_dir: file_type.is_dir(),
                name,
                path,
                ignored,
            })
        })
        .collect();
    out.sort_by(|a, b| {
        b.is_dir
            .cmp(&a.is_dir)
            .then_with(|| natural_cmp(&a.name, &b.name))
    });
    Ok(out)
}

/// Compares names the way people sort them: runs of digits by value, the
/// rest ignoring case; ties fall back to the exact text.
pub fn natural_cmp(a: &str, b: &str) -> Ordering {
    let (mut x, mut y) = (a, b);
    loop {
        match (x.is_empty(), y.is_empty()) {
            (true, true) => return a.cmp(b),
            (true, false) => return Ordering::Less,
            (false, true) => return Ordering::Greater,
            _ => {}
        }
        let (cx, rx) = chunk(x);
        let (cy, ry) = chunk(y);
        let digits = |s: &str| s.starts_with(|c: char| c.is_ascii_digit());
        let ord = if digits(cx) && digits(cy) {
            let (tx, ty) = (cx.trim_start_matches('0'), cy.trim_start_matches('0'));
            tx.len().cmp(&ty.len()).then_with(|| tx.cmp(ty))
        } else {
            cx.to_lowercase().cmp(&cy.to_lowercase())
        };
        if ord != Ordering::Equal {
            return ord;
        }
        (x, y) = (rx, ry);
    }
}

/// The leading run of digits or of non-digits, and the rest.
fn chunk(s: &str) -> (&str, &str) {
    let digit = s.starts_with(|c: char| c.is_ascii_digit());
    let end = s
        .find(|c: char| c.is_ascii_digit() != digit)
        .unwrap_or(s.len());
    s.split_at(end)
}

/// A finished listing: the folder asked for and its entries or the error.
pub type Listing = (PathBuf, Result<Vec<Entry>, String>);

/// Lists folders on a background thread.
pub struct Lister {
    tx: Option<Sender<(PathBuf, bool)>>,
    rx: Receiver<Listing>,
    pending: usize,
}

impl Lister {
    /// Starts the thread; `wake` is called after each listing (to repaint).
    pub fn spawn(wake: impl Fn() + Send + 'static) -> Self {
        let (tx, requests) = channel::<(PathBuf, bool)>();
        let (done, rx) = channel();
        let started = std::thread::Builder::new()
            .name("ochrepad-lister".to_owned())
            .spawn(move || {
                for (dir, show_ignored) in requests {
                    let listing = list_dir(&dir, show_ignored);
                    if done.send((dir, listing)).is_err() {
                        break;
                    }
                    wake();
                }
            })
            .is_ok();
        Self {
            tx: started.then_some(tx),
            rx,
            pending: 0,
        }
    }

    /// Asks for a listing of `dir`.
    pub fn request(&mut self, dir: PathBuf, show_ignored: bool) {
        if let Some(tx) = &self.tx
            && tx.send((dir, show_ignored)).is_ok()
        {
            self.pending += 1;
        }
    }

    /// Listings finished since the last call.
    pub fn poll(&mut self) -> Vec<Listing> {
        let out: Vec<Listing> = self.rx.try_iter().collect();
        self.pending -= out.len().min(self.pending);
        out
    }

    /// Listings asked for and not yet returned by `poll`.
    pub fn pending(&self) -> usize {
        self.pending
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn natural_order() {
        let mut names = vec!["file10", "File2", "file1", "b", "A", "file02x", "file2"];
        names.sort_by(|a, b| natural_cmp(a, b));
        assert_eq!(
            names,
            vec!["A", "b", "file1", "File2", "file2", "file02x", "file10"]
        );
    }

    fn names(entries: &[Entry]) -> Vec<String> {
        entries
            .iter()
            .map(|e| {
                format!(
                    "{}{}{}",
                    e.name,
                    if e.is_dir { "/" } else { "" },
                    if e.ignored { " (ignored)" } else { "" }
                )
            })
            .collect()
    }

    #[test]
    fn listings_sort_folders_first_and_honour_ignore_files() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        for d in ["src", "target", ".git", "docs"] {
            std::fs::create_dir(root.join(d)).unwrap();
        }
        for f in ["b.txt", "a10.txt", "a2.txt", "debug.log", ".gitignore"] {
            std::fs::write(root.join(f), "").unwrap();
        }
        std::fs::write(root.join(".gitignore"), "target/\n*.log\n").unwrap();
        std::fs::write(root.join("src").join("x.log"), "").unwrap();
        std::fs::write(root.join("src").join("main.rs"), "").unwrap();

        let shown = list_dir(root, false).unwrap();
        assert_eq!(
            names(&shown),
            vec!["docs/", "src/", ".gitignore", "a2.txt", "a10.txt", "b.txt"]
        );
        let all = list_dir(root, true).unwrap();
        assert_eq!(
            names(&all),
            vec![
                ".git/ (ignored)",
                "docs/",
                "src/",
                "target/ (ignored)",
                ".gitignore",
                "a2.txt",
                "a10.txt",
                "b.txt",
                "debug.log (ignored)",
            ]
        );
        // A parent folder's .gitignore applies to a subfolder's listing.
        assert_eq!(
            names(&list_dir(&root.join("src"), false).unwrap()),
            vec!["main.rs"]
        );
        assert!(list_dir(&root.join("missing"), false).is_err());
    }

    #[test]
    fn the_lister_works_in_the_background() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(tmp.path().join("a"), "").unwrap();
        let mut lister = Lister::spawn(|| {});
        lister.request(tmp.path().to_path_buf(), false);
        assert_eq!(lister.pending(), 1);
        let mut got = Vec::new();
        for _ in 0..500 {
            got.extend(lister.poll());
            if !got.is_empty() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        assert_eq!(got.len(), 1);
        assert_eq!(names(&got[0].1.clone().unwrap()), vec!["a"]);
        assert_eq!(lister.pending(), 0);
    }
}
