use crate::model::Snippet;

/// Default library seeded on first launch when no snippets.json exists yet.
/// Provides examples of common Git workflows and AI prompt templates to get users started.
/// Each snippet has a unique sequential ID (1-based) for initial ordering.
/// Users can edit, delete, or add to these snippets—they are just a starting point.
pub fn defaults() -> Vec<Snippet> {
    let items: &[(&str, &str, &str)] = &[
        (
            "Update all repos (PowerShell)",
            "Git",
            "# Pull latest on every git repo one level under the current folder\r\nGet-ChildItem -Directory | Where-Object { Test-Path \"$($_.FullName)\\.git\" } | ForEach-Object {\r\n    Write-Host \"== $($_.Name) ==\" -ForegroundColor Cyan\r\n    git -C $_.FullName pull --ff-only\r\n}",
        ),
        (
            "Update all repos (bash)",
            "Git",
            "#!/usr/bin/env bash\n# Pull latest on every git repo under the current directory\nfind . -maxdepth 2 -type d -name .git | while read -r gitdir; do\n  repo=\"$(dirname \"$gitdir\")\"\n  echo \"== $repo ==\"\n  git -C \"$repo\" pull --ff-only\ndone",
        ),
        (
            "Discard all local changes",
            "Git",
            "# Throw away uncommitted changes and untracked files (irreversible)\ngit reset --hard HEAD\ngit clean -fd",
        ),
        (
            "Summarize this conversation",
            "Prompt",
            "Summarize our conversation so far into a concise brief. Include:\n1. The objective\n2. Key decisions and their rationale\n3. Any code/artifacts produced (reference by name)\n4. Open questions\n5. Concrete next steps\n\nUse tight bullet points. Omit small talk.",
        ),
        (
            "Generate documentation",
            "Prompt",
            "You are a senior technical writer. Given the code/context below, produce clear documentation covering: purpose, prerequisites, setup/usage with examples, configuration options, and common pitfalls. Prefer prose with short code blocks. Do not invent behavior that isn't present in the source.\n\n---\n<paste code/context here>",
        ),
        (
            "Session handoff / context transfer",
            "Prompt",
            "Export the full context of this session into a single self-contained block I can paste into a fresh chat to continue seamlessly. Include, verbatim where it matters:\n- Current objective\n- The narrative of what we've done\n- Decisions made (with rationale)\n- All artifacts in full\n- Open questions\n- Immediate next steps\n\nWrite it so a model with zero prior context can pick up exactly where we left off.",
        ),
    ];

    items
        .iter()
        .enumerate()
        .map(|(i, (title, category, body))| Snippet {
            id: (i as u64) + 1,
            title: (*title).to_string(),
            description: String::new(),
            category: (*category).to_string(),
            body: (*body).to_string(),
            protection: None,
        })
        .collect()
}
