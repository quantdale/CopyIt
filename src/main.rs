// Hide the console window on Windows in release builds (GUI app).
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod app;
mod model;
mod seed;
mod storage;
mod theme;

use app::CopyIt;

fn main() -> eframe::Result<()> {
    let native_options = eframe::NativeOptions {
        viewport: eframe::egui::ViewportBuilder::default()
            .with_inner_size([1000.0, 700.0])  // Default window size: wide enough for ~2-3 card columns
            .with_min_inner_size([560.0, 400.0]) // Minimum size: ensures UI doesn't break on resize
            .with_title("CopyIt"),
        ..Default::default()
    };

    eframe::run_native(
        "CopyIt",
        native_options,
        Box::new(|cc| Box::new(CopyIt::new(cc))),
    )
}
