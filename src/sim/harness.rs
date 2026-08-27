//! The in-process simulation harness.
//!
//! [`SimApp`] wraps the real `CopyIt` UI behind a bare `egui::Context` and
//! pumps frames through `egui::Context::run` with synthesized `RawInput`
//! events — pointer moves/buttons, key presses, text events — on a virtual
//! clock and a seeded PRNG. Interaction targets are resolved by scanning the
//! tessellated frame for visible text, so the robot clicks what a user sees.
//!
//! Key invariants (each with a test):
//! - Time only advances through `RawInput.time`; nothing sleeps.
//! - All randomness comes from one `StdRng` seeded per run and recorded.
//! - The app is only ever built from a `Store::at(path)` under
//!   `<temp>/copyit-sim/<pid>/<run>`; anything else is refused before the
//!   first frame. `Store::open` and `migrate_legacy` are never called here.

use crate::app::CopyIt;
use crate::grid::{CARD_H, CARD_W};
use crate::store::Store;
use eframe::egui;
use eframe::egui::{Event, Key, Modifiers, PointerButton, Pos2, RawInput, Rect, Vec2};
use rand::rngs::StdRng;
use rand::Rng;
use rand::SeedableRng;
use std::path::PathBuf;

use super::journey::Step;
use super::persona::Persona;
use super::report::{EventRecord, Report, Snapshot};

/// Window size the simulated app runs in, matching the default 1000x700 window.
pub const SCREEN_W: f32 = 1000.0;
pub const SCREEN_H: f32 = 700.0;

/// A fully assembled simulated run: the real app, its bare egui context, the
/// virtual clock, the seeded PRNG, and the live report.
pub struct SimApp {
    pub app: CopyIt,
    pub ctx: egui::Context,
    /// Virtual time in milliseconds since the run started.
    pub time_ms: u64,
    pub rng: StdRng,
    pub seed: u64,
    pub store: Store,
    pub persona: Persona,
    last_output: Option<egui::FullOutput>,
    /// The last text the app copied to the clipboard. `copied_text` in
    /// `PlatformOutput` is per-frame output, so the harness captures it
    /// persistently — the system clipboard does not clear when a frame ends.
    last_clipboard: String,
    /// Shared observer for the deterministic Sim clipboard backend installed at
    /// `build` time, so protected copies (which bypass `copied_text`) are
    /// inspectable by journey assertions.
    clipboard_handle: crate::clipboard::SimHandle,
    pub report: Report,
}

/// The per-run sim-root directory: `<temp>/copyit-sim`.
pub fn sim_root() -> PathBuf {
    std::env::temp_dir().join("copyit-sim")
}

/// The per-run store directory: `<temp>/copyit-sim/<pid>/<run>`.
pub fn run_dir_for(name: &str, seed: u64) -> PathBuf {
    let safe_name: String = name
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '-'
            }
        })
        .collect();
    sim_root()
        .join(std::process::id().to_string())
        .join(format!("{safe_name}-{seed}"))
}

impl SimApp {
    /// Constructs a run from an explicit store. Refuses (before the first
    /// frame) any store whose data files do not live under the per-run temp
    /// directory, so a journey can never touch the real `%APPDATA%` data.
    pub fn build(
        journey: &str,
        store: Store,
        persona: Persona,
        run_dir: PathBuf,
        seed: u64,
    ) -> Result<Self, String> {
        // Isolation guard, enforced at runtime (not just debug_assert).
        let base = sim_root();
        if !run_dir.starts_with(&base) {
            return Err(format!(
                "refusing store dir outside {}: {}",
                base.display(),
                run_dir.display()
            ));
        }
        for path in [&store.snippets_path, &store.config_path] {
            if !path.starts_with(&run_dir) {
                return Err(format!(
                    "refusing store at {}: data files must live under {}",
                    path.display(),
                    run_dir.display()
                ));
            }
        }

        let ctx = egui::Context::default();
        // `Store` isn't `Clone`, and `CopyIt::from_store` consumes it; build the
        // harness's and the report's copies from the (pub) paths up front.
        let harness_store = Store {
            snippets_path: store.snippets_path.clone(),
            config_path: store.config_path.clone(),
        };
        let mut app = CopyIt::from_store(store, &ctx);
        let rng = StdRng::seed_from_u64(seed);
        let report = Report::new(journey, seed, run_dir.clone(), harness_store);
        let store_for_report = Store {
            snippets_path: report.store.snippets_path.clone(),
            config_path: report.store.config_path.clone(),
        };
        // Install a deterministic Sim clipboard so protected copies (which bypass egui's
        // `copied_text`) are observable by the journey assertions.
        let (clipboard, clipboard_handle) = crate::clipboard::SimClipboard::new();
        app.clipboard = Box::new(clipboard);
        Ok(SimApp {
            app,
            ctx,
            time_ms: 0,
            rng,
            seed,
            store: store_for_report,
            persona,
            last_output: None,
            last_clipboard: String::new(),
            clipboard_handle,
            report,
        })
    }

    /// Pumps a single frame with the given events at the current virtual time.
    pub fn pump(&mut self, events: Vec<Event>) {
        self.pump_with_modifiers(events, Modifiers::default());
    }

    /// Pumps a frame with the given events and frame-level modifier state.
    /// egui's keyboard shortcuts (e.g. Ctrl+A select-all) read the frame-level
    /// modifiers, not the per-event ones, so synthesized shortcuts must set the
    /// frame state here.
    pub fn pump_with_modifiers(&mut self, events: Vec<Event>, modifiers: Modifiers) {
        let raw = RawInput {
            screen_rect: Some(Rect::from_min_size(
                Pos2::ZERO,
                Vec2::new(SCREEN_W, SCREEN_H),
            )),
            time: Some(self.time_ms as f64 / 1000.0),
            modifiers,
            events,
            ..Default::default()
        };
        let output = self.ctx.run(raw, |ctx| self.app.ui(ctx));
        if !output.platform_output.copied_text.is_empty() {
            self.last_clipboard = output.platform_output.copied_text.clone();
        }
        self.last_output = Some(output);
    }

    /// Advances the virtual clock by `ms` (no wall-clock sleep) and pumps one
    /// frame so time-dependent UI (the transient "Copied" feedback, ...) sees it.
    pub fn wait(&mut self, ms: u64) -> Result<(), String> {
        self.time_ms = self.time_ms.saturating_add(ms);
        self.pump(vec![]);
        Ok(())
    }

    /// A persona think-time pause, drawn from the seeded RNG.
    fn think(&mut self) {
        let ms = self.persona.think_ms(&mut self.rng);
        self.time_ms = self.time_ms.saturating_add(ms);
    }

    /// Returns every visible text of the last frame with its screen rect, in
    /// render order (windows/modals on top come last).
    pub fn visible_texts(&self) -> Vec<(String, Rect)> {
        let mut out = Vec::new();
        if let Some(output) = &self.last_output {
            for clip in &output.shapes {
                if let egui::Shape::Text(shape) = &clip.shape {
                    let text = shape.galley.text();
                    if !text.trim().is_empty() {
                        let rect = Rect::from_min_size(shape.pos, shape.galley.size());
                        out.push((text.to_string(), rect));
                    }
                }
            }
        }
        out
    }

    /// Human-readable listing of everything on screen — used in failure messages.
    pub fn on_screen(&self) -> String {
        let texts: Vec<String> = self.visible_texts().into_iter().map(|(t, _)| t).collect();
        if texts.is_empty() {
            "(nothing rendered)".to_string()
        } else {
            texts.join(" | ")
        }
    }

    /// Locates a label, preferring the topmost (last-rendered) exact match and
    /// falling back to a contains match. Fails naming the missing label.
    pub fn locate(&self, label: &str) -> Result<Rect, String> {
        let texts = self.visible_texts();
        // Exact match, topmost last.
        for (text, rect) in texts.iter().rev() {
            if text.trim() == label {
                return Ok(*rect);
            }
        }
        // Contains match, topmost last.
        for (text, rect) in texts.iter().rev() {
            if text.contains(label) {
                return Ok(*rect);
            }
        }
        Err(format!(
            "label `{label}` not visible; on screen: {}",
            self.on_screen()
        ))
    }

    /// Locates the first (earliest-rendered, usually topmost-in-panels) match —
    /// used for combo boxes in the top bar, whose selected text is rendered
    /// before the grid's card badges with the same category name.
    pub fn locate_first(&self, label: &str) -> Result<Rect, String> {
        let texts = self.visible_texts();
        for (text, rect) in &texts {
            if text.trim() == label {
                return Ok(*rect);
            }
        }
        for (text, rect) in &texts {
            if text.contains(label) {
                return Ok(*rect);
            }
        }
        Err(format!(
            "label `{label}` not visible; on screen: {}",
            self.on_screen()
        ))
    }

    /// Clicks the center of a located label.
    pub fn click_text(&mut self, label: &str) -> Result<(), String> {
        let rect = self.locate(label)?;
        self.think();
        self.click(rect.center())
    }

    /// Clicks the first (earliest-rendered) match of a label — see
    /// [`Self::locate_first`].
    pub fn click_first_text(&mut self, label: &str) -> Result<(), String> {
        let rect = self.locate_first(label)?;
        self.think();
        self.click(rect.center())
    }

    /// Synthesizes a full pointer click: hover, press, release.
    pub fn click(&mut self, pos: Pos2) -> Result<(), String> {
        self.pump(vec![Event::PointerMoved(pos)]);
        self.pump(vec![Event::PointerButton {
            pos,
            button: PointerButton::Primary,
            pressed: true,
            modifiers: Modifiers::default(),
        }]);
        self.pump(vec![Event::PointerButton {
            pos,
            button: PointerButton::Primary,
            pressed: false,
            modifiers: Modifiers::default(),
        }]);
        // A settle frame lets the click's effect (a popup opening, an editor
        // appearing) register before the next step asserts on it.
        self.pump(vec![]);
        Ok(())
    }

    /// Types text into a field, one `Event::Text` per character at the persona's
    /// cadence, with occasional seeded typo-and-correct (backspace) sequences.
    /// `field` is either a field's label (a layout hint like "Title") or the
    /// field's hint text (e.g. the search field's "Search title, text, category…").
    pub fn type_into(&mut self, field: &str, text: &str) -> Result<(), String> {
        let rect = self.field_rect(field)?;
        self.click(rect.center())?;
        self.type_into_focused(text)
    }

    /// Replaces the contents of a field: click it, select all, delete, then type.
    pub fn replace_into(&mut self, field: &str, text: &str) -> Result<(), String> {
        let rect = self.field_rect(field)?;
        self.click(rect.center())?;
        self.clear_focused_field()?;
        self.type_into_focused(text)
    }

    /// Types text into the currently-focused field.
    fn type_into_focused(&mut self, text: &str) -> Result<(), String> {
        for ch in text.chars() {
            if self.persona.should_typo(&mut self.rng) {
                let wrong = self.random_char();
                self.type_char(wrong);
                self.press_key(Key::Backspace)?;
            }
            self.type_char(ch);
        }
        Ok(())
    }

    /// A single typed character with a seeded cadence delay and a text event.
    pub(crate) fn type_char(&mut self, ch: char) {
        let ms = self.persona.type_ms(&mut self.rng);
        self.time_ms = self.time_ms.saturating_add(ms);
        self.pump(vec![Event::Text(ch.to_string())]);
        // A tiny settle frame lets the TextEdit apply the character and keeps
        // focus wherever it is.
        self.pump(vec![]);
    }

    fn random_char(&mut self) -> char {
        const CHARS: &[char] = &['a', 'e', 'i', 'o', 'u', 't', 'n', 's'];
        CHARS[self.rng.gen_range(0..CHARS.len())]
    }

    /// The clickable rect of a text field, derived from its label/hint text.
    pub fn field_rect(&self, field: &str) -> Result<Rect, String> {
        let label = self.locate(field)?;
        Ok(match field {
            // Editor single-line Title: the field sits directly below the label.
            "Title" => Rect::from_min_size(
                Pos2::new(label.min.x + 40.0, label.bottom() + 6.0),
                Vec2::new(240.0, 22.0),
            ),
            // Editor multiline Content: below the label, tall.
            "Content" => Rect::from_min_size(
                Pos2::new(label.min.x + 40.0, label.bottom() + 6.0),
                Vec2::new(360.0, 120.0),
            ),
            // Everything else is located by its own rendered hint text (search
            // field, "New category" inputs), which is inside the field.
            _ => label,
        })
    }

    /// Presses and releases a key.
    pub fn press_key(&mut self, key: Key) -> Result<(), String> {
        self.pump(vec![Event::Key {
            key,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: Modifiers::default(),
        }]);
        self.pump(vec![Event::Key {
            key,
            physical_key: None,
            pressed: false,
            repeat: false,
            modifiers: Modifiers::default(),
        }]);
        Ok(())
    }

    /// Selects all text in the currently focused field and deletes it. The
    /// Ctrl+A is sent with the frame-level ctrl *and* command modifier set, via
    /// `RawInput.modifiers` — egui's select-all shortcut reads
    /// `Modifiers::command`, which on Windows tracks ctrl.
    pub fn clear_focused_field(&mut self) -> Result<(), String> {
        let ctrl = Modifiers {
            ctrl: true,
            command: true,
            ..Modifiers::default()
        };
        self.pump_with_modifiers(
            vec![Event::Key {
                key: Key::A,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: ctrl,
            }],
            ctrl,
        );
        self.pump_with_modifiers(
            vec![Event::Key {
                key: Key::A,
                physical_key: None,
                pressed: false,
                repeat: false,
                modifiers: ctrl,
            }],
            Modifiers::default(),
        );
        self.press_key(Key::Backspace)
    }

    /// Asserts a label is visible on screen.
    /// Asserts a label is visible on screen. Tries exact match first, then
    /// substring, so `expect_visible("Add")` matches a button labelled exactly
    /// "Add" rather than "+ Add new category".
    pub fn expect_visible(&self, text: &str) -> Result<(), String> {
        let texts = self.visible_texts();
        let found = texts.iter().any(|(t, _)| t.trim() == text)
            || texts.iter().any(|(t, _)| t.contains(text));
        if found {
            Ok(())
        } else {
            Err(format!(
                "expected `{text}` visible; on screen: {}",
                self.on_screen()
            ))
        }
    }

    /// Asserts a label is absent from screen. Tries exact match first, then
    /// substring.
    pub fn expect_absent(&self, text: &str) -> Result<(), String> {
        let texts = self.visible_texts();
        let found = texts.iter().any(|(t, _)| t.trim() == text)
            || texts.iter().any(|(t, _)| t.contains(text));
        if found {
            Err(format!("expected `{text}` absent, but it is visible"))
        } else {
            Ok(())
        }
    }

    /// Asserts the clipboard now holds exactly `text`.
    pub fn expect_clipboard(&self, text: &str) -> Result<(), String> {
        let current = self.clipboard_handle.text();
        if current == text {
            Ok(())
        } else {
            Err(format!("expected clipboard `{text}`, got `{current}`"))
        }
    }

    /// The current clipboard contents (the last text the app copied). In the sim harness
    /// every copy routes through the deterministic Sim backend, so this reflects what a
    /// real system clipboard would hold — including protected copies that bypass egui's
    /// `copied_text` field.
    pub fn clipboard(&self) -> String {
        self.clipboard_handle.text()
    }

    /// Asserts something about the persisted store, reloading from disk.
    pub fn expect_store(&self, pred: &dyn Fn(&Store) -> Result<(), String>) -> Result<(), String> {
        pred(&self.store)
    }

    /// Drags the card titled `from_title` onto the gap before the card titled
    /// `to_title`, entirely through synthesized pointer events. The drop flows
    /// through the real `DragMachine` in `CopyIt::ui`.
    ///
    /// The release point is just inside the target card's top-left corner: it is
    /// guaranteed to be inside the scroll area (so `DragMachine::release` accepts
    /// the drop) and `nearest_gap` resolves it to the gap immediately before the
    /// target card, whether the target starts a row or sits mid-row.
    pub fn drag_card(&mut self, from_title: &str, to_title: &str) -> Result<(), String> {
        let from_card = self.card_rect_for_title(from_title)?;
        let to_card = self.card_rect_for_title(to_title)?;
        let press = from_card.center();
        let release = Pos2::new(to_card.left() + 2.0, to_card.top() + 2.0);

        self.think();
        self.pump(vec![Event::PointerMoved(press)]);
        self.pump(vec![Event::PointerButton {
            pos: press,
            button: PointerButton::Primary,
            pressed: true,
            modifiers: Modifiers::default(),
        }]);
        // Cross egui's drag detection, then the app's own 4px DragMachine
        // threshold, then move to the target gap.
        self.pump(vec![Event::PointerMoved(press + Vec2::splat(8.0))]);
        self.pump(vec![Event::PointerMoved(press + Vec2::splat(20.0))]);
        self.pump(vec![Event::PointerMoved(release)]);
        self.pump(vec![Event::PointerButton {
            pos: release,
            button: PointerButton::Primary,
            pressed: false,
            modifiers: Modifiers::default(),
        }]);
        // A settle frame lets the drop's reorder register.
        self.pump(vec![]);
        Ok(())
    }

    /// The card rect (full frame) of the card whose title is on screen.
    pub fn card_rect_for_title(&self, title: &str) -> Result<Rect, String> {
        let title_rect = self.locate(title)?;
        // The title sits at the top-left of the card content, 10px inside the
        // 10px frame margin.
        Ok(Rect::from_min_size(
            title_rect.min - Vec2::splat(10.0),
            Vec2::new(CARD_W, CARD_H),
        ))
    }

    /// Clicks the Copy button of a specific card.
    pub fn click_card_copy(&mut self, title: &str) -> Result<(), String> {
        let card = self.card_rect_for_title(title)?;
        let texts = self.visible_texts();
        // Try to locate the "Copy" button text inside the card. egui button text
        // is rendered as part of a galley — it may or may not appear as a
        // standalone Shape::Text — so fall back to the known card layout position
        // (Copy is top-right, ~40px from right edge, ~22px from top).
        let target = texts
            .iter()
            .find(|(t, r)| t.trim() == "Copy" && card.intersects(*r))
            .or_else(|| {
                texts
                    .iter()
                    .find(|(t, r)| t.contains("Copy") && card.intersects(*r))
            })
            .map(|(_, r)| r.center())
            .unwrap_or_else(|| Pos2::new(card.right() - 40.0, card.top() + 22.0));
        self.think();
        self.click(target)
    }

    /// Clicks the Edit button of a specific card.
    pub fn click_card_edit(&mut self, title: &str) -> Result<(), String> {
        let card = self.card_rect_for_title(title)?;
        let texts = self.visible_texts();
        // The Edit button sits bottom-left; fall back to the known card layout
        // position (left+40px, bottom-30px) if the text isn't in the output
        // shapes.
        let target = texts
            .iter()
            .find(|(t, r)| t.trim() == "Edit" && card.intersects(*r))
            .or_else(|| {
                texts
                    .iter()
                    .find(|(t, r)| t.contains("Edit") && card.intersects(*r))
            })
            .map(|(_, r)| r.center())
            .unwrap_or_else(|| Pos2::new(card.left() + 40.0, card.bottom() - 30.0));
        self.think();
        self.click(target)
    }

    /// Opens the editor's category dropdown by clicking the combo button that
    /// sits directly below the "Category" label.
    fn click_editor_category_combo(&mut self) -> Result<(), String> {
        let cat = self.locate("Category")?;
        self.think();
        self.click(Pos2::new(cat.center().x, cat.bottom() + 19.0))
    }

    /// Enters the editor's inline "new category" flow: open the category
    /// dropdown and click "+ Add new category".
    pub fn begin_editor_new_category(&mut self) -> Result<(), String> {
        self.click_editor_category_combo()?;
        self.pump(vec![]);
        self.click_text("+ Add new category")?;
        self.pump(vec![]);
        Ok(())
    }

    /// Picks an existing category in the editor via its dropdown.
    pub fn set_editor_category(&mut self, category: &str) -> Result<(), String> {
        self.click_editor_category_combo()?;
        self.pump(vec![]);
        self.click_text(category)
    }

    /// Adds a brand-new category in the editor via its dropdown and fills it in.
    pub fn add_editor_category(&mut self, name: &str) -> Result<(), String> {
        self.click_editor_category_combo()?;
        self.pump(vec![]);
        self.click_text("+ Add new category")?;
        self.pump(vec![]);
        self.type_into("New category", name)?;
        self.click_text("Add")
    }

    /// Clicks just left of a located label — used to focus an inline text field
    /// whose hint text is no longer visible (it already has content), e.g. the
    /// header's "New category" field after submitting a rejected name.
    #[allow(dead_code)] // retained as a public sim-harness utility
    pub fn click_left_of(&mut self, label: &str) -> Result<(), String> {
        let rect = self.locate(label)?;
        self.think();
        self.click(Pos2::new(rect.left() - 30.0, rect.center().y))
    }

    /// Opens the top bar's inline "add category" form.
    pub fn open_header_category_form(&mut self) -> Result<(), String> {
        self.click_first_text("All")?;
        self.pump(vec![]);
        self.click_text("+ Add new category")?;
        self.pump(vec![]);
        Ok(())
    }

    /// Submits the top bar's inline "add category" form.
    pub fn submit_header_category(&mut self, name: &str) -> Result<(), String> {
        self.type_into("New category", name)?;
        self.click_text("Add")
    }

    /// Drops the current app and rebuilds it from the same store (a simulated
    /// restart), which reloads the persisted config — used by ThemeHopper.
    pub fn rebuild(&mut self) -> Result<(), String> {
        let ctx = egui::Context::default();
        let store = crate::store::Store {
            snippets_path: self.store.snippets_path.clone(),
            config_path: self.store.config_path.clone(),
        };
        let app = CopyIt::from_store(store, &ctx);
        self.app = app;
        self.ctx = ctx;
        self.last_output = None;
        self.pump(vec![]);
        Ok(())
    }

    /// Records the frame's snapshot and an event-log line for a step.
    pub fn record_step(&mut self, step: usize, intent: &str, target: &str) -> Result<(), String> {
        let visible: Vec<String> = self.visible_texts().into_iter().map(|(t, _)| t).collect();
        self.report
            .write_snapshot(Snapshot { step, visible })
            .map_err(|e| format!("couldn't write snapshot: {e}"))?;
        let record = EventRecord {
            step,
            intent: intent.to_string(),
            target: target.to_string(),
            input: "see snapshot".to_string(),
            time_ms: self.time_ms,
            state: self.report.state_digest(),
        };
        self.report
            .push_event(record)
            .map_err(|e| format!("couldn't write event log: {e}"))
    }

    /// Executes one journey step, dispatching to the harness intents.
    pub fn execute(&mut self, step: &Step) -> Result<(), String> {
        match step {
            Step::Intent(action) => self.execute_intent(action),
            Step::Custom(f) => f(self),
        }
    }

    fn execute_intent(&mut self, action: &super::journey::Intent) -> Result<(), String> {
        use super::journey::Intent::*;
        match action {
            ClickText(label) => self.click_text(label),
            ClickFirstText(label) => self.click_first_text(label),
            TypeInto(field, text) => self.type_into(field, text),
            ReplaceField(field, text) => self.replace_into(field, text),
            PressKey(key) => self.press_key(*key),
            Wait(ms) => self.wait(*ms),
            ExpectVisible(text) => self.expect_visible(text),
            ExpectAbsent(text) => self.expect_absent(text),
            ExpectClipboard(text) => self.expect_clipboard(text),
            ExpectStore(pred) => self.expect_store(&**pred),
            DragCard(from, to) => self.drag_card(from, to),
            ClickCardCopy(title) => self.click_card_copy(title),
            ClickCardEdit(title) => self.click_card_edit(title),
            SetEditorCategory(cat) => self.set_editor_category(cat),
            AddEditorCategory(name) => self.add_editor_category(name),
            OpenHeaderCategoryForm => self.open_header_category_form(),
            SubmitHeaderCategory(name) => self.submit_header_category(name),
            ClearFocusedField => self.clear_focused_field(),
            RebuildFromStore => self.rebuild(),
        }
    }
}
