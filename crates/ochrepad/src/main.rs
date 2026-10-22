// A GUI program on Windows: no console window behind it in release builds.
// (`--bench` output still reaches a pipe or file, e.g. `> bench.txt`.)
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::path::PathBuf;
use std::time::Instant;

use ochrepad::OchrepadApp;
use ochrepad::bench::Bench;
use ochrepad::instance::{self, Primary, Start};

struct Shell {
    app: OchrepadApp,
    bench: Option<Bench>,
    /// Receives the files later launches hand over (single instance).
    instance: Option<Primary>,
}

/// Opens a command-line argument: a folder in the sidebar, else a file.
fn open_arg(app: &mut OchrepadApp, path: &std::path::Path) {
    if path.is_dir() {
        app.open_folder(path);
    } else {
        app.open_path(path);
    }
}

impl eframe::App for Shell {
    fn ui(&mut self, ui: &mut egui::Ui, frame: &mut eframe::Frame) {
        let start = Instant::now();
        if let Some(b) = &mut self.bench {
            b.drive(&mut self.app);
        }
        let launches = self
            .instance
            .as_ref()
            .map(Primary::take)
            .unwrap_or_default();
        for launch in &launches {
            for path in &launch.paths {
                open_arg(&mut self.app, path);
            }
        }
        if !launches.is_empty() {
            // Bring the window forward for the launch (Windows may only
            // flash the taskbar button instead).
            let ctx = ui.ctx();
            ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(false));
            ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
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
    let launched = Instant::now();
    let args: Vec<String> = std::env::args().skip(1).collect();
    let bench = args.iter().any(|a| a == "--bench");
    let paths: Vec<PathBuf> = args
        .iter()
        .filter(|a| !a.starts_with("--"))
        .map(PathBuf::from)
        .collect();

    // Files opened while ochrepad runs (Open with, a second launch) go to
    // the running window, which owns the session. Without a configuration
    // folder, or if the hand-over fails, this runs on its own as before.
    let instance = match ochrepad_fileio::config_dir() {
        Some(dir) if !bench => match instance::start(&dir, &paths) {
            Ok(Start::Forwarded) => return Ok(()),
            Ok(Start::Primary(primary)) => Some(primary),
            Err(e) => {
                eprintln!("ochrepad: could not reach the running instance: {e}");
                None
            }
        },
        _ => None,
    };

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
            // `--bench --wrap` measures with word wrap on; `--bench --viewer`
            // opens files over 1 MB in the viewer for large files.
            app.settings.word_wrap = bench && args.iter().any(|a| a == "--wrap");
            if bench && args.iter().any(|a| a == "--viewer") {
                app.settings.viewer_threshold_mb = 1;
            }
            if let Some(dir) = ochrepad_fileio::config_dir() {
                app.set_macro_file(dir.join("macros.json"));
                if let Some(data) = ochrepad_fileio::data_dir()
                    && !bench
                {
                    app.set_storage(&dir, &data);
                }
            }
            if let Some(primary) = &instance {
                primary.set_context(&cc.egui_ctx);
            }
            for path in paths {
                if path.is_dir() {
                    app.open_folder(&path);
                    continue;
                }
                let t = Instant::now();
                app.open_path(&path);
                if bench && app.focused_viewer().is_some() {
                    println!(
                        "opened {} in the viewer in {:?}",
                        path.display(),
                        t.elapsed()
                    );
                } else if bench {
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
                bench: bench.then(|| {
                    let mut b = Bench::default();
                    b.launched = Some(launched);
                    b
                }),
                instance,
            }))
        }),
    )
}
