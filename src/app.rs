use crate::model::Snippet;
use crate::storage::{self, Config};
use crate::theme::Theme;
use eframe::egui;
use sha2::{Digest, Sha256};
use std::path::PathBuf;

/// Hashes a password using SHA256 for storage.
fn hash_password(password: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(password.as_bytes());
    format!("{:x}", hasher.finalize())
}

/// Verifies a password against a stored hash.
fn verify_password(password: &str, hash: &str) -> bool {
    hash_password(password) == hash
}

/// Prefix of the banner message for a failed `snippets.json` write. Shared so a later
/// successful snippet save can retire exactly its own error and nothing else.
const SNIPPETS_SAVE_ERROR: &str = "Couldn't save snippets";
/// Prefix of the banner message for a failed `config.json` write.
const CONFIG_SAVE_ERROR: &str = "Couldn't save settings";
/// Red used for the warning banner and the delete-confirmation button.
const WARNING_COLOR: egui::Color32 = egui::Color32::from_rgb(0xef, 0x44, 0x44);
/// Characters of a snippet body shown in a card's preview line.
const PREVIEW_CHARS: usize = 220;

// ---- Card grid geometry ----
// The grid is uniform, and both the renderer and the drag-and-drop hit-testing derive
// card positions from these numbers, so they live in one place.
/// Width of a card's inner content area.
const CARD_INNER_W: f32 = 300.0;
/// Height of a card's inner content area.
const CARD_INNER_H: f32 = 168.0;
/// The group frame around each card adds 10 px of inner margin on each side.
const CARD_FRAME_MARGIN: f32 = 20.0;
/// Full visible width of a card, frame included.
const CARD_W: f32 = CARD_INNER_W + CARD_FRAME_MARGIN;
/// Full visible height of a card, frame included.
const CARD_H: f32 = CARD_INNER_H + CARD_FRAME_MARGIN;
/// Gap between neighbouring cards, horizontally and vertically.
const CARD_SPACING: f32 = 12.0;
/// Vertical distance from the top of one row of cards to the top of the next.
const ROW_PITCH: f32 = CARD_H + CARD_SPACING;
/// Blank space above the first row inside the scroll area.
const GRID_TOP_SPACE: f32 = 4.0;
/// Horizontal padding on both sides of the grid.
const GRID_MARGIN_X: f32 = 18.0;

/// Per-snippet data derived from the snippet's own text: the lowercase forms the
/// search filter matches against, and the collapsed one-line card preview.
///
/// These used to be recomputed inside the render loop, which meant lowercasing
/// every snippet body and re-collapsing every visible preview on *every* frame —
/// egui repaints on each mouse move, so a large library re-walked all of its text
/// dozens of times a second. They only change when a snippet changes, so they are
/// cached here and rebuilt by [`CopyIt::rebuild_derived`].
struct Derived {
    title_lower: String,
    category_lower: String,
    body_lower: String,
    preview: String,
}

impl Derived {
    fn new(s: &Snippet) -> Self {
        // For secure snippets, censor the preview but keep full body for search
        let preview = if s.is_secure {
            let body = &s.body;
            if body.chars().count() > 5 {
                format!("{}*****", body.chars().take(5).collect::<String>())
            } else {
                "*****".to_string()
            }
        } else {
            preview_text(&s.body, PREVIEW_CHARS)
        };
        Derived {
            title_lower: s.title.to_lowercase(),
            category_lower: s.category.to_lowercase(),
            body_lower: s.body.to_lowercase(),
            preview,
        }
    }

    /// True when the snippet matches an already-lowercased, already-trimmed query.
    fn matches(&self, query_lower: &str) -> bool {
        self.title_lower.contains(query_lower)
            || self.body_lower.contains(query_lower)
            || self.category_lower.contains(query_lower)
    }
}

/// Memoized result of the search/category filter: the indices into `snippets` of
/// the cards that are currently visible, plus the inputs they were computed from.
///
/// The filter only has to run when the query, the category selection, or the
/// library itself changes. Without this the grid rebuilt the whole index list —
/// scanning every snippet's title, body and category — on every repaint.
#[derive(Default)]
struct FilterCache {
    indices: Vec<usize>,
    search: String,
    category: String,
    generation: u64,
    valid: bool,
}

impl FilterCache {
    /// True when the cached indices still describe the given inputs.
    fn is_current(&self, search: &str, category: &str, generation: u64) -> bool {
        self.valid
            && self.generation == generation
            && self.search == search
            && self.category == category
    }
}

/// Main application state and UI coordinator.
/// Maintains the full snippet library, handles search/filter/category logic,
/// manages the editor modal, drag-and-drop reordering, clipboard operations,
/// and persists all changes to disk automatically after mutations.
pub struct CopyIt {
    snippets: Vec<Snippet>,                        // Full snippet library; order is preserved and user-draggable
    derived: Vec<Derived>,                         // Cached lowercase text + card preview, one entry per snippet (same order)
    generation: u64,                               // Bumped whenever `snippets`/`derived` change; invalidates the filter cache
    filter: FilterCache,                           // Memoized indices of the snippets visible under the current search/category
    next_id: u64,                                  // Next ID to assign to a new snippet; incremented on creation
    path: PathBuf,                                 // Path to snippets.json in the stable data directory
    config_path: PathBuf,                          // Path to config.json (categories + theme selection)
    categories: Vec<String>,                       // Sorted, deduplicated list of all known categories
    search: String,                                // Active search query; filters snippets by title/body/category
    category_filter: String,                       // "All" or a specific category; filters visible snippets
    theme: Theme,                                  // Currently selected theme; applied to egui visuals each frame
    editor: Option<Editor>,                        // Modal editor state; None when no editor is open
    copied: Option<(u64, f64)>,                    // (id, time) for the transient "Copied" feedback (1.2s visibility)
    drag: Option<DragState>,                       // In-progress drag operation; None when idle
    adding_header_category: bool,                  // True when the user is typing a new category in the top bar
    new_header_category: String,                   // Input buffer for the new category name in the top bar
    category_error: Option<String>,                // Validation error for the new category (e.g., "All" is reserved)
    save_error: Option<String>,                    // File I/O error message to display at the top
    password_prompt: Option<PasswordPrompt>,       // Modal password prompt for secure snippets; None when not prompting
}

/// Tracks an in-progress drag operation. Initialized when the user clicks and holds on a card,
/// but remains in a "pre-drag" state until the pointer moves >4px (to avoid accidental
/// drags from single clicks). Once `dragging` becomes true, visual feedback (insertion
/// line, ghost box) appears to guide the user to a drop location. The threshold prevents
/// accidental reordering when the user simply clicks a card to interact with its buttons.
///
/// Only the stable `snippet_id` is retained — never the index the card had at drag
/// start. An index captured then and used at drop time would be stale if the library
/// changed in between (a delete would move the wrong card, or panic on a
/// now-out-of-bounds `Vec::remove`).
struct DragState {
    snippet_id: u64, // The snippet being dragged (stable ID across reordering)
    start_pos: egui::Pos2, // Pointer position at drag initiation; tracks distance for threshold
    dragging: bool, // true only after pointer has moved >4px; prevents accidental drags on click
}

/// Modal editor state for creating or editing a snippet. The `adding_category` and
/// `new_category` fields track an inline sub-form (entered via the category dropdown)
/// that lets users add a category without closing the editor. The `confirm_delete`
/// flag requires a second click to prevent accidental deletions. This separation of concerns
/// allows the editor to support category creation inline while keeping the main app's category
/// list management separate, improving UX for workflows where the user invents a new category mid-edit.
struct Editor {
    id: Option<u64>,               // None = creating a new snippet; Some(id) = editing existing with this stable ID
    title: String,
    category: String,
    new_category: String,           // input buffer for inline category creation; cleared when user confirms
    adding_category: bool,          // true when user clicked "+ Add new category" in the dropdown
    body: String,
    confirm_delete: bool,           // set to true on first "Delete" click; requires second "Confirm delete" to prevent accidents
    is_secure: bool,                // whether this snippet is password-protected
    password: String,               // password input (for new secure snippets)
    password_confirm: String,       // password confirmation input
    show_password_fields: bool,     // whether to show password fields in the editor
}

impl Editor {
    /// Creates a new blank editor state for adding a new snippet.
    /// Initializes with empty title and body, the first category (or empty string if no categories exist),
    /// and disables inline category creation and delete confirmation.
    fn blank(categories: &[String]) -> Self {
        Editor {
            id: None,
            title: String::new(),
            category: categories.first().cloned().unwrap_or_default(),
            new_category: String::new(),
            adding_category: false,
            body: String::new(),
            confirm_delete: false,
            is_secure: false,
            password: String::new(),
            password_confirm: String::new(),
            show_password_fields: false,
        }
    }

    /// Creates an editor state pre-populated from an existing snippet for editing.
    /// Loads the snippet's title, body, and category; ensures the category matches one
    /// from the canonical list (case-insensitive), falling back to the original if no match found.
    /// Used when the user clicks Edit on a card.
    fn from_snippet(s: &Snippet, categories: &[String]) -> Self {
        let category = categories
            .iter()
            .find(|c| c.eq_ignore_ascii_case(&s.category))
            .cloned()
            .unwrap_or_else(|| s.category.clone());
        Editor {
            id: Some(s.id),
            title: s.title.clone(),
            category,
            new_category: String::new(),
            adding_category: false,
            body: s.body.clone(),
            confirm_delete: false,
            is_secure: s.is_secure,
            password: String::new(),
            password_confirm: String::new(),
            show_password_fields: s.is_secure,
        }
    }
}

/// User action triggered from card interaction (Copy or Edit button click).
/// Used to defer action handling until after UI rendering to avoid borrowing conflicts.
enum Action {
    Copy(u64),   // User clicked Copy button on a snippet; copy its body to clipboard
    Edit(u64),   // User clicked Edit button on a snippet; open editor modal
}

/// Result of the editor modal interaction: whether to save, delete, cancel, or add a new category.
/// The editor modal handles inline category creation, so AddCategory is returned when the user
/// creates a new category within the editor and then the main app adds it to the canonical list.
enum EditorResult {
    None,                     // No action (editor still open); keep editor visible
    Save,                     // User clicked Save in editor; persist changes and close editor
    Cancel,                   // User clicked Cancel or closed the window; discard changes
    Delete,                   // User confirmed deletion (second click); remove the snippet
    AddCategory(String),      // User created a new category in the editor; add to canonical list
}

/// State for the password prompt modal shown when copying or editing a secure snippet.
struct PasswordPrompt {
    snippet_id: u64,           // The snippet requiring a password
    action: PasswordAction,    // What to do after password verification (Copy or Edit)
    password: String,          // User's password input
    error: Option<String>,     // Error message if verification failed
}

/// Action to take after successful password verification.
enum PasswordAction {
    Copy,
    Edit,
}

/// Layout and response data returned from rendering a single snippet card.
/// Separates the card frame's bounding rect from the button responses, used for
/// drag-and-drop interaction detection and click handling.
struct CardWidgets {
    frame_rect: egui::Rect,   // Bounding rectangle of the entire card frame (includes padding)
    copy: egui::Response,     // Response from the Copy button; checked for clicks
    edit: egui::Response,     // Response from the Edit button; checked for clicks
}

/// One-time recovery for users upgrading from earlier versions that stored
/// `snippets.json`/`config.json` next to the .exe: if the new stable location
/// doesn't have a file yet, pull in the first non-empty copy found in a
/// legacy location (next to the exe, `target/debug`, `target/release`, cwd).
/// This migration runs once per file per session; after that, the stable location
/// owns the data and legacy locations are ignored. Users who have data in multiple
/// locations get the first non-empty match (search order: exe dir, debug, release, cwd).
fn migrate_legacy_file(new_path: &std::path::Path, filename: &str) {
    if new_path.exists() {
        return; // Already migrated or was created fresh; don't search legacy locations
    }
    for dir in storage::legacy_candidate_dirs() {
        let candidate = dir.join(filename);
        if candidate == new_path {
            continue; // Skip the new location itself (shouldn't happen, but be safe)
        }
        if let Ok(data) = std::fs::read_to_string(&candidate) {
            let trimmed = data.trim();
            // Skip empty or dummy JSON (e.g. "[]" or "{}" from a failed write).
            if trimmed.is_empty() || trimmed == "[]" || trimmed == "{}" {
                continue;
            }
            let _ = std::fs::write(new_path, data);
            return; // Success: migrate and stop searching
        }
    }
}

/// Builds the banner text for a data file that exists but couldn't be parsed, moving the
/// original aside first so it is never silently replaced by the seeded defaults.
fn describe_corrupt_file(path: &std::path::Path, filename: &str, error: &str) -> String {
    match storage::backup_corrupt(path) {
        Ok(backup) => format!(
            "{filename} couldn't be read ({error}). It was kept as {} and the default library was loaded.",
            backup.display()
        ),
        Err(e) => format!(
            "{filename} couldn't be read ({error}) and couldn't be backed up ({e}). \
             The default library was loaded — copy the file elsewhere before making changes."
        ),
    }
}

impl CopyIt {
    /// Creates a new CopyIt instance on app launch.
    /// Loads snippets and config from disk (with one-time migration from legacy locations),
    /// seeds defaults if snippets.json doesn't exist, normalizes all categories, and applies
    /// the saved theme. Reports any file I/O errors in save_error for display in the UI.
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        let path = storage::data_path();
        let config_path = storage::config_path();
        migrate_legacy_file(&path, "snippets.json");
        migrate_legacy_file(&config_path, "config.json");

        let mut save_error: Option<String> = None;

        // A file that exists but doesn't parse is *not* a first launch: preserve it
        // before the seeded defaults claim its name, and tell the user where it went.
        let (mut snippets, seeded) = match storage::load(&path) {
            storage::Load::Loaded(snippets) => (snippets, false),
            storage::Load::Missing => (crate::seed::defaults(), true),
            storage::Load::Corrupt(e) => {
                save_error = Some(describe_corrupt_file(&path, "snippets.json", &e));
                (crate::seed::defaults(), true)
            }
        };
        for s in &mut snippets {
            s.category = storage::canonical_category(&s.category);
        }

        let mut config = match storage::load_config(&config_path) {
            storage::Load::Loaded(config) => config,
            storage::Load::Missing => Config::from_snippets(&snippets),
            storage::Load::Corrupt(e) => {
                // Always move the unreadable file aside, even when the banner ends up
                // showing the snippet-library notice instead of this one.
                let note = describe_corrupt_file(&config_path, "config.json", &e);
                // The snippet library is the more important loss; don't bury its notice.
                if save_error.is_none() {
                    save_error = Some(note);
                }
                Config::from_snippets(&snippets)
            }
        };
        config.add_categories(snippets.iter().map(|s| s.category.as_str()));
        if let Err(e) = storage::save_config(&config_path, &config) {
            save_error = Some(format!("{CONFIG_SAVE_ERROR}: {e}"));
        }

        let theme = config.theme.parse::<Theme>().unwrap_or(Theme::Dark);
        cc.egui_ctx.set_visuals(theme.visuals());

        let next_id = snippets.iter().map(|s| s.id).max().unwrap_or(0) + 1;
        if seeded {
            if let Err(e) = storage::save(&path, &snippets) {
                save_error = Some(format!("{SNIPPETS_SAVE_ERROR}: {e}"));
            }
        }

        let derived = snippets.iter().map(Derived::new).collect();

        Self {
            snippets,
            derived,
            generation: 0,
            filter: FilterCache::default(),
            next_id,
            path,
            config_path,
            categories: config.categories,
            search: String::new(),
            category_filter: "All".to_string(),
            theme,
            editor: None,
            copied: None,
            drag: None,
            adding_header_category: false,
            new_header_category: String::new(),
            category_error: None,
            save_error,
            password_prompt: None,
        }
    }

    /// Recomputes the per-snippet derived text caches and invalidates the filter cache.
    /// Called from [`Self::snippets_changed`] after every mutation of `snippets`, which is
    /// what keeps `derived` index-aligned with `snippets`.
    fn rebuild_derived(&mut self) {
        self.derived.clear();
        self.derived.reserve(self.snippets.len());
        self.derived.extend(self.snippets.iter().map(Derived::new));
        self.generation = self.generation.wrapping_add(1);
        self.filter.valid = false;
    }

    /// The single entry point for "the library changed": refreshes the derived caches
    /// and persists the new library. Every add/edit/delete/reorder goes through here, so
    /// `derived` can never drift out of sync with `snippets`.
    fn snippets_changed(&mut self) {
        self.rebuild_derived();
        self.save_snippets();
    }

    /// Returns the indices of the snippets visible under the current search and
    /// category filter, recomputing them only when one of the inputs (or the library)
    /// has changed. Ownership of the buffer is handed to the caller for the rest of the
    /// frame so the render loop can borrow `self` mutably; [`Self::restore_filtered`]
    /// puts it back, keeping its allocation for the next frame.
    fn take_filtered(&mut self) -> Vec<usize> {
        if !self
            .filter
            .is_current(&self.search, &self.category_filter, self.generation)
        {
            // Trim so a query of only spaces behaves like an empty one.
            let query = self.search.trim().to_lowercase();
            let all_categories = self.category_filter == "All";
            let category = self.category_filter.as_str();
            let snippets = &self.snippets;

            let mut indices = std::mem::take(&mut self.filter.indices);
            indices.clear();
            indices.extend(
                self.derived
                    .iter()
                    .enumerate()
                    .filter(|(i, d)| {
                        (all_categories || snippets[*i].category == category)
                            && (query.is_empty() || d.matches(&query))
                    })
                    .map(|(i, _)| i),
            );

            self.filter.indices = indices;
            self.filter.search.clear();
            self.filter.search.push_str(&self.search);
            self.filter.category.clear();
            self.filter.category.push_str(&self.category_filter);
            self.filter.generation = self.generation;
        }
        // The cache is only trusted while it actually holds the buffer: marking it
        // invalid on the way out means a second `take_filtered` before the matching
        // restore recomputes, instead of handing back an empty (i.e. "no cards") list.
        self.filter.valid = false;
        std::mem::take(&mut self.filter.indices)
    }

    /// Returns the buffer handed out by [`Self::take_filtered`] so its allocation is
    /// reused next frame. A mutation later in the same frame bumps `generation`, which
    /// makes the restored indices stale-checked (not trusted) on the following frame.
    fn restore_filtered(&mut self, filtered: Vec<usize>) {
        self.filter.indices = filtered;
        // Trustworthy again — `is_current` still re-checks the query, the category and
        // the generation, so a mutation made later in this frame invalidates it anyway.
        self.filter.valid = true;
    }

    /// Persists the full snippet library to snippets.json in the stable data directory.
    /// Updates save_error if an I/O error occurs; the error is shown in the top bar.
    fn save_snippets(&mut self) {
        match storage::save(&self.path, &self.snippets) {
            Ok(()) => self.clear_save_error(SNIPPETS_SAVE_ERROR),
            Err(e) => self.save_error = Some(format!("{SNIPPETS_SAVE_ERROR}: {e}")),
        }
    }

    /// Persists the config (categories and theme selection) to config.json.
    /// Separated from snippets.json so snippet data stays backward-compatible.
    /// Updates save_error if an I/O error occurs.
    fn save_config(&mut self) {
        let config = Config {
            categories: self.categories.clone(),
            theme: self.theme.to_string(),
        };
        match storage::save_config(&self.config_path, &config) {
            Ok(()) => self.clear_save_error(CONFIG_SAVE_ERROR),
            Err(e) => self.save_error = Some(format!("{CONFIG_SAVE_ERROR}: {e}")),
        }
    }

    /// Retires the warning banner only when it is reporting a failure of the kind that
    /// just succeeded. Clearing it unconditionally let an incidental config write (say,
    /// switching themes) hide the fact that the snippet library still isn't on disk.
    fn clear_save_error(&mut self, prefix: &str) {
        if self
            .save_error
            .as_deref()
            .is_some_and(|e| e.starts_with(prefix))
        {
            self.save_error = None;
        }
    }

    /// Clears the inline category warning as soon as the user starts editing
    /// the category input, so the error doesn't linger while they correct it.
    fn clear_category_error_on_input_change(&mut self, previous: &str) {
        if self.new_header_category != previous {
            self.category_error = None;
        }
    }

    /// Normalizes `raw`, adds it to the canonical list if it isn't already
    /// present (case-insensitive), persists the config, and returns the
    /// canonical form (or an existing match if one collides).
    fn add_category(&mut self, raw: &str) -> String {
        let cat = storage::normalize_category(raw);
        if storage::is_reserved_category(&cat) {
            return String::new();
        }
        if let Some(existing) = self
            .categories
            .iter()
            .find(|c| storage::same_category(c, &cat))
        {
            return existing.clone();
        }
        self.categories.push(cat.clone());
        self.categories.sort();
        self.save_config();
        cat
    }

    /// Moves the snippet identified by `snippet_id` to a new position in the full list, respecting
    /// the drag-and-drop user's intended placement within the *filtered* (visible) subset.
    /// The `target_filtered_gap` is a gap index into the *filtered* (visible after search/category filter)
    /// subset, but self.snippets is the *full* unfiltered list. This function converts the filtered
    /// gap to an absolute index in self.snippets, accounting for the fact that removing the origin
    /// shifts indices of everything after it. Edge cases: if `target_filtered_gap == filtered.len()`,
    /// the card is dropped after the last visible card. The adjustment (`t > origin_index ? t - 1 : t`)
    /// handles the index shift caused by removal. Automatically persists the reordered list to disk.
    ///
    /// The origin is looked up by id at drop time rather than trusting an index captured at
    /// drag start, so a library that changed mid-drag reorders the right card or nothing at all.
    fn reorder(&mut self, snippet_id: u64, target_filtered_gap: usize, filtered: &[usize]) {
        if filtered.is_empty() {
            return;
        }
        let Some(origin_index) = self.snippets.iter().position(|s| s.id == snippet_id) else {
            return; // The dragged snippet is gone (e.g. deleted mid-drag); nothing to move.
        };
        let snippet = self.snippets.remove(origin_index);
        let target_abs = if target_filtered_gap == 0 {
            let mut t = filtered[0];
            if t > origin_index {
                t -= 1; // Adjust for removal of origin_index
            }
            t
        } else if target_filtered_gap < filtered.len() {
            let mut t = filtered[target_filtered_gap];
            if t > origin_index {
                t -= 1; // Adjust for removal of origin_index
            }
            t
        } else {
            // Dropped after the last filtered card: insert after it
            let mut t = filtered[filtered.len() - 1];
            if t > origin_index {
                t -= 1; // Adjust for removal of origin_index
            }
            t + 1
        };
        let target_abs = target_abs.min(self.snippets.len());
        self.snippets.insert(target_abs, snippet);
        self.snippets_changed();
    }

    /// Renders a single snippet card with all visual elements: title (strong, truncated), category badge
    /// (color-coded), preview text (collapsed to single line), and action buttons (Copy, Edit).
    /// The Copy button shows a "Copied" confirmation checkmark for 1.2 seconds after the user clicks it.
    /// If is_dragged is true, the card is faded out (40% opacity) to provide clear visual feedback that
    /// it is being dragged. The card is 300x168 pixels plus 20px frame padding on each side.
    /// Returns the frame rect and button responses for subsequent drag/click detection and action dispatch.
    /// Actions are added to the actions vector (deferred processing) rather than handled immediately.
    fn card(
        &self,
        ui: &mut egui::Ui,
        idx: usize,
        width: f32,
        now: f64,
        actions: &mut Vec<Action>,
        is_dragged: bool,
    ) -> CardWidgets {
        let s = &self.snippets[idx];
        let card_h = CARD_INNER_H;

        // The preview is cached per snippet (already censored for secure snippets);
        // fall back to computing it only if the caches somehow got out of step.
        debug_assert_eq!(self.derived.len(), self.snippets.len());
        let fallback_preview;
        let preview: &str = match self.derived.get(idx) {
            Some(d) => &d.preview,
            None => {
                fallback_preview = if s.is_secure {
                    let body = &s.body;
                    if body.chars().count() > 5 {
                        format!("{}*****", body.chars().take(5).collect::<String>())
                    } else {
                        "*****".to_string()
                    }
                } else {
                    preview_text(&s.body, PREVIEW_CHARS)
                };
                &fallback_preview
            }
        };

        if is_dragged {
            ui.set_opacity(0.4);
        }

        let frame = egui::Frame::group(ui.style())
            .rounding(egui::Rounding::same(8.0))
            .inner_margin(egui::Margin::same(10.0))
            .fill(ui.visuals().extreme_bg_color)
            .show(ui, |ui| {
                ui.set_width(width);
                ui.set_height(card_h);

                ui.vertical(|ui| {
                    // Header: title (left) + copy button (top-right)
                    let copy_resp = ui
                        .horizontal(|ui| {
                            let recently = self
                                .copied
                                .is_some_and(|(cid, t)| cid == s.id && now - t < 1.2);
                            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                let copy_label = if recently {
                                    "\u{2714} Copied"
                                } else if s.is_secure {
                                    "\u{1F512} Copy"
                                } else {
                                    "\u{29C9} Copy"
                                };
                                let resp = ui.button(copy_label);
                                if resp.clicked() {
                                    actions.push(Action::Copy(s.id));
                                }
                                ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
                                    ui.add(
                                        egui::Label::new(
                                            egui::RichText::new(&s.title).strong().size(15.0),
                                        )
                                        .truncate(true),
                                    );
                                    // Secure badge
                                    if s.is_secure {
                                        ui.add_space(4.0);
                                        egui::Frame::none()
                                            .fill(egui::Color32::from_rgb(0xef, 0x44, 0x44))
                                            .rounding(egui::Rounding::same(4.0))
                                            .inner_margin(egui::Margin::symmetric(4.0, 1.0))
                                            .show(ui, |ui| {
                                                ui.label(
                                                    egui::RichText::new("\u{1F512} Secure")
                                                        .small()
                                                        .color(egui::Color32::WHITE),
                                                );
                                            });
                                    }
                                });
                                resp
                            })
                            .inner
                        })
                        .inner;

                    // Category badge
                    let badge_text = if ui.visuals().dark_mode {
                        egui::Color32::WHITE
                    } else {
                        egui::Color32::BLACK
                    };
                    egui::Frame::none()
                        .fill(category_color(&s.category))
                        .rounding(egui::Rounding::same(4.0))
                        .inner_margin(egui::Margin::symmetric(6.0, 2.0))
                        .show(ui, |ui| {
                            ui.label(
                                egui::RichText::new(&s.category)
                                    .small()
                                    .color(badge_text),
                            );
                        });

                    ui.add_space(6.0);

                    // Preview + Edit pinned to the bottom
                    let edit_resp = ui
                        .with_layout(egui::Layout::bottom_up(egui::Align::LEFT), |ui| {
                            let edit_label = if s.is_secure { "\u{1F512} Edit" } else { "Edit" };
                            let resp = ui.small_button(edit_label);
                            if resp.clicked() {
                                actions.push(Action::Edit(s.id));
                            }
                            ui.add_space(4.0);
                            ui.with_layout(egui::Layout::top_down(egui::Align::LEFT), |ui| {
                                ui.label(egui::RichText::new(preview).weak());
                            });
                            resp
                        })
                        .inner;

                    (copy_resp, edit_resp)
                })
                .inner
            });

        if is_dragged {
            ui.set_opacity(1.0);
        }

        let frame_rect = frame.response.rect;
        let (copy, edit) = frame.inner;
        CardWidgets { frame_rect, copy, edit }
    }

    /// Lays out the responsive card grid inside the scroll area and returns the column
    /// count together with the screen-space rect of *every* filtered card.
    ///
    /// Only the rows that intersect the viewport (plus one row of overscan) are actually
    /// built; the rest are replaced by blank space of exactly the same height. A library
    /// of hundreds of snippets used to construct every card — text layout, galleys,
    /// interaction ids — on every repaint, including the ones scrolled far out of sight.
    ///
    /// The returned rects cover the skipped cards too: the grid is uniform, so their
    /// geometry is computed from the grid origin rather than harvested from the layout.
    /// Drag-and-drop therefore still sees the whole grid and can drop onto an off-screen
    /// gap exactly as before. Card rects are already in absolute screen space (see the
    /// note in `update`), so they need no further translation.
    ///
    /// Collected interactions are appended to `actions` / `drag_start` / `hover_cursor`
    /// instead of being applied here, so the caller can act on them once the grid's
    /// borrow of `self` has ended.
    fn card_grid(
        &self,
        ui: &mut egui::Ui,
        filtered: &[usize],
        now: f64,
        actions: &mut Vec<Action>,
        drag_start: &mut Option<(u64, egui::Pos2)>,
        hover_cursor: &mut Option<egui::CursorIcon>,
    ) -> (usize, Vec<egui::Rect>) {
        egui::Frame::none()
            .inner_margin(egui::Margin::symmetric(GRID_MARGIN_X, 0.0))
            .show(ui, |ui| {
                let avail = ui.available_width();
                let cols = ((avail / (CARD_W + CARD_SPACING)).floor() as usize).max(1);

                let origin = ui.cursor().min;
                let rows = filtered.len().div_ceil(cols);
                let card_rects: Vec<egui::Rect> = (0..filtered.len())
                    .map(|i| grid_card_rect(i, cols, origin, CARD_W, CARD_H, CARD_SPACING))
                    .collect();

                let (first_row, last_row) =
                    visible_rows(ui.clip_rect(), origin.y, ROW_PITCH, rows);
                // Reserve the height of the rows above the viewport.
                if first_row > 0 {
                    ui.add_space(first_row as f32 * ROW_PITCH);
                }

                for row_start in (first_row..=last_row).map(|r| r * cols) {
                    let row_end = (row_start + cols).min(filtered.len());
                    let row = &filtered[row_start..row_end];
                    // Key each row's widget ids on the row itself. egui otherwise derives
                    // them from a per-parent counter, which would make every id inside the
                    // grid depend on how many rows above the viewport were skipped — so a
                    // card's buttons would change identity as the user scrolled.
                    ui.push_id(row_start, |ui| {
                        ui.horizontal(|ui| {
                            ui.spacing_mut().item_spacing = egui::vec2(0.0, 0.0);
                            for (row_offset, &idx) in row.iter().enumerate() {
                                let is_dragged = self.drag.as_ref().is_some_and(|d| {
                                    d.dragging && d.snippet_id == self.snippets[idx].id
                                });

                                let drag_id = ui.id().with("card_drag").with(idx);
                                let expected_rect = egui::Rect::from_min_size(
                                    ui.cursor().min,
                                    egui::vec2(CARD_W, CARD_H),
                                );
                                let drag_resp =
                                    ui.interact(expected_rect, drag_id, egui::Sense::drag());

                                let widgets =
                                    self.card(ui, idx, CARD_INNER_W, now, actions, is_dragged);

                                debug_assert!(
                                    card_rects
                                        .get(row_start + row_offset)
                                        .is_some_and(|r| r.min.distance(widgets.frame_rect.min)
                                            < 0.5),
                                    "computed card rect must match the rendered one"
                                );

                                let pointer_over_buttons =
                                    widgets.copy.hovered() || widgets.edit.hovered();

                                // Initiate drag only if: pointer is not over buttons, no active drag, and drag sensor triggered
                                if drag_resp.drag_started()
                                    && !pointer_over_buttons
                                    && self.drag.is_none()
                                {
                                    if let Some(pos) = drag_resp.interact_pointer_pos() {
                                        *drag_start = Some((self.snippets[idx].id, pos));
                                    }
                                }

                                if drag_resp.hovered()
                                    && self.drag.is_none()
                                    && !pointer_over_buttons
                                {
                                    *hover_cursor = Some(egui::CursorIcon::Grab);
                                }

                                ui.add_space(CARD_SPACING);
                            }
                        });
                    });
                    ui.add_space(CARD_SPACING);
                }

                // Reserve the height of the rows below the viewport, so the scrollbar
                // still spans the whole library.
                if last_row + 1 < rows {
                    ui.add_space((rows - 1 - last_row) as f32 * ROW_PITCH);
                }

                (cols, card_rects)
            })
            .inner
    }
}

impl eframe::App for CopyIt {
    /// Main UI render loop called once per frame.
    /// Renders the top bar (search, category filter, theme selector, new button),
    /// the responsive grid of snippet cards, and the editor modal if open.
    /// Handles all user input: search/filter/category management, copy/edit/delete actions,
    /// drag-and-drop reordering with visual insertion lines, and theme switching.
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        let now = ctx.input(|i| i.time);
        let previous_theme = self.theme;
        // The visuals are *not* rebuilt here every frame: `Theme::visuals()` constructs a
        // whole `egui::Visuals` (and `set_visuals` clones the context style) for a value
        // that only changes when the user picks a different theme. `CopyIt::new` applies
        // the loaded theme once, and the theme selector below re-applies it on change.

        // ---- Top bar: search, category filter, theme selector, new ----
        egui::TopBottomPanel::top("top").show(ctx, |ui| {
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                ui.heading("CopyIt");
                ui.add_space(16.0);
                ui.label("\u{1F50D}");
                ui.add(
                    egui::TextEdit::singleline(&mut self.search)
                        .hint_text("Search title, text, category\u{2026}")
                        .desired_width(260.0),
                );
                if !self.search.is_empty() && ui.button("\u{2715}").clicked() {
                    self.search.clear();
                }
                ui.add_space(12.0);

                // Category filter (and inline category creation).
                let mut filter_selected = self.category_filter.clone();
                let mut start_adding_category = false;
                egui::ComboBox::from_id_source("cat_filter")
                    .selected_text(self.category_filter.as_str())
                    .show_ui(ui, |ui| {
                        ui.selectable_value(&mut filter_selected, "All".to_string(), "All");
                        for c in &self.categories {
                            ui.selectable_value(&mut filter_selected, c.clone(), c);
                        }
                        if ui
                            .selectable_value(
                                &mut filter_selected,
                                "+ Add new category".to_string(),
                                "+ Add new category",
                            )
                            .clicked()
                        {
                            start_adding_category = true;
                        }
                    });
                if start_adding_category {
                    self.adding_header_category = true;
                    self.new_header_category.clear();
                    self.category_error = None;
                } else if filter_selected != self.category_filter
                    && filter_selected != "+ Add new category"
                {
                    self.category_filter = filter_selected;
                    self.adding_header_category = false;
                    self.new_header_category.clear();
                    self.category_error = None;
                }
                if self.adding_header_category {
                    let previous_category = self.new_header_category.clone();
                    let input = ui.add(
                        egui::TextEdit::singleline(&mut self.new_header_category)
                            .hint_text("New category")
                            .desired_width(120.0),
                    );
                    self.clear_category_error_on_input_change(&previous_category);
                    // Only treat Enter as "submit" when it was typed into *this* field.
                    // A bare `key_pressed(Enter)` also fired for Enter pressed in the
                    // search box or the editor modal, submitting behind the user's back.
                    let enter_pressed =
                        input.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                    if ui.button("Add").clicked() || enter_pressed {
                        let raw = self.new_header_category.trim().to_string();
                        if raw.eq_ignore_ascii_case("all") {
                            self.category_error =
                                Some("\"All\" is reserved and can't be used as a category".to_string());
                        } else if !raw.is_empty() {
                            let canonical = self.add_category(&raw);
                            if !canonical.is_empty() {
                                self.category_filter = canonical;
                                self.adding_header_category = false;
                                self.new_header_category.clear();
                                self.category_error = None;
                            }
                        } else {
                            self.adding_header_category = false;
                            self.new_header_category.clear();
                            self.category_error = None;
                        }
                    }
                    if let Some(err) = &self.category_error {
                        ui.colored_label(WARNING_COLOR, err);
                    }
                }

                ui.add_space(12.0);
                egui::ComboBox::from_id_source("theme_select")
                    .selected_text(self.theme.name())
                    .show_ui(ui, |ui| {
                        for t in Theme::all() {
                            ui.selectable_value(&mut self.theme, *t, t.name());
                        }
                    });
                if self.theme != previous_theme {
                    ctx.set_visuals(self.theme.visuals());
                    // The top bar above has already been painted with the old visuals, so
                    // ask for one more frame to redraw everything with the new ones.
                    ctx.request_repaint();
                    self.save_config();
                }

                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.button("\u{FF0B} New").clicked() {
                        self.editor = Some(Editor::blank(&self.categories));
                    }
                });
            });
            // Borrow the message instead of cloning it every frame; the dismiss click is
            // recorded in a local and applied after the closure releases the borrow.
            if let Some(err) = self.save_error.as_deref() {
                ui.add_space(4.0);
                let mut dismissed = false;
                ui.horizontal(|ui| {
                    ui.add_space(16.0);
                    ui.colored_label(WARNING_COLOR, format!("\u{26A0} {err}"));
                    // Startup notices (e.g. a recovered corrupt file) are not tied to a
                    // later successful save, so give the user a way to acknowledge them.
                    if ui
                        .small_button("\u{2715}")
                        .on_hover_text("Dismiss")
                        .clicked()
                    {
                        dismissed = true;
                    }
                });
                if dismissed {
                    self.save_error = None;
                }
            }
            ui.add_space(6.0);
        });

        // ---- Main grid ----
        // Visible-card indices come from the memoized filter: the scan over every
        // snippet's title/body/category only reruns when the query, the category
        // selection, or the library itself changes — not on every repaint.
        let filtered = self.take_filtered();

        egui::CentralPanel::default().show(ctx, |ui| {
            if filtered.is_empty() {
                // No cards are on screen, so there is nothing to drop onto and the
                // drag-handling code below is skipped entirely. Abandon any drag now;
                // leaving one live would wedge `self.drag` as `Some` forever and block
                // every future drag.
                self.drag = None;
                ui.add_space(40.0);
                ui.vertical_centered(|ui| {
                    ui.label(egui::RichText::new("No snippets match.").weak());
                });
                return;
            }

            let mut actions: Vec<Action> = Vec::new();
            let mut drag_start: Option<(u64, egui::Pos2)> = None;
            let mut hover_cursor: Option<egui::CursorIcon> = None;

            let scroll_output = egui::ScrollArea::vertical()
                .auto_shrink([false; 2])
                .show(ui, |ui| {
                    ui.spacing_mut().item_spacing = egui::vec2(0.0, 0.0);
                    ui.add_space(GRID_TOP_SPACE);
                    self.card_grid(
                        ui,
                        &filtered,
                        now,
                        &mut actions,
                        &mut drag_start,
                        &mut hover_cursor,
                    )
                });

                    // Process normal click actions.
                    for a in actions {
                        match a {
                            Action::Copy(id) => {
                                if let Some(s) = self.snippets.iter().find(|s| s.id == id) {
                                    if s.is_secure {
                                        // Show password prompt for secure snippets
                                        self.password_prompt = Some(PasswordPrompt {
                                            snippet_id: id,
                                            action: PasswordAction::Copy,
                                            password: String::new(),
                                            error: None,
                                        });
                                    } else {
                                        let text = s.body.clone();
                                        ui.output_mut(|o| o.copied_text = text);
                                        self.copied = Some((id, now));
                                        ctx.request_repaint_after(std::time::Duration::from_millis(1300));
                                    }
                                }
                            }
                            Action::Edit(id) => {
                                if let Some(s) = self.snippets.iter().find(|s| s.id == id) {
                                    if s.is_secure {
                                        // Show password prompt for secure snippets
                                        self.password_prompt = Some(PasswordPrompt {
                                            snippet_id: id,
                                            action: PasswordAction::Edit,
                                            password: String::new(),
                                            error: None,
                                        });
                                    } else {
                                        self.editor = Some(Editor::from_snippet(s, &self.categories));
                                    }
                                }
                            }
                        }
                    }

                    // Start a new drag if requested.
                    if let Some((id, pos)) = drag_start {
                        self.drag = Some(DragState {
                            snippet_id: id,
                            start_pos: pos,
                            dragging: false,
                        });
                    }

                    // Card rects collected inside a ScrollArea are ALREADY in screen space:
                    // the scroll area places its content Ui at `inner_rect.min - offset`, so
                    // every widget rect below it is absolute and scroll-adjusted. Translating
                    // them again (by that same origin) shifted all drop geometry down by the
                    // height of the top bar, and further off with every pixel scrolled — the
                    // insertion line and the chosen drop slot no longer matched the cursor.
                    let card_screen_rects: &[egui::Rect] = &scroll_output.inner.1;
                    let grid_area = scroll_output.inner_rect;
                    let cols = scroll_output.inner.0;

                    // Update drag threshold.
                    if let Some(drag) = &mut self.drag {
                        if let Some(pos) = ctx.input(|i| i.pointer.interact_pos()) {
                            if !drag.dragging && drag.start_pos.distance(pos) > 4.0 {
                                drag.dragging = true;
                            }
                        }
                    }

                    // Draw drag visuals and handle drop.
                    let drag_state = self.drag.as_ref().map(|d| (d.dragging, d.snippet_id));
                    if let Some((dragging, snippet_id)) = drag_state {
                        let pointer_pos = ctx.input(|i| i.pointer.interact_pos());
                        let pointer_released = ctx.input(|i| i.pointer.primary_released());
                        let pointer_moved = ctx.input(|i| i.pointer.delta().length_sq() > 0.0);

                        if dragging {
                            hover_cursor = Some(egui::CursorIcon::Grabbing);

                            if let Some(pointer) = pointer_pos {
                                let gap = nearest_gap(pointer, card_screen_rects, cols, CARD_SPACING, CARD_W);
                                draw_insertion_line(
                                    ctx,
                                    gap,
                                    card_screen_rects,
                                    cols,
                                    CARD_SPACING,
                                    CARD_W,
                                    CARD_H,
                                );

                                // Hollow ghost box following the cursor.
                                let ghost_rect = egui::Rect::from_min_size(
                                    pointer + egui::vec2(8.0, 8.0),
                                    egui::vec2(CARD_W, CARD_H),
                                );
                                let painter = ctx.layer_painter(egui::LayerId::new(
                                    egui::Order::Tooltip,
                                    egui::Id::new("drag_ghost"),
                                ));
                                painter.rect_stroke(
                                    ghost_rect,
                                    egui::Rounding::same(8.0),
                                    egui::Stroke::new(
                                        2.0_f32,
                                        egui::Color32::from_rgb(0x60, 0xb0, 0xff),
                                    ),
                                );
                            }

                            if pointer_released {
                                if let Some(pointer) = pointer_pos {
                                    if grid_area.contains(pointer) {
                                        let gap =
                                            nearest_gap(pointer, card_screen_rects, cols, CARD_SPACING, CARD_W);
                                        self.reorder(snippet_id, gap, &filtered);
                                    }
                                }
                                self.drag = None;
                            } else if pointer_moved {
                                ctx.request_repaint();
                            }
                        } else if pointer_released {
                            // Released before crossing the drag threshold: cancel.
                            self.drag = None;
                        }
                    }

                    if let Some(cursor) = hover_cursor {
                        ctx.output_mut(|o| o.cursor_icon = cursor);
                    }
                });

        // Hand the index buffer back so its allocation is reused next frame.
        self.restore_filtered(filtered);

        // ---- Editor window (new / edit / delete) ----
        // Modal editor for creating or modifying snippets; supports inline category creation via the dropdown
        if self.editor.is_some() {
            let mut ed = self.editor.take().unwrap();
            // Borrowed, not cloned: the whole category list used to be duplicated on
            // every frame the editor was open.
            let categories = &self.categories;
            let mut window_open = true;
            let mut result = EditorResult::None;
            let title = if ed.id.is_some() {
                "Edit snippet"
            } else {
                "New snippet"
            };

            // Keep the modal on-screen even for very long snippets: cap the
            // window height to a fraction of the viewport, and make only the
            // Content editor scroll internally.
            let screen_height = ctx.screen_rect().height();
            let max_window_height = screen_height * 0.85;
            let reserved_for_fixed = 170.0; // title/category row + label + buttons + spacing
            let max_content_height = (max_window_height - reserved_for_fixed).max(120.0);

            egui::Window::new(title)
                .collapsible(false)
                .resizable(true)
                .default_width(540.0)
                .max_height(max_window_height)
                .open(&mut window_open)
                .show(ctx, |ui| {
                    // Keep the header row width-constrained so the window stays
                    // a compact, centered dialog (~540px) instead of stretching
                    // to fill the whole application window.
                    ui.horizontal(|ui| {
                        ui.vertical(|ui| {
                            ui.label("Title");
                            ui.add(
                                egui::TextEdit::singleline(&mut ed.title)
                                    .desired_width(280.0),
                            );
                        });

                        ui.add_space(12.0);

                        ui.vertical(|ui| {
                            ui.label("Category");
                            let display = if ed.adding_category {
                                "+ Add new category".to_string()
                            } else {
                                ed.category.clone()
                            };
                            let mut selected = display.clone();
                            egui::ComboBox::from_id_source("cat_select")
                                .width(180.0)
                                .selected_text(display)
                                .show_ui(ui, |ui| {
                                    for c in categories {
                                        ui.selectable_value(&mut selected, c.clone(), c);
                                    }
                                    ui.selectable_value(
                                        &mut selected,
                                        "+ Add new category".to_string(),
                                        "+ Add new category",
                                    );
                                });
                            if selected == "+ Add new category" {
                                ed.adding_category = true;
                            } else {
                                ed.adding_category = false;
                                ed.category = selected;
                            }

                            if ed.adding_category {
                                ui.horizontal(|ui| {
                                    ui.add(
                                        egui::TextEdit::singleline(&mut ed.new_category)
                                            .hint_text("New category")
                                            .desired_width(120.0),
                                    );
                                    if ui.button("Add").clicked()
                                        && !ed.new_category.trim().is_empty()
                                    {
                                        result = EditorResult::AddCategory(ed.new_category.clone());
                                    }
                                });
                            }
                        });
                    });
                    ui.add_space(6.0);

                    // Secure snippet checkbox and password fields
                    ui.horizontal(|ui| {
                        let secure_response = ui.checkbox(&mut ed.is_secure, "🔒 Secure (password-protected)");
                        if secure_response.changed() {
                            ed.show_password_fields = ed.is_secure;
                            if !ed.is_secure {
                                ed.password.clear();
                                ed.password_confirm.clear();
                            }
                        }
                        if ed.is_secure {
                            ui.label(egui::RichText::new("Requires password to copy/edit").weak().small());
                        }
                    });
                    if ed.show_password_fields {
                        ui.horizontal(|ui| {
                            ui.vertical(|ui| {
                                ui.label("Password");
                                ui.add(
                                    egui::TextEdit::singleline(&mut ed.password)
                                        .password(true)
                                        .desired_width(280.0)
                                        .hint_text("Enter password"),
                                );
                            });
                            ui.add_space(12.0);
                            ui.vertical(|ui| {
                                ui.label("Confirm Password");
                                ui.add(
                                    egui::TextEdit::singleline(&mut ed.password_confirm)
                                        .password(true)
                                        .desired_width(280.0)
                                        .hint_text("Confirm password"),
                                );
                            });
                        });
                        if !ed.password.is_empty() && ed.password != ed.password_confirm {
                            ui.colored_label(WARNING_COLOR, "Passwords do not match");
                        }
                    }
                    ui.add_space(6.0);

                    ui.label("Content");
                    egui::ScrollArea::vertical()
                        .auto_shrink([false, true])
                        .max_height(max_content_height)
                        .show(ui, |ui| {
                            ui.add(
                                egui::TextEdit::multiline(&mut ed.body)
                                    .desired_width(f32::INFINITY)
                                    .desired_rows(12)
                                    .font(egui::TextStyle::Monospace),
                            );
                        });
                    ui.add_space(10.0);

                    ui.horizontal(|ui| {
                        let can_save = !ed.title.trim().is_empty();
                        if ui
                            .add_enabled(can_save, egui::Button::new("Save"))
                            .clicked()
                        {
                            result = EditorResult::Save;
                        }
                        if ui.button("Cancel").clicked() {
                            result = EditorResult::Cancel;
                        }

                        if ed.id.is_some() {
                            ui.with_layout(
                                egui::Layout::right_to_left(egui::Align::Center),
                                |ui| {
                                    if !ed.confirm_delete {
                                        if ui.button("Delete").clicked() {
                                            ed.confirm_delete = true;
                                        }
                                    } else if ui
                                        .button(
                                            egui::RichText::new("\u{26A0} Confirm delete")
                                                .color(WARNING_COLOR),
                                        )
                                        .clicked()
                                    {
                                        result = EditorResult::Delete;
                                    }
                                },
                            );
                        }
                    });
                });

            match result {
                EditorResult::Save => {
                    // Validate password if creating a secure snippet
                    if ed.is_secure && ed.show_password_fields {
                        if ed.password.is_empty() || ed.password != ed.password_confirm {
                            ed.password.clear();
                            ed.password_confirm.clear();
                            // Keep editor open with error (will show "Passwords do not match" in UI)
                            self.editor = Some(ed);
                        } else {
                            // Normalize category: ensure it's canonical, fall back to "Uncategorized" if empty
                            let category = {
                                let canonical = self.add_category(&ed.category);
                                if canonical.is_empty() {
                                    self.add_category("Uncategorized")
                                } else {
                                    canonical
                                }
                            };
                            let title = ed.title.trim().to_string();
                            let password_hash = if ed.is_secure && ed.show_password_fields {
                                hash_password(&ed.password)
                            } else {
                                String::new()
                            };
                            if let Some(id) = ed.id {
                                if let Some(s) = self.snippets.iter_mut().find(|s| s.id == id) {
                                    s.title = title;
                                    s.category = category;
                                    s.is_secure = ed.is_secure;
                                    s.password_hash = password_hash;
                                    // The editor is closing, so its buffer can be moved
                                    // instead of copied — snippet bodies can be large.
                                    s.body = std::mem::take(&mut ed.body);
                                }
                            } else {
                                let id = self.next_id;
                                self.next_id += 1;
                                self.snippets.push(Snippet {
                                    id,
                                    title,
                                    category,
                                    body: std::mem::take(&mut ed.body),
                                    is_secure: ed.is_secure,
                                    password_hash,
                                });
                            }
                            self.snippets_changed();
                        }
                    } else {
                        // Normalize category: ensure it's canonical, fall back to "Uncategorized" if empty
                        let category = {
                            let canonical = self.add_category(&ed.category);
                            if canonical.is_empty() {
                                self.add_category("Uncategorized")
                            } else {
                                canonical
                            }
                        };
                        let title = ed.title.trim().to_string();
                        let password_hash = if ed.is_secure && ed.show_password_fields {
                            hash_password(&ed.password)
                        } else {
                            String::new()
                        };
                        if let Some(id) = ed.id {
                            if let Some(s) = self.snippets.iter_mut().find(|s| s.id == id) {
                                s.title = title;
                                s.category = category;
                                s.is_secure = ed.is_secure;
                                s.password_hash = password_hash;
                                // The editor is closing, so its buffer can be moved
                                // instead of copied — snippet bodies can be large.
                                s.body = std::mem::take(&mut ed.body);
                            }
                        } else {
                            let id = self.next_id;
                            self.next_id += 1;
                            self.snippets.push(Snippet {
                                id,
                                title,
                                category,
                                body: std::mem::take(&mut ed.body),
                                is_secure: ed.is_secure,
                                password_hash,
                            });
                        }
                        self.snippets_changed();
                    }
                }
                EditorResult::Delete => {
                    if let Some(id) = ed.id {
                        self.snippets.retain(|s| s.id != id);
                        self.snippets_changed();
                    }
                }
                EditorResult::Cancel => {}
                EditorResult::AddCategory(name) => {
                    let canonical = self.add_category(&name);
                    if !canonical.is_empty() {
                        ed.category = canonical;
                    }
                    ed.new_category.clear();
                    ed.adding_category = false;
                    self.editor = Some(ed);
                }
                EditorResult::None => {
                    // Keep editing unless the user closed the window via the X.
                    if window_open {
                        self.editor = Some(ed);
                    }
                }
            }
        }

        // ---- Password prompt window (for secure snippets) ----
        if let Some(mut prompt) = self.password_prompt.take() {
            let mut window_open = true;
            let mut verified = false;
            let snippet_title = self.snippets.iter().find(|s| s.id == prompt.snippet_id).map(|s| s.title.clone()).unwrap_or_default();

            let mut close_window = false;
            egui::Window::new("🔒 Password Required")
                .collapsible(false)
                .resizable(false)
                .default_width(360.0)
                .open(&mut window_open)
                .show(ctx, |ui| {
                    ui.label(format!("Enter password for \"{}\"", snippet_title));
                    ui.add_space(8.0);
                    ui.horizontal(|ui| {
                        ui.label("Password:");
                        ui.add_space(8.0);
                        let resp = ui.add(
                            egui::TextEdit::singleline(&mut prompt.password)
                                .password(true)
                                .desired_width(200.0),
                        );
                        if resp.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                            verified = true;
                        }
                    });
                    if let Some(err) = &prompt.error {
                        ui.colored_label(WARNING_COLOR, err);
                    }
                    ui.add_space(12.0);
                    ui.horizontal(|ui| {
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if ui.button("Cancel").clicked() {
                                close_window = true;
                            }
                            if ui.button("Verify").clicked() {
                                verified = true;
                            }
                        });
                    });
                });

            if close_window {
                window_open = false;
            }

            if verified {
                // Verify password
                if let Some(s) = self.snippets.iter().find(|s| s.id == prompt.snippet_id) {
                    if verify_password(&prompt.password, &s.password_hash) {
                        // Password correct - perform the action
                        match prompt.action {
                            PasswordAction::Copy => {
                                ctx.output_mut(|o| o.copied_text = s.body.clone());
                                self.copied = Some((s.id, now));
                                ctx.request_repaint_after(std::time::Duration::from_millis(1300));
                            }
                            PasswordAction::Edit => {
                                self.editor = Some(Editor::from_snippet(s, &self.categories));
                            }
                        }
                    } else {
                        // Password incorrect - show error and re-prompt
                        prompt.error = Some("Incorrect password".to_string());
                        prompt.password.clear();
                        self.password_prompt = Some(prompt);
                    }
                }
            } else if window_open {
                // User didn't verify yet, keep prompt open
                self.password_prompt = Some(prompt);
            }
            // If window_open is false and not verified, prompt is dropped (cancelled)
        }
    }
}

/// Truncates a string to a maximum number of characters, adding an ellipsis if truncated.
/// Counts Unicode characters, not bytes, to correctly handle multi-byte characters.
fn truncate_chars(s: &str, max: usize) -> String {
    if s.chars().count() > max {
        let mut t: String = s.chars().take(max.saturating_sub(1)).collect();
        t.push('\u{2026}');
        t
    } else {
        s.to_string()
    }
}

/// Collapses a snippet body into a single-line preview: splits on whitespace, joins with single spaces,
/// and truncates to max characters. Used to display a short preview in each card.
///
/// Collapsing stops as soon as enough characters have been gathered to fill the preview,
/// so a megabyte-long body costs the same as a one-line one instead of being copied whole.
fn preview_text(body: &str, max: usize) -> String {
    let mut collapsed = String::new();
    let mut chars = 0usize;
    for word in body.split_whitespace() {
        if !collapsed.is_empty() {
            collapsed.push(' ');
            chars += 1;
        }
        collapsed.push_str(word);
        chars += word.chars().count();
        // One char past the limit is all `truncate_chars` needs to know it must trim.
        if chars > max {
            break;
        }
    }
    truncate_chars(&collapsed, max)
}

/// Position of the card at index `i` of the filtered grid, computed from the grid's
/// origin rather than from layout. The grid is uniform — `cols` cards of `card_w` x
/// `card_h` per row, separated by `spacing` — so off-screen cards still have exact
/// rects for drag-and-drop hit-testing without being laid out or painted.
fn grid_card_rect(
    i: usize,
    cols: usize,
    origin: egui::Pos2,
    card_w: f32,
    card_h: f32,
    spacing: f32,
) -> egui::Rect {
    let cols = cols.max(1);
    let col = i % cols;
    let row = i / cols;
    egui::Rect::from_min_size(
        origin
            + egui::vec2(
                col as f32 * (card_w + spacing),
                row as f32 * (card_h + spacing),
            ),
        egui::vec2(card_w, card_h),
    )
}

/// Inclusive range of grid rows that intersect `clip` (the visible part of the scroll
/// area), with one row of overscan on each side so a row entering the viewport is
/// already laid out and edge rounding can't reveal a gap. Rows outside the range are
/// replaced by blank space of the same height, so scrolling and card positions are
/// unaffected. Falls back to "every row" if the geometry isn't finite.
fn visible_rows(clip: egui::Rect, origin_y: f32, row_pitch: f32, rows: usize) -> (usize, usize) {
    let max_row = rows.saturating_sub(1);
    if rows == 0 {
        return (0, 0);
    }
    if !row_pitch.is_finite()
        || row_pitch <= 0.0
        || !origin_y.is_finite()
        || !clip.top().is_finite()
        || !clip.bottom().is_finite()
    {
        return (0, max_row);
    }
    let first = (((clip.top() - origin_y) / row_pitch).floor() - 1.0).max(0.0);
    let last = (((clip.bottom() - origin_y) / row_pitch).ceil() + 1.0).max(0.0);
    // `as usize` saturates, so an absurd clip rect clamps instead of wrapping.
    let first = (first as usize).min(max_row);
    let last = (last as usize).clamp(first, max_row);
    (first, last)
}

/// Deterministically maps a category name to a color from a 6-color palette via hashing.
/// Same category name always maps to the same color. Used to visually distinguish categories in badges.
fn category_color(cat: &str) -> egui::Color32 {
    let palette = [
        egui::Color32::from_rgb(0x3b, 0x82, 0xf6),
        egui::Color32::from_rgb(0x8b, 0x5c, 0xf6),
        egui::Color32::from_rgb(0x10, 0xb9, 0x81),
        egui::Color32::from_rgb(0xf5, 0x9e, 0x0b),
        egui::Color32::from_rgb(0xef, 0x44, 0x44),
        egui::Color32::from_rgb(0x06, 0xb6, 0xd4),
    ];
    let mut h: usize = 0;
    for b in cat.bytes() {
        h = h.wrapping_mul(31).wrapping_add(b as usize);
    }
    palette[h % palette.len()]
}

/// Finds the closest insertion gap (0 to n, inclusive) based on the pointer position.
/// A "gap" is a logical position between cards in the filtered grid: gap 0 is before the first card,
/// gap n is after the last card, and gaps in between are the spaces between adjacent cards (both
/// vertical within a row and horizontal between rows). The function computes a representative
/// point for each gap (via gap_point) and returns the index of the gap whose point is closest
/// to the current pointer position. This guides the insertion line and drop target.
/// Returns 0 if the card list is empty (edge case: no cards to reorder against).
fn nearest_gap(
    pointer: egui::Pos2,
    rects: &[egui::Rect],
    cols: usize,
    spacing: f32,
    card_w: f32,
) -> usize {
    let n = rects.len();
    if n == 0 {
        return 0;
    }

    let mut best = 0;
    let mut best_dist = f32::INFINITY;

    for g in 0..=n {
        let p = gap_point(g, rects, cols, spacing, card_w, pointer);
        let d = pointer.distance(p);
        if d < best_dist {
            best_dist = d;
            best = g;
        }
    }

    best
}

/// Computes a representative point for a given gap in the grid, used for distance-based
/// gap selection. The grid is arranged in rows of `cols` cards each.
/// - If g is a vertical gap (between cards in the same row), return the midpoint between them.
/// - If g is a horizontal gap (between rows), return a point centered on the column nearest
///   the pointer's x-coordinate. This ensures the insertion line aligns with the pointer's
///   intended column even when dragging over empty space between rows.
fn gap_point(
    g: usize,
    rects: &[egui::Rect],
    cols: usize,
    spacing: f32,
    _card_w: f32,
    pointer: egui::Pos2,
) -> egui::Pos2 {
    let n = rects.len();

    // Same-row vertical gap: return the point midway between the two adjacent cards.
    if g > 0 && g < n && !g.is_multiple_of(cols) {
        let x = (rects[g - 1].right() + rects[g].left()) * 0.5;
        let y = rects[g].center().y;
        return egui::pos2(x, y);
    }

    // Row-boundary horizontal gap: y-coordinate centered in the gap; x-coordinate follows pointer.
    let y = if g == 0 {
        rects[0].top() - spacing * 0.5
    } else if g == n {
        rects[n - 1].bottom() + spacing * 0.5
    } else {
        (rects[g - 1].bottom() + rects[g].top()) * 0.5
    };

    // Find the card column whose x-center is closest to the pointer's x position.
    // `total_cmp` keeps this from panicking if a coordinate is ever NaN.
    let nearest_x = rects
        .iter()
        .map(|r| r.center().x)
        .min_by(|a, b| (a - pointer.x).abs().total_cmp(&(b - pointer.x).abs()))
        .unwrap_or(rects[0].center().x);

    egui::pos2(nearest_x, y)
}

/// Renders a dashed insertion line indicating where the dragged card would be dropped.
/// For vertical gaps (between cards in the same row), draws a short vertical dashed line centered
/// between the two cards. For horizontal gaps (between rows), draws a horizontal dashed line
/// centered on the column nearest the pointer's x-position. Both cases include clearance checks
/// to ensure the line doesn't visually overlap with adjacent cards (which would be confusing).
/// The line only draws if its computed length is non-zero and it won't intersect any cards.
fn draw_insertion_line(
    ctx: &egui::Context,
    gap: usize,
    rects: &[egui::Rect],
    cols: usize,
    spacing: f32,
    card_w: f32,
    card_h: f32,
) {
    let n = rects.len();
    if n == 0 {
        return;
    }
    let pointer = ctx.input(|i| i.pointer.interact_pos().unwrap_or_default());

    let color = egui::Color32::from_rgb(0x60, 0xb0, 0xff);
    let stroke = egui::Stroke::new(2.0_f32, color);
    let clearance = 4.0_f32; // Minimum distance the line must maintain from card edges

    let painter = ctx.layer_painter(egui::LayerId::new(
        egui::Order::Tooltip,
        egui::Id::new("drag_ghost"),
    ));

    if gap > 0 && gap < n && !gap.is_multiple_of(cols) {
        // Vertical dashed line between two cards on the same row.
        let a = &rects[gap - 1];
        let b = &rects[gap];
        let x = (a.right() + b.left()) * 0.5;
        let y_center = a.center().y;
        // Make the line span ~35% of the card height, but never extend past the card boundaries.
        let half_len = (card_h * 0.35).min(a.height() * 0.5 - clearance);
        let from = egui::pos2(x, y_center - half_len);
        let to = egui::pos2(x, y_center + half_len);
        let line_rect = line_segment_rect(from, to, stroke.width);
        if half_len > 0.0 && !line_rect.intersects(*a) && !line_rect.intersects(*b) {
            draw_dashed_line(&painter, from, to, 6.0, 4.0, stroke);
        }
    } else {
        // Horizontal dashed line at a row boundary (top, between rows, or bottom).
        let (y, gap_top, gap_bottom, above, below) = if gap == 0 {
            (
                rects[0].top() - spacing * 0.5,
                rects[0].top() - spacing,
                rects[0].top(),
                None,
                Some(&rects[0]),
            )
        } else if gap == n {
            (
                rects[n - 1].bottom() + spacing * 0.5,
                rects[n - 1].bottom(),
                rects[n - 1].bottom() + spacing,
                Some(&rects[n - 1]),
                None,
            )
        } else {
            (
                (rects[gap - 1].bottom() + rects[gap].top()) * 0.5,
                rects[gap - 1].bottom(),
                rects[gap].top(),
                Some(&rects[gap - 1]),
                Some(&rects[gap]),
            )
        };

        // Center the horizontal line on the column of the nearest card to guide the drop location.
        let nearest = rects
            .iter()
            .min_by(|a, b| {
                (a.center().x - pointer.x)
                    .abs()
                    .total_cmp(&(b.center().x - pointer.x).abs())
            })
            .unwrap_or(&rects[0]);
        let x_center = nearest.center().x;
        let half_len = (card_w * 0.35).min(nearest.width() * 0.5 - clearance);
        let from = egui::pos2(x_center - half_len, y);
        let to = egui::pos2(x_center + half_len, y);
        let line_rect = line_segment_rect(from, to, stroke.width);
        // Only draw if the line has positive length and maintains clearance from adjacent cards.
        let mut clear = half_len > 0.0
            && y > gap_top + clearance
            && y < gap_bottom - clearance;
        if let Some(a) = above {
            clear &= !line_rect.intersects(*a);
        }
        if let Some(b) = below {
            clear &= !line_rect.intersects(*b);
        }
        if clear {
            draw_dashed_line(&painter, from, to, 6.0, 4.0, stroke);
        }
    }
}

/// Computes a tight bounding rect of a line segment, including its stroke thickness on all sides.
/// Used to check for visual overlap between the insertion line and adjacent cards during drag-and-drop.
fn line_segment_rect(from: egui::Pos2, to: egui::Pos2, stroke_width: f32) -> egui::Rect {
    let half = stroke_width * 0.5;
    egui::Rect::from_min_max(
        (from.min(to)) - egui::vec2(half, half),
        (from.max(to)) + egui::vec2(half, half),
    )
}

/// Draws a dashed line by rendering alternating solid segments (dashes) and transparent gaps.
/// This creates a visual "dashed" effect without needing special stroke rendering. The line
/// is drawn along the direction from `from` to `to`, and `dash_len` / `gap_len` control
/// the length of each dash and the space between them (both in screen pixels).
fn draw_dashed_line(
    painter: &egui::Painter,
    from: egui::Pos2,
    to: egui::Pos2,
    dash_len: f32,
    gap_len: f32,
    stroke: egui::Stroke,
) {
    let vec = to - from;
    let total = vec.length();
    if total <= 0.0 {
        return;
    }
    let dir = vec / total;
    let mut pos = 0.0;
    let mut drawing_dash = true;
    while pos < total {
        let seg_len = if drawing_dash {
            dash_len.min(total - pos)
        } else {
            gap_len.min(total - pos)
        };
        if drawing_dash {
            // Only draw the solid segment; skip gaps by not painting them.
            let a = from + dir * pos;
            let b = from + dir * (pos + seg_len);
            painter.line_segment([a, b], stroke);
        }
        pos += seg_len;
        drawing_dash = !drawing_dash; // Alternate between drawing and skipping
    }
}

/// Unit tests for grid layout and drag-and-drop logic.
/// Validates that card positioning, gap detection, and insertion line rendering work correctly
/// across different grid configurations (single and multi-card layouts).
#[cfg(test)]
mod layout_tests {
    use super::*;

    /// Builds an app whose data files live in a throwaway temp directory, so tests that
    /// exercise the auto-save paths never write into the repository or clobber real user data.
    fn test_app(name: &str, snippets: Vec<Snippet>, categories: Vec<String>) -> CopyIt {
        let dir = std::env::temp_dir().join(format!("copyit-app-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("create temp data dir");
        let next_id = snippets.iter().map(|s| s.id).max().unwrap_or(0) + 1;
        let mut app = CopyIt {
            snippets,
            derived: Vec::new(),
            generation: 0,
            filter: FilterCache::default(),
            next_id,
            path: dir.join("snippets.json"),
            config_path: dir.join("config.json"),
            categories,
            search: String::new(),
            category_filter: "All".into(),
            theme: Theme::Dark,
            editor: None,
            copied: None,
            drag: None,
            adding_header_category: false,
            new_header_category: String::new(),
            category_error: None,
            save_error: None,
            password_prompt: None,
        };
        app.rebuild_derived();
        app
    }

    fn snippet(id: u64, category: &str) -> Snippet {
        Snippet {
            id,
            title: format!("Snippet {id}"),
            category: category.to_string(),
            body: format!("body {id}"),
            is_secure: false,
            password_hash: String::new(),
        }
    }

    /// Snippet ids in stored order — the thing drag-and-drop reordering has to get right.
    fn ids(app: &CopyIt) -> Vec<u64> {
        app.snippets.iter().map(|s| s.id).collect()
    }

    #[test]
    fn real_card_rects_and_gaps() {
        let ctx = egui::Context::default();
        let input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::pos2(0.0, 0.0),
                egui::vec2(1000.0, 700.0),
            )),
            ..Default::default()
        };
        let app = test_app(
            "card-rects",
            vec![snippet(1, "Git"), snippet(2, "Git")],
            vec!["Git".into()],
        );
        let _ = ctx.run(input, |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing = egui::vec2(0.0, 0.0);
                    let mut actions = Vec::new();
                    let w1 = app.card(ui, 0, 300.0, 0.0, &mut actions, false).frame_rect;
                    ui.add_space(12.0);
                    let w2 = app.card(ui, 1, 300.0, 0.0, &mut actions, false).frame_rect;
                    assert!((w1.width() - 320.0).abs() < 0.1);
                    assert!((w1.height() - 188.0).abs() < 0.1);
                    let gap = w2.left() - w1.right();
                    assert!((gap - 12.0).abs() < 0.1, "gap = {}", gap);
                    let midpoint = (w1.right() + w2.left()) * 0.5;
                    assert!((midpoint - (w1.right() + gap * 0.5)).abs() < 0.1);
                });
            });
        });
    }

    #[test]
    fn grid_rects_and_gaps() {
        let ctx = egui::Context::default();
        let input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::pos2(0.0, 0.0),
                egui::vec2(1000.0, 700.0),
            )),
            ..Default::default()
        };
        let mut cols_out = 0;
        let mut rects_out: Vec<egui::Rect> = Vec::new();
        let _ = ctx.run(input, |ctx| {
            egui::TopBottomPanel::top("top_test").show(ctx, |ui| {
                ui.add_space(6.0);
                ui.horizontal(|ui| {
                    ui.heading("CopyIt");
                    ui.add_space(16.0);
                    ui.label("Search");
                    ui.add(egui::TextEdit::singleline(&mut String::new()).desired_width(260.0));
                    ui.add_space(12.0);
                    ui.label("Category");
                    ui.add_space(12.0);
                    ui.label("Theme");
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        let _ = ui.button("New");
                    });
                });
                ui.add_space(6.0);
            });
            egui::CentralPanel::default().show(ctx, |ui| {
                let scroll = egui::ScrollArea::vertical()
                    .auto_shrink([false; 2])
                    .show(ui, |ui| {
                        ui.spacing_mut().item_spacing = egui::vec2(0.0, 0.0);
                        ui.add_space(4.0);

                        let card_inner_w = 300.0_f32;
                        let card_inner_h = 168.0_f32;
                        let card_frame_margin = 20.0_f32;
                        let card_w = card_inner_w + card_frame_margin;
                        let spacing = 12.0_f32;
                        let margin_x = 18.0_f32;

                        egui::Frame::none()
                            .inner_margin(egui::Margin::symmetric(margin_x, 0.0))
                            .show(ui, |ui| {
                                let avail = ui.available_width();
                                let cols = ((avail / (card_w + spacing)).floor() as usize).max(1);
                                let mut rects: Vec<egui::Rect> = Vec::new();
                                let indices: Vec<usize> = (0..7).collect();
                                for row in indices.chunks(cols) {
                                    ui.horizontal(|ui| {
                                        ui.spacing_mut().item_spacing = egui::vec2(0.0, 0.0);
                                        for _ in row {
                                            let resp = egui::Frame::group(ui.style())
                                                .rounding(egui::Rounding::same(8.0))
                                                .inner_margin(egui::Margin::same(10.0))
                                                .fill(ui.visuals().extreme_bg_color)
                                                .show(ui, |ui| {
                                                    ui.set_width(card_inner_w);
                                                    ui.set_height(card_inner_h);
                                                });
                                            rects.push(resp.response.rect);
                                            ui.add_space(spacing);
                                        }
                                    });
                                    ui.add_space(spacing);
                                }
                                (cols, rects)
                            })
                            .inner
                    });
                cols_out = scroll.inner.0;
                rects_out = scroll.inner.1.clone();
            });
        });

        assert_eq!(cols_out, 2);
        for r in &rects_out {
            assert!((r.width() - 320.0).abs() < 0.1);
            assert!((r.height() - 188.0).abs() < 0.1);
        }
        for g in 1..rects_out.len() {
            if g % cols_out != 0 {
                let a = &rects_out[g - 1];
                let b = &rects_out[g];
                let gap = b.left() - a.right();
                let midpoint = (a.right() + b.left()) * 0.5;
                assert!((gap - 12.0).abs() < 0.1, "vertical gap {} = {}", g, gap);
                assert!((midpoint - (a.right() + gap * 0.5)).abs() < 0.1);
            } else {
                let a = &rects_out[g - 1];
                let b = &rects_out[g];
                let gap = b.top() - a.bottom();
                let midpoint = (a.bottom() + b.top()) * 0.5;
                assert!(
                    (gap - 12.0).abs() < 0.1,
                    "horizontal gap {} = {}",
                    g,
                    gap
                );
                assert!((midpoint - (a.bottom() + gap * 0.5)).abs() < 0.1);
            }
        }
    }

    #[test]
    fn real_card_grid_rects_and_gaps() {
        let ctx = egui::Context::default();
        let input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::pos2(0.0, 0.0),
                egui::vec2(1000.0, 700.0),
            )),
            ..Default::default()
        };
        let app = test_app(
            "card-grid",
            vec![
                snippet(1, "Git"),
                snippet(2, "Git"),
                snippet(3, "Prompt"),
                snippet(4, "Prompt"),
            ],
            vec!["Git".into(), "Prompt".into()],
        );
        let filtered: Vec<usize> = (0..app.snippets.len()).collect();
        let mut rects_out: Vec<egui::Rect> = Vec::new();
        let _ = ctx.run(input, |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                let scroll = egui::ScrollArea::vertical()
                    .auto_shrink([false; 2])
                    .show(ui, |ui| {
                        ui.spacing_mut().item_spacing = egui::vec2(0.0, 0.0);
                        ui.add_space(4.0);

                        let card_inner_w = 300.0_f32;
                        let card_frame_margin = 20.0_f32;
                        let card_w = card_inner_w + card_frame_margin;
                        let spacing = 12.0_f32;
                        let margin_x = 18.0_f32;

                        egui::Frame::none()
                            .inner_margin(egui::Margin::symmetric(margin_x, 0.0))
                            .show(ui, |ui| {
                                let avail = ui.available_width();
                                let cols = ((avail / (card_w + spacing)).floor() as usize).max(1);
                                let mut rects: Vec<egui::Rect> = Vec::new();
                                for row in filtered.chunks(cols) {
                                    ui.horizontal(|ui| {
                                        ui.spacing_mut().item_spacing = egui::vec2(0.0, 0.0);
                                        for &idx in row {
                                            let mut actions = Vec::new();
                                            let widgets = app.card(
                                                ui, idx, card_inner_w, 0.0, &mut actions, false,
                                            );
                                            rects.push(widgets.frame_rect);
                                            ui.add_space(spacing);
                                        }
                                    });
                                    ui.add_space(spacing);
                                }
                                (cols, rects)
                            })
                            .inner
                    });
                rects_out = scroll.inner.1.clone();
                assert_eq!(scroll.inner.0, 2);
            });
        });

        assert_eq!(rects_out.len(), 4);
        for r in &rects_out {
            assert!((r.width() - 320.0).abs() < 0.1);
            assert!((r.height() - 188.0).abs() < 0.1);
        }
        let v_gap = rects_out[1].left() - rects_out[0].right();
        let v_mid = (rects_out[0].right() + rects_out[1].left()) * 0.5;
        assert!((v_gap - 12.0).abs() < 0.1, "vertical gap = {}", v_gap);
        assert!((v_mid - (rects_out[0].right() + v_gap * 0.5)).abs() < 0.1);
        let h_gap = rects_out[2].top() - rects_out[0].bottom();
        let h_mid = (rects_out[0].bottom() + rects_out[2].top()) * 0.5;
        assert!((h_gap - 12.0).abs() < 0.1, "horizontal gap = {}", h_gap);
        assert!((h_mid - (rects_out[0].bottom() + h_gap * 0.5)).abs() < 0.1);
    }

    #[test]
    fn insertion_line_positions_are_centered_and_clear() {
        // Replicates the exact grid layout and checks that gap_point / line math
        // produces a point centered in the gap with clearance from both cards.
        let ctx = egui::Context::default();
        let input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::pos2(0.0, 0.0),
                egui::vec2(1000.0, 700.0),
            )),
            ..Default::default()
        };
        let mut rects: Vec<egui::Rect> = Vec::new();
        let mut cols = 0;
        let _ = ctx.run(input, |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                let scroll = egui::ScrollArea::vertical()
                    .auto_shrink([false; 2])
                    .show(ui, |ui| {
                        ui.spacing_mut().item_spacing = egui::vec2(0.0, 0.0);
                        ui.add_space(4.0);

                        let card_inner_w = 300.0_f32;
                        let card_frame_margin = 20.0_f32;
                        let card_w = card_inner_w + card_frame_margin;
                        let card_inner_h = 168.0_f32;
                        let spacing = 12.0_f32;
                        let margin_x = 18.0_f32;

                        let (c, rs) = egui::Frame::none()
                            .inner_margin(egui::Margin::symmetric(margin_x, 0.0))
                            .show(ui, |ui| {
                                let avail = ui.available_width();
                                let c = ((avail / (card_w + spacing)).floor() as usize).max(1);
                                let mut rs: Vec<egui::Rect> = Vec::new();
                                let indices: Vec<usize> = (0..7).collect();
                                for row in indices.chunks(c) {
                                    ui.horizontal(|ui| {
                                        ui.spacing_mut().item_spacing = egui::vec2(0.0, 0.0);
                                        for _ in row {
                                            let resp = egui::Frame::group(ui.style())
                                                .rounding(egui::Rounding::same(8.0))
                                                .inner_margin(egui::Margin::same(10.0))
                                                .fill(ui.visuals().extreme_bg_color)
                                                .show(ui, |ui| {
                                                    ui.set_width(card_inner_w);
                                                    ui.set_height(card_inner_h);
                                                });
                                            rs.push(resp.response.rect);
                                            ui.add_space(spacing);
                                        }
                                    });
                                    ui.add_space(spacing);
                                }
                                (c, rs)
                            })
                            .inner;
                        (c, rs)
                    });
                cols = scroll.inner.0;
                rects = scroll.inner.1.clone();
            });
        });

        assert_eq!(cols, 2);
        let spacing = 12.0_f32;
        let clearance = 4.0_f32;
        let stroke_width = 2.0_f32;

        for g in 1..rects.len() {
            let pointer = if g % cols != 0 {
                // Vertical gap: pointer in the middle of the gap.
                let a = &rects[g - 1];
                let b = &rects[g];
                let x = (a.right() + b.left()) * 0.5;
                let y = a.center().y;
                egui::pos2(x, y)
            } else {
                // Horizontal gap: pointer in the middle of the gap, under the left column.
                let a = &rects[g - 1];
                let b = &rects[g];
                let x = a.center().x;
                let y = (a.bottom() + b.top()) * 0.5;
                egui::pos2(x, y)
            };
            let p = gap_point(g, &rects, cols, spacing, 320.0, pointer);

            if g % cols != 0 {
                let a = &rects[g - 1];
                let b = &rects[g];
                let gap = b.left() - a.right();
                let expected_x = a.right() + gap * 0.5;
                assert!((p.x - expected_x).abs() < 0.1, "vertical gap {} x = {}, expected {}", g, p.x, expected_x);
                assert!((p.y - a.center().y).abs() < 0.1);
                // 2px vertical line centered in x must not intersect either card.
                let line_rect = egui::Rect::from_min_max(
                    egui::pos2(p.x - stroke_width * 0.5, p.y - 60.0),
                    egui::pos2(p.x + stroke_width * 0.5, p.y + 60.0),
                );
                assert!(!line_rect.intersects(*a), "vertical line intersects left card");
                assert!(!line_rect.intersects(*b), "vertical line intersects right card");
                assert!(p.x > a.right() + clearance - stroke_width * 0.5);
                assert!(p.x < b.left() - clearance + stroke_width * 0.5);
            } else {
                let a = &rects[g - 1];
                let b = &rects[g];
                let gap = b.top() - a.bottom();
                let expected_y = a.bottom() + gap * 0.5;
                assert!((p.y - expected_y).abs() < 0.1, "horizontal gap {} y = {}, expected {}", g, p.y, expected_y);
                // 2px horizontal line centered in y must not intersect either card.
                let line_rect = egui::Rect::from_min_max(
                    egui::pos2(p.x - 112.0, p.y - stroke_width * 0.5),
                    egui::pos2(p.x + 112.0, p.y + stroke_width * 0.5),
                );
                assert!(!line_rect.intersects(*a), "horizontal line intersects above card");
                assert!(!line_rect.intersects(*b), "horizontal line intersects below card");
                assert!(p.y > a.bottom() + clearance - stroke_width * 0.5);
                assert!(p.y < b.top() - clearance + stroke_width * 0.5);
            }
        }
    }

    /// Runs a single frame of the real card grid in a 1000x700 window at the given
    /// scroll offset. Returns the number of paint shapes the frame emitted (a proxy for
    /// how many cards were actually built), the rects the grid reported for every card,
    /// the scroll area's content size, and the column count.
    fn run_grid(
        app: &CopyIt,
        scroll_offset: f32,
    ) -> (usize, Vec<egui::Rect>, egui::Vec2, usize) {
        let ctx = egui::Context::default();
        let input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::pos2(0.0, 0.0),
                egui::vec2(1000.0, 700.0),
            )),
            ..Default::default()
        };
        let filtered: Vec<usize> = (0..app.snippets.len()).collect();
        let mut rects = Vec::new();
        let mut content_size = egui::Vec2::ZERO;
        let mut cols = 0;

        let output = ctx.run(input, |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                let scroll = egui::ScrollArea::vertical()
                    .auto_shrink([false; 2])
                    .vertical_scroll_offset(scroll_offset)
                    .show(ui, |ui| {
                        ui.spacing_mut().item_spacing = egui::vec2(0.0, 0.0);
                        ui.add_space(GRID_TOP_SPACE);
                        let mut actions = Vec::new();
                        let mut drag_start = None;
                        let mut hover_cursor = None;
                        app.card_grid(
                            ui,
                            &filtered,
                            0.0,
                            &mut actions,
                            &mut drag_start,
                            &mut hover_cursor,
                        )
                    });
                cols = scroll.inner.0;
                rects = scroll.inner.1.clone();
                content_size = scroll.content_size;
            });
        });

        (output.shapes.len(), rects, content_size, cols)
    }

    /// The grid must only build the cards near the viewport, while still reporting the
    /// geometry of the whole library and reserving its full scroll height.
    #[test]
    fn card_grid_only_builds_the_rows_in_view() {
        let small = test_app(
            "grid-small",
            (1..=40).map(|i| snippet(i, "Git")).collect(),
            vec!["Git".into()],
        );
        let large = test_app(
            "grid-large",
            (1..=400).map(|i| snippet(i, "Git")).collect(),
            vec!["Git".into()],
        );

        let (small_shapes, small_rects, small_content, cols) = run_grid(&small, 0.0);
        let (large_shapes, large_rects, large_content, large_cols) = run_grid(&large, 0.0);
        assert_eq!(cols, 2);
        assert_eq!(large_cols, 2);

        // Every card is accounted for, on-screen or not.
        assert_eq!(small_rects.len(), 40);
        assert_eq!(large_rects.len(), 400);

        // The scroll range still spans the whole library.
        let expected_height = |n: usize| GRID_TOP_SPACE + (n as f32 / 2.0).ceil() * ROW_PITCH;
        assert!(
            (small_content.y - expected_height(40)).abs() < 1.0,
            "content height {} != {}",
            small_content.y,
            expected_height(40)
        );
        assert!(
            (large_content.y - expected_height(400)).abs() < 1.0,
            "content height {} != {}",
            large_content.y,
            expected_height(400)
        );

        // Ten times the library, but the same viewport: the frame's paint work must stay
        // roughly constant. Without virtualization the 400-snippet grid emitted ten times
        // the shapes of the 40-snippet one.
        assert!(
            large_shapes <= small_shapes * 3 / 2,
            "large grid emitted {large_shapes} shapes vs {small_shapes} for a tenth of the library"
        );
        assert!(small_shapes > 20, "the visible cards must actually be painted");

        // Scrolled deep into the library, cards are still painted (i.e. the visible band
        // follows the viewport instead of staying at the top)...
        let deep_offset = 60.0 * ROW_PITCH;
        let (deep_shapes, deep_rects, _, _) = run_grid(&large, deep_offset);
        assert!(
            deep_shapes >= small_shapes / 2,
            "scrolled grid emitted only {deep_shapes} shapes"
        );

        // ...and the reported geometry is a uniform grid whose rows are ROW_PITCH apart,
        // shifted by the scroll offset.
        for (i, r) in deep_rects.iter().enumerate() {
            assert!((r.width() - CARD_W).abs() < 0.1);
            assert!((r.height() - CARD_H).abs() < 0.1);
            let expected = grid_card_rect(i, 2, deep_rects[0].min, CARD_W, CARD_H, CARD_SPACING);
            assert!(r.min.distance(expected.min) < 0.1, "card {i} at {:?}", r.min);
        }
        assert!(
            (deep_rects[0].min.y - (large_rects[0].min.y - deep_offset)).abs() < 1.0,
            "scrolling must shift the grid geometry by the scroll offset"
        );

        // Drag-and-drop can still target a gap on a row that was never laid out: the
        // pointer sits in the row-boundary gap after card 150.
        let gap_index = 150;
        let above = deep_rects[gap_index - 2];
        let below = deep_rects[gap_index];
        let pointer = egui::pos2(
            above.center().x,
            (above.bottom() + below.top()) * 0.5,
        );
        assert_eq!(
            nearest_gap(pointer, &deep_rects, 2, CARD_SPACING, CARD_W),
            gap_index,
            "an off-screen gap must still be a valid drop target"
        );
    }

    /// The grid only lays out the rows in view, so the rects it hands to the
    /// drag-and-drop code are computed from the grid origin instead of harvested from
    /// the layout. Those computed rects must match what an actually-rendered card gets,
    /// or every drop target would be off.
    #[test]
    fn computed_grid_rects_match_rendered_cards() {
        let ctx = egui::Context::default();
        let input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::pos2(0.0, 0.0),
                egui::vec2(1000.0, 700.0),
            )),
            ..Default::default()
        };
        let app = test_app(
            "computed-rects",
            (1..=7).map(|i| snippet(i, "Git")).collect(),
            vec!["Git".into()],
        );
        let filtered: Vec<usize> = (0..app.snippets.len()).collect();

        let _ = ctx.run(input, |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                egui::ScrollArea::vertical()
                    .auto_shrink([false; 2])
                    .show(ui, |ui| {
                        ui.spacing_mut().item_spacing = egui::vec2(0.0, 0.0);
                        ui.add_space(4.0);

                        let card_inner_w = 300.0_f32;
                        let card_inner_h = 168.0_f32;
                        let card_frame_margin = 20.0_f32;
                        let card_w = card_inner_w + card_frame_margin;
                        let card_h = card_inner_h + card_frame_margin;
                        let spacing = 12.0_f32;

                        egui::Frame::none()
                            .inner_margin(egui::Margin::symmetric(18.0, 0.0))
                            .show(ui, |ui| {
                                let avail = ui.available_width();
                                let cols = ((avail / (card_w + spacing)).floor() as usize).max(1);
                                let origin = ui.cursor().min;
                                assert_eq!(cols, 2);

                                for (i, chunk_start) in (0..filtered.len()).step_by(cols).enumerate()
                                {
                                    let row = &filtered[chunk_start
                                        ..(chunk_start + cols).min(filtered.len())];
                                    ui.horizontal(|ui| {
                                        ui.spacing_mut().item_spacing = egui::vec2(0.0, 0.0);
                                        for (offset, &idx) in row.iter().enumerate() {
                                            let mut actions = Vec::new();
                                            let rendered = app
                                                .card(ui, idx, card_inner_w, 0.0, &mut actions, false)
                                                .frame_rect;
                                            let computed = grid_card_rect(
                                                i * cols + offset,
                                                cols,
                                                origin,
                                                card_w,
                                                card_h,
                                                spacing,
                                            );
                                            assert!(
                                                rendered.min.distance(computed.min) < 0.1
                                                    && rendered.max.distance(computed.max) < 0.1,
                                                "card {} rendered at {:?}, computed {:?}",
                                                i * cols + offset,
                                                rendered,
                                                computed,
                                            );
                                            ui.add_space(spacing);
                                        }
                                    });
                                    ui.add_space(spacing);
                                }
                            });
                    });
            });
        });
    }

    /// Row virtualization must cover everything the viewport can show (plus a row of
    /// overscan) and never hand back a range outside the grid.
    #[test]
    fn visible_rows_covers_the_viewport_and_stays_in_bounds() {
        let row_pitch = 200.0_f32;
        let origin_y = 100.0_f32;
        let rows = 50;

        // Scrolled to the top: starts at row 0, reaches past the bottom of the viewport.
        let clip = egui::Rect::from_min_max(egui::pos2(0.0, 100.0), egui::pos2(1000.0, 700.0));
        let (first, last) = visible_rows(clip, origin_y, row_pitch, rows);
        assert_eq!(first, 0);
        assert!(last >= 3, "viewport spans 3 rows, got last = {last}");
        assert!(last < rows);

        // Scrolled into the middle: the visible band is covered with overscan on both
        // sides, and rows far above/below are skipped.
        let clip = egui::Rect::from_min_max(egui::pos2(0.0, 2100.0), egui::pos2(1000.0, 2700.0));
        let (first, last) = visible_rows(clip, origin_y, row_pitch, rows);
        assert!((8..=9).contains(&first), "first = {first}");
        assert!(last >= 13, "last = {last}");
        assert!(first > 0, "rows above the viewport must be skipped");
        assert!(last < rows - 1, "rows below the viewport must be skipped");

        // Every row of the grid is reachable by some scroll position.
        for row in 0..rows {
            let y = origin_y + row as f32 * row_pitch;
            let clip = egui::Rect::from_min_max(egui::pos2(0.0, y), egui::pos2(1000.0, y + 10.0));
            let (first, last) = visible_rows(clip, origin_y, row_pitch, rows);
            assert!(
                first <= row && row <= last,
                "row {row} not rendered for its own scroll position ({first}..={last})"
            );
            assert!(last < rows);
        }

        // Degenerate inputs fall back to rendering everything rather than nothing.
        assert_eq!(visible_rows(clip, origin_y, 0.0, rows), (0, rows - 1));
        assert_eq!(visible_rows(clip, f32::NAN, row_pitch, rows), (0, rows - 1));
        assert_eq!(visible_rows(clip, origin_y, row_pitch, 1), (0, 0));
        assert_eq!(visible_rows(clip, origin_y, row_pitch, 0), (0, 0));
    }

    /// The filter is memoized: it must return the same indices the old
    /// scan-every-frame code did, and it must be recomputed when the query, the
    /// category, or the library changes.
    #[test]
    fn filter_cache_matches_a_fresh_scan_and_invalidates_on_change() {
        let mut app = test_app(
            "filter-cache",
            vec![
                Snippet {
                    id: 1,
                    title: "Rebase onto main".into(),
                    category: "Git".into(),
                    body: "git rebase origin/MAIN".into(),
                },
                Snippet {
                    id: 2,
                    title: "Summarize".into(),
                    category: "Prompt".into(),
                    body: "Summarize the following text".into(),
                },
                Snippet {
                    id: 3,
                    title: "Stash".into(),
                    category: "Git".into(),
                    body: "git stash pop".into(),
                },
            ],
            vec!["Git".into(), "Prompt".into()],
        );

        // Reference implementation: the un-cached filter this replaced.
        let expected = |app: &CopyIt| -> Vec<usize> {
            let q = app.search.trim().to_lowercase();
            app.snippets
                .iter()
                .enumerate()
                .filter(|(_, s)| {
                    (app.category_filter == "All" || s.category == app.category_filter)
                        && (q.is_empty()
                            || s.title.to_lowercase().contains(&q)
                            || s.body.to_lowercase().contains(&q)
                            || s.category.to_lowercase().contains(&q))
                })
                .map(|(i, _)| i)
                .collect()
        };
        let check = |app: &mut CopyIt| {
            let want = expected(app);
            let got = app.take_filtered();
            assert_eq!(got, want, "search {:?} / cat {:?}", app.search, app.category_filter);
            app.restore_filtered(got);
        };

        check(&mut app); // everything

        // Taking the buffer twice without restoring it must not report "no matches".
        let first = app.take_filtered();
        let second = app.take_filtered();
        assert_eq!(first, second, "a checked-out cache must be recomputed, not reused");
        app.restore_filtered(second);

        // A cached result must not survive a changed query...
        app.search = "GIT".into(); // case-insensitive, matches bodies and the category
        check(&mut app);
        app.search = "  summarize  ".into(); // trimmed
        check(&mut app);
        app.search = "   ".into(); // whitespace-only behaves like empty
        check(&mut app);
        assert_eq!(app.take_filtered().len(), 3);
        let restored = vec![0, 1, 2];
        app.restore_filtered(restored);

        // ...a changed category filter...
        app.search.clear();
        app.category_filter = "Git".into();
        check(&mut app);
        assert_eq!(app.take_filtered(), vec![0, 2]);
        app.restore_filtered(vec![0, 2]);

        // ...or a changed library. Reordering keeps ids and derived data aligned.
        app.category_filter = "All".into();
        app.reorder(1, 3, &[0, 1, 2]);
        assert_eq!(ids(&app), vec![2, 3, 1]);
        check(&mut app);
        app.search = "rebase".into();
        assert_eq!(app.take_filtered(), vec![2], "the moved card is still findable");
        app.restore_filtered(vec![2]);

        // Deriving must track edits to a snippet's text, not just its position.
        app.snippets[2].body = "git rebase --abort".into();
        app.snippets[2].title = "Abort".into();
        app.snippets_changed();
        app.search = "abort".into();
        check(&mut app);
        assert_eq!(app.take_filtered(), vec![2]);
    }

    /// Card previews are cached; they must still collapse whitespace, truncate with an
    /// ellipsis, and count characters rather than bytes.
    #[test]
    fn preview_text_collapses_and_truncates() {
        assert_eq!(preview_text("  git   stash \n pop  ", 220), "git stash pop");
        assert_eq!(preview_text("", 220), "");

        let long = "word ".repeat(400);
        let preview = preview_text(&long, 10);
        assert_eq!(preview.chars().count(), 10);
        assert!(preview.ends_with('\u{2026}'));
        assert!(preview.starts_with("word word"));

        // Multi-byte characters are counted as characters.
        let unicode = "\u{00e9}".repeat(50);
        assert_eq!(preview_text(&unicode, 10).chars().count(), 10);
        assert_eq!(preview_text(&unicode, 100), unicode);

        // The cached preview is what the card renders.
        let app = test_app(
            "preview-cache",
            vec![Snippet {
                id: 1,
                title: "T".into(),
                category: "Git".into(),
                body: "  first    line\nsecond line  ".into(),
            }],
            vec!["Git".into()],
        );
        assert_eq!(app.derived[0].preview, "first line second line");
        assert_eq!(app.derived[0].body_lower, "  first    line\nsecond line  ");
    }

    #[test]
    fn add_category_normalizes_and_dedups() {
        let mut app = test_app(
            "add-category",
            vec![],
            vec!["Git".into(), "Prompt".into()],
        );
        assert_eq!(app.add_category("  git "), "Git"); // existing, case-insensitive
        assert_eq!(app.add_category("werner"), "Werner"); // new
        assert_eq!(app.add_category("Werner"), "Werner"); // duplicate
        assert!(app.categories.contains(&"Werner".to_string()));
        assert_eq!(app.categories.len(), 3);

        assert_eq!(app.add_category("all"), "");
        assert_eq!(app.add_category("All"), "");
        assert_eq!(app.add_category("ALL"), "");
        assert!(!app
            .categories
            .iter()
            .any(|c| c.eq_ignore_ascii_case("all")));
        assert_eq!(app.categories.len(), 3);
    }

    #[test]
    fn uncategorized_fallback_is_registered_in_categories() {
        let mut app = test_app("uncategorized", vec![], vec![]);
        // Mirrors the Save-path category resolution in `update()`: a blank
        // `ed.category` (reachable via `Editor::blank` when `categories` is
        // empty) must fall back to "Uncategorized" AND register it.
        let ed_category = "";
        let category = {
            let canonical = app.add_category(ed_category);
            if canonical.is_empty() {
                app.add_category("Uncategorized")
            } else {
                canonical
            }
        };
        assert_eq!(category, "Uncategorized");
        assert!(app.categories.contains(&"Uncategorized".to_string()));
    }

    #[test]
    fn category_error_clears_when_input_changes() {
        let mut app = test_app("category-error-clears", vec![], vec!["Git".into()]);
        app.adding_header_category = true;
        app.new_header_category = "al".into();
        app.category_error = Some("\"All\" is reserved and can't be used as a category".into());

        let previous = app.new_header_category.clone();
        app.new_header_category.push('l');
        app.clear_category_error_on_input_change(&previous);
        assert!(app.category_error.is_none());
    }

    #[test]
    fn category_error_lingers_when_input_unchanged() {
        let mut app = test_app("category-error-lingers", vec![], vec!["Git".into()]);
        app.adding_header_category = true;
        app.new_header_category = "all".into();
        app.category_error = Some("\"All\" is reserved and can't be used as a category".into());

        let previous = app.new_header_category.clone();
        app.clear_category_error_on_input_change(&previous);
        assert!(app.category_error.is_some());
    }

    /// Regression test for the drag-and-drop hit-testing bug.
    ///
    /// Rects harvested from inside a `ScrollArea` are already absolute screen
    /// coordinates with the scroll offset applied. The drag code used to translate them
    /// again by `inner_rect.min - state.offset`, so every gap the drop logic compared the
    /// cursor against sat one top-bar-height too low — and drifted further with every
    /// pixel scrolled. This pins the coordinate space in both scroll positions.
    #[test]
    fn scroll_area_card_rects_are_already_in_screen_space() {
        let spacing = 12.0_f32;
        let top_space = 4.0_f32;
        let margin_x = 18.0_f32;

        for scroll_offset in [0.0_f32, 150.0_f32] {
            let ctx = egui::Context::default();
            let input = egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::pos2(0.0, 0.0),
                    egui::vec2(1000.0, 400.0),
                )),
                ..Default::default()
            };
            let _ = ctx.run(input, |ctx| {
                // A top panel makes the CentralPanel start well below y = 0, which is
                // exactly what the bogus translation was silently adding back in.
                egui::TopBottomPanel::top("top_probe").show(ctx, |ui| {
                    ui.add_space(6.0);
                    ui.heading("CopyIt");
                    ui.add_space(6.0);
                });
                egui::CentralPanel::default().show(ctx, |ui| {
                    let scroll = egui::ScrollArea::vertical()
                        .auto_shrink([false; 2])
                        .vertical_scroll_offset(scroll_offset)
                        .show(ui, |ui| {
                            ui.spacing_mut().item_spacing = egui::vec2(0.0, 0.0);
                            ui.add_space(top_space);
                            egui::Frame::none()
                                .inner_margin(egui::Margin::symmetric(margin_x, 0.0))
                                .show(ui, |ui| {
                                    let mut rects = Vec::new();
                                    for _ in 0..6 {
                                        let resp = egui::Frame::group(ui.style())
                                            .inner_margin(egui::Margin::same(10.0))
                                            .show(ui, |ui| {
                                                ui.set_width(300.0);
                                                ui.set_height(168.0);
                                            });
                                        rects.push(resp.response.rect);
                                        ui.add_space(spacing);
                                    }
                                    rects
                                })
                                .inner
                        });

                    let rects = &scroll.inner;
                    assert_eq!(scroll.state.offset.y, scroll_offset);

                    // The untranslated rect already accounts for the panel origin, the
                    // frame margins, and the scroll offset.
                    let expected_first = egui::pos2(
                        scroll.inner_rect.min.x + margin_x,
                        scroll.inner_rect.min.y + top_space - scroll_offset,
                    );
                    assert!(
                        rects[0].min.distance(expected_first) < 0.1,
                        "offset {scroll_offset}: first card at {:?}, expected {expected_first:?}",
                        rects[0].min,
                    );
                    // Applying the old translation would have moved the cards away from
                    // where they are actually painted.
                    let stale_shift = (scroll.inner_rect.min - scroll.state.offset).to_vec2();
                    assert!(
                        stale_shift.length() > 1.0,
                        "the scenario must be one where the old translation was non-trivial"
                    );
                    assert!(
                        rects[0].translate(stale_shift).min.distance(expected_first) > 1.0,
                        "offset {scroll_offset}: translating again must be wrong"
                    );

                    // A pointer parked in the gap between cards 1 and 2 must select gap 2.
                    let cols = 1;
                    let pointer = egui::pos2(
                        rects[0].center().x,
                        (rects[1].bottom() + rects[2].top()) * 0.5,
                    );
                    assert_eq!(
                        nearest_gap(pointer, rects, cols, spacing, 320.0),
                        2,
                        "offset {scroll_offset}: drop target must match the cursor"
                    );

                    // With the old translation the insertion line was always painted
                    // `stale_shift` away from the gap it claimed to mark. Once that shift
                    // exceeds half a row's pitch — which happens as soon as the grid is
                    // scrolled — the *chosen drop slot* moves too, and the card lands in
                    // the wrong place.
                    let shifted: Vec<egui::Rect> =
                        rects.iter().map(|r| r.translate(stale_shift)).collect();
                    let row_pitch = rects[1].top() - rects[0].top();
                    let stale_gap = nearest_gap(pointer, &shifted, cols, spacing, 320.0);
                    if stale_shift.y.abs() > row_pitch * 0.5 {
                        assert_ne!(
                            stale_gap, 2,
                            "offset {scroll_offset}: the old translation must land on the wrong gap"
                        );
                    }
                });
            });
        }
    }

    #[test]
    fn reorder_moves_the_dragged_card_within_the_full_list() {
        let mut app = test_app(
            "reorder-all",
            vec![
                snippet(1, "Git"),
                snippet(2, "Git"),
                snippet(3, "Git"),
                snippet(4, "Git"),
            ],
            vec!["Git".into()],
        );
        let filtered: Vec<usize> = (0..4).collect();

        // Drop the first card into the gap between cards 2 and 3.
        app.reorder(1, 2, &filtered);
        assert_eq!(ids(&app), vec![2, 1, 3, 4]);

        // Drop it past the end.
        app.reorder(1, 4, &filtered);
        assert_eq!(ids(&app), vec![2, 3, 4, 1]);

        // Drop it back at the front.
        app.reorder(1, 0, &filtered);
        assert_eq!(ids(&app), vec![1, 2, 3, 4]);

        // A gap adjacent to the dragged card itself is a no-op, not an off-by-one.
        app.reorder(2, 1, &filtered);
        assert_eq!(ids(&app), vec![1, 2, 3, 4]);
        app.reorder(2, 2, &filtered);
        assert_eq!(ids(&app), vec![1, 2, 3, 4]);
    }

    #[test]
    fn reorder_respects_a_filtered_view_of_the_library() {
        // Visible cards are ids 1, 3, 5 (indices 0, 2, 4 of the full list).
        let mut app = test_app(
            "reorder-filtered",
            vec![
                snippet(1, "Git"),
                snippet(2, "Prompt"),
                snippet(3, "Git"),
                snippet(4, "Prompt"),
                snippet(5, "Git"),
            ],
            vec!["Git".into(), "Prompt".into()],
        );
        let filtered = vec![0usize, 2, 4];

        // Drag the first visible card (id 1) into the last visible gap.
        app.reorder(1, 3, &filtered);
        assert_eq!(ids(&app), vec![2, 3, 4, 5, 1]);
        // Hidden snippets keep their relative position to their visible neighbours.
        let visible: Vec<u64> = app
            .snippets
            .iter()
            .filter(|s| s.category == "Git")
            .map(|s| s.id)
            .collect();
        assert_eq!(visible, vec![3, 5, 1]);
    }

    /// A drag holds only the snippet id, so a library that changed mid-drag can't make the
    /// drop move the wrong card — or panic on an out-of-bounds `Vec::remove`.
    #[test]
    fn reorder_ignores_a_snippet_that_disappeared_mid_drag() {
        let mut app = test_app(
            "reorder-missing",
            vec![snippet(1, "Git"), snippet(2, "Git")],
            vec!["Git".into()],
        );
        app.reorder(99, 0, &[0, 1]);
        assert_eq!(ids(&app), vec![1, 2]);

        // An empty filtered view has no gaps to drop into either.
        app.reorder(1, 0, &[]);
        assert_eq!(ids(&app), vec![1, 2]);
    }

    /// A successful config write (switching themes, say) must not retire a banner that is
    /// still reporting an unsaved snippet library.
    #[test]
    fn a_successful_save_only_clears_its_own_error() {
        let mut app = test_app("save-error-scope", vec![snippet(1, "Git")], vec!["Git".into()]);

        app.save_error = Some(format!("{SNIPPETS_SAVE_ERROR}: disk full"));
        app.save_config();
        assert_eq!(
            app.save_error.as_deref(),
            Some("Couldn't save snippets: disk full"),
            "a config write must not hide a snippet-save failure"
        );

        app.save_snippets();
        assert!(app.save_error.is_none(), "the snippet save clears its own error");

        // Startup notices about recovered data files survive until dismissed.
        app.save_error = Some("snippets.json couldn't be read".to_string());
        app.save_snippets();
        app.save_config();
        assert!(app.save_error.is_some());
    }

    /// Saves must be crash-safe: a failed write leaves the previous file intact.
    #[test]
    fn saving_replaces_the_library_atomically() {
        let mut app = test_app(
            "atomic-save",
            vec![snippet(1, "Git"), snippet(2, "Git")],
            vec!["Git".into()],
        );
        app.save_snippets();
        assert!(app.save_error.is_none());

        let dir = app.path.parent().unwrap().to_path_buf();
        let leftovers: Vec<String> = std::fs::read_dir(&dir)
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .filter(|name| name.ends_with(".tmp"))
            .collect();
        assert!(leftovers.is_empty(), "temp files left behind: {leftovers:?}");

        match storage::load(&app.path) {
            storage::Load::Loaded(snippets) => {
                assert_eq!(snippets.iter().map(|s| s.id).collect::<Vec<_>>(), vec![1, 2]);
            }
            _ => panic!("saved library should load back"),
        }
    }

    /// Regression test: a very long snippet body must not make the editor
    /// window grow past the screen. Only the Content area should scroll.
    #[test]
    fn editor_window_height_is_clamped_for_long_content() {
        let ctx = egui::Context::default();
        let input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::pos2(0.0, 0.0),
                egui::vec2(1000.0, 700.0),
            )),
            ..Default::default()
        };

        let mut title = String::from("Long snippet");
        let mut category = String::from("Git");
        let mut body: String = (0..500)
            .map(|i| {
                format!(
                    "Line {i}: Lorem ipsum dolor sit amet, consectetur adipiscing elit.\n"
                )
            })
            .collect();

        let _ = ctx.run(input, |ctx| {
            egui::CentralPanel::default().show(ctx, |_ui| {
                let screen_height = ctx.screen_rect().height();
                let max_window_height = screen_height * 0.85;
                let reserved_for_fixed = 170.0;
                let max_content_height = (max_window_height - reserved_for_fixed).max(120.0);

                let resp = egui::Window::new("New snippet")
                    .collapsible(false)
                    .resizable(true)
                    .default_width(540.0)
                    .max_height(max_window_height)
                    .show(ctx, |ui| {
                        ui.horizontal(|ui| {
                            ui.vertical(|ui| {
                                ui.label("Title");
                                ui.add(
                                    egui::TextEdit::singleline(&mut title)
                                        .desired_width(280.0),
                                );
                            });

                            ui.add_space(12.0);

                            ui.vertical(|ui| {
                                ui.label("Category");
                                let mut selected = category.clone();
                                egui::ComboBox::from_id_source("cat_select_test")
                                    .width(180.0)
                                    .selected_text(category.clone())
                                    .show_ui(ui, |ui| {
                                        ui.selectable_value(
                                            &mut selected,
                                            "Git".to_string(),
                                            "Git",
                                        );
                                    });
                                category = selected;
                            });
                        });
                        ui.add_space(6.0);

                        ui.label("Content");
                        egui::ScrollArea::vertical()
                            .auto_shrink([false, true])
                            .max_height(max_content_height)
                            .show(ui, |ui| {
                                ui.add(
                                    egui::TextEdit::multiline(&mut body)
                                        .desired_width(f32::INFINITY)
                                        .desired_rows(12)
                                        .font(egui::TextStyle::Monospace),
                                );
                            });
                        ui.add_space(10.0);

                        ui.horizontal(|ui| {
                            let _ = ui.button("Save");
                            let _ = ui.button("Cancel");
                        });
                    });

                let rect = resp.unwrap().response.rect;
                assert!(
                    rect.height() <= max_window_height + 1.0,
                    "editor window height {} should not exceed max {}",
                    rect.height(),
                    max_window_height
                );
                assert!(
                    rect.bottom() <= screen_height + 1.0,
                    "editor window bottom {} should not exceed screen height {}",
                    rect.bottom(),
                    screen_height
                );
            });
        });
    }
}
