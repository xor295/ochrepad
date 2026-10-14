//! `--bench`: drives the app through scrolling and typing phases and prints
//! frame-time statistics, then closes. Used for the performance checks in
//! PLAN M3/M4/M7 against generated fixtures. A file that opens in the viewer
//! for large files (PLAN_v0.3.0 P7) is measured there instead: the time to
//! the first frame, to the end of indexing, and scrolling.

use std::time::{Duration, Instant};

use ochrepad_core::{Action, ByteOffset, SelectionSet};

use crate::OchrepadApp;

const PHASES: [&str; 5] = [
    "highlight warm-up",
    "fling 60 lines/frame",
    "random jump",
    "typing, 1 key/6 frames",
    "typing, 1 key/frame",
];
const FRAMES: usize = 600;

pub struct Bench {
    /// When the program started, for the time to the first frame.
    pub launched: Option<Instant>,
    first_frame: Option<Duration>,
    /// The file is in a viewer: no highlighting or typing phases.
    viewer: bool,
    started: Option<Instant>,
    /// How long the initial highlighting pass took.
    pub highlight_done: Option<std::time::Duration>,
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
            launched: None,
            first_frame: None,
            viewer: false,
            started: None,
            highlight_done: None,
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
        let started = *self.started.get_or_insert_with(Instant::now);
        if let Some(id) = app.focused_viewer()
            && let Some(v) = app.viewers.get_mut(id)
        {
            self.viewer = true;
            let progress = v.file.progress();
            if self.phase == 0 && progress.done && self.frame > 0 {
                self.highlight_done.get_or_insert(started.elapsed());
                self.frame = usize::MAX - 1;
            }
            let lines = progress.lines.max(1);
            match self.phase {
                1 => {
                    let top = v.state.top_line();
                    v.state.scroll_to_line((top + 60) % lines);
                }
                2 => {
                    self.rng ^= self.rng << 13;
                    self.rng ^= self.rng >> 7;
                    self.rng ^= self.rng << 17;
                    v.state.scroll_to_line(self.rng % lines);
                }
                _ => {}
            }
            return;
        }
        if self.phase == 0 && !app.highlighting.is_busy() && self.frame > 0 {
            self.highlight_done.get_or_insert(started.elapsed());
            // End the warm-up phase on the next `record`.
            self.frame = usize::MAX - 1;
        }
        let lines = app.buffer().doc().line_count();
        let view = app.active_view();
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
            3 | 4 => {
                // Type at the top of the file: about 10 keys/s at 60 fps,
                // then a stress run with a key every frame.
                if self.frame == 0 {
                    let _ = app.editor.apply(
                        view,
                        Action::SetSelections(SelectionSet::caret(ByteOffset(0))),
                    );
                }
                if self.phase == 4 || self.frame.is_multiple_of(6) {
                    let _ = app.editor.apply(view, Action::InsertText("x".into()));
                    app.reveal_caret();
                }
            }
            _ => {}
        }
    }

    /// Records the frame; returns true when the benchmark is finished.
    pub fn record(&mut self, ui_ms: f64, cpu_ms: f64) -> bool {
        let now = Instant::now();
        if self.first_frame.is_none() {
            self.first_frame = self.launched.map(|l| l.elapsed());
        }
        if let Some(prev) = self.last.replace(now)
            && self.phase > 0
            && self.frame > 0
        {
            let interval = (now - prev).as_secs_f64() * 1000.0;
            self.samples[self.phase].push((interval, ui_ms, cpu_ms));
        }
        self.frame += 1;
        // Warm-up lasts until highlighting is idle (or 20,000 frames).
        let limit = if self.phase == 0 { 20_000 } else { FRAMES };
        if self.frame >= limit {
            self.frame = 0;
            self.phase += 1;
        }
        // A viewer has no typing phases.
        self.phase >= if self.viewer { 3 } else { PHASES.len() }
    }

    pub fn report(&self) -> String {
        let mut out = String::new();
        if let Some(d) = self.first_frame {
            out += &format!("first frame {d:?} after launch\n");
        }
        let what = if self.viewer {
            "indexing"
        } else {
            "initial highlighting"
        };
        out += &match self.highlight_done {
            Some(d) => format!("{what} finished after {d:?}\n"),
            None => format!("{what} did not finish during warm-up\n"),
        };
        out += &format!(
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
        for (i, (name, s)) in PHASES.iter().zip(&self.samples).enumerate().skip(1) {
            if self.viewer && i > 2 {
                break;
            }
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
