// A GUI program on Windows: no console window behind it in release builds.
// (`--bench` output still reaches a pipe or file, e.g. `> bench.txt`.)
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::path::PathBuf;
use std::time::Instant;

use ochrepad::OchrepadApp;
use ochrepad::bench::Bench;

struct Shell {
    app: OchrepadApp,
    bench: Option<Bench>,
}

impl eframe::App for Shell {
    fn ui(&mut self, ui: &mut egui::Ui, frame: &mut eframe::Frame) {
        let start = Instant::now();
        if let Some(b) = &mut self.bench {
            b.drive(&mut self.app);
        }
        let shown = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| self.app.show(ui)));
        if let Err(panic) = shown {
            // A bug: keep the unsaved work (as on exit), say so, then end as
            // the panic would have.
            let kept = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                self.app.emergency_backup()
            }))
            .unwrap_or(false);
            let what = panic
                .downcast_ref::<&str>()
                .map(|s| (*s).to_owned())
                .or_else(|| panic.downcast_ref::<String>().cloned())
                .unwrap_or_default();
            let after = if kept {
                "Unsaved work was backed up and will come back when you start ochrepad again."
            } else {
                "Backups are turned off, so unsaved work could not be kept."
            };
            rfd::MessageDialog::new()
                .set_level(rfd::MessageLevel::Error)
                .set_title("ochrepad stopped")
                .set_description(format!(
                    "ochrepad hit a bug and has to close.\n\n{after}\n\n{what}"
                ))
                .show();
            std::panic::resume_unwind(panic);
        }
        if let Some(b) = &mut self.bench {
            let ui_ms = start.elapsed().as_secs_f64() * 1000.0;
            let cpu_ms = f64::from(frame.info().cpu_usage.unwrap_or(0.0)) * 1000.0;
            if b.record(ui_ms, cpu_ms) {
                if let Some(problem) = self.app.highlighting.problem(self.app.buffer_id()) {
                    println!("{problem}");
                }
                println!("{}", b.report());
                self.bench = None;
                // The typing phases left the file unsaved: close without
                // the save prompt (which would wait forever).
                for buffer in self.app.editor.buffers_mut() {
                    buffer.mark_saved();
                }
                ui.ctx().send_viewport_cmd(egui::ViewportCommand::Close);
            }
            ui.ctx().request_repaint();
        }
    }
}

fn main() -> eframe::Result {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let bench = args.iter().any(|a| a == "--bench");
    let paths: Vec<PathBuf> = args
        .iter()
        .filter(|a| !a.starts_with("--"))
        .map(PathBuf::from)
        .collect();

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1200.0, 800.0])
            .with_title("ochrepad")
            .with_icon(ochrepad::theme::icon(64)),
        wgpu_options: eframe::WgpuConfiguration {
            surface: eframe::SurfaceConfig {
                present_mode: if bench {
                    eframe::wgpu::PresentMode::AutoNoVsync
                } else {
                    eframe::wgpu::PresentMode::AutoVsync
                },
                desired_maximum_frame_latency: Some(1),
            },
            ..Default::default()
        },
        ..Default::default()
    };
    eframe::run_native(
        "ochrepad",
        options,
        Box::new(move |cc| {
            if ochrepad::fonts::install(&cc.egui_ctx).is_none() {
                eprintln!("ochrepad: no system CJK font found; CJK text will show as boxes");
            }
            let mut app = OchrepadApp::new();
            // `--bench --wrap` measures with word wrap on.
            app.settings.word_wrap = bench && args.iter().any(|a| a == "--wrap");
            if let Some(dir) = ochrepad_fileio::config_dir() {
                app.set_macro_file(dir.join("macros.json"));
                if let Some(data) = ochrepad_fileio::data_dir()
                    && !bench
                {
                    app.set_storage(&dir, &data);
                }
            }
            for path in paths {
                if path.is_dir() {
                    app.open_folder(&path);
                    continue;
                }
                let t = Instant::now();
                app.open_path(&path);
                if bench {
                    println!(
                        "opened {} ({} lines) in {:?}",
                        path.display(),
                        app.buffer().doc().line_count(),
                        t.elapsed()
                    );
                }
            }
            Ok(Box::new(Shell {
                app,
                bench: bench.then(Bench::default),
            }))
        }),
    )
}
