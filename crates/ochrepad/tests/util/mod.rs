//! Shared by the tests that render: one wgpu device per test binary, and
//! snapshots with fixed line endings.

use std::sync::OnceLock;

use eframe::egui_wgpu::{self, WgpuSetup, WgpuSetupExisting, wgpu};
use egui_kittest::wgpu::{WgpuTestRenderer, create_render_state, default_wgpu_setup};
use egui_kittest::{Harness, LazyRenderer};
use ochrepad::OchrepadApp;
use ochrepad_core::LineEnding;

/// A renderer on the binary's shared device, created on the first render
/// (so a harness that never renders costs nothing). Use it in place of
/// `HarnessBuilder::wgpu()` and of the default renderer, both of which
/// create a wgpu instance and device per harness: creating and dropping
/// many of them at once on parallel test threads crashed the test binaries
/// now and then on CI (SIGSEGV on Mesa's lavapipe, access violations on
/// Windows), each time just after a test passed.
#[allow(dead_code)] // Not every test binary renders.
pub fn renderer() -> LazyRenderer {
    LazyRenderer::new(|| WgpuTestRenderer::from_setup(WgpuSetup::Existing(shared().clone())))
}

/// The device, made once and never dropped (statics are not).
fn shared() -> &'static WgpuSetupExisting {
    static SHARED: OnceLock<WgpuSetupExisting> = OnceLock::new();
    SHARED.get_or_init(|| {
        // kittest's default setup picks the adapter (software rasterizers
        // first). Make the instance again with only that adapter's backend:
        // each harness's `RenderState::create` lists the instance's adapters,
        // and listing the GL backend's would create a GL context every time.
        let probe = create_render_state(
            default_wgpu_setup(),
            egui_wgpu::RendererOptions::PREDICTABLE,
        );
        let backend = probe.adapter.get_info().backend;
        drop(probe);
        let mut setup = default_wgpu_setup();
        if let WgpuSetup::CreateNew(new) = &mut setup {
            new.instance_descriptor.backends = wgpu::Backends::from(backend);
        }
        let state = create_render_state(setup, egui_wgpu::RendererOptions::PREDICTABLE);
        WgpuSetupExisting {
            instance: state.instance.clone(),
            adapter: state.adapter.clone(),
            device: state.device.clone(),
            queue: state.queue.clone(),
        }
    })
}

/// Takes a snapshot after giving every buffer CR LF line endings. New tabs
/// use the operating system's ending, which the status bar shows, so without
/// this the images would differ between Windows and Linux/macOS.
#[allow(dead_code)] // Not every test binary takes snapshots.
pub fn snapshot(h: &mut Harness<'_, OchrepadApp>, name: &str) {
    for b in h.state_mut().editor.buffers_mut() {
        b.meta.line_ending = LineEnding::CrLf;
    }
    h.run();
    h.snapshot(name);
}
