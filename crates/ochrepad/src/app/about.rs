//! ? > About: the app's name, tagline, version, links and license, all from
//! the package metadata.

use egui::{Id, Modal, RichText, TextureHandle, TextureOptions, Ui};

use super::OchrepadApp;

pub(super) const NAME: &str = env!("CARGO_PKG_NAME");
pub(super) const TAGLINE: &str = env!("CARGO_PKG_DESCRIPTION");
pub(super) const VERSION: &str = env!("CARGO_PKG_VERSION");
pub(super) const LICENSE: &str = env!("CARGO_PKG_LICENSE");
pub(super) const REPOSITORY: &str = env!("CARGO_PKG_REPOSITORY");
pub(super) const CRATES_IO: &str = concat!("https://crates.io/crates/", env!("CARGO_PKG_NAME"));

/// Logical size of the logo in the dialog.
const LOGO_SIZE: f32 = 64.0;

#[derive(Default)]
pub(super) struct AboutDialog {
    logo: Option<TextureHandle>,
}

impl OchrepadApp {
    pub(super) fn about_dialog(&mut self, ui: &mut Ui) {
        let Some(d) = &mut self.about else {
            return;
        };
        let logo = d.logo.get_or_insert_with(|| {
            // Drawn at twice the size, so it stays sharp on scaled displays.
            let px = (LOGO_SIZE * 2.0) as u32;
            let image =
                egui::ColorImage::from_rgba_unmultiplied([px as usize; 2], &crate::logo::rgba(px));
            ui.ctx()
                .load_texture("ochrepad-about-logo", image, TextureOptions::LINEAR)
        });
        let mut close = false;
        let resp = Modal::new(Id::new("ochrepad-about")).show(ui.ctx(), |ui| {
            ui.vertical_centered(|ui| {
                ui.add(egui::Image::new((logo.id(), egui::Vec2::splat(LOGO_SIZE))));
                ui.add_space(8.0);
                ui.label(RichText::new(NAME).heading().strong());
                ui.label(TAGLINE);
                ui.label(RichText::new(format!("Version {VERSION}")).weak());
                ui.add_space(8.0);
                ui.hyperlink_to("Source code on GitHub", REPOSITORY);
                ui.hyperlink_to("ochrepad on crates.io", CRATES_IO);
                ui.add_space(8.0);
                ui.label(format!("Released under the {LICENSE} license."));
                ui.add_space(8.0);
                let button = ui.button("Close");
                // Keep the keyboard here, so typing does not reach the editor.
                if ui.memory(|m| m.focused().is_none()) {
                    button.request_focus();
                }
                close = button.clicked();
            });
        });
        if close || resp.should_close() {
            self.about = None;
            self.view_state_mut().request_focus();
        }
    }
}
