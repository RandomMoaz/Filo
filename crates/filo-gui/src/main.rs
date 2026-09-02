#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod app;

use app::FiloApp;

fn main() -> eframe::Result<()> {
    let native_options = eframe::NativeOptions {
        viewport: eframe::egui::ViewportBuilder::default()
            .with_inner_size([980.0, 640.0])
            .with_min_inner_size([640.0, 400.0])
            .with_title("filo — file manager"),
        ..Default::default()
    };

    eframe::run_native(
        "filo",
        native_options,
        Box::new(|_cc| Ok(Box::new(FiloApp::new()) as Box<dyn eframe::App>)),
    )
}
