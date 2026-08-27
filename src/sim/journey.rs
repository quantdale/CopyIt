//! The scenario layer: journey definition DSL, data fixtures, the runner, and
//! the journey library covering the app's complete workflows.
//!
//! A [`Journey`] is a plain, compile-time-checked Rust value: a persona, a set
//! of fixtures materialized into the per-run temp store before the first frame,
//! and a list of [`Step`]s interleaving harness intents with assertions. The
//! only custom code lives in `Custom` step closures, which is also the
//! extension hook promised for future features (e.g. vault journeys).
//!
//! The test suite lives here too: one test per journey, plus the determinism,
//! isolation, harness-refusal, and meta (failure-bundle) tests. All are named
//! `sim_journeys_*` so CI can run exactly this suite headlessly.

use crate::model::Snippet;
#[cfg(test)]
use crate::sim::harness::run_dir_for;
use crate::sim::harness::SimApp;
use crate::sim::persona::{
    Persona, ERROR_HANDLER, FIRST_RUN_EXPLORER, POWER_ORGANIZER, THEME_HOPPER,
};
#[cfg(test)]
use crate::sim::report::Report;
use crate::storage::Config;
use crate::store::Store;
use crate::vault::{self, VaultMeta};
use eframe::egui;
use std::path::{Path, PathBuf};

/// The assertion closure behind `Intent::ExpectStore`: re-reads the run's
/// store from disk and describes any deviation.
type StorePredicate = dyn Fn(&Store) -> Result<(), String>;

/// The custom-step hook: a closure that runs against the live harness, so
/// future features can add flows without touching the harness.
type CustomStep = dyn Fn(&mut SimApp) -> Result<(), String>;

/// Why a run failed. Carries the report location so tests can inspect the
/// bundle (and so the meta-test can assert it is complete).
#[derive(Debug)]
#[allow(dead_code)] // fields read by tests; type returned by run_into (used by the headed CLI)
pub struct SimulationError {
    pub journey: String,
    pub seed: u64,
    pub step: usize,
    pub message: String,
    pub report_dir: PathBuf,
}

impl SimulationError {
    fn failed(journey: &str, seed: u64, step: usize, message: String, report_dir: PathBuf) -> Self {
        SimulationError {
            journey: journey.to_string(),
            seed,
            step,
            message,
            report_dir,
        }
    }
}

/// The intent vocabulary a journey is built from. These map 1:1 onto harness
/// methods; the event log records each one as it runs. String payloads are
/// `&'static str` because the journey library is a static, compile-time-checked
/// declaration; dynamic values go through `Custom` steps instead.
pub enum Intent {
    ClickText(&'static str),
    ClickFirstText(&'static str),
    TypeInto(&'static str, &'static str),
    ReplaceField(&'static str, &'static str),
    #[allow(dead_code)] // retained as a journey-step variant
    PressKey(egui::Key),
    Wait(u64),
    ExpectVisible(&'static str),
    ExpectAbsent(&'static str),
    ExpectClipboard(&'static str),
    ExpectStore(Box<StorePredicate>),
    DragCard(&'static str, &'static str),
    ClickCardCopy(&'static str),
    ClickCardEdit(&'static str),
    SetEditorCategory(&'static str),
    AddEditorCategory(&'static str),
    OpenHeaderCategoryForm,
    SubmitHeaderCategory(&'static str),
    ClearFocusedField,
    RebuildFromStore,
}

impl Intent {
    fn describe(&self) -> (String, String) {
        match self {
            Intent::ClickText(label) => ("click_text".into(), label.to_string()),
            Intent::ClickFirstText(label) => ("click_first_text".into(), label.to_string()),
            Intent::TypeInto(field, text) => ("type_into".into(), format!("{field} <- {text}")),
            Intent::ReplaceField(field, text) => {
                ("replace_field".into(), format!("{field} <- {text}"))
            }
            Intent::PressKey(key) => ("press_key".into(), format!("{key:?}")),
            Intent::Wait(ms) => ("wait".into(), format!("{ms}ms")),
            Intent::ExpectVisible(text) => ("expect_visible".into(), text.to_string()),
            Intent::ExpectAbsent(text) => ("expect_absent".into(), text.to_string()),
            Intent::ExpectClipboard(text) => ("expect_clipboard".into(), text.to_string()),
            Intent::ExpectStore(_) => ("expect_store".into(), String::new()),
            Intent::DragCard(from, to) => ("drag_card".into(), format!("{from} -> {to}")),
            Intent::ClickCardCopy(title) => ("click_card_copy".into(), title.to_string()),
            Intent::ClickCardEdit(title) => ("click_card_edit".into(), title.to_string()),
            Intent::SetEditorCategory(cat) => ("set_editor_category".into(), cat.to_string()),
            Intent::AddEditorCategory(name) => ("add_editor_category".into(), name.to_string()),
            Intent::OpenHeaderCategoryForm => ("open_header_category_form".into(), String::new()),
            Intent::SubmitHeaderCategory(name) => {
                ("submit_header_category".into(), name.to_string())
            }
            Intent::ClearFocusedField => ("clear_focused_field".into(), String::new()),
            Intent::RebuildFromStore => ("rebuild_from_store".into(), String::new()),
        }
    }
}

/// One journey step: an intent, or a custom closure for flows the intent
/// vocabulary does not cover (the extension hook for future features).
pub enum Step {
    Intent(Intent),
    Custom(Box<CustomStep>),
}

impl Step {
    fn describe(&self) -> (String, String) {
        match self {
            Step::Intent(intent) => intent.describe(),
            Step::Custom(_) => ("custom".into(), String::new()),
        }
    }
}

/// The data a journey starts from, materialized into the run's temp store
/// before the first frame. Fixtures never reference real user data files.
pub struct Fixtures {
    /// `true`: no `snippets.json` is written and the app seeds its default
    /// library (a true first launch).
    pub use_seed_defaults: bool,
    /// The explicit library; used when `use_seed_defaults` is false.
    pub snippets: Vec<Snippet>,
    /// Explicit config (categories / theme); `None` lets the app derive it.
    pub config: Option<Config>,
    /// Write a corrupt `snippets.json` to exercise the recovery path.
    pub corrupt_snippets: bool,
    /// Make saves fail by blocking the atomic temp file (a read-only-data-dir
    /// stand-in that works on Windows).
    pub block_save: bool,
}

impl Fixtures {
    pub fn seed_defaults() -> Self {
        Fixtures {
            use_seed_defaults: true,
            snippets: Vec::new(),
            config: None,
            corrupt_snippets: false,
            block_save: false,
        }
    }

    pub fn library(snippets: Vec<Snippet>, categories: &[&str], theme: &str) -> Self {
        Fixtures {
            use_seed_defaults: false,
            snippets,
            config: Some(Config {
                categories: categories.iter().map(|c| (*c).to_string()).collect(),
                theme: theme.to_string(),
                vault: None,
            }),
            corrupt_snippets: false,
            block_save: false,
        }
    }

    /// A library fixture that also provisions a vault: `config.json` carries the
    /// vault metadata (salt + canary), so the app loads the vault — but the
    /// session still starts locked, exactly like a real relaunch. Used by the
    /// vault journeys, whose fixtures are built by encrypting real bodies
    /// through `crate::vault` (the same code path the app uses to protect).
    pub fn library_with_vault(
        snippets: Vec<Snippet>,
        categories: &[&str],
        theme: &str,
        vault: Option<VaultMeta>,
    ) -> Self {
        Fixtures {
            use_seed_defaults: false,
            snippets,
            config: Some(Config {
                categories: categories.iter().map(|c| (*c).to_string()).collect(),
                theme: theme.to_string(),
                vault,
            }),
            corrupt_snippets: false,
            block_save: false,
        }
    }

    pub fn corrupt() -> Self {
        Fixtures {
            corrupt_snippets: true,
            ..Fixtures::seed_defaults()
        }
    }

    /// A library whose saves fail: the atomic temp-file write is blocked,
    /// standing in for a read-only data directory (which Windows makes hard to
    /// set up portably).
    pub fn read_only(snippets: Vec<Snippet>, categories: &[&str], theme: &str) -> Self {
        Fixtures {
            block_save: true,
            ..Fixtures::library(snippets, categories, theme)
        }
    }
}

/// A complete, runnable journey.
pub struct Journey {
    pub name: &'static str,
    pub persona: Persona,
    pub fixtures: Fixtures,
    pub steps: Vec<Step>,
}

/// Writes a journey's fixtures into the run directory and returns its store.
pub fn materialize(run_dir: &Path, fixtures: &Fixtures) -> Result<Store, String> {
    std::fs::create_dir_all(run_dir)
        .map_err(|e| format!("create run dir {}: {e}", run_dir.display()))?;
    let store = Store::at(run_dir.to_path_buf());
    if fixtures.corrupt_snippets {
        std::fs::write(&store.snippets_path, "this is definitely not json")
            .map_err(|e| format!("write corrupt fixture: {e}"))?;
    } else if !fixtures.use_seed_defaults {
        store
            .save_snippets(&fixtures.snippets)
            .map_err(|e| format!("write fixture snippets: {e}"))?;
    }
    if let Some(config) = &fixtures.config {
        store
            .save_config(config)
            .map_err(|e| format!("write fixture config: {e}"))?;
    }
    if fixtures.block_save {
        // Make the canonical db file read-only: the app opens it read-only for
        // loads (so the library still loads), but any save attempt opens it
        // read-write and fails cleanly — the portable stand-in for a read-only
        // data directory.
        let db = crate::sqlite::db_path_for(run_dir);
        let mut perms = std::fs::metadata(&db)
            .map_err(|e| format!("block save (stat): {e}"))?
            .permissions();
        perms.set_readonly(true);
        std::fs::set_permissions(&db, perms).map_err(|e| format!("block save: {e}"))?;
    }
    Ok(store)
}

/// Executes a journey's steps against an existing harness, recording per-step
/// snapshots and the event log, and on failure writes the failure bundle.
pub fn run_into(sim: &mut SimApp, journey: &Journey) -> Result<(), SimulationError> {
    sim.pump(vec![]);
    sim.record_step(0, "launch", "first frame").map_err(|e| {
        SimulationError::failed(journey.name, sim.seed, 0, e, sim.report.report_dir.clone())
    })?;
    for (i, step) in journey.steps.iter().enumerate() {
        let step_idx = i + 1;
        let (intent, target) = step.describe();
        let result = sim.execute(step);
        if let Err(message) = sim.record_step(step_idx, &intent, &target) {
            return Err(SimulationError::failed(
                journey.name,
                sim.seed,
                step_idx,
                message,
                sim.report.report_dir.clone(),
            ));
        }
        if let Err(message) = result {
            let _ = sim.report.fail(step_idx, &message);
            return Err(SimulationError::failed(
                journey.name,
                sim.seed,
                step_idx,
                message,
                sim.report.report_dir.clone(),
            ));
        }
    }
    Ok(())
}

#[cfg(test)]
/// Runs a journey headlessly: fresh temp store, harness, steps, report.
pub fn run(journey: &Journey, seed: u64) -> Result<Report, SimulationError> {
    let run_dir = run_dir_for(journey.name, seed);
    let _ = std::fs::remove_dir_all(&run_dir);
    let store = materialize(&run_dir, &journey.fixtures).map_err(|e| {
        SimulationError::failed(journey.name, seed, 0, e, PathBuf::from("sim-report"))
    })?;
    let mut sim = SimApp::build(journey.name, store, journey.persona, run_dir.clone(), seed)
        .map_err(|e| {
            SimulationError::failed(journey.name, seed, 0, e, PathBuf::from("sim-report"))
        })?;
    run_into(&mut sim, journey)?;
    let report = sim.report;
    let _ = report.write_summary();
    // The throwaway store isn't needed after a successful run (the report and
    // failure bundle already carry the data files), so don't leave it in the
    // system temp dir. On failure it is kept for inspection.
    let _ = std::fs::remove_dir_all(&run_dir);
    Ok(report)
}

/// Looks a journey up by name for the headed CLI (feature `sim` only).
#[cfg(feature = "sim")]
pub fn by_name(name: &str) -> Option<Journey> {
    let all: Vec<Journey> = vec![
        first_run_explorer(),
        power_organizer_add(),
        power_organizer_edit_delete(),
        power_organizer_drag(),
        error_corrupt(),
        error_read_only(),
        error_reserved_category(),
        theme_hopper(),
        vault_happy_unlock_copy(),
        vault_wrong_password(),
        vault_censored_while_locked(),
        vault_lock_unlock_cycle(),
    ];
    all.into_iter().find(|j| j.name == name)
}

fn snippet(id: u64, title: &str, category: &str, body: &str) -> Snippet {
    Snippet {
        id,
        title: title.to_string(),
        description: String::new(),
        category: category.to_string(),
        body: body.to_string(),
        protection: None,
    }
}

// ---------------------------------------------------------------------------
// Vault journey helpers
//
// The vault journeys below start from a fixture built by encrypting a real
// body through `crate::vault` — the same code path the app uses when the user
// protects a snippet — so the on-disk data is exactly what a real protect
// would write. Unlocking is driven through the real UI: the harness's
// `type_into` locates fields by their hint text, but the unlock modal's
// password field is password-masked with no in-field hint, so these helpers
// click a point inside the field (it sits directly right of its "Password"
// label) and type into the focused field directly.
// ---------------------------------------------------------------------------

/// The vault password every vault fixture is created with; a journey "knows" it
/// the way the app's user would and types it into the unlock modal.
const VAULT_PASSWORD: &str = "password1";

/// Types into the unlock modal's password field, clearing any existing content
/// first (e.g. the failed first attempt in the wrong-password journey). The
/// field is focused directly because it has no hint text for `type_into` to
/// locate; per-character text events go through the virtual clock like every
/// other harness input.
fn type_password(sim: &mut SimApp, password: &str) -> Result<(), String> {
    let label = sim.locate("Password")?;
    let field_center = egui::Pos2::new(label.right() + 118.0, label.center().y);
    sim.click(field_center)?;
    sim.clear_focused_field()?;
    for ch in password.chars() {
        sim.type_char(ch);
    }
    Ok(())
}

/// A one-snippet fixture whose card is protected under a vault created with
/// [`VAULT_PASSWORD`]. The vault metadata rides in `config.json`; the snippet's
/// `body` is empty on disk and its plaintext exists only in the ciphertext.
fn protected_fixture(body: &'static str, title: &'static str) -> Fixtures {
    let (meta, key) = vault::create_vault(VAULT_PASSWORD).expect("vault fixture: create vault");
    let protection = vault::encrypt_body(&key, body).expect("vault fixture: encrypt body");
    Fixtures::library_with_vault(
        vec![Snippet {
            id: 1,
            title: title.to_string(),
            description: String::new(),
            category: "Git".to_string(),
            body: String::new(),
            protection: Some(protection),
        }],
        &["Git"],
        "Dark",
        Some(meta),
    )
}

/// Asserts the persisted library still holds the protected snippet encrypted:
/// empty plaintext `body`, `protection` carrying the expected hint plus nonce
/// and ciphertext, and the plaintext nowhere in the file.
fn expect_protected_store(body: &'static str, hint: &'static str) -> Box<StorePredicate> {
    Box::new(move |store: &Store| match store.load_snippets() {
        crate::storage::Load::Loaded(snips) => {
            let s = snips
                .iter()
                .find(|s| s.id == 1)
                .ok_or("snippet 1 missing")?;
            if !s.body.is_empty() {
                return Err(format!(
                    "protected body must be empty on disk, got {:?}",
                    s.body
                ));
            }
            let p = s
                .protection
                .as_ref()
                .ok_or("protection missing on stored snippet")?;
            if p.hint != hint {
                return Err(format!("stored hint {:?}, expected {hint:?}", p.hint));
            }
            if p.nonce.is_empty() || p.ciphertext.is_empty() {
                return Err("protection must carry a nonce and ciphertext".into());
            }
            if s.protection.is_none() {
                return Err("protection must be present".into());
            }
            let raw = std::fs::read(crate::sqlite::db_path_for(
                store
                    .snippets_path
                    .parent()
                    .unwrap_or_else(|| Path::new(".")),
            ))
            .map_err(|e| e.to_string())?;
            let raw = String::from_utf8_lossy(&raw);
            if raw.contains(body) {
                return Err("plaintext body leaked into the database file".into());
            }
            Ok(())
        }
        _other => Err("snippets.json should exist".into()),
    })
}

/// Asserts `config.json` still carries the vault metadata (salt + canary), i.e.
/// the vault survives the session and is never dropped from the persisted file.
fn expect_vault_in_config() -> Box<StorePredicate> {
    Box::new(|store: &Store| match store.load_config() {
        crate::storage::Load::Loaded(cfg) => {
            if cfg.vault.is_some() {
                Ok(())
            } else {
                Err("vault metadata missing from config.json".into())
            }
        }
        _other => Err("config.json should exist".into()),
    })
}

// ---------------------------------------------------------------------------
// Journey library
// ---------------------------------------------------------------------------

/// FirstRunExplorer: explore the seeded library, search, filter, copy (with the
/// transient "Copied" feedback driven by the virtual clock), and open/cancel
/// the editor without mutating anything on disk.
pub fn first_run_explorer() -> Journey {
    Journey {
        name: "first-run-explorer",
        persona: FIRST_RUN_EXPLORER,
        fixtures: Fixtures::seed_defaults(),
        steps: vec![
            Step::Intent(Intent::ExpectVisible("Update all repos (PowerShell)")),
            Step::Intent(Intent::ExpectVisible("Summarize this conversation")),
            // Search narrows the grid.
            Step::Intent(Intent::TypeInto(
                "Search title, text, category\u{2026}",
                "git",
            )),
            Step::Intent(Intent::Wait(150)),
            Step::Intent(Intent::ExpectVisible("Discard all local changes")),
            Step::Intent(Intent::ExpectAbsent("Summarize this conversation")),
            // Clear the search and see everything again.
            Step::Intent(Intent::ClearFocusedField),
            Step::Intent(Intent::Wait(150)),
            Step::Intent(Intent::ExpectVisible("Summarize this conversation")),
            // Category filter narrows the grid.
            Step::Intent(Intent::ClickFirstText("All")),
            Step::Intent(Intent::Wait(60)),
            Step::Intent(Intent::ClickText("Prompt")),
            Step::Intent(Intent::Wait(150)),
            Step::Intent(Intent::ExpectVisible("Generate documentation")),
            Step::Intent(Intent::ExpectAbsent("Update all repos (bash)")),
            // Back to All.
            Step::Intent(Intent::ClickFirstText("Prompt")),
            Step::Intent(Intent::Wait(60)),
            Step::Intent(Intent::ClickText("All")),
            Step::Intent(Intent::Wait(150)),
            Step::Intent(Intent::ExpectVisible("Update all repos (bash)")),
            // Copy a card: clipboard contents + transient "Copied" feedback.
            Step::Intent(Intent::ClickCardCopy("Summarize this conversation")),
            Step::Custom(Box::new(|sim| {
                if sim
                    .clipboard()
                    .contains("Summarize our conversation so far")
                {
                    Ok(())
                } else {
                    Err(format!("clipboard was {:?}", sim.clipboard()))
                }
            })),
            Step::Intent(Intent::ExpectVisible("Copied")),
            Step::Intent(Intent::Wait(1400)),
            Step::Intent(Intent::ExpectAbsent("Copied")),
            // Open and cancel the editor.
            Step::Intent(Intent::ClickText("+ New")),
            Step::Intent(Intent::ExpectVisible("New snippet")),
            Step::Intent(Intent::ClickText("Cancel")),
            Step::Intent(Intent::ExpectAbsent("New snippet")),
            // Nothing was mutated.
            Step::Intent(Intent::ExpectStore(Box::new(|store: &Store| {
                match store.load_snippets() {
                    crate::storage::Load::Loaded(snips) => {
                        let ids: Vec<u64> = snips.iter().map(|s| s.id).collect();
                        if ids == [1, 2, 3, 4, 5, 6] {
                            Ok(())
                        } else {
                            Err(format!("library changed unexpectedly: {ids:?}"))
                        }
                    }
                    _other => Err("snippets.json should exist".into()),
                }
            }))),
        ],
    }
}

/// Which category a new snippet should get in the PowerOrganizer adds.
enum CategoryTarget {
    Default,
    Set(&'static str),
    New(&'static str),
}

/// Builds the steps for adding a snippet through the real editor UI.
fn add_snippet_steps(
    title: &'static str,
    body: &'static str,
    category: CategoryTarget,
) -> Vec<Step> {
    let mut steps = vec![
        Step::Intent(Intent::ClickText("+ New")),
        Step::Intent(Intent::ExpectVisible("New snippet")),
        Step::Intent(Intent::TypeInto("Title", title)),
        Step::Intent(Intent::TypeInto("Content", body)),
    ];
    match category {
        CategoryTarget::Default => {}
        CategoryTarget::Set(cat) => steps.push(Step::Intent(Intent::SetEditorCategory(cat))),
        CategoryTarget::New(cat) => steps.push(Step::Intent(Intent::AddEditorCategory(cat))),
    }
    steps.push(Step::Intent(Intent::ClickText("Save")));
    steps.push(Step::Intent(Intent::ExpectAbsent("Save")));
    steps
}

/// PowerOrganizer: add ~20 snippets across new and existing categories,
/// asserting the persisted library after the batch and at the end.
pub fn power_organizer_add() -> Journey {
    let mut steps = Vec::new();
    for (title, body, target) in [
        (
            "Git log",
            "git log --oneline --graph",
            CategoryTarget::Default,
        ),
        ("Git branch", "git branch -vv", CategoryTarget::Default),
        (
            "Git stash",
            "git stash push -m work",
            CategoryTarget::Default,
        ),
        ("Docker ps", "docker ps -a", CategoryTarget::New("Docker")),
        (
            "Docker compose up",
            "docker compose up -d",
            CategoryTarget::New("Docker"),
        ),
        (
            "Docker logs",
            "docker logs -f app",
            CategoryTarget::New("Docker"),
        ),
        (
            "Docker prune",
            "docker system prune -af",
            CategoryTarget::New("Docker"),
        ),
        (
            "Git rebase",
            "git rebase -i HEAD~3",
            CategoryTarget::Set("Git"),
        ),
        (
            "Git blame",
            "git blame -L 10,20 file",
            CategoryTarget::Set("Git"),
        ),
        ("Git clean", "git clean -fd", CategoryTarget::Set("Git")),
        ("Git bisect", "git bisect start", CategoryTarget::Set("Git")),
        (
            "Prompt summarize",
            "summarize this thread",
            CategoryTarget::New("Prompt"),
        ),
        (
            "Prompt draft email",
            "draft a polite rejection",
            CategoryTarget::New("Prompt"),
        ),
        (
            "Prompt code review",
            "review this diff",
            CategoryTarget::New("Prompt"),
        ),
        (
            "Prompt unit tests",
            "write unit tests for fn",
            CategoryTarget::New("Prompt"),
        ),
        (
            "Docker exec",
            "docker exec -it app sh",
            CategoryTarget::Default,
        ),
        (
            "Docker cp",
            "docker cp app:/etc/app.conf .",
            CategoryTarget::Default,
        ),
        (
            "Docker stop",
            "docker stop $(docker ps -q)",
            CategoryTarget::Default,
        ),
        (
            "Docker inspect",
            "docker inspect app",
            CategoryTarget::Default,
        ),
        (
            "Docker volume ls",
            "docker volume ls",
            CategoryTarget::Default,
        ),
    ] {
        steps.extend(add_snippet_steps(title, body, target));
    }
    steps.push(Step::Intent(Intent::Wait(200)));
    steps.push(Step::Intent(Intent::ExpectVisible("Git log")));
    steps.push(Step::Intent(Intent::ExpectStore(Box::new(
        |store: &Store| match store.load_snippets() {
            crate::storage::Load::Loaded(snips) => {
                if snips.len() != 22 {
                    return Err(format!(
                        "expected 22 snippets (2 starters + 20 added), got {}",
                        snips.len()
                    ));
                }
                let ids: Vec<u64> = snips.iter().map(|s| s.id).collect();
                let mut sorted = ids.clone();
                sorted.sort_unstable();
                sorted.dedup();
                if sorted.len() != 22 || sorted[0] != 1 {
                    return Err(format!("ids must be 1..=22 unique, got {ids:?}"));
                }
                let cats: std::collections::HashSet<&str> =
                    snips.iter().map(|s| s.category.as_str()).collect();
                for want in ["Git", "Docker", "Prompt"] {
                    if !cats.contains(want) {
                        return Err(format!("category {want} missing: {cats:?}"));
                    }
                }
                Ok(())
            }
            _other => Err("snippets.json should exist".into()),
        },
    ))));
    Journey {
        name: "power-organizer-add",
        persona: POWER_ORGANIZER,
        fixtures: Fixtures::library(
            vec![
                snippet(1, "Starter one", "Git", "starter body one"),
                snippet(2, "Starter two", "Git", "starter body two"),
            ],
            &["Git"],
            "Dark",
        ),
        steps,
    }
}

/// PowerOrganizer: edit a snippet's title and body, then delete another, with
/// the persisted file checked after each mutation.
pub fn power_organizer_edit_delete() -> Journey {
    Journey {
        name: "power-organizer-edit-delete",
        persona: POWER_ORGANIZER,
        fixtures: Fixtures::library(
            vec![
                snippet(1, "Red", "Git", "red body"),
                snippet(2, "Green", "Git", "green body"),
                snippet(3, "Blue", "Prompt", "blue body"),
                snippet(4, "Yellow", "Git", "yellow body"),
                snippet(5, "Purple", "Prompt", "purple body"),
            ],
            &["Git", "Prompt"],
            "Dark",
        ),
        steps: vec![
            Step::Intent(Intent::ExpectVisible("Blue")),
            // Edit: retitle and replace the body.
            Step::Intent(Intent::ClickCardEdit("Blue")),
            Step::Intent(Intent::ExpectVisible("Edit snippet")),
            Step::Intent(Intent::ReplaceField("Title", "Azure")),
            Step::Intent(Intent::ReplaceField("Content", "azure new body")),
            Step::Intent(Intent::ClickText("Save")),
            Step::Intent(Intent::ExpectAbsent("Edit snippet")),
            Step::Intent(Intent::ExpectStore(Box::new(|store: &Store| {
                match store.load_snippets() {
                    crate::storage::Load::Loaded(snips) => {
                        let s = snips.iter().find(|s| s.id == 3).ok_or("id 3 missing")?;
                        if s.title != "Azure" || s.body != "azure new body" {
                            return Err(format!(
                                "edit not persisted: {:?}",
                                (s.title.clone(), s.body.clone())
                            ));
                        }
                        if snips.len() != 5 {
                            return Err(format!("expected 5 snippets, got {}", snips.len()));
                        }
                        Ok(())
                    }
                    _other => Err("snippets.json should exist".into()),
                }
            }))),
            // A second copy checks the clipboard through the intent vocabulary.
            Step::Intent(Intent::ClickCardCopy("Red")),
            Step::Intent(Intent::ExpectClipboard("red body")),
            // Delete: confirm-then-delete flow.
            Step::Intent(Intent::ClickCardEdit("Purple")),
            Step::Intent(Intent::ClickText("Delete")),
            Step::Intent(Intent::ClickText("Confirm delete")),
            Step::Intent(Intent::ExpectAbsent("Purple")),
            Step::Intent(Intent::ExpectStore(Box::new(|store: &Store| {
                match store.load_snippets() {
                    crate::storage::Load::Loaded(snips) => {
                        if snips.iter().any(|s| s.id == 5) {
                            return Err("id 5 should have been deleted".into());
                        }
                        if snips.len() != 4 {
                            return Err(format!("expected 4 snippets, got {}", snips.len()));
                        }
                        Ok(())
                    }
                    _other => Err("snippets.json should exist".into()),
                }
            }))),
        ],
    }
}

/// PowerOrganizer: drag-and-drop reordering, unfiltered and within a filtered
/// view, each drop flowing through the real `DragMachine` and persisting.
pub fn power_organizer_drag() -> Journey {
    Journey {
        name: "power-organizer-drag",
        persona: POWER_ORGANIZER,
        fixtures: Fixtures::library(
            vec![
                snippet(1, "Alpha", "Git", "alpha"),
                snippet(2, "Excess", "Prompt", "excess"),
                snippet(3, "Beta", "Git", "beta"),
                snippet(4, "Yield", "Prompt", "yield"),
                snippet(5, "Gamma", "Git", "gamma"),
                snippet(6, "Zebra", "Prompt", "zebra"),
            ],
            &["Git", "Prompt"],
            "Dark",
        ),
        steps: vec![
            Step::Intent(Intent::Wait(120)),
            // Unfiltered: drag Beta before Alpha.
            Step::Intent(Intent::DragCard("Beta", "Alpha")),
            Step::Intent(Intent::Wait(120)),
            Step::Intent(Intent::ExpectStore(Box::new(|store: &Store| {
                match store.load_snippets() {
                    crate::storage::Load::Loaded(snips) => {
                        let ids: Vec<u64> = snips.iter().map(|s| s.id).collect();
                        if ids == [3, 1, 2, 4, 5, 6] {
                            Ok(())
                        } else {
                            Err(format!("unfiltered reorder wrong: {ids:?}"))
                        }
                    }
                    _other => Err("snippets.json should exist".into()),
                }
            }))),
            // Filtered: only Git cards are visible; drag Gamma before Alpha.
            Step::Intent(Intent::ClickFirstText("All")),
            Step::Intent(Intent::Wait(60)),
            Step::Intent(Intent::ClickText("Git")),
            Step::Intent(Intent::Wait(150)),
            Step::Intent(Intent::ExpectVisible("Gamma")),
            Step::Intent(Intent::ExpectAbsent("Zebra")),
            Step::Intent(Intent::DragCard("Gamma", "Alpha")),
            Step::Intent(Intent::Wait(120)),
            Step::Intent(Intent::ExpectStore(Box::new(|store: &Store| {
                match store.load_snippets() {
                    crate::storage::Load::Loaded(snips) => {
                        let ids: Vec<u64> = snips.iter().map(|s| s.id).collect();
                        if ids == [3, 5, 1, 2, 4, 6] {
                            Ok(())
                        } else {
                            Err(format!("filtered reorder wrong: {ids:?}"))
                        }
                    }
                    _other => Err("snippets.json should exist".into()),
                }
            }))),
        ],
    }
}

/// ErrorHandler: a corrupt `snippets.json` is backed up and replaced by the
/// seeded defaults, with a banner telling the user where the file went.
pub fn error_corrupt() -> Journey {
    Journey {
        name: "error-corrupt",
        persona: ERROR_HANDLER,
        fixtures: Fixtures::corrupt(),
        steps: vec![
            Step::Intent(Intent::ExpectVisible("couldn't be read")),
            Step::Intent(Intent::ExpectVisible("Update all repos (PowerShell)")),
            Step::Intent(Intent::ExpectStore(Box::new(|store: &Store| {
                match store.load_snippets() {
                    crate::storage::Load::Loaded(snips) => {
                        if snips.len() != 6 {
                            return Err(format!("expected seeded defaults, got {}", snips.len()));
                        }
                        Ok(())
                    }
                    _other => Err("defaults should exist".into()),
                }
            }))),
            Step::Custom(Box::new(|sim| {
                let backup = sim
                    .store
                    .snippets_path
                    .with_file_name("snippets.json.corrupt");
                if !backup.exists() {
                    return Err("snippets.json.corrupt backup is missing".into());
                }
                let contents = std::fs::read_to_string(&backup).unwrap_or_default();
                if contents == "this is definitely not json" {
                    Ok(())
                } else {
                    Err(format!("backup bytes wrong: {contents:?}"))
                }
            })),
        ],
    }
}

/// ErrorHandler: a save-failure (simulated read-only data dir) shows the
/// warning banner, keeps the in-memory library intact, and writes no partial
/// file.
pub fn error_read_only() -> Journey {
    let mut steps = add_snippet_steps("Third", "third body", CategoryTarget::Default);
    steps.push(Step::Intent(Intent::ExpectVisible(
        "Couldn't save snippets",
    )));
    steps.push(Step::Intent(Intent::ExpectVisible("Third")));
    steps.push(Step::Intent(Intent::ExpectStore(Box::new(
        |store: &Store| match store.load_snippets() {
            crate::storage::Load::Loaded(snips) => {
                let ids: Vec<u64> = snips.iter().map(|s| s.id).collect();
                if ids == [1, 2] {
                    Ok(())
                } else {
                    Err(format!("nothing may be persisted, got {ids:?}"))
                }
            }
            _other => Err("snippets.json should exist".into()),
        },
    ))));
    steps.push(Step::Custom(Box::new(|sim| {
        // A blocked save must leave no partial temp file behind (the old JSON
        // atomic write left a `.tmp`; the SQLite path writes straight to the
        // read-only db and fails before producing any sidecar).
        let dir = sim.store.snippets_path.parent().expect("store dir");
        let stray: Vec<String> = std::fs::read_dir(dir)
            .map(|entries| {
                entries
                    .flatten()
                    .map(|e| e.file_name().to_string_lossy().into_owned())
                    .filter(|n| n.ends_with(".tmp"))
                    .collect()
            })
            .unwrap_or_default();
        if stray.is_empty() {
            Ok(())
        } else {
            Err(format!("unexpected .tmp entries: {stray:?}"))
        }
    })));
    Journey {
        name: "error-read-only",
        persona: ERROR_HANDLER,
        fixtures: Fixtures::read_only(
            vec![
                snippet(1, "First", "Git", "first body"),
                snippet(2, "Second", "Git", "second body"),
            ],
            &["Git"],
            "Dark",
        ),
        steps,
    }
}

/// ErrorHandler: the reserved category name "All" is rejected in the header
/// form with the form kept open; a blank inline category in the editor is
/// rejected by keeping the form open without adding anything; a valid category
/// then succeeds. All through the real UI.
pub fn error_reserved_category() -> Journey {
    Journey {
        name: "error-reserved-category",
        persona: ERROR_HANDLER,
        fixtures: Fixtures::seed_defaults(),
        steps: vec![
            // Header: "All" is reserved.
            Step::Intent(Intent::OpenHeaderCategoryForm),
            Step::Intent(Intent::SubmitHeaderCategory("all")),
            Step::Intent(Intent::ExpectVisible(
                "\"All\" is reserved and can't be used as a category",
            )),
            Step::Intent(Intent::ExpectVisible("Add")),
            // Replace the reserved "all" text with a valid category name.
            Step::Custom(Box::new(|sim: &mut SimApp| {
                sim.replace_into("all", "Docker")?;
                sim.press_key(egui::Key::Enter)
            })),
            Step::Intent(Intent::ExpectVisible("Docker")),
            Step::Intent(Intent::ExpectStore(Box::new(|store: &Store| {
                match store.load_config() {
                    crate::storage::Load::Loaded(config) => {
                        if config.categories.iter().any(|c| c == "Docker") {
                            Ok(())
                        } else {
                            Err(format!("Docker missing from {:?}", config.categories))
                        }
                    }
                    _other => Err("config.json should exist".into()),
                }
            }))),
            // Editor: blank inline category keeps the form open without adding.
            Step::Intent(Intent::ClickText("+ New")),
            Step::Intent(Intent::ExpectVisible("New snippet")),
            Step::Custom(Box::new(|sim| sim.begin_editor_new_category())),
            Step::Intent(Intent::ClickText("Add")),
            Step::Intent(Intent::ExpectVisible("Add")),
            Step::Intent(Intent::ExpectVisible("Save")),
            // Editor: reserved name shows the same error and keeps the form open.
            Step::Intent(Intent::TypeInto("New category", "all")),
            Step::Intent(Intent::ClickText("Add")),
            Step::Intent(Intent::ExpectVisible(
                "\"All\" is reserved and can't be used as a category",
            )),
            // Replace the reserved "all" text with a valid category, then submit.
            Step::Custom(Box::new(|sim: &mut SimApp| {
                sim.replace_into("all", "Docker")?;
                sim.click_text("Add")
            })),
            Step::Intent(Intent::TypeInto("Title", "Docker tip")),
            Step::Intent(Intent::TypeInto("Content", "docker info")),
            Step::Intent(Intent::ClickText("Save")),
            Step::Intent(Intent::ExpectAbsent("New snippet")),
            Step::Intent(Intent::ExpectStore(Box::new(|store: &Store| {
                match store.load_snippets() {
                    crate::storage::Load::Loaded(snips) => {
                        let has_docker = snips.iter().any(|s| s.category == "Docker");
                        if snips.len() == 7 && has_docker {
                            Ok(())
                        } else {
                            Err(format!(
                                "expected 7 snippets with Docker, got {}",
                                snips.len()
                            ))
                        }
                    }
                    _other => Err("snippets.json should exist".into()),
                }
            }))),
        ],
    }
}

/// ThemeHopper: switch themes, verify `config.json` persists the selection,
/// then drop and rebuild the app from the same store and verify the restarted
/// app applied the persisted theme.
pub fn theme_hopper() -> Journey {
    Journey {
        name: "theme-hopper",
        persona: THEME_HOPPER,
        fixtures: Fixtures::library(
            vec![
                snippet(1, "Alpha", "Git", "alpha body"),
                snippet(2, "Beta", "Git", "beta body"),
            ],
            &["Git"],
            "Dark",
        ),
        steps: vec![
            Step::Intent(Intent::ExpectVisible("Dark")),
            Step::Intent(Intent::ClickText("Dark")),
            Step::Intent(Intent::Wait(60)),
            Step::Intent(Intent::ClickText("Nord")),
            Step::Intent(Intent::ExpectVisible("Nord")),
            Step::Intent(Intent::ExpectStore(Box::new(|store: &Store| {
                match store.load_config() {
                    crate::storage::Load::Loaded(config) => {
                        if config.theme == "Nord" {
                            Ok(())
                        } else {
                            Err(format!("theme should be Nord, got {:?}", config.theme))
                        }
                    }
                    _other => Err("config.json should exist".into()),
                }
            }))),
            Step::Intent(Intent::ClickText("Nord")),
            Step::Intent(Intent::Wait(60)),
            Step::Intent(Intent::ClickText("Dracula")),
            Step::Intent(Intent::ExpectStore(Box::new(|store: &Store| {
                match store.load_config() {
                    crate::storage::Load::Loaded(config) => {
                        if config.theme == "Dracula" {
                            Ok(())
                        } else {
                            Err(format!("theme should be Dracula, got {:?}", config.theme))
                        }
                    }
                    _other => Err("config.json should exist".into()),
                }
            }))),
            // Simulated restart: drop the app, rebuild from the same store.
            Step::Intent(Intent::RebuildFromStore),
            Step::Intent(Intent::ExpectVisible("Dracula")),
            Step::Intent(Intent::ExpectVisible("Alpha")),
            Step::Intent(Intent::ExpectStore(Box::new(|store: &Store| {
                match store.load_config() {
                    crate::storage::Load::Loaded(config) => {
                        if config.theme == "Dracula" {
                            Ok(())
                        } else {
                            Err(format!(
                                "theme should survive restart, got {:?}",
                                config.theme
                            ))
                        }
                    }
                    _other => Err("config.json should exist".into()),
                }
            }))),
        ],
    }
}

/// VaultHappyUnlockCopy: a protected card prints its masked preview while the
/// vault is locked; Copy gates behind the unlock modal, and once the right
/// password is entered the suspended copy runs — the decrypted body lands on
/// the clipboard and the transient "Copied" feedback appears. The on-disk
/// snippet stays encrypted throughout.
pub fn vault_happy_unlock_copy() -> Journey {
    const BODY: &str = "super-secret-api-key-123456"; // >= 12 chars: hint "super"
    Journey {
        name: "vault-happy-unlock-copy",
        persona: POWER_ORGANIZER,
        fixtures: protected_fixture(BODY, "Secret"),
        steps: vec![
            // While locked: censored card (hint + bullets), lock chip, locked vault.
            Step::Intent(Intent::ExpectVisible("Secret")),
            Step::Intent(Intent::ExpectVisible("Vault locked")),
            Step::Custom(Box::new(|sim| {
                sim.expect_visible(&vault::masked_preview("super"))
            })),
            Step::Intent(Intent::ExpectVisible("PROTECTED")),
            // Copy gates behind the unlock prompt.
            Step::Intent(Intent::ClickCardCopy("Secret")),
            Step::Intent(Intent::ExpectVisible("Unlock vault")),
            Step::Intent(Intent::ExpectAbsent("Copied")),
            Step::Custom(Box::new(|sim| type_password(sim, VAULT_PASSWORD))),
            Step::Intent(Intent::ClickText("Unlock")),
            Step::Intent(Intent::Wait(60)),
            // The suspended copy ran: plaintext on the clipboard, "Copied" shown.
            Step::Intent(Intent::ExpectClipboard(BODY)),
            Step::Intent(Intent::ExpectVisible("Copied")),
            Step::Intent(Intent::Wait(1400)),
            Step::Intent(Intent::ExpectAbsent("Copied")),
            // The store still holds the encrypted form and the vault metadata.
            Step::Intent(Intent::ExpectStore(expect_protected_store(BODY, "super"))),
            Step::Intent(Intent::ExpectStore(expect_vault_in_config())),
        ],
    }
}

/// VaultWrongPassword: entering a wrong password into the unlock prompt is
/// rejected inline with the vault kept locked and nothing copied; retrying with
/// the correct password then succeeds and runs the pending copy.
pub fn vault_wrong_password() -> Journey {
    const BODY: &str = "sensitive-access-token-abcdef"; // hint "sensi"
    Journey {
        name: "vault-wrong-password",
        persona: ERROR_HANDLER,
        fixtures: protected_fixture(BODY, "Secret"),
        steps: vec![
            Step::Intent(Intent::ExpectVisible("Secret")),
            Step::Intent(Intent::ClickCardCopy("Secret")),
            Step::Intent(Intent::ExpectVisible("Unlock vault")),
            // A wrong password is rejected inline; the clipboard stays untouched.
            Step::Custom(Box::new(|sim| type_password(sim, "wrongpw"))),
            Step::Intent(Intent::ClickText("Unlock")),
            Step::Intent(Intent::ExpectVisible("Wrong password. Try again.")),
            Step::Custom(Box::new(|sim| {
                if sim.clipboard().is_empty() {
                    Ok(())
                } else {
                    Err(format!(
                        "wrong password must not copy anything, clipboard {:?}",
                        sim.clipboard()
                    ))
                }
            })),
            // Retry with the correct password: the pending copy now runs.
            Step::Custom(Box::new(|sim| type_password(sim, VAULT_PASSWORD))),
            Step::Intent(Intent::ClickText("Unlock")),
            Step::Intent(Intent::Wait(60)),
            Step::Intent(Intent::ExpectClipboard(BODY)),
            Step::Intent(Intent::ExpectVisible("Copied")),
            Step::Intent(Intent::ExpectAbsent("Wrong password. Try again.")),
            Step::Intent(Intent::ExpectStore(expect_protected_store(BODY, "sensi"))),
            Step::Intent(Intent::ExpectStore(expect_vault_in_config())),
        ],
    }
}

/// VaultCensoredWhileLocked: with the vault locked, a protected card renders
/// only hint + masking bullets (and its PROTECTED chip) — the plaintext body
/// appears nowhere in the visible frames — and searching for a string that
/// exists only inside the protected body matches no cards, while the card's
/// title remains searchable and still shows the masked preview.
pub fn vault_censored_while_locked() -> Journey {
    const BODY: &str = "plains3cret-QUERY-42-never-rendered"; // hint "plain"
    const SECRET: &str = "QUERY-42"; // exists only inside the protected body
    Journey {
        name: "vault-censored-while-locked",
        persona: FIRST_RUN_EXPLORER,
        fixtures: protected_fixture(BODY, "Secret"),
        steps: vec![
            // Masked preview (hint + bullets) and the lock chip render...
            Step::Intent(Intent::ExpectVisible("Secret")),
            Step::Custom(Box::new(|sim| {
                sim.expect_visible(&vault::masked_preview("plain"))
            })),
            Step::Intent(Intent::ExpectVisible("PROTECTED")),
            // ...and the plaintext appears nowhere in the visible frame.
            Step::Custom(Box::new(|sim| {
                for (t, _) in sim.visible_texts() {
                    if t.contains(BODY) {
                        return Err(format!("protected body leaked into the UI: {t:?}"));
                    }
                }
                Ok(())
            })),
            // Search: a string that only lives in the protected body matches
            // nothing; clearing the search brings the (still masked) card back.
            Step::Intent(Intent::TypeInto(
                "Search title, text, category\u{2026}",
                SECRET,
            )),
            Step::Intent(Intent::Wait(150)),
            Step::Intent(Intent::ExpectAbsent("Secret")),
            Step::Intent(Intent::ClearFocusedField),
            Step::Intent(Intent::Wait(150)),
            Step::Intent(Intent::ExpectVisible("Secret")),
            // The on-disk snippet is still encrypted; the vault metadata persists.
            Step::Intent(Intent::ExpectStore(expect_protected_store(BODY, "plain"))),
            Step::Intent(Intent::ExpectStore(expect_vault_in_config())),
        ],
    }
}

/// VaultLockUnlockCycle: unlocking lasts for the session — protected actions
/// proceed without prompting — until the top bar's Lock control re-locks the
/// vault, after which a protected action prompts for the password again.
pub fn vault_lock_unlock_cycle() -> Journey {
    const BODY: &str = "session-token-rogue-123456"; // hint "sessi"
    Journey {
        name: "vault-lock-unlock-cycle",
        persona: ERROR_HANDLER,
        fixtures: protected_fixture(BODY, "Secret"),
        steps: vec![
            // Every launch starts locked; no Lock control exists yet.
            Step::Intent(Intent::ExpectVisible("Vault locked")),
            Step::Intent(Intent::ExpectAbsent("Lock Vault")),
            // Unlock through the prompt; the suspended copy runs immediately.
            Step::Intent(Intent::ClickCardCopy("Secret")),
            Step::Intent(Intent::ExpectVisible("Unlock vault")),
            Step::Custom(Box::new(|sim| type_password(sim, VAULT_PASSWORD))),
            Step::Intent(Intent::ClickText("Unlock")),
            Step::Intent(Intent::Wait(60)),
            Step::Intent(Intent::ExpectClipboard(BODY)),
            Step::Intent(Intent::ExpectVisible("Vault unlocked")),
            Step::Intent(Intent::ExpectVisible("Lock Vault")),
            // While unlocked a protected action proceeds with no prompt.
            Step::Intent(Intent::ClickCardCopy("Secret")),
            Step::Intent(Intent::ExpectClipboard(BODY)),
            Step::Intent(Intent::ExpectAbsent("Unlock vault")),
            // The top-bar Lock button drops the key mid-session...
            Step::Intent(Intent::ClickText("Lock Vault")),
            Step::Intent(Intent::ExpectVisible("Vault locked")),
            Step::Intent(Intent::ExpectAbsent("Lock Vault")),
            // ...so a protected action prompts again.
            Step::Intent(Intent::ClickCardCopy("Secret")),
            Step::Intent(Intent::ExpectVisible("Unlock vault")),
            Step::Intent(Intent::ExpectStore(expect_protected_store(BODY, "sessi"))),
            Step::Intent(Intent::ExpectStore(expect_vault_in_config())),
        ],
    }
}

#[cfg(test)]
/// A deliberately broken journey used by the meta-test.
fn meta_broken() -> Journey {
    Journey {
        name: "meta-broken",
        persona: FIRST_RUN_EXPLORER,
        fixtures: Fixtures::seed_defaults(),
        steps: vec![Step::Intent(Intent::ExpectVisible(
            "this text is never rendered by the app",
        ))],
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sim::report::EventRecord;

    fn run_ok(journey: &Journey, seed: u64) -> Report {
        match run(journey, seed) {
            Ok(report) => report,
            Err(err) => panic!(
                "journey {} failed at step {}: {}",
                err.journey, err.step, err.message
            ),
        }
    }

    fn normalized(report: &Report) -> String {
        report
            .events
            .iter()
            .map(|e| serde_json::to_string(e).unwrap_or_default())
            .collect::<Vec<String>>()
            .join("\n")
    }

    #[test]
    fn sim_journeys_first_run_explorer() {
        run_ok(&first_run_explorer(), 42);
    }

    #[test]
    fn sim_journeys_power_organizer_add() {
        run_ok(&power_organizer_add(), 43);
    }

    #[test]
    fn sim_journeys_power_organizer_edit_delete() {
        run_ok(&power_organizer_edit_delete(), 44);
    }

    #[test]
    fn sim_journeys_power_organizer_drag() {
        run_ok(&power_organizer_drag(), 45);
    }

    #[test]
    fn sim_journeys_error_corrupt() {
        run_ok(&error_corrupt(), 46);
    }

    #[test]
    fn sim_journeys_error_read_only() {
        run_ok(&error_read_only(), 47);
    }

    #[test]
    fn sim_journeys_error_reserved_category() {
        run_ok(&error_reserved_category(), 48);
    }

    #[test]
    fn sim_journeys_theme_hopper() {
        run_ok(&theme_hopper(), 49);
    }

    #[test]
    fn sim_journeys_vault_happy_unlock_copy() {
        run_ok(&vault_happy_unlock_copy(), 50);
    }

    #[test]
    fn sim_journeys_vault_wrong_password() {
        run_ok(&vault_wrong_password(), 51);
    }

    #[test]
    fn sim_journeys_vault_censored_while_locked() {
        run_ok(&vault_censored_while_locked(), 52);
    }

    #[test]
    fn sim_journeys_vault_lock_unlock_cycle() {
        run_ok(&vault_lock_unlock_cycle(), 53);
    }

    /// Determinism: same journey + persona + seed produces byte-identical
    /// normalized event logs; a different seed produces a valid but different
    /// run.
    #[test]
    fn sim_journeys_determinism() {
        let journey = first_run_explorer();
        let a = report_of(run(&journey, 4242));
        let b = report_of(run(&journey, 4242));
        assert_eq!(
            normalized(&a),
            normalized(&b),
            "same seed must produce identical event logs"
        );
        let c = report_of(run(&journey, 4243));
        assert_ne!(
            normalized(&a),
            normalized(&c),
            "different seeds must differ (timing at least)"
        );
    }

    fn report_of(result: Result<Report, SimulationError>) -> Report {
        match result {
            Ok(r) => r,
            Err(e) => panic!("run failed at step {}: {}", e.step, e.message),
        }
    }

    /// After running a representative set of journeys, the real data directory
    /// (if it exists) is untouched.
    #[test]
    fn sim_journeys_isolation() {
        let dir = real_data_dir();
        let before = dir_snapshot(dir.as_deref());
        for journey in [
            first_run_explorer(),
            power_organizer_add(),
            error_corrupt(),
            error_read_only(),
            theme_hopper(),
        ] {
            let _ = run(&journey, 7777);
        }
        let after = dir_snapshot(dir.as_deref());
        assert_eq!(
            before, after,
            "journeys must never create/modify/delete files in the real data dir"
        );
    }

    /// The real data directory, without creating it (contrast `storage::data_dir`,
    /// which does create it).
    fn real_data_dir() -> Option<PathBuf> {
        if let Ok(appdata) = std::env::var("APPDATA") {
            if !appdata.is_empty() {
                return Some(PathBuf::from(appdata).join("CopyIt"));
            }
        }
        None
    }

    fn dir_snapshot(dir: Option<&Path>) -> Option<Vec<(String, u64)>> {
        let dir = dir?;
        if !dir.is_dir() {
            return Some(Vec::new());
        }
        let mut entries = std::fs::read_dir(dir)
            .ok()?
            .filter_map(|e| e.ok())
            .map(|e| {
                let name = e.file_name().to_string_lossy().into_owned();
                let len = e.metadata().map(|m| m.len()).unwrap_or(0);
                (name, len)
            })
            .collect::<Vec<_>>();
        entries.sort();
        Some(entries)
    }

    /// Meta-test: a journey with a deliberately false expectation fails and
    /// produces a complete failure bundle (event log, snapshot, seed, data
    /// files, repro command).
    #[test]
    fn sim_journeys_meta_failure_bundle() {
        let err = run(&meta_broken(), 99)
            .err()
            .expect("broken journey must fail");
        assert_eq!(err.seed, 99);
        assert_eq!(err.journey, "meta-broken");
        let dir = &err.report_dir;
        for file in [
            "event.log",
            "failure.log",
            "seed.txt",
            "REPRO.md",
            "copyit.db",
        ] {
            assert!(
                dir.join(file).exists(),
                "failure bundle missing {file} in {}",
                dir.display()
            );
        }
        assert!(dir.join("snapshots").is_dir(), "snapshots dir missing");
        let log = std::fs::read_to_string(dir.join("event.log")).unwrap();
        assert!(
            log.contains("expect_visible"),
            "event log missing failing intent"
        );
        let details = std::fs::read_to_string(dir.join("failure.log")).unwrap();
        assert!(
            details.contains("this text is never rendered"),
            "failure detail must name the failing expectation"
        );
        // A failing run must not claim success.
        assert!(!dir.join("summary.txt").exists());
    }

    /// The harness refuses to build an app whose store is not under the
    /// per-run temp directory, before any frame is pumped.
    #[test]
    fn sim_journeys_harness_refuses_real_store() {
        let outside = std::env::temp_dir().join("copyit-not-a-sim-dir");
        let _ = std::fs::remove_dir_all(&outside);
        std::fs::create_dir_all(&outside).unwrap();
        let store = Store::at(outside.clone());
        let err = SimApp::build(
            "probe",
            store,
            FIRST_RUN_EXPLORER,
            run_dir_for("probe", 1),
            1,
        )
        .err()
        .expect("must refuse a store outside the run dir");
        assert!(err.contains("refusing"), "unexpected message: {err}");
        let _ = std::fs::remove_dir_all(&outside);
    }

    /// The harness accepts a store under the per-run temp directory.
    #[test]
    fn sim_journeys_harness_accepts_per_run_store() {
        let run_dir = run_dir_for("accept-probe", 1);
        let _ = std::fs::remove_dir_all(&run_dir);
        std::fs::create_dir_all(&run_dir).unwrap();
        let store = Store::at(run_dir.clone());
        store.save_snippets(&[]).unwrap();
        store
            .save_config(&Config {
                categories: vec![],
                theme: "Dark".into(),
                vault: None,
            })
            .unwrap();
        let sim = SimApp::build("accept-probe", store, FIRST_RUN_EXPLORER, run_dir, 1);
        assert!(sim.is_ok());
        let sim = sim.unwrap();
        assert_eq!(sim.persona.name, FIRST_RUN_EXPLORER.name);
    }

    /// The state digest used in the event log is stable for the same store.
    #[test]
    fn sim_journeys_event_record_serializes_deterministically() {
        let rec = EventRecord {
            step: 1,
            intent: "click_text".into(),
            target: "Save".into(),
            input: "see snapshot".into(),
            time_ms: 150,
            state: "3 snippets, snippets.json 120B, config.json 80B".into(),
        };
        let a = serde_json::to_string(&rec).unwrap();
        let b = serde_json::to_string(&rec).unwrap();
        assert_eq!(a, b);
    }
}
