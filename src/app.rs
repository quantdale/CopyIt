use crate::model::Snippet;
use crate::storage::{self, Config};
use crate::theme::Theme;
use eframe::egui;
use std::path::PathBuf;

pub struct CopyIt {
    snippets: Vec<Snippet>,
    next_id: u64,
    path: PathBuf,
    config_path: PathBuf,
    categories: Vec<String>,
    search: String,
    category_filter: String, // "All" or a specific category
    theme: Theme,
    editor: Option<Editor>,
    copied: Option<(u64, f64)>, // (id, time) for the transient "Copied" state
    drag: Option<DragState>,
    adding_header_category: bool,
    new_header_category: String,
    category_error: Option<String>,
    save_error: Option<String>,
}

struct DragState {
    snippet_id: u64,     // which snippet is being dragged
    origin_index: usize, // its index in self.snippets at drag start
    start_pos: egui::Pos2,
    dragging: bool,
}

struct Editor {
    id: Option<u64>, // None = creating a new snippet
    title: String,
    category: String,
    new_category: String,
    adding_category: bool,
    body: String,
    confirm_delete: bool,
}

impl Editor {
    fn blank(categories: &[String]) -> Self {
        Editor {
            id: None,
            title: String::new(),
            category: categories.first().cloned().unwrap_or_default(),
            new_category: String::new(),
            adding_category: false,
            body: String::new(),
            confirm_delete: false,
        }
    }

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
        }
    }
}

enum Action {
    Copy(u64),
    Edit(u64),
}

enum EditorResult {
    None,
    Save,
    Cancel,
    Delete,
    AddCategory(String),
}

struct CardWidgets {
    frame_rect: egui::Rect,
    copy: egui::Response,
    edit: egui::Response,
}

/// One-time recovery for users upgrading from earlier versions that stored
/// `snippets.json`/`config.json` next to the .exe: if the new stable location
/// doesn't have a file yet, pull in the first non-empty copy found in a
/// legacy location (next to the exe, `target/debug`, `target/release`, cwd).
fn migrate_legacy_file(new_path: &std::path::Path, filename: &str) {
    if new_path.exists() {
        return;
    }
    for dir in storage::legacy_candidate_dirs() {
        let candidate = dir.join(filename);
        if candidate == new_path {
            continue;
        }
        if let Ok(data) = std::fs::read_to_string(&candidate) {
            let trimmed = data.trim();
            if trimmed.is_empty() || trimmed == "[]" || trimmed == "{}" {
                continue;
            }
            let _ = std::fs::write(new_path, data);
            return;
        }
    }
}

impl CopyIt {
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        let path = storage::data_path();
        let config_path = storage::config_path();
        migrate_legacy_file(&path, "snippets.json");
        migrate_legacy_file(&config_path, "config.json");

        let mut snippets = storage::load(&path).unwrap_or_else(crate::seed::defaults);
        for s in &mut snippets {
            s.category = storage::normalize_category(&s.category);
        }

        let mut config = storage::load_config(&config_path)
            .unwrap_or_else(|| Config::from_snippets(&snippets));
        for s in &snippets {
            config.add_category(&s.category);
        }
        let mut save_error: Option<String> = None;
        if let Err(e) = storage::save_config(&config_path, &config) {
            save_error = Some(format!("Couldn't save settings: {e}"));
        }

        let theme = config.theme.parse::<Theme>().unwrap_or(Theme::Dark);
        cc.egui_ctx.set_visuals(theme.visuals());

        let next_id = snippets.iter().map(|s| s.id).max().unwrap_or(0) + 1;
        if !path.exists() {
            if let Err(e) = storage::save(&path, &snippets) {
                save_error = Some(format!("Couldn't save snippets: {e}"));
            }
        }

        Self {
            snippets,
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
        }
    }

    fn save_snippets(&mut self) {
        self.save_error = storage::save(&self.path, &self.snippets)
            .err()
            .map(|e| format!("Couldn't save snippets: {e}"));
    }

    fn save_config(&mut self) {
        let config = Config {
            categories: self.categories.clone(),
            theme: self.theme.to_string(),
        };
        self.save_error = storage::save_config(&self.config_path, &config)
            .err()
            .map(|e| format!("Couldn't save settings: {e}"));
    }

    /// Normalizes `raw`, adds it to the canonical list if it isn't already
    /// present (case-insensitive), persists the config, and returns the
    /// canonical form (or an existing match if one collides).
    fn add_category(&mut self, raw: &str) -> String {
        let cat = storage::normalize_category(raw);
        if cat.is_empty() || cat.eq_ignore_ascii_case("all") {
            return String::new();
        }
        if let Some(existing) = self
            .categories
            .iter()
            .find(|c| c.eq_ignore_ascii_case(&cat))
        {
            return existing.clone();
        }
        self.categories.push(cat.clone());
        self.categories.sort();
        self.save_config();
        cat
    }

    fn reorder(&mut self, origin_index: usize, target_filtered_gap: usize, filtered: &[usize]) {
        if filtered.is_empty() {
            return;
        }
        let snippet = self.snippets.remove(origin_index);
        let target_abs = if target_filtered_gap == 0 {
            let mut t = filtered[0];
            if t > origin_index {
                t -= 1;
            }
            t
        } else if target_filtered_gap < filtered.len() {
            let mut t = filtered[target_filtered_gap];
            if t > origin_index {
                t -= 1;
            }
            t
        } else {
            let mut t = filtered[filtered.len() - 1];
            if t > origin_index {
                t -= 1;
            }
            t + 1
        };
        let target_abs = target_abs.min(self.snippets.len());
        self.snippets.insert(target_abs, snippet);
        self.save_snippets();
    }

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
        let card_h = 168.0;

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
                                ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
                                    ui.add(
                                        egui::Label::new(
                                            egui::RichText::new(&s.title).strong().size(15.0),
                                        )
                                        .truncate(true),
                                    );
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
                            let resp = ui.small_button("Edit");
                            if resp.clicked() {
                                actions.push(Action::Edit(s.id));
                            }
                            ui.add_space(4.0);
                            ui.with_layout(egui::Layout::top_down(egui::Align::LEFT), |ui| {
                                ui.label(egui::RichText::new(preview_text(&s.body, 220)).weak());
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
}

impl eframe::App for CopyIt {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        let now = ctx.input(|i| i.time);
        let previous_theme = self.theme;
        ctx.set_visuals(self.theme.visuals());

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
                    .selected_text(self.category_filter.clone())
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
                    ui.add(
                        egui::TextEdit::singleline(&mut self.new_header_category)
                            .hint_text("New category")
                            .desired_width(120.0),
                    );
                    let enter_pressed = ui.input(|i| i.key_pressed(egui::Key::Enter));
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
                        ui.colored_label(egui::Color32::from_rgb(0xef, 0x44, 0x44), err);
                    }
                }

                ui.add_space(12.0);
                egui::ComboBox::from_id_source("theme_select")
                    .selected_text(self.theme.to_string())
                    .show_ui(ui, |ui| {
                        for t in Theme::all() {
                            ui.selectable_value(&mut self.theme, *t, t.to_string());
                        }
                    });
                if self.theme != previous_theme {
                    self.save_config();
                }

                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.button("\u{FF0B} New").clicked() {
                        self.editor = Some(Editor::blank(&self.categories));
                    }
                });
            });
            if let Some(err) = &self.save_error {
                ui.add_space(4.0);
                ui.horizontal(|ui| {
                    ui.add_space(16.0);
                    ui.colored_label(egui::Color32::from_rgb(0xef, 0x44, 0x44), format!("\u{26A0} {err}"));
                });
            }
            ui.add_space(6.0);
        });

        // ---- Main grid ----
        egui::CentralPanel::default().show(ctx, |ui| {
            let q = self.search.to_lowercase();
            let filtered: Vec<usize> = self
                .snippets
                .iter()
                .enumerate()
                .filter(|(_, s)| {
                    (self.category_filter == "All" || s.category == self.category_filter)
                        && (q.is_empty()
                            || s.title.to_lowercase().contains(&q)
                            || s.body.to_lowercase().contains(&q)
                            || s.category.to_lowercase().contains(&q))
                })
                .map(|(i, _)| i)
                .collect();

            if filtered.is_empty() {
                ui.add_space(40.0);
                ui.vertical_centered(|ui| {
                    ui.label(egui::RichText::new("No snippets match.").weak());
                });
                return;
            }

            let mut actions: Vec<Action> = Vec::new();
            let mut drag_start: Option<(u64, usize, egui::Pos2)> = None;
            let mut hover_cursor: Option<egui::CursorIcon> = None;

            let card_inner_w = 300.0_f32;
            let card_inner_h = 168.0_f32;
            // The group frame around each card has 10 px inner margin on each side.
            let card_frame_margin = 20.0_f32;
            let card_w = card_inner_w + card_frame_margin;
            let card_h = card_inner_h + card_frame_margin;
            let spacing = 12.0_f32;
            let top_space = 4.0_f32;
            let margin_x = 18.0_f32;

            let scroll_output = egui::ScrollArea::vertical()
                .auto_shrink([false; 2])
                .show(ui, |ui| {
                    ui.spacing_mut().item_spacing = egui::vec2(0.0, 0.0);
                    ui.add_space(top_space);

                    egui::Frame::none()
                        .inner_margin(egui::Margin::symmetric(margin_x, 0.0))
                        .show(ui, |ui| {
                            let avail = ui.available_width();
                            let cols =
                                ((avail / (card_w + spacing)).floor() as usize).max(1);

                            let mut card_content_rects: Vec<egui::Rect> =
                                Vec::with_capacity(filtered.len());

                            for row in filtered.chunks(cols) {
                                ui.horizontal(|ui| {
                                    ui.spacing_mut().item_spacing = egui::vec2(0.0, 0.0);
                                    for &idx in row.iter() {
                                        let is_dragged = self.drag.as_ref().is_some_and(|d| {
                                            d.dragging && d.snippet_id == self.snippets[idx].id
                                        });

                                        let drag_id = ui.id().with("card_drag").with(idx);
                                        let expected_rect = egui::Rect::from_min_size(
                                            ui.cursor().min,
                                            egui::vec2(card_w, card_h),
                                        );
                                        let drag_resp = ui.interact(
                                            expected_rect,
                                            drag_id,
                                            egui::Sense::drag(),
                                        );

                                        let widgets = self.card(
                                            ui, idx, card_inner_w, now, &mut actions, is_dragged,
                                        );

                                        card_content_rects.push(widgets.frame_rect);

                                        let pointer_over_buttons = widgets.copy.hovered()
                                            || widgets.edit.hovered();

                                        if drag_resp.drag_started()
                                            && !pointer_over_buttons
                                            && self.drag.is_none()
                                        {
                                            if let Some(pos) = drag_resp.interact_pointer_pos() {
                                                drag_start =
                                                    Some((self.snippets[idx].id, idx, pos));
                                            }
                                        }

                                        if drag_resp.hovered()
                                            && self.drag.is_none()
                                            && !pointer_over_buttons
                                        {
                                            hover_cursor = Some(egui::CursorIcon::Grab);
                                        }

                                        ui.add_space(spacing);
                                    }
                                });
                                ui.add_space(spacing);
                            }

                            (cols, card_content_rects)
                        })
                        .inner
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
                    if let Some((id, origin, pos)) = drag_start {
                        self.drag = Some(DragState {
                            snippet_id: id,
                            origin_index: origin,
                            start_pos: pos,
                            dragging: false,
                        });
                    }

                    // Convert content rects to screen-space for drag visuals / drop testing.
                    let content_to_screen =
                        (scroll_output.inner_rect.min - scroll_output.state.offset).to_vec2();
                    let card_screen_rects: Vec<egui::Rect> = scroll_output
                        .inner
                        .1
                        .iter()
                        .map(|r| r.translate(content_to_screen))
                        .collect();
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
                    let drag_state = self.drag.as_ref().map(|d| (d.dragging, d.origin_index));
                    if let Some((dragging, origin_index)) = drag_state {
                        let pointer_pos = ctx.input(|i| i.pointer.interact_pos());
                        let pointer_released = ctx.input(|i| i.pointer.primary_released());
                        let pointer_moved = ctx.input(|i| i.pointer.delta().length_sq() > 0.0);

                        if dragging {
                            hover_cursor = Some(egui::CursorIcon::Grabbing);

                            if let Some(pointer) = pointer_pos {
                                let gap = nearest_gap(pointer, &card_screen_rects, cols, spacing, card_w);
                                draw_insertion_line(
                                    ctx,
                                    gap,
                                    &card_screen_rects,
                                    cols,
                                    spacing,
                                    card_w,
                                    card_h,
                                );

                                // Hollow ghost box following the cursor.
                                let ghost_rect = egui::Rect::from_min_size(
                                    pointer + egui::vec2(8.0, 8.0),
                                    egui::vec2(card_w, card_h),
                                );
                                let painter = ctx.layer_painter(egui::LayerId::new(
                                    egui::Order::Tooltip,
                                    egui::Id::new("drag_ghost"),
                                ));
                                painter.rect_stroke(
                                    ghost_rect,
                                    egui::Rounding::same(8.0),
                                    egui::Stroke::new(
                                        2.0,
                                        egui::Color32::from_rgb(0x60, 0xb0, 0xff),
                                    ),
                                );
                            }

                            if pointer_released {
                                if let Some(pointer) = pointer_pos {
                                    if grid_area.contains(pointer) {
                                        let gap =
                                            nearest_gap(pointer, &card_screen_rects, cols, spacing, card_w);
                                        self.reorder(origin_index, gap, &filtered);
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

        // ---- Editor window (new / edit / delete) ----
        if self.editor.is_some() {
            let mut ed = self.editor.take().unwrap();
            let categories = self.categories.clone();
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
                                    for c in &categories {
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
                                                .color(egui::Color32::from_rgb(0xef, 0x44, 0x44)),
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
                            s.body = ed.body.clone();
                        }
                    } else {
                        let id = self.next_id;
                        self.next_id += 1;
                        self.snippets.push(Snippet {
                            id,
                            title,
                            category,
                            body: ed.body.clone(),
                        });
                    }
                    self.save_snippets();
                }
                EditorResult::Delete => {
                    if let Some(id) = ed.id {
                        self.snippets.retain(|s| s.id != id);
                        self.save_snippets();
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
    }
}

fn truncate_chars(s: &str, max: usize) -> String {
    if s.chars().count() > max {
        let t: String = s.chars().take(max.saturating_sub(1)).collect();
        format!("{t}\u{2026}")
    } else {
        s.to_string()
    }
}

fn preview_text(body: &str, max: usize) -> String {
    let collapsed: String = body.split_whitespace().collect::<Vec<_>>().join(" ");
    truncate_chars(&collapsed, max)
}

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

fn gap_point(
    g: usize,
    rects: &[egui::Rect],
    cols: usize,
    spacing: f32,
    _card_w: f32,
    pointer: egui::Pos2,
) -> egui::Pos2 {
    let n = rects.len();

    // Same-row vertical gap.
    if g > 0 && g < n && !g.is_multiple_of(cols) {
        let x = (rects[g - 1].right() + rects[g].left()) * 0.5;
        let y = rects[g].center().y;
        return egui::pos2(x, y);
    }

    // Row-boundary horizontal gap.
    let y = if g == 0 {
        rects[0].top() - spacing * 0.5
    } else if g == n {
        rects[n - 1].bottom() + spacing * 0.5
    } else {
        (rects[g - 1].bottom() + rects[g].top()) * 0.5
    };

    // Center the horizontal line on the column of the nearest card.
    let nearest_x = rects
        .iter()
        .map(|r| r.center().x)
        .min_by(|a, b| (a - pointer.x).abs().partial_cmp(&(b - pointer.x).abs()).unwrap())
        .unwrap_or(rects[0].center().x);

    egui::pos2(nearest_x, y)
}

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
    let stroke = egui::Stroke::new(2.0, color);
    let clearance = 4.0_f32;

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
        let half_len = (card_h * 0.35).min(a.height() * 0.5 - clearance);
        let from = egui::pos2(x, y_center - half_len);
        let to = egui::pos2(x, y_center + half_len);
        let line_rect = line_segment_rect(from, to, stroke.width);
        if half_len > 0.0 && !line_rect.intersects(*a) && !line_rect.intersects(*b) {
            draw_dashed_line(&painter, from, to, 6.0, 4.0, stroke);
        }
    } else {
        // Horizontal dashed line at a row boundary.
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

        // Center the horizontal line on the column of the nearest card.
        let nearest = rects
            .iter()
            .min_by(|a, b| (a.center().x - pointer.x).abs().partial_cmp(&(b.center().x - pointer.x).abs()).unwrap())
            .unwrap_or(&rects[0]);
        let x_center = nearest.center().x;
        let half_len = (card_w * 0.35).min(nearest.width() * 0.5 - clearance);
        let from = egui::pos2(x_center - half_len, y);
        let to = egui::pos2(x_center + half_len, y);
        let line_rect = line_segment_rect(from, to, stroke.width);
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

/// Tight bounding rect of a line segment, including its stroke thickness.
fn line_segment_rect(from: egui::Pos2, to: egui::Pos2, stroke_width: f32) -> egui::Rect {
    let half = stroke_width * 0.5;
    egui::Rect::from_min_max(
        (from.min(to)) - egui::vec2(half, half),
        (from.max(to)) + egui::vec2(half, half),
    )
}

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
            let a = from + dir * pos;
            let b = from + dir * (pos + seg_len);
            painter.line_segment([a, b], stroke);
        }
        pos += seg_len;
        drawing_dash = !drawing_dash;
    }
}

#[cfg(test)]
mod layout_tests {
    use super::*;

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
        let app = CopyIt {
            snippets: vec![
                Snippet { id: 1, title: "One".into(), category: "Git".into(), body: "body one".into() },
                Snippet { id: 2, title: "Two".into(), category: "Git".into(), body: "body two".into() },
            ],
            next_id: 3,
            path: std::path::PathBuf::from("snippets.json"),
            config_path: std::path::PathBuf::from("config.json"),
            categories: vec!["Git".into()],
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
                let content_to_screen =
                    (scroll.inner_rect.min - scroll.state.offset).to_vec2();
                rects_out = scroll
                    .inner
                    .1
                    .iter()
                    .map(|r| r.translate(content_to_screen))
                    .collect();
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
        let app = CopyIt {
            snippets: vec![
                Snippet { id: 1, title: "One".into(), category: "Git".into(), body: "body one".into() },
                Snippet { id: 2, title: "Two".into(), category: "Git".into(), body: "body two".into() },
                Snippet { id: 3, title: "Three".into(), category: "Prompt".into(), body: "body three".into() },
                Snippet { id: 4, title: "Four".into(), category: "Prompt".into(), body: "body four".into() },
            ],
            next_id: 5,
            path: std::path::PathBuf::from("snippets.json"),
            config_path: std::path::PathBuf::from("config.json"),
            categories: vec!["Git".into(), "Prompt".into()],
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
                let content_to_screen =
                    (scroll.inner_rect.min - scroll.state.offset).to_vec2();
                rects_out = scroll.inner.1.iter().map(|r| r.translate(content_to_screen)).collect();
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
                let content_to_screen =
                    (scroll.inner_rect.min - scroll.state.offset).to_vec2();
                rects = scroll.inner.1.iter().map(|r| r.translate(content_to_screen)).collect();
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

    #[test]
    fn add_category_normalizes_and_dedups() {
        let mut app = CopyIt {
            snippets: vec![],
            next_id: 1,
            path: std::path::PathBuf::from("snippets.json"),
            config_path: std::path::PathBuf::from("config.json"),
            categories: vec!["Git".into(), "Prompt".into()],
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
        let mut app = CopyIt {
            snippets: vec![],
            next_id: 1,
            path: std::path::PathBuf::from("snippets.json"),
            config_path: std::path::PathBuf::from("config.json"),
            categories: vec![],
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
