//! The open folder's index for completion: what it finds, its cache, and
//! keeping up with changes.

use std::path::Path;
use std::sync::{Arc, LazyLock};
use std::time::{Duration, Instant};

use ochrepad::folder_index::{FolderIndex, Progress, Snapshot};
use ochrepad_syntax::{Registry, SymbolKind};

static REG: LazyLock<Arc<Registry>> = LazyLock::new(|| Arc::new(Registry::load()));

fn write(path: &Path, text: &str) {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).unwrap();
    }
    std::fs::write(path, text).unwrap();
}

/// Polls until `done` holds for the snapshot, for at most 20 seconds.
fn wait(index: &mut FolderIndex, done: impl Fn(&Snapshot, Progress) -> bool) -> Arc<Snapshot> {
    let start = Instant::now();
    loop {
        index.poll();
        if let Some(s) = &index.snapshot
            && done(s, index.progress)
        {
            return s.clone();
        }
        assert!(start.elapsed() < Duration::from_secs(20), "timed out");
        std::thread::sleep(Duration::from_millis(20));
    }
}

fn ready(_: &Snapshot, p: Progress) -> bool {
    p == Progress::Ready
}

fn open(root: &Path, cache: Option<std::path::PathBuf>) -> FolderIndex {
    FolderIndex::open(
        root.to_path_buf(),
        cache,
        REG.clone(),
        egui::Context::default(),
    )
}

fn folder() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    write(
        &dir.path().join("src").join("lib.rs"),
        "pub fn compute_total(items: &[u32]) -> u32 { items.iter().sum() }\npub struct Basket;\n",
    );
    write(&dir.path().join("notes.txt"), "remember the milkshake\n");
    write(&dir.path().join(".gitignore"), "target/\n");
    write(
        &dir.path().join("target").join("gen.rs"),
        "fn ignored_generated() {}\n",
    );
    write(&dir.path().join("big.txt"), &"hugeword ".repeat(300_000));
    dir
}

#[test]
fn words_and_definitions_of_the_folder() {
    let dir = folder();
    let mut index = open(dir.path(), None);
    let s = wait(&mut index, ready);
    assert!(s.words.count("milkshake") > 0);
    assert!(s.words.count("compute_total") > 0);
    assert_eq!(
        s.words.count("ignored_generated"),
        0,
        "ignored by .gitignore"
    );
    assert_eq!(s.words.count("hugeword"), 0, "over the size limit");
    assert!(s.members.count("iter") > 0, "after a member separator");
    assert_eq!(s.members.count("compute_total"), 0);
    let d = &s.definitions["compute_total"];
    assert_eq!(d.kind, SymbolKind::Function);
    assert_eq!(d.file, "src/lib.rs");
    assert_eq!(s.definitions["Basket"].kind, SymbolKind::Type);
}

#[test]
fn the_cache_comes_back_and_changed_files_are_read_again() {
    let dir = folder();
    let data = tempfile::tempdir().unwrap();
    let cache = data.path().join("index.json");
    {
        let mut index = open(dir.path(), Some(cache.clone()));
        wait(&mut index, ready);
        // The app writes the cache through its worker; here, directly.
        let start = Instant::now();
        let (path, bytes) = loop {
            index.poll();
            if let Some(c) = index.take_cache() {
                break c;
            }
            assert!(start.elapsed() < Duration::from_secs(20));
            std::thread::sleep(Duration::from_millis(20));
        };
        assert_eq!(path, cache);
        std::fs::write(&path, bytes).unwrap();
    }
    // A file changed while the folder was closed.
    std::thread::sleep(Duration::from_millis(50));
    write(&dir.path().join("notes.txt"), "remember the smoothie\n");
    let mut index = open(dir.path(), Some(cache));
    let s = wait(&mut index, ready);
    assert!(s.words.count("compute_total") > 0, "from the cache");
    assert!(s.words.count("smoothie") > 0, "read again");
    assert_eq!(s.words.count("milkshake"), 0);
}

#[test]
fn saves_and_rescans_keep_it_up_to_date() {
    let dir = folder();
    let mut index = open(dir.path(), None);
    wait(&mut index, ready);
    let saved = dir.path().join("src").join("lib.rs");
    write(&saved, "pub fn renamed_function() {}\n");
    index.saved(&saved);
    let s = wait(&mut index, |s, _| {
        s.definitions.contains_key("renamed_function")
    });
    assert!(!s.definitions.contains_key("compute_total"));
    // A file another program added, and one it removed.
    write(&dir.path().join("added.md"), "freshly added words\n");
    std::fs::remove_file(dir.path().join("notes.txt")).unwrap();
    index.rescan();
    let s = wait(&mut index, |s, _| s.words.count("freshly") > 0);
    assert_eq!(s.words.count("milkshake"), 0);
}

#[test]
fn a_damaged_cache_is_ignored() {
    let dir = folder();
    let data = tempfile::tempdir().unwrap();
    for junk in [
        "not json",
        r#"{"version":2,"words":["a"],"files":[{"path":"notes.txt","size":1,"secs":0,"nanos":0,"words":[99,1],"members":[],"definitions":null}]}"#,
        r#"{"version":2,"words":["a"],"files":[{"path":"notes.txt","size":1,"secs":0,"nanos":0,"words":[0,1],"members":[0],"definitions":null}]}"#,
        r#"{"version":2,"words":[],"files":[{"path":"../../outside","size":1,"secs":0,"nanos":0,"words":[0],"members":[],"definitions":null}]}"#,
    ] {
        let cache = data.path().join("index.json");
        std::fs::write(&cache, junk).unwrap();
        let mut index = open(dir.path(), Some(cache));
        let s = wait(&mut index, ready);
        assert!(s.words.count("milkshake") > 0);
    }
}
