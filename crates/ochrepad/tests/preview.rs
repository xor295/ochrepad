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

/// Writes `flag.jpg`, `flag.webp` and `flag.svg` into `dir`.
fn write_other_formats(dir: &Path) {
    let img = image::RgbImage::from_fn(32, 24, |_, y| {
        if y < 12 {
            image::Rgb([40, 170, 70])
        } else {
            image::Rgb([240, 200, 40])
        }
    });
    img.save_with_format(dir.join("flag.jpg"), image::ImageFormat::Jpeg)
        .unwrap();
    img.save_with_format(dir.join("flag.webp"), image::ImageFormat::WebP)
        .unwrap();
    std::fs::write(
        dir.join("flag.svg"),
        r##"<svg xmlns="http://www.w3.org/2000/svg" width="32" height="24">
<rect width="32" height="12" fill="#8040c0"/><circle cx="16" cy="18" r="6" fill="#40c0c0"/></svg>"##,
    )
    .unwrap();
}

#[test]
fn jpeg_webp_and_svg_images_load() {
    let tmp = tempfile::tempdir().unwrap();
    write_other_formats(tmp.path());
    let file = tmp.path().join("doc.md");
    std::fs::write(&file, "![a](flag.jpg) ![b](flag.webp) ![c](flag.svg)\n").unwrap();
    let mut h = harness(OchrepadApp::new());
    h.state_mut().open_path(&file);
    h.state_mut().open_preview();
    h.run();
    let base = ochrepad::preview::base_uri(Some(&file));
    for name in ["flag.jpg", "flag.webp", "flag.svg"] {
        let uri = format!("{base}{name}");
        let mut ready = false;
        for _ in 0..500 {
            match h.ctx.try_load_image(&uri, egui::SizeHint::default()) {
                Ok(egui::load::ImagePoll::Ready { image }) => {
                    assert!(image.width() > 0 && image.height() > 0);
                    ready = true;
                    break;
                }
                Ok(egui::load::ImagePoll::Pending { .. }) => {
                    std::thread::sleep(std::time::Duration::from_millis(5));
                }
                Err(e) => panic!("{name}: {e}"),
            }
        }
        assert!(ready, "{name} did not load");
    }
}

#[test]
fn snapshot_with_a_relative_image() {
    let tmp = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(tmp.path().join("img")).unwrap();
    write_png(&tmp.path().join("img").join("flag.png"));
    write_other_formats(&tmp.path().join("img"));
    let file = tmp.path().join("doc.md");
    std::fs::write(
        &file,
        "# Title\n\nA list:\n\n- one\n- **two**\n\n![flag](img/flag.png) ![jpeg](img/flag.jpg) ![webp](img/flag.webp) ![svg](img/flag.svg)\n\n```rust\nfn main() {}\n```\n",
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

/// The URLs the last frame asked the browser to open.
fn opened_urls(h: &Harness<'_, OchrepadApp>) -> Vec<String> {
    h.output()
        .platform_output
        .commands
        .iter()
        .filter_map(|c| match c {
            egui::OutputCommand::OpenUrl(o) => Some(o.url.clone()),
            _ => None,
        })
        .collect()
}

#[test]
fn links_open_files_here_and_only_web_links_go_to_the_browser() {
    let tmp = tempfile::tempdir().unwrap();
    std::fs::write(tmp.path().join("other.md"), "# Other\n").unwrap();
    let file = tmp.path().join("doc.md");
    std::fs::write(
        &file,
        "[web](https://example.com/) and [settings](ms-settings:privacy) and [other](other.md)\n",
    )
    .unwrap();
    let mut h = harness(OchrepadApp::new());
    h.state_mut().open_path(&file);
    h.state_mut().open_preview();
    h.run();

    h.get_by_label("web").click();
    h.step();
    assert_eq!(opened_urls(&h), vec!["https://example.com/".to_owned()]);
    h.run();

    // A handler the operating system would run is refused, with a note.
    h.get_by_label("settings").click();
    h.step();
    assert!(opened_urls(&h).is_empty());
    h.run();
    h.get_by_label_contains("are not opened");

    // A relative link to a file opens it in ochrepad.
    h.get_by_label("other").click();
    h.run();
    let path = h.state().buffer().meta.path.clone().unwrap();
    assert_eq!(path.file_name().unwrap(), "other.md");
}
