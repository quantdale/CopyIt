//! Headed mode (`feature = "sim"` only): `copyit --simulate <journey> [--seed N]`
//! runs a journey against the real eframe window, captures a PNG screenshot via
//! `ViewportCommand::Screenshot`, and writes it into the report bundle.
//!
//! The journey itself is executed by the same headless harness used by the test
//! suite (identical event stream per seed), so correctness gates never depend
//! on the window. This module exists so a human can watch the real window once
//! and eyeball the result; it is never part of CI.

use crate::sim::harness::{run_dir_for, SimApp};
use crate::sim::journey;
use crate::sim::report;
use eframe::egui;
use std::path::PathBuf;

/// Parses `--simulate <journey> [--seed N]` from argv. Returns `None` when the
/// flag is absent (the default app path runs).
pub fn parse() -> Option<(String, u64)> {
    let mut args = std::env::args().skip(1);
    let mut journey = None;
    let mut seed = 0u64;
    while let Some(arg) = args.next() {
        if arg == "--simulate" {
            journey = args.next();
        } else if arg == "--seed" {
            seed = args.next().and_then(|s| s.parse().ok()).unwrap_or(0);
        }
    }
    journey.map(|j| (j, seed))
}

/// Runs the named journey in a real window.
pub fn run_headed(journey_name: String, seed: u64) -> eframe::Result<()> {
    let journey = match journey::by_name(&journey_name) {
        Some(j) => j,
        None => {
            eprintln!("unknown journey: {journey_name}");
            std::process::exit(1);
        }
    };
    eprintln!(
        "running journey '{}' with persona '{}' (seed {seed})",
        journey.name, journey.persona.name
    );
    let native_options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1000.0, 700.0])
            .with_min_inner_size([560.0, 400.0])
            .with_title("CopyIt — simulation"),
        ..Default::default()
    };
    eframe::run_native(
        "CopyIt — simulation",
        native_options,
        Box::new(move |cc| {
            Box::new(SimDriver::new(journey, seed, &cc.egui_ctx))
        }),
    )
}

/// eframe app driving the journey in the real window.
pub struct SimDriver {
    sim: SimApp,
    journey: journey::Journey,
    started: bool,
    screenshot_path: Option<PathBuf>,
}

impl SimDriver {
    pub fn new(journey: journey::Journey, seed: u64, ctx: &egui::Context) -> Self {
        // The same temp-store build path as the headless suite — the isolation
        // guard enforces it, so a headed run can never touch real data.
        let run_dir = run_dir_for(journey.name, seed);
        let _ = std::fs::remove_dir_all(&run_dir);
        let store = journey::materialize(&run_dir, &journey.fixtures)
            .expect("materialize journey fixtures");
        let sim = SimApp::build(journey.name, store, journey.persona, run_dir, seed)
            .expect("build simulated app");
        let _ = ctx; // Visuals are applied by the harness's own context.
        SimDriver {
            sim,
            journey,
            started: false,
            screenshot_path: None,
        }
    }
}

impl eframe::App for SimDriver {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        if !self.started {
            self.started = true;
            // Execute the whole journey headlessly through the same harness the
            // tests use; the window then shows the final app state.
            let _ = journey::run_into(&mut self.sim, &self.journey);
            self.sim.app.ui(ctx);
            ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot);
            self.screenshot_path = Some(
                self.sim
                    .report
                    .report_dir
                    .join("screenshot-final.png"),
            );
            ctx.request_repaint();
        } else {
            self.sim.app.ui(ctx);
            if let Some(path) = self.screenshot_path.take() {
                let events = ctx.input(|i| i.events.clone());
                for event in events {
                    if let egui::Event::Screenshot { image, .. } = event {
                        match report::write_png(&path, &image) {
                            Ok(()) => eprintln!("screenshot saved to {}", path.display()),
                            Err(e) => eprintln!("couldn't save screenshot: {e}"),
                        }
                    }
                }
            }
        }
    }
}
