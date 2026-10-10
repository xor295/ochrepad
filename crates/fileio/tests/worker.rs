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
