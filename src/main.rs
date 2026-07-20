// CopyIt — a native Windows GUI app for storing and quickly copying scripts and AI prompts.
// Built with egui/eframe for a single-binary, zero-dependency release build.
// Hide the console window on Windows in release builds (GUI app).
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod app;    // Main UI and interaction logic; handles rendering, user input, and state updates
mod model;  // Core data structure: Snippet (id, title, category, body)
mod seed;   // Default snippet library seeded on first launch for new users
mod storage;// JSON persistence, data directory resolution, and category normalization
mod theme;  // 37 selectable color themes via custom egui::Visuals

use app::CopyIt;

/// Entry point: initializes the egui/eframe window and runs the app event loop.
/// Configures a 1000x700 default window with a 560x400 minimum to ensure the UI
/// remains usable during resize. The app auto-saves after every add/edit/delete/reorder.
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
