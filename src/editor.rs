//! The snippet add/edit modal: its state, its constructors, and the pure
//! "what should happen next" decision the UI hands back to the app.
//!
//! The `egui::Window` rendering lives in `app.rs`; this module owns the
//! editor's *state* and the *transition* (`[`decide`]`) that turns a button
//! click plus the window's open/close flag into an [`EditorOutcome`]. The
//! app then applies the outcome, so all editor policy is testable without a
//! UI context.

use crate::model::Snippet;

/// Modal editor state for creating or editing a snippet. The `adding_category` and
/// `new_category` fields track an inline sub-form (entered via the category dropdown)
/// that lets users add a category without closing the editor. The `confirm_delete`
/// flag requires a second click to prevent accidental deletions.
pub struct Editor {
    /// None = creating a new snippet; Some(id) = editing existing with this stable ID.
    pub id: Option<u64>,
    pub title: String,
    pub category: String,
    /// Input buffer for inline category creation; cleared when the user confirms.
    pub new_category: String,
    /// True when the user clicked "+ Add new category" in the dropdown.
    pub adding_category: bool,
    pub body: String,
    /// Set to true on first "Delete" click; requires a second "Confirm delete" to prevent accidents.
    pub confirm_delete: bool,
}

impl Editor {
    /// A blank editor for adding a new snippet: starts with the first category
    /// (or empty if there are none) and disables inline category creation and
    /// delete confirmation.
    pub fn blank(categories: &[String]) -> Self {
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

    /// An editor pre-populated from an existing snippet. The category is matched
    /// against the canonical list case-insensitively, falling back to the
    /// snippet's own category if no canonical match exists.
    pub fn from_snippet(s: &Snippet, categories: &[String]) -> Self {
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

/// The action the user triggered in the editor modal.
pub enum EditorResult {
    /// No action (editor still open); keep the editor visible.
    None,
    /// User clicked Save; persist the current fields and close.
    Save,
    /// User clicked Cancel; discard changes.
    Cancel,
    /// User confirmed deletion (second click); remove the snippet.
    Delete,
    /// User created a new category in the editor; add it to the canonical list.
    AddCategory(String),
}

/// What the app should do with the editor after a button click.
pub enum EditorOutcome {
    /// Keep editing — hand the editor state back to the app.
    Keep(Editor),
    /// Discard the editor (cancel, or the window was closed via the X).
    Close,
    /// Save the editor's current fields, then close.
    Save(Editor),
    /// Remove the snippet with this id, then close.
    Delete(u64),
    /// Register the category (editor stays open so the user can pick it).
    AddCategory(String, Editor),
}

/// Decides what to do next given the button that was clicked, the editor state,
/// and whether the window is still open. Pure: no app state, no IO.
pub fn decide(result: EditorResult, ed: Editor, window_open: bool) -> EditorOutcome {
    match result {
        EditorResult::None => {
            // Keep editing unless the user closed the window via the X.
            if window_open {
                EditorOutcome::Keep(ed)
            } else {
                EditorOutcome::Close
            }
        }
        EditorResult::Save => EditorOutcome::Save(ed),
        EditorResult::Cancel => EditorOutcome::Close,
        EditorResult::Delete => {
            // The "Confirm delete" button only renders when editing an existing
            // snippet, so the id is always present; be defensive anyway.
            match ed.id {
                Some(id) => EditorOutcome::Delete(id),
                None => EditorOutcome::Close,
            }
        }
        EditorResult::AddCategory(name) => EditorOutcome::AddCategory(name, ed),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn editor(id: Option<u64>) -> Editor {
        Editor {
            id,
            title: "Title".into(),
            category: "Git".into(),
            new_category: String::new(),
            adding_category: false,
            body: "Body".into(),
            confirm_delete: false,
        }
    }

    fn categories() -> Vec<String> {
        vec!["Git".into(), "Prompt".into()]
    }

    #[test]
    fn blank_uses_the_first_category() {
        let ed = Editor::blank(&categories());
        assert_eq!(ed.category, "Git");
        assert!(ed.id.is_none());
        assert!(ed.title.is_empty());
        assert!(ed.body.is_empty());
        assert!(!ed.confirm_delete);

        let empty = Editor::blank(&[]);
        assert_eq!(empty.category, "");
    }

    #[test]
    fn from_snippet_matches_the_canonical_category_case_insensitively() {
        let s = Snippet {
            id: 7,
            title: "Stash".into(),
            category: "pRoMpT".into(),
            body: "git stash".into(),
        };
        let ed = Editor::from_snippet(&s, &categories());
        assert_eq!(ed.id, Some(7));
        assert_eq!(ed.category, "Prompt", "must pick the canonical form");

        let unknown = Snippet {
            id: 8,
            title: "X".into(),
            category: "Uncategorized".into(),
            body: "y".into(),
        };
        let ed = Editor::from_snippet(&unknown, &categories());
        assert_eq!(ed.category, "Uncategorized", "no match keeps the original");
    }

    #[test]
    fn decide_keeps_the_editor_open_on_no_action() {
        let ed = editor(Some(1));
        match decide(EditorResult::None, ed, true) {
            EditorOutcome::Keep(kept) => assert_eq!(kept.id, Some(1)),
            other => panic!("expected Keep, got {other:?}"),
        }
        match decide(EditorResult::None, editor(Some(1)), false) {
            EditorOutcome::Close => {}
            other => panic!("expected Close when window closed, got {other:?}"),
        }
    }

    #[test]
    fn decide_save_cancel_and_delete() {
        assert!(matches!(
            decide(EditorResult::Save, editor(Some(1)), true),
            EditorOutcome::Save(_)
        ));
        assert!(matches!(
            decide(EditorResult::Cancel, editor(Some(1)), true),
            EditorOutcome::Close
        ));
        match decide(EditorResult::Delete, editor(Some(1)), true) {
            EditorOutcome::Delete(id) => assert_eq!(id, 1),
            other => panic!("expected Delete, got {other:?}"),
        }
    }

    #[test]
    fn decide_add_category_keeps_the_editor_open() {
        match decide(
            EditorResult::AddCategory("Docker".into()),
            editor(Some(2)),
            true,
        ) {
            EditorOutcome::AddCategory(name, kept) => {
                assert_eq!(name, "Docker");
                assert_eq!(kept.id, Some(2));
            }
            other => panic!("expected AddCategory, got {other:?}"),
        }
    }

    #[test]
    fn decide_delete_on_an_editor_with_no_id_closes_without_deleting() {
        match decide(EditorResult::Delete, editor(None), true) {
            EditorOutcome::Close => {}
            other => panic!("expected Close, got {other:?}"),
        }
    }

    /// The derive-free enums end up as plain data, so test asserts need a
    /// human-readable rendering. Keep it small on purpose.
    impl std::fmt::Debug for EditorOutcome {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            match self {
                EditorOutcome::Keep(_) => write!(f, "Keep(..)"),
                EditorOutcome::Close => write!(f, "Close"),
                EditorOutcome::Save(_) => write!(f, "Save(..)"),
                EditorOutcome::Delete(id) => write!(f, "Delete({id})"),
                EditorOutcome::AddCategory(name, _) => write!(f, "AddCategory({name}, ..)"),
            }
        }
    }
}
