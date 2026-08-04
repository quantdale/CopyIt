use crate::editor::{self, Editor, EditorResult};
use crate::grid;
use crate::model::Snippet;
use crate::storage::{self, Config};
use crate::store::Store;
use crate::theme::Theme;
use eframe::egui;
use std::collections::HashSet;

/// Prefix of the banner message for a failed `snippets.json` write. Shared so a later
/// successful snippet save can retire exactly its own error and nothing else.
const SNIPPETS_SAVE_ERROR: &str = "Couldn't save snippets";
/// Prefix of the banner message for a failed `config.json` write.
const CONFIG_SAVE_ERROR: &str = "Couldn't save settings";
/// Red used for the warning banner and the delete-confirmation button.
const WARNING_COLOR: egui::Color32 = egui::Color32::from_rgb(0xef, 0x44, 0x44);
/// Characters of a snippet body shown in a card's preview line.
const PREVIEW_CHARS: usize = 220;

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
        Derived {
            title_lower: s.title.to_lowercase(),
            category_lower: s.category.to_lowercase(),
            body_lower: s.body.to_lowercase(),
            preview: preview_text(&s.body, PREVIEW_CHARS),
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
    snippets: Vec<Snippet>, // Full snippet library; order is preserved and user-draggable
    derived: Vec<Derived>, // Cached lowercase text + card preview, one entry per snippet (same order)
    generation: u64, // Bumped whenever `snippets`/`derived` change; invalidates the filter cache
    filter: FilterCache, // Memoized indices of the snippets visible under the current search/category
    next_id: u64,        // Next ID to assign to a new snippet; incremented on creation
    store: Store,        // Persistence seam: where data lives and how it's loaded/saved
    categories: Vec<String>, // Sorted, deduplicated list of all known categories
    search: String,      // Active search query; filters snippets by title/body/category
    category_filter: String, // "All" or a specific category; filters visible snippets
    theme: Theme,        // Currently selected theme; visuals are applied at startup and on change
    editor: Option<Editor>, // Modal editor state; None when no editor is open
    copied: Option<(u64, f64)>, // (id, time) for the transient "Copied" feedback (1.2s visibility)
    drag: Option<DragState>, // In-progress drag operation; None when idle
    adding_header_category: bool, // True when the user is typing a new category in the top bar
    new_header_category: String, // Input buffer for the new category name in the top bar
    category_error: Option<String>, // Validation error for the new category (e.g., "All" is reserved)
    save_error: Option<String>,     // File I/O error message to display at the top
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
///
/// The threshold, drop hit-testing, and gap math all live in `grid.rs`; this app-level
/// wrapper keeps the id and pointer position and defers geometry to the grid module.
type DragState = grid::DragMachine;

/// User action triggered from card interaction (Copy or Edit button click).
/// Used to defer action handling until after UI rendering to avoid borrowing conflicts.
enum Action {
    Copy(u64), // User clicked Copy button on a snippet; copy its body to clipboard
    Edit(u64), // User clicked Edit button on a snippet; open editor modal
}

/// Layout and response data returned from rendering a single snippet card.
/// Separates the card frame's bounding rect from the button responses, used for
/// drag-and-drop interaction detection and click handling.
struct CardWidgets {
    frame_rect: egui::Rect, // Bounding rectangle of the entire card frame (includes padding)
    copy: egui::Response,   // Response from the Copy button; checked for clicks
    edit: egui::Response,   // Response from the Edit button; checked for clicks
}

/// Builds the banner text for a data file that exists but couldn't be parsed, moving the
/// original aside first so it is never silently replaced by the seeded defaults.
/// Returns the banner text and whether the backup rename succeeded. When it fails
/// (e.g. the file is locked), the caller must NOT overwrite the corrupt file with
/// defaults — it is the only remaining copy of the user's data.
fn describe_corrupt_file(path: &std::path::Path, filename: &str, error: &str) -> (String, bool) {
    match storage::backup_corrupt(path) {
        Ok(backup) => (
            format!(
                "{filename} couldn't be read ({error}). It was kept as {} and the default library was loaded.",
                backup.display()
            ),
            true,
        ),
        Err(e) => (
            format!(
                "{filename} couldn't be read ({error}) and couldn't be backed up ({e}). \
                 The file was left in place — copy it elsewhere before making changes."
            ),
            false,
        ),
    }
}

impl CopyIt {
    /// Creates a new CopyIt instance on app launch.
    /// Loads snippets and config from disk (with one-time migration from legacy locations),
    /// seeds defaults if snippets.json doesn't exist, sanitizes categories, deduplicates
    /// snippet ids, and applies the saved theme. A corrupt data file is moved aside and
    /// reported in the banner; when the backup rename fails, the corrupt file is left in
    /// place and the seeded defaults are *not* written over it, so the user's data is not
    /// destroyed. Reports any file I/O errors in save_error for display in the UI.
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        let store = Store::open();
        store.migrate_legacy();

        let mut save_error: Option<String> = None;

        // A file that exists but doesn't parse is *not* a first launch: preserve it
        // before the seeded defaults claim its name, and tell the user where it went.
        // `seeded` is true only when writing the defaults over the file is safe — the
        // file was missing, or its corrupt copy was successfully moved aside. If the
        // backup rename fails, the corrupt file stays on disk untouched.
        let (mut snippets, seeded) = match store.load_snippets() {
            storage::Load::Loaded(snippets) => (snippets, false),
            storage::Load::Missing => (crate::seed::defaults(), true),
            storage::Load::Corrupt(e) => {
                let (note, backed_up) =
                    describe_corrupt_file(&store.snippets_path, "snippets.json", &e);
                save_error = Some(note);
                (crate::seed::defaults(), backed_up)
            }
        };
        for s in &mut snippets {
            s.category = storage::canonical_category(&s.category);
        }

        // Snippet ids come from hand-editable JSON. Deduplicate them: keep the first
        // occurrence of each id and reassign a fresh id to later duplicates, so a
        // duplicate id can't make Delete (`retain(|s| s.id != id)`) remove every copy
        // at once.
        dedup_snippet_ids(&mut snippets);

        // Load the user config, remembering whether it's safe to overwrite the on-disk
        // file later (it was missing, or its corrupt copy was moved aside) and what the
        // loaded categories were, verbatim, so a save happens only when the canonical
        // config actually differs from what's on disk.
        let mut loaded_categories: Option<Vec<String>> = None;
        let (mut config, config_write_ok) = match store.load_config() {
            storage::Load::Loaded(config) => {
                loaded_categories = Some(config.categories.clone());
                (config, false)
            }
            storage::Load::Missing => (Config::from_snippets(&snippets), true),
            storage::Load::Corrupt(e) => {
                let (note, backed_up) =
                    describe_corrupt_file(&store.config_path, "config.json", &e);
                // When both data files are corrupt, surface both notices: the config
                // was moved aside too, and the user needs to know where it went.
                save_error = Some(match save_error.take() {
                    Some(prev) => format!("{prev}\n{note}"),
                    None => note,
                });
                (Config::from_snippets(&snippets), backed_up)
            }
        };
        // Hand-edited or pre-fix config can carry non-canonical category names ("all",
        // near-duplicates differing only in whitespace or case): sanitize them so no
        // dropdown entry collides with the reserved "All" filter sentinel. The save
        // below persists the sanitized list when it differs from what was loaded.
        config.categories = sanitize_categories(&config.categories);
        config.add_categories(snippets.iter().map(|s| s.category.as_str()));

        // Persist the config only when it's safe to overwrite the on-disk file and the
        // canonical config differs from what was loaded. Writing a freshly derived
        // config over a corrupt original that couldn't be backed up would destroy the
        // only copy of the user's settings.
        let config_changed = matches!(&loaded_categories, Some(loaded) if *loaded != config.categories);
        if config_write_ok || config_changed {
            if let Err(e) = store.save_config(&config) {
                save_error = Some(format!("{CONFIG_SAVE_ERROR}: {e}"));
            }
        }

        let theme = config.theme.parse::<Theme>().unwrap_or(Theme::Dark);
        cc.egui_ctx.set_visuals(theme.visuals());

        // The next id is derived from the (deduplicated) library with a check for a
        // saturated id space, so a hand-edited `id == u64::MAX` can't wrap the
        // new-snippet counter to 0 and collide with an existing id.
        let next_id = next_snippet_id(&snippets);
        if seeded {
            if let Err(e) = store.save_snippets(&snippets) {
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
            store,
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
        match self.store.save_snippets(&self.snippets) {
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
        match self.store.save_config(&config) {
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

    /// Applies the `EditorOutcome::Save` outcome: normalizes the category (falling back
    /// to "Uncategorized" if blank), updates or inserts the snippet, and persists the
    /// library. Factored out of `update()` so tests can drive it without a UI context.
    fn apply_save(&mut self, mut ed: Editor) {
        let category = {
            let canonical = self.add_category(&ed.category);
            if canonical.is_empty() {
                self.add_category("Uncategorized")
            } else {
                canonical
            }
        };
        let title = ed.title.trim().to_string();
        if let Some(id) = ed.id {
            if let Some(s) = self.snippets.iter_mut().find(|s| s.id == id) {
                s.title = title;
                s.category = category;
                // The editor is closing, so its buffer can be moved instead of copied —
                // snippet bodies can be large.
                s.body = std::mem::take(&mut ed.body);
            }
        } else {
            // The cursor is always a free id (it mirrors the library's max, or a scanned
            // free id). Use it, then advance it to the next free id from the updated
            // library — recomputing avoids `cursor + 1` landing on an existing id when
            // the id space is non-contiguous (e.g. a hand-edited `id == u64::MAX`), and
            // can never wrap to 0 and collide.
            let id = self.next_id;
            self.snippets.push(Snippet {
                id,
                title,
                category,
                body: std::mem::take(&mut ed.body),
            });
            self.next_id = next_snippet_id(&self.snippets);
        }
        self.snippets_changed();
    }

    /// Applies the `EditorOutcome::AddCategory` outcome: registers the new category and,
    /// if its name is unusable (blank or the reserved "All"), keeps the inline form open
    /// with an explanatory error instead of silently discarding the input. Returns the
    /// editor state to store back. Factored out of `update()` so tests can drive it
    /// without a UI context.
    fn apply_add_category(&mut self, name: String, mut ed: Editor) -> Editor {
        let canonical = self.add_category(&name);
        if !canonical.is_empty() {
            ed.category = canonical;
            ed.new_category.clear();
            ed.adding_category = false;
            ed.category_error = None;
        } else {
            ed.adding_category = true;
            ed.category_error = Some(if name.trim().is_empty() {
                "Category can't be blank".to_string()
            } else {
                "\"All\" is reserved and can't be used as a category".to_string()
            });
        }
        ed
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
        let card_h = grid::CARD_INNER_H;

        // The preview is cached per snippet; fall back to computing it only if the
        // caches somehow got out of step, so a stale index can never panic or blank
        // a card in a release build.
        debug_assert_eq!(self.derived.len(), self.snippets.len());
        let fallback_preview;
        let preview: &str = match self.derived.get(idx) {
            Some(d) => &d.preview,
            None => {
                fallback_preview = preview_text(&s.body, PREVIEW_CHARS);
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
                                let resp = ui.button(if recently {
                                    "\u{2714} Copied"
                                } else {
                                    "\u{29C9} Copy"
                                });
                                if resp.clicked() {
                                    actions.push(Action::Copy(s.id));
                                }
                                ui.with_layout(
                                    egui::Layout::left_to_right(egui::Align::Center),
                                    |ui| {
                                        ui.add(
                                            egui::Label::new(
                                                egui::RichText::new(&s.title).strong().size(15.0),
                                            )
                                            .truncate(true),
                                        );
                                    },
                                );
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
                            ui.label(egui::RichText::new(&s.category).small().color(badge_text));
                        });

                    ui.add_space(6.0);

                    // Preview + Edit pinned to the bottom
                    let edit_resp = ui
                        .with_layout(egui::Layout::bottom_up(egui::Align::LEFT), |ui| {
                            let resp = ui.small_button("Edit");
                            if resp.clicked() {
                                actions.push(Action::Edit(s.id));
                            }
                            ui.add_space(4.0);
                            ui.with_layout(egui::Layout::top_down(egui::Align::LEFT), |ui| {
                                ui.add(
                                    egui::Label::new(egui::RichText::new(preview).weak())
                                        .truncate(true),
                                );
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
        CardWidgets {
            frame_rect,
            copy,
            edit,
        }
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
            .inner_margin(egui::Margin::symmetric(grid::GRID_MARGIN_X, 0.0))
            .show(ui, |ui| {
                let avail = ui.available_width();
                let cols = grid::cols_for(avail);

                let origin = ui.cursor().min;
                let rows = filtered.len().div_ceil(cols);
                let card_rects: Vec<egui::Rect> = (0..filtered.len())
                    .map(|i| {
                        grid::grid_card_rect(
                            i,
                            cols,
                            origin,
                            grid::CARD_W,
                            grid::CARD_H,
                            grid::CARD_SPACING,
                        )
                    })
                    .collect();

                let (first_row, last_row) =
                    grid::visible_rows(ui.clip_rect(), origin.y, grid::ROW_PITCH, rows);
                // Reserve the height of the rows above the viewport.
                if first_row > 0 {
                    ui.add_space(first_row as f32 * grid::ROW_PITCH);
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
                                    d.is_dragging() && d.snippet_id() == self.snippets[idx].id
                                });

                                let drag_id = ui.id().with("card_drag").with(idx);
                                let expected_rect = egui::Rect::from_min_size(
                                    ui.cursor().min,
                                    egui::vec2(grid::CARD_W, grid::CARD_H),
                                );
                                let drag_resp =
                                    ui.interact(expected_rect, drag_id, egui::Sense::drag());

                                let widgets = self.card(
                                    ui,
                                    idx,
                                    grid::CARD_INNER_W,
                                    now,
                                    actions,
                                    is_dragged,
                                );

                                debug_assert!(
                                    card_rects
                                        .get(row_start + row_offset)
                                        .is_some_and(
                                            |r| r.min.distance(widgets.frame_rect.min) < 0.5
                                        ),
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

                                ui.add_space(grid::CARD_SPACING);
                            }
                        });
                    });
                    ui.add_space(grid::CARD_SPACING);
                }

                // Reserve the height of the rows below the viewport, so the scrollbar
                // still spans the whole library.
                if last_row + 1 < rows {
                    ui.add_space((rows - 1 - last_row) as f32 * grid::ROW_PITCH);
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
                            self.category_error = Some(
                                "\"All\" is reserved and can't be used as a category".to_string(),
                            );
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

            let scroll_output =
                egui::ScrollArea::vertical()
                    .auto_shrink([false; 2])
                    .show(ui, |ui| {
                        ui.spacing_mut().item_spacing = egui::vec2(0.0, 0.0);
                        ui.add_space(grid::GRID_TOP_SPACE);
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
                            let text = s.body.clone();
                            ui.output_mut(|o| o.copied_text = text);
                            self.copied = Some((id, now));
                            ctx.request_repaint_after(std::time::Duration::from_millis(1300));
                        }
                    }
                    Action::Edit(id) => {
                        if let Some(s) = self.snippets.iter().find(|s| s.id == id) {
                            self.editor = Some(Editor::from_snippet(s, &self.categories));
                        }
                    }
                }
            }

            // Start a new drag if requested.
            if let Some((id, pos)) = drag_start {
                self.drag = Some(DragState::begin(id, pos));
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

            // Update drag threshold: the machine promotes itself to a real
            // drag once the pointer has moved far enough past the start.
            if let Some(drag) = &mut self.drag {
                if let Some(pos) = ctx.input(|i| i.pointer.interact_pos()) {
                    drag.pointer(pos);
                }
            }

            // Draw drag visuals and handle drop.
            if self.drag.as_ref().is_some_and(|d| d.is_dragging()) {
                let pointer_pos = ctx.input(|i| i.pointer.interact_pos());
                let pointer_released = ctx.input(|i| i.pointer.primary_released());
                let pointer_moved = ctx.input(|i| i.pointer.delta().length_sq() > 0.0);

                hover_cursor = Some(egui::CursorIcon::Grabbing);

                if let Some(pointer) = pointer_pos {
                    let gap = grid::nearest_gap(
                        pointer,
                        card_screen_rects,
                        cols,
                        grid::CARD_SPACING,
                        grid::CARD_W,
                    );
                    grid::draw_insertion_line(
                        ctx,
                        gap,
                        card_screen_rects,
                        cols,
                        grid::CARD_SPACING,
                        grid::CARD_W,
                        grid::CARD_H,
                    );

                    // Hollow ghost box following the cursor.
                    let ghost_rect = egui::Rect::from_min_size(
                        pointer + egui::vec2(8.0, 8.0),
                        egui::vec2(grid::CARD_W, grid::CARD_H),
                    );
                    let painter = ctx.layer_painter(egui::LayerId::new(
                        egui::Order::Tooltip,
                        egui::Id::new("drag_ghost"),
                    ));
                    painter.rect_stroke(
                        ghost_rect,
                        egui::Rounding::same(8.0),
                        egui::Stroke::new(2.0_f32, egui::Color32::from_rgb(0x60, 0xb0, 0xff)),
                    );
                }

                if pointer_released {
                    // Consume the machine regardless of where the release
                    // happened; only a drop inside the grid reorders.
                    let drag_ctx = grid::DragContext {
                        grid_area,
                        card_rects: card_screen_rects,
                        cols,
                    };
                    let dropped = self
                        .drag
                        .take()
                        .and_then(|drag| pointer_pos.map(|p| drag.release(p, &drag_ctx)));
                    if let Some(Some((id, gap))) = dropped {
                        self.reorder(id, gap, &filtered);
                    }
                } else if pointer_moved {
                    ctx.request_repaint();
                }
            } else if ctx.input(|i| i.pointer.primary_released()) {
                // Released before crossing the drag threshold: cancel.
                self.drag = None;
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
                            ui.add(egui::TextEdit::singleline(&mut ed.title).desired_width(280.0));
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
                                let previous_category = ed.new_category.clone();
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
                                // Clear the inline-category warning as the user edits the
                                // field, so the error doesn't linger while they fix it.
                                if ed.new_category != previous_category {
                                    ed.category_error = None;
                                }
                                if let Some(err) = &ed.category_error {
                                    ui.colored_label(WARNING_COLOR, err);
                                }
                            }
                        });
                    });
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

            // Pure transition: the button click plus the window's open/close flag
            // decide what happens next; the app only applies the outcome.
            match editor::decide(result, ed, window_open) {
                editor::EditorOutcome::Save(ed) => self.apply_save(ed),
                editor::EditorOutcome::Delete(id) => {
                    self.snippets.retain(|s| s.id != id);
                    self.snippets_changed();
                }
                editor::EditorOutcome::Close => {}
                editor::EditorOutcome::AddCategory(name, ed) => {
                    self.editor = Some(self.apply_add_category(name, ed));
                }
                editor::EditorOutcome::Keep(ed) => {
                    self.editor = Some(ed);
                }
            }
        }
    }
}

/// Normalizes a freshly-loaded category list into the canonical form the app filters on:
/// each entry is normalized, reserved names (blank, "All") are dropped, the list is
/// deduplicated case-insensitively and sorted. A hand-edited or pre-fix `config.json`
/// containing `"all"` could otherwise produce a dropdown entry indistinguishable from the
/// reserved "All" filter sentinel.
fn sanitize_categories(categories: &[String]) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for c in categories {
        let cat = storage::normalize_category(c);
        if storage::is_reserved_category(&cat) {
            continue;
        }
        if out.iter().any(|existing| storage::same_category(existing, &cat)) {
            continue;
        }
        out.push(cat);
    }
    out.sort();
    out
}

/// Finds the smallest positive id not present in `snippets`. Only used once the
/// sequential id space is exhausted (every id up to `u64::MAX` is taken), which in
/// practice means the library holds a hand-edited `id == u64::MAX`.
fn next_free_id_from_1(snippets: &[Snippet]) -> u64 {
    (1u64..)
        .find(|id| !snippets.iter().any(|s| s.id == *id))
        .unwrap_or(0)
}

/// Reassigns ids to duplicates in a library loaded from hand-editable JSON: the first
/// occurrence of each id is kept, later duplicates get a fresh id. Prevents a duplicate
/// id from making Delete (`retain(|s| s.id != id)`) remove every copy at once.
fn dedup_snippet_ids(snippets: &mut [Snippet]) {
    let mut seen: HashSet<u64> = HashSet::new();
    for i in 0..snippets.len() {
        if seen.insert(snippets[i].id) {
            continue;
        }
        snippets[i].id = next_snippet_id(snippets);
        seen.insert(snippets[i].id);
    }
}

/// Picks the id for a brand-new snippet: the largest existing id plus one. If the largest
/// id is `u64::MAX`, one more would either wrap to 0 (in release) or saturate back to
/// `u64::MAX` — both collide — so fall back to scanning from 1 for the first free id.
fn next_snippet_id(snippets: &[Snippet]) -> u64 {
    let max_id = snippets.iter().map(|s| s.id).max().unwrap_or(0);
    if max_id == u64::MAX {
        next_free_id_from_1(snippets)
    } else {
        max_id + 1
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

/// Collapses a snippet body into a single-line preview: splits on whitespace, joins with
/// single spaces, and truncates to `max` characters. Used to display a short preview in
/// each card.
///
/// Collapsing stops as soon as the next word would overflow the preview, so a long,
/// white-space-separated body costs about the same to render as a one-line one instead of
/// being copied whole. The one case that can't be shortcut is a single token longer than
/// `max` (a base64 blob with no spaces, say): the first word is always gathered, but only
/// a prefix of it is copied — just enough to overflow `max` so `truncate_chars` can clip
/// it with an ellipsis — never the token in full.
fn preview_text(body: &str, max: usize) -> String {
    let mut collapsed = String::new();
    let mut chars = 0usize;
    for word in body.split_whitespace() {
        if !collapsed.is_empty() {
            collapsed.push(' ');
            chars += 1;
        }
        let word_chars = word.chars().count();
        if chars + word_chars > max {
            // This word would overflow the preview: copy only a prefix large enough to
            // push the total past `max`, so `truncate_chars` below clips it with an
            // ellipsis. A giant token is never copied in full.
            let room = max.saturating_add(1).saturating_sub(chars);
            collapsed.push_str(&word.chars().take(room).collect::<String>());
            break;
        }
        collapsed.push_str(word);
        chars += word_chars;
    }
    truncate_chars(&collapsed, max)
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

/// Unit tests for grid layout and drag-and-drop logic.
/// Validates that card positioning, gap detection, and insertion line rendering work correctly
/// across different grid configurations (single and multi-card layouts).
#[cfg(test)]
mod layout_tests {
    use super::*;
    use crate::grid::{
        gap_point, grid_card_rect, nearest_gap, visible_rows, CARD_H, CARD_SPACING, CARD_W,
        GRID_TOP_SPACE, ROW_PITCH,
    };

    /// Builds an app whose data files live in a throwaway temp directory, so tests that
    /// exercise the auto-save paths never write into the repository or clobber real user data.
    fn test_app(name: &str, snippets: Vec<Snippet>, categories: Vec<String>) -> CopyIt {
        let dir = std::env::temp_dir().join(format!("copyit-app-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("create temp data dir");
        let next_id = next_snippet_id(&snippets);
        let mut app = CopyIt {
            snippets,
            derived: Vec::new(),
            generation: 0,
            filter: FilterCache::default(),
            next_id,
            store: Store::at(dir.clone()),
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
                assert!((gap - 12.0).abs() < 0.1, "horizontal gap {} = {}", g, gap);
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
                                                ui,
                                                idx,
                                                card_inner_w,
                                                0.0,
                                                &mut actions,
                                                false,
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
                assert!(
                    (p.x - expected_x).abs() < 0.1,
                    "vertical gap {} x = {}, expected {}",
                    g,
                    p.x,
                    expected_x
                );
                assert!((p.y - a.center().y).abs() < 0.1);
                // 2px vertical line centered in x must not intersect either card.
                let line_rect = egui::Rect::from_min_max(
                    egui::pos2(p.x - stroke_width * 0.5, p.y - 60.0),
                    egui::pos2(p.x + stroke_width * 0.5, p.y + 60.0),
                );
                assert!(
                    !line_rect.intersects(*a),
                    "vertical line intersects left card"
                );
                assert!(
                    !line_rect.intersects(*b),
                    "vertical line intersects right card"
                );
                assert!(p.x > a.right() + clearance - stroke_width * 0.5);
                assert!(p.x < b.left() - clearance + stroke_width * 0.5);
            } else {
                let a = &rects[g - 1];
                let b = &rects[g];
                let gap = b.top() - a.bottom();
                let expected_y = a.bottom() + gap * 0.5;
                assert!(
                    (p.y - expected_y).abs() < 0.1,
                    "horizontal gap {} y = {}, expected {}",
                    g,
                    p.y,
                    expected_y
                );
                // 2px horizontal line centered in y must not intersect either card.
                let line_rect = egui::Rect::from_min_max(
                    egui::pos2(p.x - 112.0, p.y - stroke_width * 0.5),
                    egui::pos2(p.x + 112.0, p.y + stroke_width * 0.5),
                );
                assert!(
                    !line_rect.intersects(*a),
                    "horizontal line intersects above card"
                );
                assert!(
                    !line_rect.intersects(*b),
                    "horizontal line intersects below card"
                );
                assert!(p.y > a.bottom() + clearance - stroke_width * 0.5);
                assert!(p.y < b.top() - clearance + stroke_width * 0.5);
            }
        }
    }

    /// Runs a single frame of the real card grid in a 1000x700 window at the given
    /// scroll offset. Returns the number of paint shapes the frame emitted (a proxy for
    /// how many cards were actually built), the rects the grid reported for every card,
    /// the scroll area's content size, and the column count.
    fn run_grid(app: &CopyIt, scroll_offset: f32) -> (usize, Vec<egui::Rect>, egui::Vec2, usize) {
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
                        ui.add_space(grid::GRID_TOP_SPACE);
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
        assert!(
            small_shapes > 20,
            "the visible cards must actually be painted"
        );

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
            assert!(
                r.min.distance(expected.min) < 0.1,
                "card {i} at {:?}",
                r.min
            );
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
        let pointer = egui::pos2(above.center().x, (above.bottom() + below.top()) * 0.5);
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

                                for (i, chunk_start) in
                                    (0..filtered.len()).step_by(cols).enumerate()
                                {
                                    let row = &filtered
                                        [chunk_start..(chunk_start + cols).min(filtered.len())];
                                    ui.horizontal(|ui| {
                                        ui.spacing_mut().item_spacing = egui::vec2(0.0, 0.0);
                                        for (offset, &idx) in row.iter().enumerate() {
                                            let mut actions = Vec::new();
                                            let rendered = app
                                                .card(
                                                    ui,
                                                    idx,
                                                    card_inner_w,
                                                    0.0,
                                                    &mut actions,
                                                    false,
                                                )
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
            assert_eq!(
                got, want,
                "search {:?} / cat {:?}",
                app.search, app.category_filter
            );
            app.restore_filtered(got);
        };

        check(&mut app); // everything

        // Taking the buffer twice without restoring it must not report "no matches".
        let first = app.take_filtered();
        let second = app.take_filtered();
        assert_eq!(
            first, second,
            "a checked-out cache must be recomputed, not reused"
        );
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
        assert_eq!(
            app.take_filtered(),
            vec![2],
            "the moved card is still findable"
        );
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

    /// A single gigantic token (no spaces) must be clipped to the preview width, not
    /// copied in full — the early-exit savings only apply to white-space-separated bodies.
    #[test]
    fn preview_text_clips_a_single_gigantic_token() {
        let blob = "A".repeat(1_000_000);
        let preview = preview_text(&blob, PREVIEW_CHARS);
        assert_eq!(preview.chars().count(), PREVIEW_CHARS);
        assert!(preview.ends_with('\u{2026}'));
        assert!(preview.starts_with('A'));

        // A huge token after a normal word stops the collapse at the boundary too.
        let preview = preview_text(&format!("git {blob}"), PREVIEW_CHARS);
        assert_eq!(preview.chars().count(), PREVIEW_CHARS);
        assert!(preview.ends_with('\u{2026}'));
    }

    #[test]
    fn sanitize_categories_drops_reserved_and_normalizes() {
        // "all" is the exact collision CORR-001 was meant to eliminate: without
        // sanitization it becomes a dropdown entry indistinguishable from the "All" filter.
        let raw = vec![
            "all".to_string(),
            "  GIT ".to_string(),
            "git".to_string(),
            "".to_string(),
            "Prompt".to_string(),
        ];
        let clean = sanitize_categories(&raw);
        assert_eq!(clean, vec!["Git".to_string(), "Prompt".to_string()]);
        assert!(!clean.iter().any(|c| c.eq_ignore_ascii_case("all")));
        assert!(!clean.iter().any(|c| c.is_empty()));
    }

    #[test]
    fn next_snippet_id_never_wraps_to_zero() {
        assert_eq!(next_snippet_id(&[]), 1);
        assert_eq!(next_snippet_id(&[snippet(1, "Git")]), 2);
        assert_eq!(next_snippet_id(&[snippet(5, "Git")]), 6);

        // A hand-edited `id == u64::MAX` must not wrap to 0 (which would collide): scan
        // from 1 upward for the first id not present in the library.
        let saturated = vec![snippet(u64::MAX, "Git"), snippet(2, "Git")];
        assert_eq!(next_snippet_id(&saturated), 1);
        assert_eq!(next_free_id_from_1(&saturated), 1);
    }

    /// Deleting a snippet by id removes every snippet bearing that id, so duplicate ids
    /// loaded from hand-edited JSON must be reassigned at load — otherwise one Delete
    /// clears all copies.
    #[test]
    fn dedup_snippet_ids_keeps_the_first_and_reassigns_duplicates() {
        let mut lib = vec![
            snippet(1, "Git"),
            snippet(1, "Git"),
            snippet(2, "Prompt"),
            snippet(2, "Prompt"),
        ];
        dedup_snippet_ids(&mut lib);
        assert_eq!(lib[0].id, 1, "first occurrence of 1 is kept");
        assert_eq!(lib[2].id, 2, "first occurrence of 2 is kept");
        let ids: Vec<u64> = lib.iter().map(|s| s.id).collect();
        let mut uniq = ids.clone();
        uniq.sort_unstable();
        uniq.dedup();
        assert_eq!(uniq.len(), lib.len(), "every id must stay unique");
        assert_eq!(uniq, vec![1, 2, 3, 4], "duplicates get fresh ids from the sequence");
    }

    #[test]
    fn dedup_snippet_ids_handles_a_saturated_id_space_without_wrapping() {
        let mut lib = vec![snippet(u64::MAX, "Git"), snippet(u64::MAX, "Git")];
        dedup_snippet_ids(&mut lib);
        assert_eq!(lib[0].id, u64::MAX);
        assert_eq!(lib[1].id, 1, "must not wrap to 0 on a saturated id space");
    }

    /// The editor Save outcome, applied to a real app, adds the snippet, registers the
    /// category, and persists the library to disk.
    #[test]
    fn apply_save_adds_a_new_snippet_and_persists() {
        let mut app = test_app("apply-save-add", vec![], vec!["Git".into()]);
        let mut ed = Editor::blank(&app.categories);
        ed.title = "New prompt".into();
        ed.category = "Git".into();
        ed.body = "some body".into();
        app.apply_save(ed);

        assert_eq!(ids(&app), vec![1]);
        assert_eq!(app.snippets[0].title, "New prompt");
        assert_eq!(app.snippets[0].body, "some body");
        assert!(app.save_error.is_none());
        match storage::load(&app.store.snippets_path) {
            storage::Load::Loaded(snips) => {
                assert_eq!(snips.len(), 1);
                assert_eq!(snips[0].title, "New prompt");
            }
            _ => panic!("saved library should load back"),
        }
    }

    #[test]
    fn apply_save_updates_an_existing_snippet() {
        let mut app = test_app(
            "apply-save-update",
            vec![snippet(1, "Git")],
            vec!["Git".into()],
        );
        let mut ed = Editor::from_snippet(&app.snippets[0], &app.categories);
        ed.title = "Renamed".into();
        ed.body = "new body".into();
        ed.category = "Prompt".into();
        app.apply_save(ed);

        assert_eq!(ids(&app), vec![1], "editing must not change the id");
        assert_eq!(app.snippets[0].title, "Renamed");
        assert_eq!(app.snippets[0].body, "new body");
        assert_eq!(app.snippets[0].category, "Prompt");
        assert!(app.categories.contains(&"Prompt".to_string()));
    }

    #[test]
    fn apply_add_category_registers_a_valid_category() {
        let mut app = test_app("apply-add-cat-valid", vec![], vec!["Git".into()]);
        let ed = Editor::blank(&app.categories);
        let ed = app.apply_add_category("Docker".into(), ed);
        assert!(!ed.adding_category);
        assert_eq!(ed.category, "Docker");
        assert!(ed.category_error.is_none());
        assert!(app.categories.contains(&"Docker".to_string()));
    }

    /// New snippets minted after a hand-edited `id == u64::MAX` saturates the sequential
    /// id space must never wrap to 0 or collide with an existing id.
    #[test]
    fn apply_save_mints_unique_ids_across_a_saturated_library() {
        let mut app = test_app(
            "apply-save-saturated",
            vec![snippet(u64::MAX, "Git"), snippet(2, "Git")],
            vec!["Git".into()],
        );
        // next_id is derived at load: the first free id is 1 (the library has MAX and 2).
        assert_eq!(app.next_id, 1);

        let mut ed = Editor::blank(&app.categories);
        ed.title = "First".into();
        ed.body = "b".into();
        app.apply_save(ed);
        assert_eq!(app.snippets[2].id, 1);

        let mut ed = Editor::blank(&app.categories);
        ed.title = "Second".into();
        ed.body = "b".into();
        app.apply_save(ed);
        // The advanced cursor is the next free id (3), not 2 — which is already in the
        // library, so a naive `cursor + 1` would collide.
        assert_eq!(app.snippets[3].id, 3);

        let mut ids: Vec<u64> = app.snippets.iter().map(|s| s.id).collect();
        ids.sort_unstable();
        ids.dedup();
        assert_eq!(ids.len(), app.snippets.len(), "no id collisions");
    }

    /// The inline "Add new category" form must not silently swallow a reserved name: it
    /// keeps the form open and explains why, mirroring the top bar's error.
    #[test]
    fn apply_add_category_rejects_reserved_names_with_an_error() {
        let mut app = test_app("apply-add-cat-all", vec![], vec!["Git".into()]);
        let ed = Editor::blank(&app.categories);
        let ed = app.apply_add_category("all".into(), ed);
        assert!(ed.adding_category, "the form must stay open");
        assert_eq!(
            ed.category_error.as_deref(),
            Some("\"All\" is reserved and can't be used as a category")
        );
        assert!(!app.categories.iter().any(|c| c.eq_ignore_ascii_case("all")));

        // A blank name is rejected too, with its own message.
        let ed = app.apply_add_category("   ".into(), ed);
        assert!(ed.adding_category);
        assert_eq!(ed.category_error.as_deref(), Some("Category can't be blank"));
    }

    #[test]
    fn add_category_normalizes_and_dedups() {
        let mut app = test_app("add-category", vec![], vec!["Git".into(), "Prompt".into()]);
        assert_eq!(app.add_category("  git "), "Git"); // existing, case-insensitive
        assert_eq!(app.add_category("werner"), "Werner"); // new
        assert_eq!(app.add_category("Werner"), "Werner"); // duplicate
        assert!(app.categories.contains(&"Werner".to_string()));
        assert_eq!(app.categories.len(), 3);

        assert_eq!(app.add_category("all"), "");
        assert_eq!(app.add_category("All"), "");
        assert_eq!(app.add_category("ALL"), "");
        assert!(!app.categories.iter().any(|c| c.eq_ignore_ascii_case("all")));
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
        let mut app = test_app(
            "save-error-scope",
            vec![snippet(1, "Git")],
            vec!["Git".into()],
        );

        app.save_error = Some(format!("{SNIPPETS_SAVE_ERROR}: disk full"));
        app.save_config();
        assert_eq!(
            app.save_error.as_deref(),
            Some("Couldn't save snippets: disk full"),
            "a config write must not hide a snippet-save failure"
        );

        app.save_snippets();
        assert!(
            app.save_error.is_none(),
            "the snippet save clears its own error"
        );

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

        let dir = app.store.snippets_path.parent().unwrap().to_path_buf();
        let leftovers: Vec<String> = std::fs::read_dir(&dir)
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .filter(|name| name.ends_with(".tmp"))
            .collect();
        assert!(
            leftovers.is_empty(),
            "temp files left behind: {leftovers:?}"
        );

        match storage::load(&app.store.snippets_path) {
            storage::Load::Loaded(snippets) => {
                assert_eq!(
                    snippets.iter().map(|s| s.id).collect::<Vec<_>>(),
                    vec![1, 2]
                );
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
                format!("Line {i}: Lorem ipsum dolor sit amet, consectetur adipiscing elit.\n")
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
                                ui.add(egui::TextEdit::singleline(&mut title).desired_width(280.0));
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
