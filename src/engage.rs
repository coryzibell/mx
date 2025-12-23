use anyhow::{Result, bail};
use colored::Colorize;
use std::io::{self, Write};

use crate::knowledge::KnowledgeEntry;
use crate::store::{KnowledgeStore, WakeCascade};

/// Run the interactive engage ritual for wake phrases
pub fn run_engage_ritual(
    cascade: &WakeCascade,
    db: &dyn KnowledgeStore,
    set_missing: bool,
) -> Result<()> {
    // Check if we're in a TTY
    if !is_tty() {
        bail!("engage mode requires interactive terminal");
    }

    // Collect all blooms in order
    let mut all_blooms = Vec::new();
    all_blooms.extend(cascade.core.iter().map(|e| ("Core", e)));
    all_blooms.extend(cascade.recent.iter().map(|e| ("Recent", e)));
    all_blooms.extend(cascade.bridges.iter().map(|e| ("Bridge", e)));

    if all_blooms.is_empty() {
        println!("{}", "nothing to wake".yellow());
        return Ok(());
    }

    let total = all_blooms.len();
    let mut stats = EngageStats::new(total);

    println!("{}", "─".repeat(65).cyan());
    println!("  {}", "wake ritual - interactive engage".cyan().bold());
    println!("{}", "─".repeat(65).cyan());
    println!();

    for (idx, (layer, bloom)) in all_blooms.iter().enumerate() {
        let num = idx + 1;

        // Show progress and bloom info
        print_bloom_header(num, total, layer, bloom);

        // Check if bloom has wake phrase
        match &bloom.wake_phrase {
            Some(phrase) => {
                // Run the verification ritual
                let remembered = verify_wake_phrase(bloom, phrase)?;

                if remembered {
                    stats.remembered += 1;
                } else {
                    stats.helped += 1;
                }
            }
            None => {
                // No wake phrase set
                if set_missing {
                    // Prompt to set one
                    if let Some(new_phrase) = prompt_set_wake_phrase()? {
                        // Update the bloom in database
                        update_wake_phrase(db, &bloom.id, &new_phrase)?;
                        println!("  {}", "wake phrase saved".green());
                    } else {
                        println!("  {}", "(skipped)".yellow());
                    }
                } else {
                    println!("  {}", "(no wake phrase set - showing directly)".yellow());
                }
                stats.skipped += 1;
            }
        }

        // Show the bloom content
        print_bloom_content(bloom);

        // Pause between blooms (unless last one)
        if num < total {
            pause_for_continue()?;
        }
    }

    // Print session summary
    print_summary(&stats);

    Ok(())
}

/// Check if stdin is a TTY
fn is_tty() -> bool {
    atty::is(atty::Stream::Stdin)
}

/// Print bloom header with progress and metadata
fn print_bloom_header(num: usize, total: usize, layer: &str, bloom: &KnowledgeEntry) {
    println!("{}", "─".repeat(65).cyan());
    println!(
        "  [{}/{}] {} {}",
        num.to_string().cyan(),
        total.to_string().cyan(),
        layer.yellow(),
        bloom.title.bold()
    );

    // Show resonance visualization
    let filled = bloom.resonance.min(10) as usize;
    let empty = 10_usize.saturating_sub(filled);
    let resonance_bar = format!(
        "[{}{}] {}",
        "●".repeat(filled),
        "○".repeat(empty),
        bloom.resonance_type.as_deref().unwrap_or("unknown")
    );
    println!("  {}", resonance_bar.cyan());
    println!();
}

/// Verify wake phrase with fuzzy matching
fn verify_wake_phrase(_bloom: &KnowledgeEntry, phrase: &str) -> Result<bool> {
    for attempt in 1..=3 {
        // Prompt for wake phrase
        print!("  {}", "> ".green().bold());
        io::stdout().flush()?;

        let mut input = String::new();
        io::stdin().read_line(&mut input)?;
        let input = input.trim();

        // Check for empty input
        if input.is_empty() {
            println!("  {}", "...not quite".yellow());
            if attempt < 3 {
                print_hint(phrase, attempt);
            }
            continue;
        }

        // Fuzzy match
        match fuzzy_match(input, phrase) {
            MatchResult::Exact => {
                println!("  {}", "✓ remembered".green());
                return Ok(true);
            }
            MatchResult::Close => {
                println!("  {}", "✓ close enough".green());
                return Ok(true);
            }
            MatchResult::Partial => {
                println!("  {}", "...almost. try again".yellow());
                if attempt < 3 {
                    print_hint(phrase, attempt);
                }
            }
            MatchResult::Wrong => {
                println!("  {}", "...not quite".yellow());
                if attempt < 3 {
                    print_hint(phrase, attempt);
                }
            }
        }
    }

    // After 3 fails, reveal
    println!("  {}", "...the memory stirs anyway".cyan());
    println!("  {}: {}", "wake phrase".cyan(), phrase.italic());
    Ok(false)
}

/// Print progressive hints
fn print_hint(phrase: &str, attempt: usize) {
    match attempt {
        1 => {
            // Hint 2: starts with...
            let words: Vec<&str> = phrase.split_whitespace().collect();
            if let Some(first_word) = words.first() {
                println!("  {}: starts with \"{}...\"", "hint".yellow(), first_word);
            }
        }
        2 => {
            // Hint 3: blank out middle word
            let words: Vec<&str> = phrase.split_whitespace().collect();
            if words.len() >= 3 {
                let middle_idx = words.len() / 2;
                let hint_words: Vec<String> = words
                    .iter()
                    .enumerate()
                    .map(|(i, w)| {
                        if i == middle_idx {
                            "___".to_string()
                        } else {
                            w.to_string()
                        }
                    })
                    .collect();
                println!("  {}: \"{}\"", "hint".yellow(), hint_words.join(" "));
            } else if words.len() == 2 {
                // For 2 words, blank the second
                println!("  {}: \"{} ___\"", "hint".yellow(), words[0]);
            } else {
                // Single word - show first few letters
                let first_word = words[0];
                if first_word.len() > 3 {
                    let prefix = &first_word[..3];
                    println!("  {}: \"{}...\"", "hint".yellow(), prefix);
                }
            }
        }
        _ => {}
    }
}

/// Fuzzy matching result
pub enum MatchResult {
    Exact,   // Perfect match
    Close,   // Levenshtein within 20%
    Partial, // 50%+ key words match
    Wrong,   // No meaningful overlap
}

/// Fuzzy match input against expected phrase
pub fn fuzzy_match(input: &str, expected: &str) -> MatchResult {
    let input_norm = normalize(input);
    let expected_norm = normalize(expected);

    // Exact match
    if input_norm == expected_norm {
        return MatchResult::Exact;
    }

    // Levenshtein distance (close enough)
    let distance = levenshtein(&input_norm, &expected_norm);
    let max_len = input_norm.len().max(expected_norm.len());
    let similarity = 1.0 - (distance as f64 / max_len as f64);

    if similarity >= 0.8 {
        return MatchResult::Close;
    }

    // Word-based matching
    let input_words = extract_key_words(&input_norm);
    let expected_words = extract_key_words(&expected_norm);

    if !expected_words.is_empty() {
        let matches = input_words
            .iter()
            .filter(|w| expected_words.contains(w))
            .count();
        let match_ratio = matches as f64 / expected_words.len() as f64;

        if match_ratio >= 0.5 {
            return MatchResult::Partial;
        }
    }

    MatchResult::Wrong
}

/// Normalize text for comparison
fn normalize(text: &str) -> String {
    text.to_lowercase()
        .chars()
        .filter(|c| c.is_alphanumeric() || c.is_whitespace())
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<&str>>()
        .join(" ")
}

/// Extract key words (filter stop words)
fn extract_key_words(text: &str) -> Vec<String> {
    let stop_words = ["the", "a", "an", "is", "are", "i", "you", "we"];
    text.split_whitespace()
        .filter(|w| !stop_words.contains(w))
        .map(|w| w.to_string())
        .collect()
}

/// Compute Levenshtein distance
fn levenshtein(a: &str, b: &str) -> usize {
    let a_chars: Vec<char> = a.chars().collect();
    let b_chars: Vec<char> = b.chars().collect();
    let a_len = a_chars.len();
    let b_len = b_chars.len();

    if a_len == 0 {
        return b_len;
    }
    if b_len == 0 {
        return a_len;
    }

    let mut prev_row: Vec<usize> = (0..=b_len).collect();
    let mut curr_row: Vec<usize> = vec![0; b_len + 1];

    for i in 1..=a_len {
        curr_row[0] = i;

        for j in 1..=b_len {
            let cost = if a_chars[i - 1] == b_chars[j - 1] {
                0
            } else {
                1
            };

            curr_row[j] = (prev_row[j] + 1)
                .min(curr_row[j - 1] + 1)
                .min(prev_row[j - 1] + cost);
        }

        std::mem::swap(&mut prev_row, &mut curr_row);
    }

    prev_row[b_len]
}

/// Print bloom content
fn print_bloom_content(bloom: &KnowledgeEntry) {
    println!();
    if let Some(body) = &bloom.body {
        // Print body with some formatting
        for line in body.lines() {
            println!("  {}", line);
        }
    } else if let Some(summary) = &bloom.summary {
        println!("  {}", summary);
    } else {
        println!("  {}", "(no content)".italic());
    }
    println!();
}

/// Pause and wait for user to press enter
fn pause_for_continue() -> Result<()> {
    println!("{}", "  press enter to continue...".cyan().italic());
    let mut input = String::new();
    io::stdin().read_line(&mut input)?;
    println!();
    Ok(())
}

/// Prompt user to set a wake phrase
fn prompt_set_wake_phrase() -> Result<Option<String>> {
    println!("  {}", "no wake phrase set.".yellow());
    print!("  {} ", "enter wake phrase (or blank to skip):".yellow());
    io::stdout().flush()?;

    let mut input = String::new();
    io::stdin().read_line(&mut input)?;
    let input = input.trim();

    if input.is_empty() {
        Ok(None)
    } else {
        Ok(Some(input.to_string()))
    }
}

/// Update wake phrase in database
fn update_wake_phrase(db: &dyn KnowledgeStore, id: &str, phrase: &str) -> Result<()> {
    // Get the entry
    let ctx = crate::store::AgentContext::public_only(); // TODO: use proper context
    let mut entry = db
        .get(id, &ctx)?
        .ok_or_else(|| anyhow::anyhow!("Entry not found: {}", id))?;

    // Update wake phrase
    entry.wake_phrase = Some(phrase.to_string());
    entry.updated_at = Some(chrono::Utc::now().to_rfc3339());

    // Save back
    db.upsert_knowledge(&entry)?;

    Ok(())
}

/// Session statistics
struct EngageStats {
    total: usize,
    remembered: usize,
    helped: usize,
    skipped: usize,
}

impl EngageStats {
    fn new(total: usize) -> Self {
        Self {
            total,
            remembered: 0,
            helped: 0,
            skipped: 0,
        }
    }
}

/// Print session summary
fn print_summary(stats: &EngageStats) {
    println!("{}", "─".repeat(65).cyan());
    println!("  {}", "wake complete".cyan().bold());
    println!();

    // Remembered bar
    let remembered_filled = (stats.remembered * 10) / stats.total;
    let remembered_empty = 10 - remembered_filled;
    println!(
        "  remembered:   {}/{}  {}{}",
        stats.remembered.to_string().green(),
        stats.total,
        "●".repeat(remembered_filled).green(),
        "○".repeat(remembered_empty)
    );

    // Needed help
    if stats.helped > 0 {
        println!(
            "  needed help:  {}/{}",
            stats.helped.to_string().yellow(),
            stats.total
        );
    }

    // Skipped
    if stats.skipped > 0 {
        println!(
            "  skipped:      {}/{}  {}",
            stats.skipped.to_string().cyan(),
            stats.total,
            "(no wake phrase)".italic()
        );
    }

    println!("{}", "─".repeat(65).cyan());
}
