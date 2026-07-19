use serde::{Deserialize, Serialize};

/// A single stored item: a script or a prompt.
/// The `id` field is stable across edits/saves (never reassigned) and is used as a unique key
/// for the Editor modal and drag-and-drop operations. Snippets are serialized to JSON in
/// insertion order (they remain in self.snippets in whatever order the user drags them to).
#[derive(Clone, Serialize, Deserialize)]
pub struct Snippet {
    pub id: u64,           // Stable unique identifier; incremented on creation, never reused
    pub title: String,     // Display name of the snippet
    pub category: String,  // User-defined category (normalized to title-case)
    pub body: String,      // The actual content to copy to clipboard
}
