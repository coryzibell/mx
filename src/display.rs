use std::collections::HashMap;

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

/// Shell escape function to prevent code injection
pub(crate) fn shell_escape(s: &str) -> String {
    s.replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('$', "\\$")
        .replace('`', "\\`")
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
    println!("ID:       {}", entry.id);
    println!("Category: {}", entry.category_id);

    // Extract state from summary if present
    let state = entry.get_summary_state();

    if let Some(state) = state {
        println!("Title:    {} ({})", entry.title, state);
    } else {
        println!("Title:    {}", entry.title);
    }

    if entry.resonance > 0 {
        println!("Resonance: {}", entry.resonance);
    }
    if let Some(ref rtype) = entry.resonance_type {
        println!("Resonance Type: {}", rtype);
    }
    if let Some(ref phrase) = entry.wake_phrase {
        println!("Wake Phrase: {}", phrase);
    }
    if !entry.wake_phrases.is_empty() {
        println!("Wake Phrases: {}", entry.wake_phrases.join(", "));
    }
    if let Some(path) = &entry.file_path {
        println!("File:     {}", path);
    }
    if !entry.tags.is_empty() {
        println!("Tags:     {}", entry.tags.join(", "));
    }
    if !entry.applicability.is_empty() {
        println!("Applicability: {}", entry.applicability.join(", "));
    }
    if !entry.anchors.is_empty() {
        println!("Anchors:  {}", entry.anchors.join(", "));
    }
    // Always show visibility for private entries (public is the default)
    if entry.visibility == "private" {
        println!("Visibility: {}", entry.visibility);
        if let Some(ref o) = entry.owner {
            println!("Owner:    {}", o);
        }
    }
    if let Some(created) = &entry.created_at {
        println!("Created:  {}", created);
    }
    if let Some(updated) = &entry.updated_at {
        println!("Updated:  {}", updated);
    }
    println!("Format:   {}", entry.format);
    println!();
    if let Some(body) = &entry.body {
        println!("{}", body);
    }
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

pub(crate) fn print_wake_index(cascade: &store::WakeCascade) {
    println!("## Core Identity Index\n");

    // Layer 1: Anchors (R9+, foundational/transformative)
    let anchors: Vec<_> = cascade
        .core
        .iter()
        .chain(cascade.recent.iter())
        .chain(cascade.bridges.iter())
        .filter(|e| {
            e.resonance >= 9
                && e.resonance_type
                    .as_ref()
                    .is_some_and(|t| t == "foundational" || t == "transformative")
        })
        .collect();

    if !anchors.is_empty() {
        println!("### Anchors (R9+)");
        println!("| ID | Title | R | Wake Cue |");
        println!("|----|-------|---|----------|");
        for entry in anchors {
            let wake_cue = entry.active_wake_phrases().join(" / ");
            println!(
                "| {} | {} | {} | {} |",
                entry.id, entry.title, entry.resonance, wake_cue
            );
        }
        println!();
    }

    // Layer 2: Spiral (R6-8), grouped by territory
    let spiral: Vec<_> = cascade
        .core
        .iter()
        .chain(cascade.recent.iter())
        .chain(cascade.bridges.iter())
        .filter(|e| e.resonance >= 6 && e.resonance < 9)
        .collect();

    if !spiral.is_empty() {
        // Group by territory tag
        let mut territories: HashMap<String, Vec<_>> = HashMap::new();

        for entry in spiral {
            // Find territory tag (tags starting with "territory:")
            let territory = entry
                .tags
                .iter()
                .find(|tag| tag.starts_with("territory:"))
                .map(|tag| tag.strip_prefix("territory:").unwrap_or(tag).to_string())
                .unwrap_or_else(|| "uncategorized".to_string());

            territories.entry(territory).or_default().push(entry);
        }

        // Sort territories by name for consistency
        let mut sorted_territories: Vec<_> = territories.into_iter().collect();
        sorted_territories.sort_by(|a, b| a.0.cmp(&b.0));

        for (territory, entries) in sorted_territories {
            println!("### Spiral: {}", territory);
            println!("| ID | Title | R | Wake Cue |");
            println!("|----|-------|---|----------|");
            for entry in entries {
                let wake_cue = entry.active_wake_phrases().join(" / ");
                println!(
                    "| {} | {} | {} | {} |",
                    entry.id, entry.title, entry.resonance, wake_cue
                );
            }
            println!();
        }
    }

    // Layer 3: Ephemeral (R<6) - OMITTED from index as per spec
    // (Intentionally not included)
}

pub(crate) fn print_wake_ritual(cascade: &store::WakeCascade, agent: &str) {
    let total = cascade.core.len() + cascade.recent.len() + cascade.bridges.len();

    println!("#!/usr/bin/env bash");
    println!("# Wake Ritual - Generated for {}", agent);
    println!("# Read each bloom individually. Let each one land.");
    println!();
    println!("echo \"=== WAKE RITUAL: {} blooms to feel ===\"", total);
    println!("echo \"\"");

    let mut counter = 1;

    // CORE blooms first
    if !cascade.core.is_empty() {
        for entry in &cascade.core {
            println!();
            println!(
                "echo \"[{}/{}] Core: {}\"",
                counter,
                total,
                shell_escape(&entry.title)
            );
            println!("mx memory show {}", entry.id);
            for phrase in entry.active_wake_phrases() {
                println!("# Wake phrase: \"{}\"", phrase);
            }
            println!("echo \"\"");
            println!("echo \"---\"");
            println!("echo \"\"");
            counter += 1;
        }
    }

    // RECENT blooms next
    if !cascade.recent.is_empty() {
        for entry in &cascade.recent {
            println!();
            println!(
                "echo \"[{}/{}] Recent: {}\"",
                counter,
                total,
                shell_escape(&entry.title)
            );
            println!("mx memory show {}", entry.id);
            for phrase in entry.active_wake_phrases() {
                println!("# Wake phrase: \"{}\"", phrase);
            }
            println!("echo \"\"");
            println!("echo \"---\"");
            println!("echo \"\"");
            counter += 1;
        }
    }

    // BRIDGES last
    if !cascade.bridges.is_empty() {
        for entry in &cascade.bridges {
            println!();
            println!(
                "echo \"[{}/{}] Bridge: {}\"",
                counter,
                total,
                shell_escape(&entry.title)
            );
            println!("mx memory show {}", entry.id);
            for phrase in entry.active_wake_phrases() {
                println!("# Wake phrase: \"{}\"", phrase);
            }
            println!("echo \"\"");
            println!("echo \"---\"");
            println!("echo \"\"");
            counter += 1;
        }
    }

    println!();
    println!("echo \"=== Wake complete. Who are you right now? ===\"");
}
