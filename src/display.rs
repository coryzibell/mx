use crate::knowledge;
use crate::store;

/// Truncate a string to a maximum number of characters, adding "..." if truncated
///
/// This is UTF-8 safe - it counts characters, not bytes, avoiding panics on
/// multi-byte characters like emoji.
pub(crate) fn safe_truncate(s: &str, max_chars: usize) -> String {
    let char_count = s.chars().count();
    if char_count > max_chars {
        let truncated: String = s.chars().take(max_chars.saturating_sub(3)).collect();
        format!("{}...", truncated)
    } else {
        s.to_string()
    }
}

pub(crate) fn print_entry_summary(entry: &knowledge::KnowledgeEntry) {
    println!("  {} [{}]", entry.id, entry.category_id);
    println!("  {}", entry.title);
    if let Some(summary) = &entry.summary {
        let short = safe_truncate(summary, 80);
        println!("  {}", short);
    }
    if !entry.tags.is_empty() {
        println!("  Tags: {}", entry.tags.join(", "));
    }
    println!();
}

pub(crate) fn print_entry_full(entry: &knowledge::KnowledgeEntry) {
    print!("{}", format_entry_full(entry));
}

/// Render the full detail view of an entry into a String.
///
/// Split out from `print_entry_full` so callers (e.g. `mx memory show`) can
/// measure the rendered byte length before deciding whether to print it to
/// stdout or divert it to a temp file. The returned string ends with a
/// trailing newline, matching the previous `println!`-based output exactly.
pub(crate) fn format_entry_full(entry: &knowledge::KnowledgeEntry) -> String {
    use std::fmt::Write;
    let mut out = String::new();

    let _ = writeln!(out, "ID:       {}", entry.id);
    let _ = writeln!(out, "Category: {}", entry.category_id);

    // Extract state from summary if present
    let state = entry.get_summary_state();

    if let Some(state) = state {
        let _ = writeln!(out, "Title:    {} ({})", entry.title, state);
    } else {
        let _ = writeln!(out, "Title:    {}", entry.title);
    }

    if entry.resonance > 0 {
        let _ = writeln!(out, "Resonance: {}", entry.resonance);
    }
    if let Some(ref rtype) = entry.resonance_type {
        let _ = writeln!(out, "Resonance Type: {}", rtype);
    }
    if let Some(ref phrase) = entry.wake_phrase {
        let _ = writeln!(out, "Wake Phrase: {}", phrase);
    }
    if !entry.wake_phrases.is_empty() {
        let _ = writeln!(out, "Wake Phrases: {}", entry.wake_phrases.join(", "));
    }
    if !entry.triggers.is_empty() {
        let _ = writeln!(out, "Triggers: {}", entry.triggers.join(", "));
    }
    if let Some(path) = &entry.file_path {
        let _ = writeln!(out, "File:     {}", path);
    }
    if !entry.tags.is_empty() {
        let _ = writeln!(out, "Tags:     {}", entry.tags.join(", "));
    }
    if !entry.applicability.is_empty() {
        let _ = writeln!(out, "Applicability: {}", entry.applicability.join(", "));
    }
    if !entry.anchors.is_empty() {
        let _ = writeln!(out, "Anchors:  {}", entry.anchors.join(", "));
    }
    // Always show visibility for private entries (public is the default)
    if entry.visibility == "private" {
        let _ = writeln!(out, "Visibility: {}", entry.visibility);
        if let Some(ref o) = entry.owner {
            let _ = writeln!(out, "Owner:    {}", o);
        }
    }
    if let Some(created) = &entry.created_at {
        let _ = writeln!(out, "Created:  {}", created);
    }
    if let Some(updated) = &entry.updated_at {
        let _ = writeln!(out, "Updated:  {}", updated);
    }
    let _ = writeln!(out, "Format:   {}", entry.format);
    let _ = writeln!(out);
    if let Some(body) = &entry.body {
        let _ = writeln!(out, "{}", body);
    }

    out
}

pub(crate) fn print_wake_cascade(cascade: &store::WakeCascade) {
    if !cascade.core.is_empty() {
        println!("\n=== CORE (Foundational) ===\n");
        for entry in &cascade.core {
            println!("  {} [{}] {}", entry.id, entry.resonance, entry.title);
        }
    }

    if !cascade.recent.is_empty() {
        println!("\n=== RECENT ===\n");
        for entry in &cascade.recent {
            println!("  {} [{}] {}", entry.id, entry.resonance, entry.title);
        }
    }

    if !cascade.bridges.is_empty() {
        println!("\n=== BRIDGES ===\n");
        for entry in &cascade.bridges {
            println!("  {} [{}] {}", entry.id, entry.resonance, entry.title);
        }
    }

    let total = cascade.core.len() + cascade.recent.len() + cascade.bridges.len();
    println!(
        "\nLoaded {} memories across {} layers.",
        total,
        [
            !cascade.core.is_empty(),
            !cascade.recent.is_empty(),
            !cascade.bridges.is_empty()
        ]
        .iter()
        .filter(|&&x| x)
        .count()
    );
}
