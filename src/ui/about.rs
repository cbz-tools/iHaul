//! About window.

use eframe::egui;

const PRODUCT_NAME: &str = "iHaul";
const GITHUB_URL: &str = "https://github.com/cbz-tools/iHaul";
const LATEST_RELEASE_URL: &str = "https://github.com/cbz-tools/iHaul/releases/latest";

pub fn show(ctx: &egui::Context, open: &mut bool, s: &crate::i18n::S) {
    if !*open {
        return;
    }

    if ctx.input(|input| input.key_pressed(egui::Key::Escape)) {
        *open = false;
        return;
    }

    let mut close_requested = false;
    egui::Window::new(PRODUCT_NAME)
        .open(open)
        .collapsible(false)
        .resizable(false)
        .default_width(320.0)
        .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
        .show(ctx, |ui| {
            ui.set_min_width(280.0);
            ui.vertical_centered(|ui| {
                ui.label(egui::RichText::new(PRODUCT_NAME).strong());
                ui.label(format!(
                    "{} {}",
                    s.about_version,
                    env!("CARGO_PKG_VERSION")
                ));
            });

            ui.add_space(12.0);
            if ui.link(s.about_github).clicked() {
                ctx.open_url(egui::OpenUrl::new_tab(GITHUB_URL));
            }
            if ui.link(s.about_latest_release).clicked() {
                ctx.open_url(egui::OpenUrl::new_tab(LATEST_RELEASE_URL));
            }

            ui.separator();
            if ui.button(s.about_close).clicked() {
                close_requested = true;
            }
        });

    if close_requested {
        *open = false;
    }
}
