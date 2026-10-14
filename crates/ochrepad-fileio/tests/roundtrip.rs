use std::path::{Path, PathBuf};

use ochrepad_core::{Action, Editor, LineEnding};
use ochrepad_fileio::{
    LoadError, SaveError, decode, default_line_ending, encode, load, normalize_line_endings,
    open_into, save, save_buffer, save_buffer_as,
};
use proptest::prelude::*;

fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(name)
}

fn read(name: &str) -> Vec<u8> {
    std::fs::read(fixture(name)).expect("fixture exists")
}

/// (file, encoding, bom, line ending, line count of decoded text)
const ROUND_TRIP: &[(&str, &str, bool, Option<LineEnding>, usize)] = &[
    ("utf8.txt", "UTF-8", false, Some(LineEnding::Lf), 4),
    ("utf8_bom.txt", "UTF-8", true, Some(LineEnding::Lf), 4),
    (
        "utf16le_bom.txt",
        "UTF-16LE",
        true,
        Some(LineEnding::CrLf),
        4,
    ),
    ("utf16be_bom.txt", "UTF-16BE", true, Some(LineEnding::Lf), 4),
    (
        "windows1252_crlf.txt",
        "windows-1252",
        false,
        Some(LineEnding::CrLf),
        3,
    ),
    ("crlf.txt", "UTF-8", false, Some(LineEnding::CrLf), 4),
    ("lf.txt", "UTF-8", false, Some(LineEnding::Lf), 4),
    ("cr.txt", "UTF-8", false, Some(LineEnding::Cr), 4),
    ("empty.txt", "UTF-8", false, None, 1),
    (
        "no_trailing_newline.txt",
        "UTF-8",
        false,
        Some(LineEnding::CrLf),
        2,
    ),
];

#[test]
fn fixtures_round_trip_byte_for_byte() {
    for &(name, encoding, bom, ending, lines) in ROUND_TRIP {
        let bytes = read(name);
        let d = decode(&bytes).unwrap_or_else(|e| panic!("{name}: {e}"));
        assert_eq!(d.meta.encoding, encoding, "{name}");
        assert_eq!(d.meta.bom, bom, "{name}");
        assert_eq!(
            d.meta.line_ending,
            ending.unwrap_or_else(default_line_ending),
            "{name}"
        );
        assert!(!d.meta.mixed_line_endings, "{name}");
        assert!(!d.text.contains('\r'), "{name}: not normalized");
        assert_eq!(d.text.split('\n').count(), lines, "{name}");
        assert_eq!(encode(&d.text, &d.meta).unwrap(), bytes, "{name}");
    }
}

#[test]
fn decoded_text_is_correct() {
    let expected = "Hello, wörld — naïve café ✓\nsecond line\tTab\n漢字 and 👨‍👩‍👧\n";
    for name in [
        "utf8.txt",
        "utf8_bom.txt",
        "utf16le_bom.txt",
        "utf16be_bom.txt",
    ] {
        assert_eq!(decode(&read(name)).unwrap().text, expected, "{name}");
    }
    let cp = decode(&read("windows1252_crlf.txt")).unwrap().text;
    assert!(cp.starts_with("Café crème") && cp.contains("“Quoted” — €5 ½"));
}

#[test]
fn round_trip_through_the_filesystem() {
    let dir = tempfile::tempdir().unwrap();
    for &(name, ..) in ROUND_TRIP {
        let d = load(&fixture(name)).unwrap();
        assert_eq!(d.meta.path.as_deref(), Some(fixture(name).as_path()));
        let out = dir.path().join(name);
        save(&out, &d.text, &d.meta).unwrap();
        assert_eq!(std::fs::read(&out).unwrap(), read(name), "{name}");
    }
}

/// Mixed endings are normalized; the file is saved with the dominant ending
/// (PLAN 3.6), so it is the one fixture that does not round-trip exactly.
#[test]
fn mixed_line_endings_are_flagged_and_saved_with_the_dominant_ending() {
    let d = decode(&read("mixed.txt")).unwrap();
    assert!(d.meta.mixed_line_endings);
    assert_eq!(d.meta.line_ending, LineEnding::CrLf);
    assert_eq!(d.text, "one\ntwo\nthree\nfour\nfive\n");
    assert_eq!(
        encode(&d.text, &d.meta).unwrap(),
        b"one\r\ntwo\r\nthree\r\nfour\r\nfive\r\n"
    );
}

#[test]
fn refuses_binary_and_undecodable_files() {
    assert!(matches!(
        decode(&read("binary.bin")),
        Err(LoadError::Binary)
    ));
    for name in ["utf16le_odd_length.txt", "utf16le_unpaired_surrogate.txt"] {
        assert!(
            matches!(
                decode(&read(name)),
                Err(LoadError::Undecodable {
                    encoding: "UTF-16LE"
                })
            ),
            "{name}"
        );
    }
    assert!(matches!(
        load(Path::new("definitely/not/here.txt")),
        Err(LoadError::Io(_))
    ));
}

#[test]
fn invalid_utf8_falls_back_to_a_detected_single_byte_encoding() {
    let d = decode(b"caf\xe9 cr\xe8me br\xfbl\xe9e\n").unwrap();
    assert_eq!(d.meta.encoding, "windows-1252");
    assert_eq!(d.text, "café crème brûlée\n");
}

#[test]
fn unencodable_text_is_an_error_and_leaves_the_file_alone() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("latin.txt");
    let original = read("windows1252_crlf.txt");
    std::fs::write(&path, &original).unwrap();

    let mut ed = Editor::new();
    let buf = open_into(&mut ed, &path).unwrap();
    let view = ed.add_view(buf).unwrap();
    ed.apply(
        view,
        Action::MoveCursor {
            motion: ochrepad_core::Motion::DocEnd,
            extend: false,
        },
    )
    .unwrap();
    ed.apply(view, Action::InsertText("漢".into())).unwrap();
    let err = save_buffer(ed.buffer_mut(buf).unwrap()).unwrap_err();
    match err {
        SaveError::Unencodable { ch, line, .. } => {
            assert_eq!(ch, '漢');
            assert_eq!(line, 3);
        }
        e => panic!("unexpected {e}"),
    }
    assert!(ed.buffer(buf).unwrap().is_dirty());
    assert_eq!(std::fs::read(&path).unwrap(), original);
}

#[test]
fn save_replaces_atomically_and_leaves_no_temp_files() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("a.txt");
    std::fs::write(&path, b"old\r\ncontent\r\n").unwrap();

    let mut ed = Editor::new();
    let buf = open_into(&mut ed, &path).unwrap();
    let view = ed.add_view(buf).unwrap();
    ed.apply(view, Action::InsertText("new ".into())).unwrap();
    assert!(ed.buffer(buf).unwrap().is_dirty());
    save_buffer(ed.buffer_mut(buf).unwrap()).unwrap();
    assert!(!ed.buffer(buf).unwrap().is_dirty());
    assert_eq!(std::fs::read(&path).unwrap(), b"new old\r\ncontent\r\n");

    let entries: Vec<_> = std::fs::read_dir(dir.path())
        .unwrap()
        .map(|e| e.unwrap().file_name())
        .collect();
    assert_eq!(entries, vec![std::ffi::OsString::from("a.txt")]);
}

#[test]
fn save_as_keeps_format_and_takes_the_new_path() {
    let dir = tempfile::tempdir().unwrap();
    let mut ed = Editor::new();
    let buf = open_into(&mut ed, &fixture("utf16le_bom.txt")).unwrap();
    let new_path = dir.path().join("copy.txt");
    save_buffer_as(ed.buffer_mut(buf).unwrap(), &new_path).unwrap();
    let b = ed.buffer(buf).unwrap();
    assert_eq!(b.meta.path.as_deref(), Some(new_path.as_path()));
    assert!(!b.is_dirty());
    assert_eq!(std::fs::read(&new_path).unwrap(), read("utf16le_bom.txt"));
}

#[test]
fn save_without_a_path_is_an_error() {
    let mut ed = Editor::new();
    let buf = ed.add_buffer("x", Default::default());
    assert!(matches!(
        save_buffer(ed.buffer_mut(buf).unwrap()),
        Err(SaveError::NoPath)
    ));
}

#[test]
fn pasted_text_is_normalized() {
    assert_eq!(normalize_line_endings("a\r\nb\rc\nd"), "a\nb\nc\nd");
    assert!(matches!(
        normalize_line_endings("plain\n"),
        std::borrow::Cow::Borrowed(_)
    ));
}

// ---------------------------------------------------------------------------

const PIECES: &[&str] = &[
    "a",
    "Z",
    " ",
    "\t",
    "é",
    "€",
    "漢",
    "👨‍👩‍👧",
    "\u{301}",
    "\n",
    "\n",
];

fn file_bytes() -> impl Strategy<Value = Vec<u8>> {
    let text = prop::collection::vec(prop::sample::select(PIECES), 0..80).prop_map(|v| v.concat());
    let ending = prop::sample::select(vec!["\n", "\r\n", "\r"]);
    let format = 0..4usize; // utf-8, utf-8 bom, utf-16le bom, utf-16be bom
    (text, ending, format).prop_map(|(text, ending, format)| {
        let text = text.replace('\n', ending);
        match format {
            0 => text.into_bytes(),
            1 => [b"\xEF\xBB\xBF".as_slice(), text.as_bytes()].concat(),
            2 => std::iter::once(0xFEFFu16)
                .chain(text.encode_utf16())
                .flat_map(u16::to_le_bytes)
                .collect(),
            _ => std::iter::once(0xFEFFu16)
                .chain(text.encode_utf16())
                .flat_map(u16::to_be_bytes)
                .collect(),
        }
    })
}

proptest! {
    #![proptest_config(ProptestConfig { cases: 2000, ..ProptestConfig::default() })]

    /// Any file with uniform line endings survives decode + encode unchanged.
    #[test]
    fn decode_then_encode_is_identity(bytes in file_bytes()) {
        let d = decode(&bytes).map_err(|e| TestCaseError::fail(e.to_string()))?;
        prop_assert!(!d.meta.mixed_line_endings);
        prop_assert_eq!(encode(&d.text, &d.meta).unwrap(), bytes);
    }
}

#[test]
fn json_settings_round_trip_atomically() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("nested").join("macros.json");
    assert_eq!(ochrepad_fileio::load_json::<Vec<String>>(&path), Ok(None));
    let value = vec!["a".to_owned(), "b".to_owned()];
    ochrepad_fileio::save_json(&path, &value).unwrap();
    assert_eq!(ochrepad_fileio::load_json(&path), Ok(Some(value)));
    std::fs::write(&path, "not json").unwrap();
    assert!(ochrepad_fileio::load_json::<Vec<String>>(&path).is_err());
}

#[test]
fn reopening_with_an_encoding_decodes_strictly() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("a.txt");
    // "café" in Windows-1252; as UTF-8 it would be invalid.
    std::fs::write(&path, b"caf\xe9\r\n").unwrap();
    let d = ochrepad_fileio::load_as(&path, "windows-1252").unwrap();
    assert_eq!(d.text, "café\n");
    assert_eq!(d.meta.encoding, "windows-1252");
    assert!(ochrepad_fileio::load_as(&path, "UTF-8").is_err());
    // Every listed name is exactly an encoding_rs encoding's name.
    for (name, _) in ochrepad_fileio::ENCODINGS {
        let e = encoding_rs::Encoding::for_label(name.as_bytes());
        assert_eq!(e.map(|e| e.name()), Some(*name));
    }
}
