//! `--bench`: drives the app through scrolling and typing phases and prints
//! frame-time statistics, then closes. Used for the performance checks in
//! PLAN M3/M4/M7 against generated fixtures.

use std::time::Instant;

use ochrepad_core::{Action, ByteOffset, SelectionSet};

use crate::OchrepadApp;

const PHASES: [&str; 4] = [
    "warmup",
    "fling 60 lines/frame",
    "random jump",
    "typing at top",
];
const FRAMES: usize = 600;

pub struct Bench {
    phase: usize,
    frame: usize,
    last: Option<Instant>,
    rng: u64,
    /// Per phase: (frame interval ms, ui ms, eframe cpu ms).
    samples: Vec<Vec<(f64, f64, f64)>>,
}

impl Default for Bench {
    fn default() -> Self {
        Self {
            phase: 0,
            frame: 0,
            last: None,
            rng: 0x2545_F491_4F6C_DD1D,
            samples: vec![Vec::new(); PHASES.len()],
        }
    }
}

impl Bench {
    /// Sets up this frame's scroll position or edit.
    pub fn drive(&mut self, app: &mut OchrepadApp) {
        let lines = app.buffer().doc().line_count();
        let view = app.view;
        match self.phase {
            1 => {
                if let Some(v) = app.editor.view_mut(view) {
                    v.scroll.first_line = (v.scroll.first_line + 60) % lines.max(1);
                }
            }
            2 => {
                self.rng ^= self.rng << 13;
                self.rng ^= self.rng >> 7;
                self.rng ^= self.rng << 17;
                if let Some(v) = app.editor.view_mut(view) {
                    v.scroll.first_line = (self.rng % lines.max(1) as u64) as usize;
                }
            }
            3 => {
                if self.frame == 0 {
                    let _ = app.editor.apply(
                        view,
                        Action::SetSelections(SelectionSet::caret(ByteOffset(0))),
                    );
                }
                let _ = app.editor.apply(view, Action::InsertText("x".into()));
                app.view_state.reveal_caret();
            }
            _ => {}
        }
    }

    /// Records the frame; returns true when the benchmark is finished.
    pub fn record(&mut self, ui_ms: f64, cpu_ms: f64) -> bool {
        let now = Instant::now();
        if let Some(prev) = self.last.replace(now)
            && self.phase > 0
            && self.frame > 0
        {
            let interval = (now - prev).as_secs_f64() * 1000.0;
            self.samples[self.phase].push((interval, ui_ms, cpu_ms));
        }
        self.frame += 1;
        let limit = if self.phase == 0 { 60 } else { FRAMES };
        if self.frame >= limit {
            self.frame = 0;
            self.phase += 1;
        }
        self.phase >= PHASES.len()
    }

    pub fn report(&self) -> String {
        let mut out = format!(
            "{:<22} {:>30} {:>30} {:>30}\n{:<22} {:>30} {:>30} {:>30}\n",
            "phase",
            "frame interval ms",
            "ui ms",
            "eframe cpu ms",
            "",
            "avg/p50/p95/p99/max",
            "avg/p50/p95/p99/max",
            "avg/p50/p95/p99/max"
        );
        for (name, s) in PHASES.iter().zip(&self.samples).skip(1) {
            if s.is_empty() {
                continue;
            }
            let col = |f: fn(&(f64, f64, f64)) -> f64| {
                let mut v: Vec<f64> = s.iter().map(f).collect();
                v.sort_by(f64::total_cmp);
                let pct = |q: f64| v[((v.len() - 1) as f64 * q) as usize];
                let avg = v.iter().sum::<f64>() / v.len() as f64;
                format!(
                    "{avg:.2}/{:.2}/{:.2}/{:.2}/{:.2}",
                    pct(0.5),
                    pct(0.95),
                    pct(0.99),
                    v[v.len() - 1]
                )
            };
            out += &format!(
                "{:<22} {:>30} {:>30} {:>30}\n",
                name,
                col(|s| s.0),
                col(|s| s.1),
                col(|s| s.2)
            );
        }
        out
    }
}
