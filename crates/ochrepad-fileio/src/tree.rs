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

/// Whether a path, as written, is on one of this machine's drives (on
/// Windows; a UNC or device path is not). Relative paths are.
fn on_this_machine(path: &Path) -> bool {
    use std::path::{Component, Prefix};
    match path.components().next() {
        Some(Component::Prefix(p)) => matches!(p.kind(), Prefix::Disk(_) | Prefix::VerbatimDisk(_)),
        _ => true,
    }
}

/// The first line of a small text file.
fn first_line(path: &Path) -> Option<String> {
    use std::io::Read;
    let mut head = Vec::new();
    std::fs::File::open(path)
        .ok()?
        .take(4096)
        .read_to_end(&mut head)
        .ok()?;
    let text = String::from_utf8_lossy(&head);
    Some(text.lines().next().unwrap_or("").to_owned())
}

/// Whether the ignore files `ignore` reads in `dir` stay on this machine:
/// `.gitignore` and `.ignore`, and git's `info/exclude`, which it finds
/// through `.git` (a folder, a link, or a worktree's file whose `gitdir:`
/// line, and the `commondir` there, name the real one). On Windows only
/// opening `\\host\share\...` sends the user's credentials to that host,
/// and a folder from someone else can point there with a link or just a
/// `.git` file. Elsewhere this is always true.
pub fn ignore_files_stay_local(dir: &Path) -> bool {
    if !cfg!(windows) {
        return true;
    }
    // A link's target, relative to its folder.
    let link_stays = |path: &Path| match std::fs::symlink_metadata(path) {
        Ok(meta) if meta.file_type().is_symlink() => std::fs::read_link(path).is_ok_and(|t| {
            let target = match path.parent() {
                Some(parent) if t.is_relative() => parent.join(t),
                _ => t,
            };
            on_this_machine(&target)
        }),
        _ => true,
    };
    if !link_stays(&dir.join(".gitignore")) || !link_stays(&dir.join(".ignore")) {
        return false;
    }
    let git = dir.join(".git");
    match std::fs::symlink_metadata(&git) {
        Ok(meta) if meta.file_type().is_symlink() => link_stays(&git),
        Ok(meta) if meta.is_file() => {
            let Some(line) = first_line(&git) else {
                return true;
            };
            let Some(gitdir) = line.strip_prefix("gitdir: ") else {
                return true;
            };
            let gitdir = PathBuf::from(gitdir);
            if !on_this_machine(&gitdir) {
                return false;
            }
            // `commondir` there names the repository's own folder.
            first_line(&gitdir.join("commondir"))
                .is_none_or(|c| c.starts_with('.') || on_this_machine(Path::new(&c)))
        }
        Ok(_) => link_stays(&git.join("info")) && link_stays(&git.join("info").join("exclude")),
        Err(_) => true,
    }
}

/// The files and folders directly in `dir`: folders first, then files, each
/// in natural order ("file2" before "file10", case ignored). Ignored entries
/// are left out unless `show_ignored`.
pub fn list_dir(dir: &Path, show_ignored: bool) -> Result<Vec<Entry>, String> {
    // Ignore files that point off this machine turn them all off here.
    let honour = dir.ancestors().all(ignore_files_stay_local);
    let shown: HashSet<PathBuf> = WalkBuilder::new(dir)
        .max_depth(Some(1))
        .hidden(false)
        .parents(honour)
        .ignore(honour)
        .git_ignore(honour)
        .git_global(honour)
        .git_exclude(honour)
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

/// Which files Find in Files looks at.
#[derive(Clone, Debug, Default)]
pub struct WalkOptions {
    /// Look in subfolders too.
    pub subfolders: bool,
    /// Also files that ignore files exclude (never `.git`).
    pub include_ignored: bool,
    /// Globs a file's name or path must match, such as `*.rs`; none means
    /// every file.
    pub filters: Vec<String>,
}

/// Splits a filter field like `*.rs; *.toml` into globs.
pub fn parse_filters(text: &str) -> Vec<String> {
    text.split([';', ',', ' '])
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
        .collect()
}

/// The files under `dir` that `o` selects, sorted by name within each
/// folder. Symbolic links are not followed.
pub fn walk_files(dir: &Path, o: &WalkOptions) -> Result<impl Iterator<Item = PathBuf>, String> {
    let mut overrides = ignore::overrides::OverrideBuilder::new(dir);
    for f in &o.filters {
        overrides.add(f).map_err(|e| format!("filter {f}: {e}"))?;
    }
    let overrides = overrides.build().map_err(|e| e.to_string())?;
    // Ignore files that point off this machine (see
    // `ignore_files_stay_local`): in the folder or above it, they are all
    // turned off; in a folder inside it, that folder is left out (the
    // filter runs before the walk reads its ignore files).
    let honour = !o.include_ignored && dir.ancestors().all(ignore_files_stay_local);
    let walk = WalkBuilder::new(dir)
        .max_depth((!o.subfolders).then_some(1))
        .hidden(false)
        .parents(honour)
        .ignore(honour)
        .git_ignore(honour)
        .git_global(honour)
        .git_exclude(honour)
        .require_git(false)
        .follow_links(false)
        .overrides(overrides)
        .sort_by_file_name(|a, b| a.cmp(b))
        .filter_entry(move |e| {
            e.file_name() != ".git"
                && (!honour
                    || !e.file_type().is_some_and(|t| t.is_dir())
                    || ignore_files_stay_local(e.path()))
        })
        .build();
    Ok(walk
        .filter_map(Result::ok)
        .filter(|e| e.file_type().is_some_and(|t| t.is_file()))
        .map(|e| e.into_path()))
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

    /// A `.git` file (as a worktree has) can name a network folder, where
    /// the walk would read git's excludes and send Windows' credentials.
    /// Such a folder is left out of a walk; listed itself, its ignore
    /// files are not read.
    #[cfg(windows)]
    #[test]
    fn a_git_file_naming_a_network_folder_is_not_followed() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        assert!(ignore_files_stay_local(root));
        let evil = root.join("evil");
        std::fs::create_dir(&evil).unwrap();
        std::fs::write(evil.join("a.txt"), "x").unwrap();
        for gitdir in [r"\\host.invalid\share\x", "//host.invalid/share/x"] {
            std::fs::write(evil.join(".git"), format!("gitdir: {gitdir}\n")).unwrap();
            assert!(!ignore_files_stay_local(&evil), "{gitdir}");
        }
        // A local gitdir whose commondir is on the network.
        let real = root.join("real");
        std::fs::create_dir(&real).unwrap();
        std::fs::write(real.join("commondir"), r"\\host.invalid\share").unwrap();
        std::fs::write(evil.join(".git"), format!("gitdir: {}\n", real.display())).unwrap();
        assert!(!ignore_files_stay_local(&evil));
        std::fs::write(real.join("commondir"), "../common").unwrap();
        assert!(ignore_files_stay_local(&evil));
        std::fs::write(real.join("commondir"), r"\\host.invalid\share").unwrap();

        std::fs::write(root.join("b.txt"), "x").unwrap();
        let o = WalkOptions {
            subfolders: true,
            ..WalkOptions::default()
        };
        let names: Vec<String> = walk_files(root, &o)
            .unwrap()
            .map(|p| p.file_name().unwrap().to_string_lossy().into_owned())
            .collect();
        assert!(names.contains(&"b.txt".to_owned()), "{names:?}");
        assert!(!names.contains(&"a.txt".to_owned()), "{names:?}");
        // Opened itself, it is listed without its ignore files.
        let listed: Vec<String> = list_dir(&evil, false)
            .unwrap()
            .into_iter()
            .map(|e| e.name)
            .collect();
        assert_eq!(listed, ["a.txt"]);
    }

    #[test]
    fn walking_honours_subfolders_ignores_and_filters() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        for d in ["src/deep", "target", ".git"] {
            std::fs::create_dir_all(root.join(d)).unwrap();
        }
        for f in [
            "a.rs",
            "b.toml",
            "c.txt",
            "src/x.rs",
            "src/deep/y.rs",
            "target/out.rs",
            ".git/HEAD",
        ] {
            std::fs::write(root.join(f), "").unwrap();
        }
        std::fs::write(root.join(".gitignore"), "target/\n").unwrap();
        let rel = |o: &WalkOptions| -> Vec<String> {
            walk_files(root, o)
                .unwrap()
                .map(|p| {
                    p.strip_prefix(root)
                        .unwrap()
                        .to_string_lossy()
                        .replace('\\', "/")
                })
                .collect()
        };
        let mut o = WalkOptions {
            subfolders: true,
            ..WalkOptions::default()
        };
        assert_eq!(
            rel(&o),
            vec![
                ".gitignore",
                "a.rs",
                "b.toml",
                "c.txt",
                "src/deep/y.rs",
                "src/x.rs"
            ]
        );
        o.filters = parse_filters("*.rs; *.toml");
        assert_eq!(rel(&o), vec!["a.rs", "b.toml", "src/deep/y.rs", "src/x.rs"]);
        o.include_ignored = true;
        assert_eq!(
            rel(&o),
            vec![
                "a.rs",
                "b.toml",
                "src/deep/y.rs",
                "src/x.rs",
                "target/out.rs"
            ]
        );
        // "!" leaves files, or whole folders, out.
        o.filters = parse_filters("*.rs; !y.rs");
        assert_eq!(rel(&o), vec!["a.rs", "src/x.rs", "target/out.rs"]);
        o.filters = parse_filters("!deep; !target");
        assert_eq!(
            rel(&o),
            vec![".gitignore", "a.rs", "b.toml", "c.txt", "src/x.rs"]
        );
        o.filters = parse_filters("*.rs; *.toml");
        o.subfolders = false;
        assert_eq!(rel(&o), vec!["a.rs", "b.toml"]);
        assert!(
            walk_files(
                root,
                &WalkOptions {
                    filters: vec!["[".into()],
                    ..o
                }
            )
            .is_err()
        );
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
