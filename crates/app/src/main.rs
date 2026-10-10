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
        self.app.show(ui);
        if let Some(b) = &mut self.bench {
            let ui_ms = start.elapsed().as_secs_f64() * 1000.0;
            let cpu_ms = f64::from(frame.info().cpu_usage.unwrap_or(0.0)) * 1000.0;
            if b.record(ui_ms, cpu_ms) {
                println!("{}", b.report());
                self.bench = None;
                ui.ctx().send_viewport_cmd(egui::ViewportCommand::Close);
            }
            ui.ctx().request_repaint();
        }
    }
}

fn main() -> eframe::Result {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let bench = args.iter().any(|a| a == "--bench");
    let path = args
        .iter()
        .find(|a| !a.starts_with("--"))
        .map(PathBuf::from);

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1200.0, 800.0])
            .with_title("ochrepad"),
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
            if let Some(path) = path {
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
