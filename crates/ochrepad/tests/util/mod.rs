//! Shared by the snapshot tests.

use egui_kittest::Harness;
use ochrepad::OchrepadApp;
use ochrepad_core::LineEnding;

/// Takes a snapshot after giving every buffer CR LF line endings. New tabs
/// use the operating system's ending, which the status bar shows, so without
/// this the images would differ between Windows and Linux/macOS.
pub fn snapshot(h: &mut Harness<'_, OchrepadApp>, name: &str) {
    for b in h.state_mut().editor.buffers_mut() {
        b.meta.line_ending = LineEnding::CrLf;
    }
    h.run();
    h.snapshot(name);
}
