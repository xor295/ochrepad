//! M11: Markdown preview tabs.

use std::path::Path;

use egui::{Event, Key, Modifiers, Vec2};
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use ochrepad::OchrepadApp;
use ochrepad::app::Tab;

fn harness(app: OchrepadApp) -> Harness<'static, OchrepadApp> {
    let mut h = Harness::builder()
        .with_size(Vec2::new(1000.0, 600.0))
        .with_step_dt(1.0 / 60.0)
        .with_max_steps(10_000)
        .build_ui_state(|ui, app: &mut OchrepadApp| app.show(ui), app);
    h.run();
    h
}

fn typ(h: &mut Harness<'_, OchrepadApp>, s: &str) {
    h.event(Event::Text(s.to_owned()));
    h.run();
}

fn key(h: &mut Harness<'_, OchrepadApp>, m: Modifiers, k: Key) {
    h.key_press_modifiers(m, k);
    h.run();
}

fn rendered(h: &Harness<'_, OchrepadApp>) -> Option<String> {
    let app = h.state();
    app.previews
        .rendered_text(app.buffer_id())
        .map(str::to_owned)
}

fn preview_tabs(h: &Harness<'_, OchrepadApp>) -> usize {
    h.state()
        .dock
        .iter_all_tabs()
        .filter(|(_, t)| matches!(t, Tab::Preview(_)))
        .count()
}

#[test]
fn ctrl_shift_m_opens_a_preview_beside_the_editor() {
    let mut h = harness(OchrepadApp::new());
    h.event(Event::Paste("# Hello preview\n\nSome *text*.".into()));
    h.run();
    key(&mut h, Modifiers::COMMAND | Modifiers::SHIFT, Key::M);
    assert_eq!(preview_tabs(&h), 1);
    assert_eq!(
        h.state().dock.iter_leaves().count(),
        2,
        "split beside the editor"
    );
    h.get_by_label("Preview: new 1");
    h.get_by_label("Hello preview");
    assert_eq!(
        rendered(&h).as_deref(),
        Some("# Hello preview\n\nSome *text*.")
    );
    // The editor keeps the keyboard.
    typ(&mut h, "!");
    assert!(h.state().buffer().doc().to_string().ends_with("text*.!"));
    // Asking again does not open a second preview.
    key(&mut h, Modifiers::COMMAND | Modifiers::SHIFT, Key::M);
    assert_eq!(preview_tabs(&h), 1);
}

#[test]
fn the_preview_waits_for_typing_to_pause() {
    let mut h = harness(OchrepadApp::new());
    h.event(Event::Paste("one".into()));
    h.run();
    h.state_mut().open_preview();
    h.run();
    assert_eq!(rendered(&h).as_deref(), Some("one"));
    typ(&mut h, " two");
    assert_eq!(rendered(&h).as_deref(), Some("one"), "not yet");
    h.run_steps(20); // > 200 ms at 60 frames per second
    assert_eq!(rendered(&h).as_deref(), Some("one two"));
}

#[test]
fn closing_the_buffer_closes_its_preview() {
    let mut h = harness(OchrepadApp::new());
    typ(&mut h, "x");
    h.state_mut().open_preview();
    h.run();
    assert_eq!(preview_tabs(&h), 1);
    key(&mut h, Modifiers::COMMAND, Key::W);
    h.get_by_label("Don't Save").click();
    h.run();
    assert_eq!(preview_tabs(&h), 0);
}

#[test]
fn previews_are_not_part_of_the_session() {
    let tmp = tempfile::tempdir().unwrap();
    let (config, data) = (tmp.path().join("config"), tmp.path().join("data"));
    let file = tmp.path().join("notes.md");
    std::fs::write(&file, "# Notes").unwrap();
    {
        let mut app = OchrepadApp::new();
        app.set_storage(&config, &data);
        let mut h = harness(app);
        h.state_mut().open_path(&file);
        h.state_mut().open_preview();
        h.run();
        h.state_mut().request_exit();
        h.run();
    }
    let mut app = OchrepadApp::new();
    app.set_storage(&config, &data);
    let h = harness(app);
    assert_eq!(preview_tabs(&h), 0);
    assert_eq!(
        h.state().dock.iter_leaves().count(),
        1,
        "no empty pane left over"
    );
    assert_eq!(h.state().file_name(), "notes.md");
}

/// A 24x24 PNG: red top half, blue bottom half (uncompressed deflate).
fn write_png(path: &Path) {
    fn crc32(data: &[u8]) -> u32 {
        let mut c = 0xffff_ffffu32;
        for &b in data {
            c ^= b as u32;
            for _ in 0..8 {
                c = if c & 1 != 0 {
                    0xedb8_8320 ^ (c >> 1)
                } else {
                    c >> 1
                };
            }
        }
        !c
    }
    fn chunk(out: &mut Vec<u8>, kind: &[u8], data: &[u8]) {
        out.extend((data.len() as u32).to_be_bytes());
        let body: Vec<u8> = kind.iter().chain(data).copied().collect();
        out.extend(&body);
        out.extend(crc32(&body).to_be_bytes());
    }
    let (w, h) = (24u32, 24u32);
    let mut raw = Vec::new();
    for y in 0..h {
        raw.push(0);
        for _ in 0..w {
            raw.extend(if y < h / 2 {
                [220, 40, 40]
            } else {
                [40, 80, 220]
            });
        }
    }
    let mut z = vec![0x78, 0x01, 1];
    z.extend((raw.len() as u16).to_le_bytes());
    z.extend((!(raw.len() as u16)).to_le_bytes());
    z.extend(&raw);
    let (mut a, mut b) = (1u32, 0u32);
    for &x in &raw {
        a = (a + x as u32) % 65521;
        b = (b + a) % 65521;
    }
    z.extend(((b << 16) | a).to_be_bytes());
    let mut ihdr = Vec::new();
    ihdr.extend(w.to_be_bytes());
    ihdr.extend(h.to_be_bytes());
    ihdr.extend([8, 2, 0, 0, 0]);
    let mut out = b"\x89PNG\r\n\x1a\n".to_vec();
    chunk(&mut out, b"IHDR", &ihdr);
    chunk(&mut out, b"IDAT", &z);
    chunk(&mut out, b"IEND", &[]);
    std::fs::write(path, out).unwrap();
}

#[test]
fn snapshot_with_a_relative_image() {
    let tmp = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(tmp.path().join("img")).unwrap();
    write_png(&tmp.path().join("img").join("flag.png"));
    let file = tmp.path().join("doc.md");
    std::fs::write(
        &file,
        "# Title\n\nA list:\n\n- one\n- **two**\n\n![flag](img/flag.png)\n\n```rust\nfn main() {}\n```\n",
    )
    .unwrap();
    let mut h = Harness::builder()
        .with_size(Vec2::new(760.0, 420.0))
        .with_step_dt(1.0 / 60.0)
        .with_max_steps(10_000)
        .with_wait_for_pending_images(true)
        .wgpu()
        .build_ui_state(|ui, app: &mut OchrepadApp| app.show(ui), OchrepadApp::new());
    h.state_mut().open_path(&file);
    h.state_mut().open_preview();
    h.run();
    h.snapshot("markdown_preview");
}
