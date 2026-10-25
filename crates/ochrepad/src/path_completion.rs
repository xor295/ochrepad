//! File and folder names for paths typed in strings (PLAN_completion C4).
//! Folders are listed on a thread, only after `safe_images::is_local_path`
//! says the folder is on this machine (on Windows, merely listing
//! `\\host\share` sends the user's credentials there), and a listing is
//! kept for a few seconds.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::mpsc::{Receiver, Sender, channel};

/// How long a listing is used before the folder is listed again.
const FRESH_SECS: f64 = 5.0;
/// At most this many names are read from a folder.
const MAX_ENTRIES: usize = 5000;
/// Listings kept (the oldest go first).
const MAX_LISTINGS: usize = 64;

/// A name in a folder.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entry {
    pub name: String,
    /// A folder (not a link to one: links are not followed).
    pub dir: bool,
}

struct Listing {
    /// `None`: not a local folder, or it could not be read.
    entries: Option<Arc<Vec<Entry>>>,
    at: f64,
}

type Listed = (PathBuf, Option<Vec<Entry>>);

/// Folder listings for path completion, made on a thread.
#[derive(Default)]
pub struct PathLister {
    jobs: Option<Sender<PathBuf>>,
    done: Option<Receiver<Listed>>,
    listings: HashMap<PathBuf, Listing>,
    pending: HashSet<PathBuf>,
    /// Counts finished listings (the popup asks again when it changes).
    pub answered: u64,
}

impl PathLister {
    /// Takes the listings the thread finished.
    pub fn poll(&mut self, now: f64) {
        let Some(rx) = &self.done else {
            return;
        };
        while let Ok((dir, entries)) = rx.try_recv() {
            self.pending.remove(&dir);
            self.listings.insert(
                dir,
                Listing {
                    entries: entries.map(Arc::new),
                    at: now,
                },
            );
            self.answered += 1;
        }
        if self.listings.len() > MAX_LISTINGS
            && let Some(oldest) = self
                .listings
                .iter()
                .min_by(|a, b| a.1.at.total_cmp(&b.1.at))
                .map(|(k, _)| k.clone())
        {
            self.listings.remove(&oldest);
        }
    }

    /// The names in `dir` as last listed (`None` before the first listing,
    /// or when it is not a local folder); lists it again when the listing
    /// is old.
    pub fn entries(
        &mut self,
        dir: &Path,
        now: f64,
        ctx: &egui::Context,
    ) -> Option<Arc<Vec<Entry>>> {
        let old = self
            .listings
            .get(dir)
            .is_none_or(|l| now - l.at > FRESH_SECS);
        if old && !self.pending.contains(dir) {
            self.request(dir, ctx);
        }
        self.listings.get(dir).and_then(|l| l.entries.clone())
    }

    /// Whether a listing is still being made.
    pub fn waiting(&self) -> bool {
        !self.pending.is_empty()
    }

    fn request(&mut self, dir: &Path, ctx: &egui::Context) {
        if self.jobs.is_none() {
            let (jobs, job_rx) = channel::<PathBuf>();
            let (done_tx, done) = channel();
            let ctx = ctx.clone();
            let spawned = std::thread::Builder::new()
                .name("ochrepad-path-completion".into())
                .spawn(move || {
                    while let Ok(dir) = job_rx.recv() {
                        let entries = list(&dir);
                        if done_tx.send((dir, entries)).is_err() {
                            return;
                        }
                        ctx.request_repaint();
                    }
                });
            if spawned.is_err() {
                return;
            }
            self.jobs = Some(jobs);
            self.done = Some(done);
        }
        if let Some(jobs) = &self.jobs
            && jobs.send(dir.to_path_buf()).is_ok()
        {
            self.pending.insert(dir.to_path_buf());
        }
    }
}

/// The names in `dir`, if it is a local folder (on the thread).
fn list(dir: &Path) -> Option<Vec<Entry>> {
    if !crate::safe_images::is_local_path(dir) {
        return None;
    }
    let mut out = Vec::new();
    for e in std::fs::read_dir(dir).ok()?.flatten().take(MAX_ENTRIES) {
        // The entry's own type: a link is not followed.
        let Ok(kind) = e.file_type() else {
            continue;
        };
        out.push(Entry {
            name: e.file_name().to_string_lossy().into_owned(),
            dir: kind.is_dir(),
        });
    }
    Some(out)
}

/// The folder a path typed in a document names: absolute as it is,
/// relative to `base` (the document's folder) otherwise. `~` and (on
/// Windows) a root without a drive are not followed.
pub fn resolve(typed: &str, base: Option<&Path>) -> Option<PathBuf> {
    if typed.starts_with('~') {
        return None;
    }
    let path = Path::new(typed);
    if path.is_absolute() {
        return Some(path.to_path_buf());
    }
    if path.has_root() {
        return None;
    }
    Some(base?.join(path))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn typed_paths_are_resolved_against_the_document() {
        let base = std::env::temp_dir();
        assert_eq!(resolve("./src/", Some(&base)), Some(base.join("./src/")));
        assert_eq!(resolve("../", Some(&base)), Some(base.join("../")));
        assert_eq!(resolve("src/", None), None, "nothing to be relative to");
        assert_eq!(resolve("~/notes/", Some(&base)), None);
        if cfg!(windows) {
            assert_eq!(
                resolve("C:\\Users\\", None),
                Some(PathBuf::from("C:\\Users\\"))
            );
            assert_eq!(resolve("/etc/", Some(&base)), None, "no drive");
        } else {
            assert_eq!(resolve("/etc/", None), Some(PathBuf::from("/etc/")));
        }
    }

    #[test]
    fn listing_a_folder() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir(dir.path().join("src")).unwrap();
        std::fs::write(dir.path().join("main.rs"), "").unwrap();
        let mut found = list(dir.path()).unwrap();
        found.sort_by(|a, b| a.name.cmp(&b.name));
        assert_eq!(
            found,
            [
                Entry {
                    name: "main.rs".into(),
                    dir: false
                },
                Entry {
                    name: "src".into(),
                    dir: true
                }
            ]
        );
        assert!(list(&dir.path().join("missing")).is_none());
        if cfg!(windows) {
            assert!(
                list(Path::new(r"\\localhost\c$")).is_none(),
                "a network path"
            );
        }
    }
}
