use ochrepad_core::{Document, FileMeta, LineEnding};
use ochrepad_fileio::worker::{Job, SnapshotInfo, Worker, read_snapshot, snapshot_paths};

#[test]
fn saves_snapshots_and_deletes_in_the_background() {
    let dir = tempfile::tempdir().unwrap();
    let mut w = Worker::spawn();
    let meta = FileMeta {
        line_ending: LineEnding::CrLf,
        ..FileMeta::default()
    };
    let file = dir.path().join("a.txt");
    let save = w.submit(Job::Save {
        path: file.clone(),
        doc: Document::new("one\ntwo"),
        meta: meta.clone(),
    });
    let info = SnapshotInfo {
        meta,
        untitled: Some("new 3".into()),
        language: Some("Rust".into()),
    };
    let backups = dir.path().join("backup");
    let snap = w.submit(Job::Snapshot {
        dir: backups.clone(),
        id: "b1".into(),
        doc: Document::new("unsaved ✓"),
        info: info.clone(),
    });
    let done = w.wait_idle();
    assert_eq!(w.in_flight(), 0);
    assert!(done.iter().all(|d| d.result.is_ok()), "{done:?}");
    let mut tags: Vec<u64> = done.iter().map(|d| d.tag).collect();
    tags.sort();
    assert_eq!(tags, vec![save, snap]);

    assert_eq!(std::fs::read(&file).unwrap(), b"one\r\ntwo");
    assert_eq!(
        read_snapshot(&backups, "b1").unwrap(),
        ("unsaved ✓".to_owned(), info)
    );

    let (txt, json) = snapshot_paths(&backups, "b1");
    w.submit(Job::Delete(vec![
        txt.clone(),
        json.clone(),
        backups.join("missing"),
    ]));
    assert!(w.wait_idle()[0].result.is_ok());
    assert!(!txt.exists() && !json.exists());
}

#[test]
fn snapshot_names_from_a_session_cannot_leave_the_backup_folder() {
    let dir = tempfile::tempdir().unwrap();
    let backups = dir.path().join("backup");
    std::fs::create_dir_all(&backups).unwrap();
    // A snapshot-shaped pair outside the folder.
    std::fs::write(dir.path().join("secret.txt"), "secret").unwrap();
    std::fs::write(
        dir.path().join("secret.json"),
        serde_json::to_string(&SnapshotInfo {
            meta: FileMeta::default(),
            untitled: None,
            language: None,
        })
        .unwrap(),
    )
    .unwrap();
    for id in ["../secret", "..\\secret", "/etc/x", "C:x", "", "a b"] {
        assert!(read_snapshot(&backups, id).is_err(), "{id}");
    }
    assert!(ochrepad_fileio::worker::is_snapshot_id("18f2a3b4c5d-12"));
}

#[test]
fn failures_come_back_as_results() {
    let dir = tempfile::tempdir().unwrap();
    let mut w = Worker::spawn();
    let tag = w.submit(Job::Save {
        path: dir.path().join("latin.txt"),
        doc: Document::new("漢"),
        meta: FileMeta {
            encoding: "windows-1252".into(),
            ..FileMeta::default()
        },
    });
    let done = w.wait_idle();
    assert_eq!(done[0].tag, tag);
    assert!(
        done[0]
            .result
            .as_ref()
            .unwrap_err()
            .contains("windows-1252")
    );
}

#[test]
fn dropping_the_worker_finishes_queued_writes() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("late.txt");
    {
        let mut w = Worker::spawn();
        for i in 0..20 {
            w.submit(Job::Write {
                path: path.clone(),
                bytes: format!("version {i}").into_bytes(),
            });
        }
    }
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "version 19");
}

#[test]
fn creating_and_renaming_never_overwrite() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path();
    let mut w = Worker::spawn();
    let a = dir.join("a.txt");
    std::fs::write(&a, "keep").unwrap();
    w.submit(Job::CreateFile(dir.join("new.txt")));
    w.submit(Job::CreateFile(a.clone()));
    w.submit(Job::CreateDir(dir.join("sub")));
    w.submit(Job::Rename {
        from: dir.join("new.txt"),
        to: a.clone(),
    });
    w.submit(Job::Rename {
        from: dir.join("new.txt"),
        to: dir.join("sub").join("moved.txt"),
    });
    w.submit(Job::Rename {
        from: a.clone(),
        to: dir.join("A.txt"),
    });
    let results: Vec<bool> = w.wait_idle().iter().map(|d| d.result.is_ok()).collect();
    assert_eq!(results, vec![true, false, true, false, true, true]);
    assert!(dir.join("sub").join("moved.txt").is_file());
    assert_eq!(std::fs::read_to_string(dir.join("A.txt")).unwrap(), "keep");
}
