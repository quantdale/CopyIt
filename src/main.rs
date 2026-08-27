// CopyIt — a native Windows GUI app for storing and quickly copying scripts and AI prompts.
// Built with egui/eframe for a single-binary, zero-dependency release build.
// Documentation: this module is the executable entry point for the desktop application.
// Documentation: application state and UI behavior are implemented primarily in `app`.
// Documentation: persistence is separated across the `storage` and `store` modules.
// Documentation: protected snippet handling is isolated in the `vault` module.
// Documentation: simulation support is excluded from normal release builds unless enabled.
// Hide the console window on Windows in release builds (GUI app).
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod app; // Main UI and interaction logic; handles rendering, user input, and state updates
mod clipboard; // Clipboard abstraction with bounded protected-copy lifetime
mod editor; // Snippet add/edit modal: state, constructors, and the transition decision
mod grid; // Card grid geometry, virtualization, insertion lines, and the drag state machine
mod model; // Core data structure: Snippet (id, title, category, body)
mod seed; // Default snippet library seeded on first launch for new users
mod sqlite; // Canonical SQLite engine backing the shared copyit.db store
mod storage; // JSON persistence, data directory resolution, and category normalization
mod store; // Persistence seam: paths, legacy migration, load/save of snippets and config
mod theme; // 37 selectable color themes via custom egui::Visuals
mod vault; // Protected snippets: vault state, key derivation, and encryption

// The simulation harness (headless journeys + headed mode) is compiled only under
// `cfg(any(test, feature = "sim"))`, so the release binary is never slowed by it.
#[cfg(any(test, feature = "sim"))]
mod sim;

use app::CopyIt;

/// Entry point: initializes the egui/eframe window and runs the app event loop.
/// Configures a 1000x700 default window with a 560x400 minimum to ensure the UI
/// remains usable during resize. The app auto-saves after every add/edit/delete/reorder.
fn main() -> eframe::Result<()> {
    // The headed simulation mode is opt-in via the `sim` cargo feature. Without
    // it no simulation code is compiled and `--simulate` is simply unknown to
    // the app (it falls through to the normal window).
    #[cfg(feature = "sim")]
    if let Some((journey, seed)) = sim::cli::parse() {
        return sim::cli::run_headed(journey, seed);
    }

    let native_options = eframe::NativeOptions {
        viewport: eframe::egui::ViewportBuilder::default()
            .with_inner_size([1000.0, 700.0]) // Default window size: wide enough for ~2-3 card columns
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
