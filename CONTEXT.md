# CopyIt — Domain Context

Terms the codebase uses for its domain concepts. Architecture discussions and code should use these names.

- **Snippet** — a stored item: `{ id, title, category, body }`. The unit of data in the library; persisted in `snippets.json`.
- **Library** — the set of snippets the app manages. Persisted automatically after every add, edit, delete, or reorder.
- **Category** — a label attached to snippets. Canonical form is title-cased, trimmed, sorted, and deduplicated case-insensitively. `"All"` is reserved for the filter's show-everything value; empty input is rejected. All category rules live in `storage.rs` (`normalize_category`, `same_category`, `is_reserved_category`, `canonical_category`); the app keeps the canonical list as a plain sorted `Vec<String>`.
- **Grid** — the card grid module (`grid.rs`): layout constants, column math, virtualization (`visible_rows`), gap/drop geometry (`gap_point`, `nearest_gap`), insertion-line drawing, and the `DragMachine` state machine. The `app.rs` render loop stays thin.
- **Store** — the persistence seam (`store.rs`): owns the data location (`%APPDATA%\CopyIt`), the one-time legacy migration, and the load/save of `snippets.json` + `config.json`. `app.rs` never touches paths.
- **Editor outcome** — the pure decision (`editor::decide`) that turns an editor button click plus the window's open/close flag into something the app applies (`EditorOutcome::Keep/Close/Save/Delete/AddCategory`). The editor modal's egui rendering stays in `app.rs`.
- **Theme** — a selectable color scheme (one of 37), persisted in `config.json` as its display name.
