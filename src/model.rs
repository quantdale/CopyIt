use serde::{Deserialize, Serialize};

/// A single stored item: a script or a prompt.
#[derive(Clone, Serialize, Deserialize)]
pub struct Snippet {
    pub id: u64,
    pub title: String,
    pub category: String,
    pub body: String,
}
