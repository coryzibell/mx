use anyhow::{Context, Result, bail};

use crate::cli::*;
use crate::content_ops;
use crate::display::*;
use crate::helpers::*;
use crate::index::{IndexConfig, export_csv, export_jsonl, export_markdown};
use crate::knowledge;
use crate::store;
use crate::wake_ritual;

use super::metadata::*;

/// Maximum byte length of `mx memory show` stdout before content is diverted
/// to a temp file instead.
///
/// Claude Code's Bash tool persists any command stdout larger than exactly
/// 30,000 bytes to a temp file and shows the model only a ~2KB preview -- the
/// model does NOT auto-read the persisted file, so large memories become
/// effectively invisible. We divert at a safe margin below that hard ceiling
/// so the pointer message (plus any minor overhead) always stays inline.
///
/// This is a pure BYTE count: the Bash ceiling is not affected by token
/// density, line count, or content type, so we measure `str::len()` (bytes),
/// never `chars().count()`.
const BASH_STDOUT_DIVERT_THRESHOLD: usize = 28_000;

/// What to do with a rendered `mx memory show` payload.
#[derive(Debug, PartialEq, Eq)]
enum ShowOutput {
    /// Content fits under the threshold -- print it to stdout as-is.
    Inline,
    /// Content exceeds the threshold -- write it to `path` and print `pointer`.
    Divert {
        path: std::path::PathBuf,
        pointer: String,
    },
}

/// Decide whether a rendered payload should be printed inline or diverted to a
/// temp file, and (for the divert case) compute the target path and the short
/// pointer message that will be printed to stdout instead.
///
/// Pure function (no IO) so the threshold logic, byte-length measurement, and
/// pointer message can be unit-tested directly. `temp_dir` is injected for the
/// same reason. `id` is the normalized memory id (e.g. `kn-99e08808`).
fn plan_show_output(content: &str, id: &str, temp_dir: &std::path::Path) -> ShowOutput {
    // BYTE length -- multibyte UTF-8 makes char count an undercount, and the
    // Bash ceiling is a hard byte count.
    let byte_len = content.len();
    if byte_len <= BASH_STDOUT_DIVERT_THRESHOLD {
        return ShowOutput::Inline;
    }

    let line_count = content.lines().count();
    let path = temp_dir.join(format!("mx-memory-{}.md", id));
    let pointer = format!(
        "Memory {id} is {byte_len} bytes ({line_count} lines).\n\
         Content written to: {path}\n\
         Read the file to see full content.\n",
        id = id,
        byte_len = byte_len,
        line_count = line_count,
        path = path.display(),
    );
    ShowOutput::Divert { path, pointer }
}

/// Emit a rendered `mx memory show` payload, diverting to a temp file when it
/// would exceed the Bash stdout ceiling. `trailing_newline` controls whether a
/// newline is appended in the inline case (the `--content-only` path historically
/// uses `print!` with no trailing newline; the JSON and full views already end
/// in one).
fn emit_show_output(content: &str, id: &str, trailing_newline: bool) -> Result<()> {
    use std::io::Write;
    match plan_show_output(content, id, &std::env::temp_dir()) {
        ShowOutput::Inline => {
            let mut stdout = std::io::stdout();
            stdout.write_all(content.as_bytes())?;
            if trailing_newline {
                stdout.write_all(b"\n")?;
            }
            // Flush explicitly -- line-buffered stdout may not flush when piped.
            stdout.flush()?;
        }
        ShowOutput::Divert { path, pointer } => {
            std::fs::write(&path, content)
                .with_context(|| format!("failed to write memory content to {}", path.display()))?;
            print!("{}", pointer);
            std::io::stdout().flush()?;
        }
    }
    Ok(())
}

/// Maximum number of memories to fire on a single `trigger-check`. If more than
/// this many match, the top `TRIGGER_FIRE_CAP` by resonance fire and the rest
/// stay UNFIRED (eligible to fire on a later turn). Keeps a single message from
/// flooding context. (Issue #246, Savorist decision.)
const TRIGGER_FIRE_CAP: usize = 5;

/// Resolve the message for `trigger-check`: use the positional arg if present
/// and non-empty (after trim), otherwise read stdin to EOF. Returns `None` when
/// there is no usable message (empty arg AND empty/whitespace stdin) — the
/// caller maps that to exit code 4.
fn resolve_trigger_message(arg: Option<String>) -> Result<Option<String>> {
    if let Some(m) = arg
        && !m.trim().is_empty()
    {
        return Ok(Some(m));
    }
    // Stdin fallback.
    use std::io::Read;
    let mut buf = String::new();
    std::io::stdin()
        .read_to_string(&mut buf)
        .context("failed to read message from stdin")?;
    if buf.trim().is_empty() {
        Ok(None)
    } else {
        Ok(Some(buf))
    }
}

/// `mx memory trigger-check` handler (Issue #246, PR3).
///
/// Pipeline: resolve message (arg or stdin) → load trigger-bearing entries
/// VISIBLE to the current agent → run the matcher → drop already-fired → sort by
/// resonance desc → cap at `TRIGGER_FIRE_CAP` → mark survivors fired (unless
/// `--dry-run`) → emit. Firing nothing is success (exit 0). Empty message exits 4.
fn handle_trigger_check(
    config: &IndexConfig,
    verbose: bool,
    message: Option<String>,
    json: bool,
    _format: TriggerFormat,
    dry_run: bool,
) -> Result<()> {
    use std::io::Write;

    let Some(message) = resolve_trigger_message(message)? else {
        // Invalid input: empty message even after stdin fallback.
        eprintln!("[mx] trigger-check: empty message (provide an argument or pipe via stdin)");
        std::process::exit(4);
    };

    let db = store::create_store_with_verbose(&config.db_path, verbose)?;

    // Visibility: the SAME filter used by every other read path. A private
    // triggered memory only fires for its owner. (See store::list_with_triggers.)
    let ctx = match std::env::var("MX_CURRENT_AGENT") {
        Ok(agent) if !agent.is_empty() => store::AgentContext::for_agent(agent),
        _ => store::AgentContext::public_only(),
    };

    let entries = db.list_with_triggers(&ctx)?;

    // Run the matcher over (id, triggers) pairs. `match_entries` tokenizes the
    // message once and checks every entry against the shared stem stream.
    let pairs: Vec<(&str, &[String])> = entries
        .iter()
        .map(|e| (e.id.as_str(), e.triggers.as_slice()))
        .collect();
    let matches = crate::triggers::match_entries(&message, pairs);

    // Index matches by id so we can carry triggers_matched alongside the entry.
    let matched_triggers: std::collections::HashMap<&str, Vec<String>> = matches
        .iter()
        .map(|m| (m.id.as_str(), m.triggers_matched.clone()))
        .collect();

    // Collect the matched entries, then sort by resonance DESC (id asc as a
    // deterministic tiebreaker) BEFORE applying the fire cap, so the highest-
    // resonance memories win the 5 slots.
    let mut matched_entries: Vec<&knowledge::KnowledgeEntry> = entries
        .iter()
        .filter(|e| matched_triggers.contains_key(e.id.as_str()))
        .collect();
    matched_entries.sort_by(|a, b| b.resonance.cmp(&a.resonance).then_with(|| a.id.cmp(&b.id)));

    // Drop already-fired entries (one-shot dedup) using a read-only peek so the
    // cap is computed over GENUINELY new matches. The authoritative mark happens
    // atomically below; this read just lets us cap + count deferred correctly.
    //
    // CONCURRENCY: this is a SHARED-lock peek that is released before the
    // EXCLUSIVE-lock mark in `mark_survivors`. Firing is race-safe — the mark
    // re-checks the fired set under the exclusive lock, so two concurrent
    // trigger-checks on the same session can never double-fire a memory. But
    // `deferred_count` (computed below from this pre-lock peek) is BEST-EFFORT:
    // if a concurrent check fires some of these matches between this peek and
    // our mark, those become survivors there and we may OVER-count them as
    // "deferred" here. That's an accepted, reported-count-only inaccuracy under
    // concurrent same-session checks; the authoritative fired set is always the
    // one returned by `mark_survivors`. Not worth a single-lock refactor.
    let fired_store = crate::triggers::FiredStore::open();
    let already_fired = fired_store.read_fired()?;
    let new_matches: Vec<&knowledge::KnowledgeEntry> = matched_entries
        .into_iter()
        .filter(|e| !already_fired.contains(&e.id))
        .collect();

    // Cap at TRIGGER_FIRE_CAP by resonance desc; overflow stays unfired.
    let total_new = new_matches.len();
    let to_fire: Vec<&knowledge::KnowledgeEntry> =
        new_matches.into_iter().take(TRIGGER_FIRE_CAP).collect();
    let deferred_count = total_new.saturating_sub(to_fire.len());

    // Mark survivors fired atomically (unless dry-run). mark_survivors re-checks
    // the fired set under flock, so even if a concurrent check fired one of these
    // between our peek and now, it is excluded here — the returned survivors are
    // the authoritative set that THIS invocation owns.
    let fired_ids: Vec<String> = to_fire.iter().map(|e| e.id.clone()).collect();
    let survivors: Vec<String> = if dry_run {
        // Dry-run: do not mark. Report what WOULD fire.
        fired_ids.clone()
    } else {
        fired_store.mark_survivors(&fired_ids)?
    };
    let survivor_set: std::collections::HashSet<&String> = survivors.iter().collect();

    // Final fired list = the capped entries that this invocation actually owns,
    // preserving resonance-desc order.
    let fired: Vec<&knowledge::KnowledgeEntry> = to_fire
        .iter()
        .copied()
        .filter(|e| survivor_set.contains(&e.id))
        .collect();

    if json {
        let fired_json: Vec<serde_json::Value> = fired
            .iter()
            .map(|e| {
                serde_json::json!({
                    "id": e.id,
                    "title": e.title,
                    "triggers_matched": matched_triggers
                        .get(e.id.as_str())
                        .cloned()
                        .unwrap_or_default(),
                })
            })
            .collect();
        println!(
            "{}",
            serde_json::to_string(&serde_json::json!({
                "fired": fired_json,
                "deferred_count": deferred_count,
            }))?
        );
    } else {
        // `context` format: title + body per fired memory, separated by `---`.
        // EMPTY stdout when nothing fires (caller injects nothing).
        let mut out = String::new();
        for (i, e) in fired.iter().enumerate() {
            if i > 0 {
                out.push_str("\n---\n\n");
            }
            out.push_str("# ");
            out.push_str(&e.title);
            out.push('\n');
            if let Some(body) = &e.body {
                out.push('\n');
                out.push_str(body);
                out.push('\n');
            }
        }
        if !out.is_empty() {
            let mut stdout = std::io::stdout();
            stdout.write_all(out.as_bytes())?;
            stdout.flush()?;
        }
    }

    Ok(())
}

/// Parse a `--exclude-tags` CSV value into a list of prefix strings.
///
/// Segments are trimmed; empty segments (from trailing commas, repeated commas,
/// or whitespace-only input) are dropped. A `None` input yields an empty list.
fn parse_exclude_prefixes(exclude_tags: Option<&str>) -> Vec<String> {
    exclude_tags
        .unwrap_or("")
        .split(',')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(String::from)
        .collect()
}

/// Whether a wake-fetch entry should be KEPT given the active exclude prefixes.
///
/// Returns `false` (exclude) when ANY of the entry's tags prefix-matches ANY of
/// the requested exclude prefixes. An empty prefix list keeps every entry.
fn keep_after_exclude(tags: &[String], exclude_prefixes: &[String]) -> bool {
    if exclude_prefixes.is_empty() {
        return true;
    }
    !tags.iter().any(|tag| {
        exclude_prefixes
            .iter()
            .any(|prefix| tag.starts_with(prefix.as_str()))
    })
}

/// Resolve the display `fact_type` label for a wake-fetch entry.
///
/// Older entries carry `fact_type` inside their `summary` JSON; newer entries
/// leave `summary` null. When the label is absent the entry's `category_id` is a
/// valid fallback, so a missing label never drops the entry from the wake set.
fn resolve_fact_type(summary: Option<&str>, category_id: &str) -> String {
    summary
        .and_then(|s| serde_json::from_str::<serde_json::Value>(s).ok())
        .and_then(|v| {
            v.get("fact_type")
                .and_then(|x| x.as_str())
                .map(String::from)
        })
        .unwrap_or_else(|| category_id.to_string())
}

/// Build the read-after-write verification context for a just-written entry.
///
/// The post-write read-back must use a context whose visibility matches the row
/// that was actually stored, otherwise a *successful* write reads back as absent
/// and is falsely reported as rejected. The visibility filter only admits a
/// private row when `owner = $current_agent`, so for a private entry the context
/// must carry the entry's stored `owner` — NOT the acting agent, which may differ
/// when `--owner` targets someone else. A public entry is visible to any context,
/// so the acting agent is fine there.
fn write_verification_ctx(
    visibility: &str,
    entry_owner: Option<&str>,
    acting_agent: &str,
) -> store::AgentContext {
    match entry_owner {
        Some(owner) if visibility == "private" => store::AgentContext::for_agent(owner),
        _ => store::AgentContext::for_agent(acting_agent),
    }
}

/// Resolved arguments for a single standard-mode `memory add` write.
///
/// Both the `Add` CLI arm and `AddBatch` JSONL path construct this struct from
/// their respective sources (flag values vs JSON fields) and call `add_one` as
/// the single shared write path. This ensures both callers are identical in
/// what they write — fact-type routing, edge creation, embedding, and anchoring
/// all happen in one place so neither path can silently drift.
struct AddOneArgs {
    agent_id: String,
    category: String,
    title: String,
    body: String,
    tag_list: Vec<String>,
    applicability_list: Vec<String>,
    anchor_list: Vec<String>,
    trigger_list: Vec<String>,
    wake_phrase_list: Vec<String>,
    wake_phrase: Option<String>,
    wake_order: Option<i32>,
    entry_visibility: String,
    entry_owner: Option<String>,
    session_id: Option<String>,
    ephemeral: bool,
    source_type: String,
    entry_type: String,
    content_type: String,
    domain: Option<String>,
    resonance: i32,
    resonance_type: Option<String>,
    project: Option<String>,
}

/// Core standard-mode write path shared by `Add` and `AddBatch`.
///
/// Inserts one entry, verifies the write, wires the EXTRACTED_FROM edge, and
/// (when not suppressed) runs embed and auto-anchor. Returns the inserted
/// `KnowledgeEntry` so the caller can format output without duplicating field
/// reads.
///
/// `embed`           — when `true` calls `auto_embed`; pass
///                     `write_embed_enabled(no_embed)` from the caller.
/// `no_auto_anchor`  — passed through to `write_anchor_enabled`; mirrors the
///                     same flag on the single-add path.
fn add_one(
    args: AddOneArgs,
    db: &dyn store::KnowledgeStore,
    embed: bool,
    no_auto_anchor: bool,
) -> Result<knowledge::KnowledgeEntry> {
    let path_hint = args.domain.unwrap_or_else(|| args.category.clone());
    let id = knowledge::KnowledgeEntry::generate_id(&path_hint, &args.title);
    let now = chrono::Utc::now().to_rfc3339();

    let entry = knowledge::KnowledgeEntry {
        id: id.clone(),
        category_id: args.category.clone(),
        title: args.title.clone(),
        body: Some(args.body.clone()),
        summary: None,
        applicability: args.applicability_list.clone(),
        source_project_id: args.project,
        source_agent_id: Some(args.agent_id.clone()),
        file_path: None,
        tags: args.tag_list,
        created_at: Some(now.clone()),
        updated_at: Some(now),
        content_hash: Some(knowledge::KnowledgeEntry::compute_hash(&args.title)),
        source_type_id: Some(args.source_type),
        entry_type_id: Some(args.entry_type),
        session_id: args.session_id.clone(),
        ephemeral: args.ephemeral,
        content_type_id: Some(args.content_type),
        owner: args.entry_owner.clone(),
        visibility: args.entry_visibility.clone(),
        resonance: args.resonance,
        resonance_type: args.resonance_type,
        last_activated: None,
        activation_count: 0,
        decay_rate: 0.0,
        anchors: args.anchor_list,
        wake_phrases: args.wake_phrase_list,
        triggers: args.trigger_list,
        wake_order: args.wake_order,
        wake_phrase: args.wake_phrase,
        embedding: None,
        embedding_model: None,
        embedded_at: None,
        chunk_count: 0,
        format: "markdown".to_string(),
        effective_resonance: None,
    };

    // Insert into database.
    db.upsert_knowledge(&entry)?;

    // Verify the write landed. SurrealDB's PERMISSIONS clause silently rejects
    // unauthorized writes (zero rows affected, no error). A read-after-write
    // catches that and turns silent data-loss into a loud failure.
    {
        let ctx = write_verification_ctx(
            &args.entry_visibility,
            args.entry_owner.as_deref(),
            &args.agent_id,
        );
        if db.get(&id, &ctx)?.is_none() {
            bail!(
                "write rejected: entry '{}' was not persisted (likely a permission denial — \
                 check that the writing agent owns the entry or has permission to create it)",
                id
            );
        }
    }

    // Wire EXTRACTED_FROM edge when session_id is provided.
    if let Some(ref sess_id) = args.session_id {
        let session_ref = normalize_id(sess_id);
        let ctx = store::AgentContext::public_only();
        if db.get(&session_ref, &ctx)?.is_none() {
            eprintln!(
                "Warning: Session {} not found - EXTRACTED_FROM edge not created",
                session_ref
            );
        } else {
            db.add_relationship(&id, &session_ref, "extracted_from")?;
        }
    }

    // Auto-generate embedding. Gated by the caller-resolved `embed` flag
    // (which reflects both --no-embed and MX_SKIP_WRITE_EMBED).
    if embed {
        auto_embed(&id, db)?;
    } else {
        println!("  (embed skipped)");
    }

    // Auto-generate anchors. Gated by --no-auto-anchor / MX_SKIP_WRITE_ANCHOR.
    if write_anchor_enabled(no_auto_anchor) {
        auto_anchor(&id, db, None)?;
    } else {
        println!("  (auto-anchor skipped)");
    }

    Ok(entry)
}

pub(crate) fn handle_memory(cmd: MemoryCommands, verbose: bool) -> Result<()> {
    let config = IndexConfig::default();

    match cmd {
        MemoryCommands::Rebuild => {
            // TODO(legacy-state-cleanup): remove this stub after one release cycle.
            bail!(
                "`mx memory rebuild` was removed -- the export-then-edit-then-rebuild \
                 flow has no users on this codebase. See the markdown-ingest follow-up \
                 issue for plans, and the `mx doctor memory rebuild` follow-up for \
                 export-wipe-reimport recovery."
            );
        }

        MemoryCommands::Seed { command } => match command {
            MemorySeedCommands::Agents { path } => {
                let db = store::create_store_with_verbose(&config.db_path, verbose)?;
                super::metadata::seed_agents(db.as_ref(), path)?;
            }
            MemorySeedCommands::Knowledge { path } => {
                let db = store::create_store_with_verbose(&config.db_path, verbose)?;
                super::metadata::seed_knowledge(db.as_ref(), path)?;
            }
        },

        MemoryCommands::Search {
            query,
            filter,
            semantic,
            activate,
        } => {
            let db = store::create_store_with_verbose(&config.db_path, verbose)?;
            let ctx = resolve_agent_context(filter.mine, filter.include_private);

            // Note: Search doesn't activate facts by default - discovery != engagement
            // Use --activate to explicitly mark results as intentionally consumed.
            // Build filter for database query (resonance and category)
            let db_filter = store::KnowledgeFilter {
                min_resonance: filter.min_resonance,
                max_resonance: filter.max_resonance,
                categories: filter.category.clone(),
            };

            // Get results from database with resonance filtering
            let entries = if semantic {
                use crate::embeddings::{EmbeddingProvider, TractProvider};

                eprintln!("Initializing semantic search...");
                let provider = TractProvider::new()?;
                let query_embedding = provider.embed(&query)?;

                // When --tags is present the in-memory filter will thin the DB results,
                // so we over-fetch to ensure enough candidates survive the tag filter.
                // Tradeoff: 5x multiplier works well at typical limits (10-50) but does
                // not scale for very large limits. The cap (limit + 200) prevents runaway
                // fetches when the caller requests hundreds of entries.
                let requested_limit = filter.limit.unwrap_or(20);
                let db_limit = if filter.tags.is_some() {
                    (requested_limit * 5).min(requested_limit + 200)
                } else {
                    requested_limit
                };

                db.semantic_search(&query_embedding, &ctx, &db_filter, db_limit)?
            } else {
                db.search(&query, &ctx, &db_filter)?
            };

            // Apply in-memory field presence filters
            let entries = apply_entry_filters(entries, &filter);

            if filter.json {
                println!("{}", serde_json::to_string_pretty(&entries)?);
            } else if entries.is_empty() {
                println!("No results for '{}'", query);
            } else {
                println!("Found {} results:\n", entries.len());
                for entry in &entries {
                    print_entry_summary(entry);
                }
            }

            // --activate: activate returned results (mark as intentionally consumed)
            if activate && !entries.is_empty() {
                let ids: Vec<String> = entries.iter().map(|e| e.id.clone()).collect();
                if let Err(e) = db.update_activations(&ids) {
                    eprintln!("Warning: failed to activate results: {}", e);
                }
                eprintln!("Activated {} result(s)", ids.len());
            }
        }

        MemoryCommands::List { filter } => {
            let db = store::create_store_with_verbose(&config.db_path, verbose)?;
            let ctx = resolve_agent_context(filter.mine, filter.include_private);

            // Validate categories if provided
            if let Some(ref cats) = filter.category {
                for cat in cats {
                    if db.get_category(cat)?.is_none() {
                        let categories = db.list_categories()?;
                        let valid_ids: Vec<&str> =
                            categories.iter().map(|c| c.id.as_str()).collect();
                        bail!(
                            "Unknown category '{}'. Valid categories: {}",
                            cat,
                            valid_ids.join(", ")
                        );
                    }
                }
            }

            // Build filter for database query (resonance only - category handled below)
            let db_filter = store::KnowledgeFilter {
                min_resonance: filter.min_resonance,
                max_resonance: filter.max_resonance,
                categories: None,
            };

            // Get results from database with resonance filtering
            let entries = if let Some(ref cats) = filter.category {
                let mut all = Vec::new();
                for cat in cats {
                    all.extend(db.list_by_category(cat, &ctx, &db_filter)?);
                }
                all
            } else {
                // List all categories from database
                let mut all = Vec::new();
                let categories = db.list_categories()?;
                for cat in categories {
                    all.extend(db.list_by_category(&cat.id, &ctx, &db_filter)?);
                }
                all
            };

            // Apply in-memory field presence filters
            let entries = apply_entry_filters(entries, &filter);

            if filter.json {
                println!("{}", serde_json::to_string_pretty(&entries)?);
            } else if entries.is_empty() {
                println!("No entries found");
            } else {
                println!("Found {} entries:\n", entries.len());
                for entry in entries {
                    print_entry_summary(&entry);
                }
            }
        }

        MemoryCommands::Show {
            id,
            json,
            content_only,
        } => {
            let db = store::create_store_with_verbose(&config.db_path, verbose)?;
            let id = normalize_id(&id);

            // For Show, we need to respect privacy but use current agent context
            // If the user has MX_CURRENT_AGENT set, they can see their own private entries
            let ctx = match std::env::var("MX_CURRENT_AGENT") {
                Ok(agent) if !agent.is_empty() => store::AgentContext::for_agent(agent),
                _ => store::AgentContext::public_only(),
            };

            match db.get(&id, &ctx)? {
                Some(entry) => {
                    // Activate fact when viewing details
                    if entry.id.starts_with("kn-")
                        && let Err(e) = db.update_activations(std::slice::from_ref(&entry.id))
                    {
                        eprintln!("Warning: failed to update activation: {}", e);
                    }

                    // Render the chosen view to a String first, then route it
                    // through emit_show_output so any view that would blow past
                    // the Bash stdout ceiling gets diverted to a temp file.
                    if content_only {
                        if let Some(body) = &entry.body {
                            // Preserve historical behavior: no trailing newline.
                            emit_show_output(body, &entry.id, false)?;
                        }
                    } else if json {
                        let rendered = serde_json::to_string_pretty(&entry)?;
                        emit_show_output(&rendered, &entry.id, true)?;
                    } else {
                        let rendered = format_entry_full(&entry);
                        // format_entry_full already ends in a newline.
                        emit_show_output(&rendered, &entry.id, false)?;
                    }
                }
                None => {
                    bail!("Entry '{}' not found", id);
                }
            }
        }

        MemoryCommands::TriggerCheck {
            message,
            json,
            format,
            dry_run,
        } => {
            handle_trigger_check(&config, verbose, message, json, format, dry_run)?;
        }

        MemoryCommands::TriggerReset { json } => {
            let store = crate::triggers::FiredStore::open();
            store.reset()?;
            if json {
                println!(
                    "{}",
                    serde_json::to_string(&serde_json::json!({
                        "reset": true,
                        "path": crate::triggers::fired_path().display().to_string(),
                    }))?
                );
            } else {
                eprintln!(
                    "[mx] trigger fired-state cleared: {}",
                    crate::triggers::fired_path().display()
                );
            }
        }

        MemoryCommands::Stats { json } => {
            let db = store::create_store_with_verbose(&config.db_path, verbose)?;

            // For stats, show counts for current agent's perspective
            let ctx = match std::env::var("MX_CURRENT_AGENT") {
                Ok(agent) if !agent.is_empty() => store::AgentContext::for_agent(agent),
                _ => store::AgentContext::public_only(),
            };

            let total = db.count()?;
            let categories = db.list_categories()?;
            let filter = store::KnowledgeFilter::default();

            if json {
                let mut cat_counts = serde_json::Map::new();
                for cat in categories {
                    let count = db.count_by_category(&cat.id, &ctx, &filter)?;
                    cat_counts.insert(cat.id, serde_json::Value::Number(count.into()));
                }
                println!(
                    "{}",
                    serde_json::to_string_pretty(&serde_json::json!({
                        "total": total,
                        "categories": cat_counts,
                    }))?
                );
            } else {
                println!("Memory Index Statistics\n");
                println!("Total entries: {}", total);
                println!();
                for cat in categories {
                    let count = db.count_by_category(&cat.id, &ctx, &filter)?;
                    println!("  {:12} {}", cat.id, count);
                }
            }
        }

        MemoryCommands::Health { json } => {
            let db = open_surreal(&config, verbose)?;
            let health = db.graph_health()?;

            if json {
                println!("{}", serde_json::to_string_pretty(&health)?);
            } else {
                let total = health["total"].as_i64().unwrap_or(0);
                let embedded_pct = health["embedded_pct"].as_i64().unwrap_or(0);
                let anchored_pct = health["anchored_pct"].as_i64().unwrap_or(0);
                let stale_pct = health["stale_high_res_pct"].as_i64().unwrap_or(0);
                println!("Graph Health\n");
                println!("  Total entries: {}", total);
                println!("  {:3}% embedded", embedded_pct);
                println!("  {:3}% anchored", anchored_pct);
                println!("  {:3}% stale (high-res, >30d)", stale_pct);
            }
        }

        MemoryCommands::Growth { json } => {
            let db = open_surreal(&config, verbose)?;
            let counts = db.growth_sparkline()?;

            if json {
                println!("{}", serde_json::to_string_pretty(&counts)?);
            } else {
                // Human-readable: label + bar
                println!("Growth (last 8 weeks)");
                if let Some(arr) = counts.as_array() {
                    for (i, v) in arr.iter().enumerate() {
                        println!("  week -{}: {}", 7 - i, v.as_i64().unwrap_or(0));
                    }
                }
            }
        }

        MemoryCommands::OpenThreads { json } => {
            let db = open_surreal(&config, verbose)?;
            let threads = db.open_threads()?;

            if json {
                println!("{}", serde_json::to_string_pretty(&threads)?);
            } else {
                let arr = threads.as_array().map(|v| v.as_slice()).unwrap_or(&[]);
                if arr.is_empty() {
                    println!("No open threads.");
                } else {
                    println!("Open threads ({})\n", arr.len());
                    for t in arr {
                        let id = t["id"].as_str().unwrap_or("");
                        let resonance = t["resonance"].as_i64().unwrap_or(0);
                        let created_at = t["created_at"].as_str().unwrap_or("");
                        let body = t["body"]
                            .as_str()
                            .unwrap_or("")
                            .chars()
                            .take(80)
                            .collect::<String>();
                        println!("  [r{}] {} {}  {}", resonance, id, created_at, body);
                    }
                }
            }
        }

        MemoryCommands::Delete { id, json } => {
            let db = store::create_store_with_verbose(&config.db_path, verbose)?;
            let id = normalize_id(&id);

            // Respect visibility: agents can only delete entries they can see
            let current_agent = std::env::var("MX_CURRENT_AGENT")
                .ok()
                .filter(|s| !s.is_empty());
            let ctx = match &current_agent {
                Some(agent) => store::AgentContext::for_agent(agent),
                None => store::AgentContext::public_only(),
            };

            // Backup before delete (Issue #206)
            if let Some(entry) = db.get(&id, &ctx)? {
                let _ = db
                    .backup_content(&entry, "delete", current_agent.as_deref())
                    .map_err(|e| eprintln!("Warning: failed to create backup: {}", e));
            }

            if db.delete(&id, &ctx)? {
                if json {
                    println!(
                        "{}",
                        serde_json::to_string_pretty(&serde_json::json!({
                            "deleted": true,
                            "id": id,
                        }))?
                    );
                } else {
                    println!("Deleted entry '{}'", id);
                }
            } else {
                bail!("Entry '{}' not found", id);
            }
        }

        MemoryCommands::Import { path } => {
            // TODO(legacy-state-cleanup): remove stub after one release cycle.
            let _ = path;
            bail!(
                "`mx memory import` was renamed to `mx memory seed knowledge`. \
                 The default seed location moved from `$MX_HOME/memory/index.jsonl` \
                 to `$MX_HOME/memory/seed/knowledge/*.jsonl` (the legacy file is \
                 still read with a warning for one release)."
            );
        }

        MemoryCommands::Add {
            category,
            title,
            content,
            file,
            tags,
            applicability,
            project,
            source_agent,
            source_type,
            entry_type,
            session_id,
            ephemeral,
            domain,
            content_type,
            private,
            visibility,
            owner,
            json,
            resonance,
            resonance_type,
            wake_phrase,
            wake_phrases,
            wake_order,
            triggers,
            anchors,
            r#type,
            session,
            thread_id,
            no_auto_anchor,
            no_embed,
        } => {
            use anyhow::Context;
            use std::fs;

            let db = store::create_store_with_verbose(&config.db_path, verbose)?;

            // Get content from either --content or --file
            let body = if let Some(text) = content {
                text
            } else if let Some(file_path) = file {
                fs::read_to_string(&file_path)
                    .with_context(|| format!("Failed to read file: {}", file_path))?
            } else {
                bail!("Either --content or --file must be provided");
            };

            // Determine agent - use source_agent or env var (no longer required)
            let agent_id = match source_agent {
                Some(ref sa) if !sa.is_empty() => sa.clone(),
                _ => match std::env::var("MX_CURRENT_AGENT") {
                    Ok(agent) if !agent.is_empty() => agent,
                    _ => {
                        bail!("--source-agent not provided and MX_CURRENT_AGENT not set");
                    }
                },
            };

            // Resolve visibility: --private flag is sugar for --visibility private
            let is_private = private || visibility.as_deref() == Some("private");
            if let Some(ref vis) = visibility
                && vis != "public"
                && vis != "private"
            {
                bail!("--visibility must be 'public' or 'private'");
            }

            // Parse triggers CSV (Issue #246): normalize + dedupe via the shared
            // helper so author-time values match the PR3 matcher. Used by both the
            // fact-routing path and the standard entry construction below.
            let trigger_list: Vec<String> = triggers
                .as_deref()
                .map(|t| knowledge::normalize_triggers(t.split(',')))
                .unwrap_or_default();

            // Handle fact type routing mode (--type flag)
            if let Some(ref fact_type) = r#type {
                // Handle thread_closed specially - updates existing thread
                if fact_type == "thread_closed" {
                    let tid = if let Some(id) = thread_id {
                        id
                    } else {
                        // Find by content match (fragile fallback)
                        find_open_thread_by_content(&*db, &body, &agent_id)?
                    };

                    // Update existing thread to closed state
                    if let Some(thread_entry) =
                        db.get(&tid, &store::AgentContext::for_agent(&agent_id))?
                    {
                        let mut meta: serde_json::Value = thread_entry
                            .summary
                            .as_deref()
                            .map(|s| {
                                serde_json::from_str(s).unwrap_or_else(|_| serde_json::json!({}))
                            })
                            .unwrap_or_else(|| serde_json::json!({}));
                        if let Some(obj) = meta.as_object_mut() {
                            obj.insert(
                                "state".to_string(),
                                serde_json::Value::String("closed".to_string()),
                            );
                        }
                        let new_summary = meta.to_string();
                        let outcome = db
                            .update(tid.as_str())
                            .summary(new_summary)
                            .execute(&store::AgentContext::for_agent(&agent_id))?;
                        if outcome.applied {
                            println!("Closed thread: {}", tid);
                        } else {
                            bail!("Entry '{}' not found", tid);
                        }
                        return Ok(());
                    } else {
                        bail!("Thread not found: {}", tid);
                    }
                }

                // Route fact type to category and tags
                let routing = route_fact_type(fact_type)?;

                // Build fact entry
                let now = chrono::Utc::now().to_rfc3339();
                let truncated_title = safe_truncate(&body, 60);
                let fact_title = format!("{}: {}", fact_type, truncated_title);

                // Generate ID using session if provided
                let session_hint = session.as_deref().unwrap_or("fact");
                let id = knowledge::KnowledgeEntry::generate_id(session_hint, &fact_title);

                // Build metadata JSON
                let mut metadata = serde_json::Map::new();
                metadata.insert(
                    "fact_type".to_string(),
                    serde_json::Value::String(fact_type.clone()),
                );
                metadata.insert(
                    "agent".to_string(),
                    serde_json::Value::String(agent_id.clone()),
                );
                metadata.insert(
                    "date".to_string(),
                    serde_json::Value::String(chrono::Local::now().format("%Y-%m-%d").to_string()),
                );

                // Add state field for threads
                if routing.category == "thread" {
                    metadata.insert(
                        "state".to_string(),
                        serde_json::Value::String("open".to_string()),
                    );
                }

                let summary_json = serde_json::Value::Object(metadata).to_string();

                // Merge routed tags with any user-provided tags
                let mut tag_list: Vec<String> =
                    routing.tags.iter().map(|s| s.to_string()).collect();
                if let Some(t) = tags {
                    tag_list.extend(
                        t.split(',')
                            .map(|s| s.trim().to_string())
                            .filter(|s| !s.is_empty()),
                    );
                }

                // Build the knowledge entry
                let entry = knowledge::KnowledgeEntry {
                    id: id.clone(),
                    category_id: routing.category.to_string(),
                    title: fact_title.clone(),
                    body: Some(body.clone()),
                    summary: Some(summary_json),
                    applicability: vec![],
                    source_project_id: project,
                    source_agent_id: Some(format!("agent:{}", agent_id)),
                    file_path: None,
                    tags: tag_list.clone(),
                    created_at: Some(now.clone()),
                    updated_at: Some(now),
                    content_hash: Some(knowledge::KnowledgeEntry::compute_hash(&body)),
                    source_type_id: Some("source_type:agent_session".to_string()),
                    entry_type_id: Some("entry_type:primary".to_string()),
                    session_id: session.clone(),
                    ephemeral: true,
                    content_type_id: Some("content_type:text".to_string()),
                    owner: Some(format!("agent:{}", agent_id)),
                    visibility: "public".to_string(),
                    resonance: resonance.unwrap_or(3),
                    resonance_type: Some("ephemeral".to_string()),
                    last_activated: None,
                    activation_count: 0,
                    decay_rate: 0.0,
                    anchors: vec![],
                    wake_phrases: vec![],
                    // Issue #246: triggers from the --triggers CLI flag (PR2).
                    triggers: trigger_list.clone(),
                    wake_order: None,
                    wake_phrase: None,
                    embedding: None,
                    embedding_model: None,
                    embedded_at: None,
                    chunk_count: 0,
                    format: "markdown".to_string(),
                    effective_resonance: None,
                };

                // Insert the fact
                db.upsert_knowledge(&entry)?;

                // Verify the write landed. SurrealDB's PERMISSIONS clause silently
                // rejects unauthorized writes (zero rows affected, no error). A
                // read-after-write catches that and turns silent data-loss into a
                // loud failure.
                {
                    let ctx = store::AgentContext::for_agent(&agent_id);
                    if db.get(&id, &ctx)?.is_none() {
                        bail!(
                            "write rejected: fact '{}' was not persisted (likely a permission denial — check that the writing agent owns the entry or has permission to create it)",
                            id
                        );
                    }
                }

                // Create EXTRACTED_FROM relationship to session if provided
                if let Some(ref sess) = session {
                    let session_ref = if sess.starts_with("kn-") {
                        sess.clone()
                    } else {
                        format!("kn-{}", sess)
                    };

                    let ctx = crate::store::AgentContext::public_only();
                    if db.get(&session_ref, &ctx)?.is_none() {
                        eprintln!(
                            "Warning: Session {} not found - relationship not created",
                            session_ref
                        );
                    } else {
                        db.add_relationship(&id, &session_ref, "extracted_from")?;
                    }
                }

                println!("Added fact: {}", id);
                println!("  Type: {}", fact_type);
                println!("  Category: {}", routing.category);
                println!("  Content: {}", body);

                // Auto-generate embedding if in network SurrealDB mode.
                // Gated by --no-embed or MX_SKIP_WRITE_EMBED (see
                // write_embed_enabled). The entry is already durable here, so
                // skipping embedding is safe; the explicit `mx memory embed --all`
                // command is never gated and still embeds deferred entries.
                if write_embed_enabled(no_embed) {
                    auto_embed(&id, db.as_ref())?;
                } else {
                    println!("  (embed skipped)");
                }

                return Ok(());
            }

            // Standard memory add mode (no --type flag)
            let category = category.expect("category required when --type not provided");
            let title = title.expect("title required when --type not provided");

            // Validate category against database
            if db.get_category(&category)?.is_none() {
                let categories = db.list_categories()?;
                let valid_ids: Vec<&str> = categories.iter().map(|c| c.id.as_str()).collect();
                bail!(
                    "Invalid category '{}'. Valid categories: {}",
                    category,
                    valid_ids.join(", ")
                );
            }

            // Parse tags
            let tag_list: Vec<String> = tags
                .map(|t| {
                    t.split(',')
                        .map(|s| s.trim().to_string())
                        .filter(|s| !s.is_empty())
                        .collect()
                })
                .unwrap_or_default();

            // Parse applicability CSV
            let applicability_list: Vec<String> = applicability
                .map(|a| {
                    a.split(',')
                        .map(|s| s.trim().to_string())
                        .filter(|s| !s.is_empty())
                        .collect()
                })
                .unwrap_or_default();

            // Parse anchors CSV
            let anchor_list: Vec<String> = anchors
                .map(|a| {
                    a.split(',')
                        .map(|s| s.trim().to_string())
                        .filter(|s| !s.is_empty())
                        .collect()
                })
                .unwrap_or_default();

            // Parse wake_phrases CSV or use single wake_phrase
            let wake_phrase_list: Vec<String> = if let Some(phrases) = wake_phrases {
                phrases
                    .split(',')
                    .map(|s| s.trim().to_string())
                    .filter(|s| !s.is_empty())
                    .collect()
            } else if let Some(ref single_phrase) = wake_phrase {
                vec![single_phrase.clone()]
            } else {
                vec![]
            };

            // Determine visibility and owner
            // FIX #123: Ensure owner matches the format expected by visibility filter.
            // The visibility filter compares `owner = $current_agent` where $current_agent
            // comes from MX_CURRENT_AGENT. Owner must be stored in the same format.
            let entry_visibility = if is_private {
                "private".to_string()
            } else {
                "public".to_string()
            };

            let entry_owner = if is_private {
                // Owner defaults to agent_id (already resolved from --source-agent or MX_CURRENT_AGENT)
                Some(owner.unwrap_or_else(|| agent_id.clone()))
            } else {
                owner
            };

            // Validate resonance_type if provided
            if let Some(ref rtype) = resonance_type {
                let valid_types = [
                    "foundational",
                    "transformative",
                    "relational",
                    "operational",
                    "ephemeral",
                    "session",
                ];
                if !valid_types.contains(&rtype.as_str()) {
                    bail!(
                        "Invalid resonance type '{}'. Valid types: {}",
                        rtype,
                        valid_types.join(", ")
                    );
                }
            }

            // Delegate to the shared single-entry write path. Both `Add` and
            // `AddBatch` route through `add_one` so the write contract (insert,
            // verify, edge, embed, anchor) stays in one place.
            let entry = add_one(
                AddOneArgs {
                    agent_id: agent_id.clone(),
                    category: category.clone(),
                    title: title.clone(),
                    body,
                    tag_list,
                    applicability_list,
                    anchor_list,
                    trigger_list,
                    wake_phrase_list,
                    wake_phrase,
                    wake_order,
                    entry_visibility: entry_visibility.clone(),
                    entry_owner: entry_owner.clone(),
                    session_id,
                    ephemeral,
                    source_type,
                    entry_type,
                    content_type,
                    domain,
                    resonance: resonance.unwrap_or(0),
                    resonance_type,
                    project,
                },
                db.as_ref(),
                write_embed_enabled(no_embed),
                no_auto_anchor,
            )?;

            let id = entry.id.clone();

            if json {
                println!(
                    "{}",
                    serde_json::to_string_pretty(&serde_json::json!({
                        "id": id,
                        "category": category,
                        "title": title,
                        "visibility": entry_visibility,
                        "owner": entry_owner,
                        "resonance": entry.resonance,
                        "resonance_type": entry.resonance_type,
                        "tags": entry.tags,
                        "applicability": entry.applicability,
                        "anchors": entry.anchors,
                        "wake_phrase": entry.wake_phrase,
                        "wake_phrases": entry.wake_phrases,
                        "triggers": entry.triggers,
                    }))?
                );
            } else {
                println!("Added entry: {}", id);
                println!("  Category: {}", category);
                println!("  Title: {}", title);
                println!("  Visibility: {}", entry_visibility);
                if let Some(ref o) = entry_owner {
                    println!("  Owner: {}", o);
                }
                if entry.resonance > 0 {
                    println!("  Resonance: {}", entry.resonance);
                }
                if let Some(ref rtype) = entry.resonance_type {
                    println!("  Resonance Type: {}", rtype);
                }
                if !entry.tags.is_empty() {
                    println!("  Tags: {}", entry.tags.join(", "));
                }
                if !entry.applicability.is_empty() {
                    println!("  Applicability: {}", entry.applicability.join(", "));
                }
                if !entry.anchors.is_empty() {
                    println!("  Anchors: {}", entry.anchors.join(", "));
                }
                if let Some(ref phrase) = entry.wake_phrase {
                    println!("  Wake Phrase: {}", phrase);
                }
                if !entry.triggers.is_empty() {
                    println!("  Triggers: {}", entry.triggers.join(", "));
                }
            }
        }

        MemoryCommands::Update {
            id,
            title,
            content,
            file,
            append_content,
            append_file,
            prepend_content,
            prepend_file,
            find,
            replace,
            replace_all,
            nth,
            category,
            tags,
            add_tag,
            remove_tag,
            applicability,
            content_type,
            resonance,
            resonance_type,
            anchors,
            add_anchor,
            remove_anchor,
            wake_phrase,
            wake_phrases,
            add_wake_phrase,
            remove_wake_phrase,
            triggers,
            add_trigger,
            remove_trigger,
            wake_order,
            private,
            visibility,
            owner,
            session_id,
            force,
            no_auto_anchor,
            no_embed,
            json,
        } => {
            use anyhow::Context;
            use std::fs;

            let db = store::create_store_with_verbose(&config.db_path, verbose)?;
            let id = normalize_id(&id);

            // For Update, use current agent context to allow updating own private entries
            // #10: read MX_CURRENT_AGENT once, reuse for both ctx and backup
            let current_agent = std::env::var("MX_CURRENT_AGENT")
                .ok()
                .filter(|s| !s.is_empty());
            let ctx = match &current_agent {
                Some(agent) => store::AgentContext::for_agent(agent),
                None => store::AgentContext::public_only(),
            };

            // Fetch existing entry
            let mut entry = db
                .get(&id, &ctx)?
                .ok_or_else(|| anyhow::anyhow!("Entry not found: {}", id))?;

            // Resolve --private as sugar for --visibility private
            let visibility = if private && visibility.is_none() {
                Some("private".to_string())
            } else {
                visibility
            };

            let mut changes = Vec::new();

            // Backup before body mutation (Issue #206)
            let will_change_body = content.is_some()
                || file.is_some()
                || append_content.is_some()
                || append_file.is_some()
                || prepend_content.is_some()
                || prepend_file.is_some()
                || find.is_some();

            if will_change_body {
                let _ = db
                    .backup_content(&entry, "update", current_agent.as_deref())
                    .map_err(|e| eprintln!("Warning: failed to create backup: {}", e));
            }

            // Update title if provided
            if let Some(new_title) = title {
                changes.push(format!("title: {} -> {}", entry.title, new_title));
                entry.title = new_title;
            }

            // Track if body was changed for hash update
            let mut body_changed = false;

            // Update content - supports multiple modes:
            // 1. Full replacement via --content or --file
            // 2. Append via --append-content or --append-file
            // 3. Prepend via --prepend-content or --prepend-file
            // 4. Find/replace via --find/--replace
            if let Some(text) = content {
                changes.push("content: updated (inline)".to_string());
                entry.body = Some(text);
                body_changed = true;
            } else if let Some(file_path) = file {
                let text = fs::read_to_string(&file_path)
                    .with_context(|| format!("Failed to read file: {}", file_path))?;
                changes.push(format!("content: updated from {}", file_path));
                entry.body = Some(text);
                body_changed = true;
            } else if let Some(ref append_text) = append_content {
                let new_body = content_ops::append_content(entry.body.as_deref(), append_text);
                changes.push(format!("content: appended {} bytes", append_text.len()));
                entry.body = Some(new_body);
                body_changed = true;
            } else if let Some(ref file_path) = append_file {
                let append_text = fs::read_to_string(file_path)
                    .with_context(|| format!("Failed to read file: {}", file_path))?;
                let new_body = content_ops::append_content(entry.body.as_deref(), &append_text);
                changes.push(format!(
                    "content: appended {} bytes from {}",
                    append_text.len(),
                    file_path
                ));
                entry.body = Some(new_body);
                body_changed = true;
            } else if let Some(ref prepend_text) = prepend_content {
                let new_body = content_ops::prepend_content(entry.body.as_deref(), prepend_text);
                changes.push(format!("content: prepended {} bytes", prepend_text.len()));
                entry.body = Some(new_body);
                body_changed = true;
            } else if let Some(ref file_path) = prepend_file {
                let prepend_text = fs::read_to_string(file_path)
                    .with_context(|| format!("Failed to read file: {}", file_path))?;
                let new_body = content_ops::prepend_content(entry.body.as_deref(), &prepend_text);
                changes.push(format!(
                    "content: prepended {} bytes from {}",
                    prepend_text.len(),
                    file_path
                ));
                entry.body = Some(new_body);
                body_changed = true;
            } else if let Some(ref find_text) = find {
                let replace_text = replace.as_deref().unwrap_or("");
                let body_text = entry
                    .body
                    .as_deref()
                    .ok_or_else(|| anyhow::anyhow!("Entry has no body content to edit"))?;
                let result = content_ops::edit_content(
                    body_text,
                    find_text,
                    replace_text,
                    replace_all,
                    nth,
                )?;
                changes.push(format!(
                    "content: {} replacement{}",
                    result.replacements,
                    if result.replacements == 1 { "" } else { "s" }
                ));
                entry.body = Some(result.new_content);
                body_changed = true;
            }

            // Update category if provided
            if let Some(new_category) = category {
                // Validate category
                if db.get_category(&new_category)?.is_none() {
                    let categories = db.list_categories()?;
                    let valid_ids: Vec<&str> = categories.iter().map(|c| c.id.as_str()).collect();
                    bail!(
                        "Invalid category '{}'. Valid categories: {}",
                        new_category,
                        valid_ids.join(", ")
                    );
                }
                changes.push(format!(
                    "category: {} -> {}",
                    entry.category_id, new_category
                ));
                entry.category_id = new_category;
            }

            // Update resonance if provided
            if let Some(new_resonance) = resonance {
                changes.push(format!(
                    "resonance: {} -> {}",
                    entry.resonance, new_resonance
                ));
                entry.resonance = new_resonance;
            }

            // Update resonance type if provided
            if let Some(ref new_type) = resonance_type {
                let valid_types = [
                    "foundational",
                    "transformative",
                    "relational",
                    "operational",
                    "ephemeral",
                    "session",
                ];
                if !valid_types.contains(&new_type.as_str()) {
                    bail!(
                        "Invalid resonance type '{}'. Valid types: {}",
                        new_type,
                        valid_types.join(", ")
                    );
                }
                changes.push(format!(
                    "resonance_type: {:?} -> {}",
                    entry.resonance_type, new_type
                ));
                entry.resonance_type = Some(new_type.clone());
            }

            // Update anchors if provided (replace all)
            // Track explicitly removed anchors so auto_anchor won't re-add them
            let mut explicitly_removed_anchors: Vec<String> = Vec::new();
            if let Some(ref new_anchors) = anchors {
                let anchor_list: Vec<String> = new_anchors
                    .split(',')
                    .map(|s| normalize_id(s.trim()))
                    .filter(|s| !s.is_empty())
                    .collect();
                // Anchors in old set but not in new set were explicitly removed
                for old_anchor in &entry.anchors {
                    if !anchor_list.contains(old_anchor) {
                        explicitly_removed_anchors.push(old_anchor.clone());
                    }
                }
                changes.push(format!("anchors: {:?} -> {:?}", entry.anchors, anchor_list));
                entry.anchors = anchor_list;
            }

            // Add a single anchor
            if let Some(ref new_anchor) = add_anchor {
                let normalized = normalize_id(new_anchor);
                if !entry.anchors.contains(&normalized) {
                    entry.anchors.push(normalized.clone());
                    changes.push(format!("anchors: added '{}'", normalized));
                }
            }

            // Remove a specific anchor
            if let Some(ref anchor_to_remove) = remove_anchor {
                let normalized = normalize_id(anchor_to_remove);
                if let Some(pos) = entry.anchors.iter().position(|a| *a == normalized) {
                    entry.anchors.remove(pos);
                    changes.push(format!("anchors: removed '{}'", normalized));
                }
            }

            // Update wake phrase if provided
            if let Some(ref new_phrase) = wake_phrase {
                changes.push(format!(
                    "wake_phrase: {:?} -> {}",
                    entry.wake_phrase, new_phrase
                ));
                entry.wake_phrase = Some(new_phrase.clone());
            }

            // Update wake_phrases (replaces all)
            if let Some(ref phrases_str) = wake_phrases {
                let phrase_list: Vec<String> = phrases_str
                    .split(',')
                    .map(|s| s.trim().to_string())
                    .filter(|s| !s.is_empty())
                    .collect();
                changes.push(format!(
                    "wake_phrases: {:?} -> {:?}",
                    entry.wake_phrases, phrase_list
                ));
                entry.wake_phrases = phrase_list;
            }

            // Add a single wake phrase
            if let Some(ref new_phrase) = add_wake_phrase
                && !entry.wake_phrases.contains(new_phrase)
            {
                entry.wake_phrases.push(new_phrase.clone());
                changes.push(format!("wake_phrases: added '{}'", new_phrase));
            }

            // Remove a specific wake phrase
            if let Some(ref phrase_to_remove) = remove_wake_phrase
                && let Some(pos) = entry
                    .wake_phrases
                    .iter()
                    .position(|p| p == phrase_to_remove)
            {
                entry.wake_phrases.remove(pos);
                changes.push(format!("wake_phrases: removed '{}'", phrase_to_remove));
            }

            // Update triggers (replaces all; Issue #246). Normalize + dedupe via the
            // shared helper so values line up with the PR3 matcher.
            if let Some(ref triggers_str) = triggers {
                let trigger_list = knowledge::normalize_triggers(triggers_str.split(','));
                changes.push(format!(
                    "triggers: {:?} -> {:?}",
                    entry.triggers, trigger_list
                ));
                entry.triggers = trigger_list;
            }

            // Add a single trigger (normalized; no-op if already present)
            if let Some(ref raw_trigger) = add_trigger
                && let Some(norm) = knowledge::normalize_trigger(raw_trigger)
                && !entry.triggers.contains(&norm)
            {
                entry.triggers.push(norm.clone());
                changes.push(format!("triggers: added '{}'", norm));
            }

            // Remove a specific trigger (compare normalized forms; clean no-op if absent)
            if let Some(ref raw_trigger) = remove_trigger
                && let Some(norm) = knowledge::normalize_trigger(raw_trigger)
                && let Some(pos) = entry.triggers.iter().position(|t| *t == norm)
            {
                entry.triggers.remove(pos);
                changes.push(format!("triggers: removed '{}'", norm));
            }

            // Update wake_order (use '-' to clear)
            if let Some(ref order_str) = wake_order {
                if order_str == "-" {
                    changes.push("wake_order: cleared".to_string());
                    entry.wake_order = None;
                } else if let Ok(order_value) = order_str.parse::<i32>() {
                    changes.push(format!(
                        "wake_order: {:?} -> {}",
                        entry.wake_order, order_value
                    ));
                    entry.wake_order = Some(order_value);
                } else {
                    bail!(
                        "Invalid wake_order value '{}' (use number or '-' to clear)",
                        order_str
                    );
                }
            }

            // Update visibility if provided
            if let Some(ref new_vis) = visibility {
                // Validate value
                if new_vis != "public" && new_vis != "private" {
                    bail!("--visibility must be 'public' or 'private'");
                }

                let old_vis = entry.visibility.clone();

                // Bloom protection: warn when making blooms public
                if new_vis == "public" && entry.category_id == "bloom" && !force {
                    bail!(
                        "Making bloom '{}' public will expose identity data. Use --force to confirm.",
                        entry.id
                    );
                }

                // Handle public -> private: require owner
                if new_vis == "private" && old_vis == "public" {
                    let new_owner = owner.clone().or_else(|| {
                        std::env::var("MX_CURRENT_AGENT")
                            .ok()
                            .filter(|s| !s.is_empty())
                    });

                    if new_owner.is_none() {
                        bail!(
                            "Cannot make entry private without an owner. Provide --owner or set MX_CURRENT_AGENT."
                        );
                    }

                    entry.owner = new_owner;
                }

                // Handle private -> public: clear owner
                if new_vis == "public" && old_vis == "private" {
                    entry.owner = None;
                }

                changes.push(format!("visibility: {} -> {}", old_vis, new_vis));
                entry.visibility = new_vis.clone();
            }

            // Update owner if provided (only for private entries)
            if let Some(ref new_owner) = owner {
                // Only allow owner update if entry is or will be private
                let is_private =
                    visibility.as_deref() == Some("private") || entry.visibility == "private";

                if !is_private {
                    bail!(
                        "Cannot set owner on public entry. Use --visibility private to make entry private first."
                    );
                }

                changes.push(format!("owner: {:?} -> {}", entry.owner, new_owner));
                entry.owner = Some(new_owner.clone());
            }

            // Update session_id if provided
            if let Some(ref new_session_id) = session_id {
                let normalized = normalize_id(new_session_id);
                changes.push(format!(
                    "session_id: {:?} -> {}",
                    entry.session_id, normalized
                ));
                entry.session_id = Some(normalized.clone());

                // Create EXTRACTED_FROM edge, mirroring the add path logic.
                // The for-session query traverses the relates_to edge, so we
                // need both the field AND the edge for consistency.
                let session_ref = normalized;
                let edge_ctx = crate::store::AgentContext::public_only();
                if db.get(&session_ref, &edge_ctx)?.is_none() {
                    eprintln!(
                        "Warning: Session {} not found - EXTRACTED_FROM edge not created",
                        session_ref
                    );
                } else {
                    db.add_relationship(&id, &session_ref, "extracted_from")?;
                }
            }

            // Update timestamp
            entry.updated_at = Some(chrono::Utc::now().to_rfc3339());

            // Update content hash if body was changed
            if body_changed && let Some(body) = entry.body.as_ref() {
                entry.content_hash = Some(knowledge::KnowledgeEntry::compute_hash(body));
            }

            // Update tags if provided - set on entry BEFORE upsert
            if let Some(tags_str) = tags {
                let tag_list: Vec<String> = tags_str
                    .split(',')
                    .map(|s| s.trim().to_string())
                    .filter(|s| !s.is_empty())
                    .collect();
                changes.push(format!("tags: {}", tag_list.join(", ")));
                entry.tags = tag_list;
            }

            // Add a single tag
            if let Some(ref new_tag) = add_tag {
                let tag = new_tag.trim().to_string();
                if !tag.is_empty() && !entry.tags.contains(&tag) {
                    entry.tags.push(tag.clone());
                    changes.push(format!("tags: added '{}'", tag));
                }
            }

            // Remove a specific tag
            if let Some(ref tag_to_remove) = remove_tag {
                let tag = tag_to_remove.trim().to_string();
                if let Some(pos) = entry.tags.iter().position(|t| *t == tag) {
                    entry.tags.remove(pos);
                    changes.push(format!("tags: removed '{}'", tag));
                }
            }

            // Upsert entry (now includes updated tags)
            db.upsert_knowledge(&entry)?;

            // Update applicability if provided
            if let Some(applicability_str) = applicability {
                let applicability_list: Vec<String> = applicability_str
                    .split(',')
                    .map(|s| s.trim().to_string())
                    .filter(|s| !s.is_empty())
                    .collect();
                changes.push(format!("applicability: {}", applicability_list.join(", ")));
                entry.applicability = applicability_list;
                db.upsert_knowledge(&entry)?;
            }

            // Update content type if provided
            if let Some(new_content_type) = content_type {
                changes.push(format!(
                    "content_type: {} -> {}",
                    entry.content_type_id.as_deref().unwrap_or("none"),
                    new_content_type
                ));
                entry.content_type_id = Some(new_content_type);
                // Re-upsert to update content_type_id
                db.upsert_knowledge(&entry)?;
            }

            // Auto-generate embedding if in network SurrealDB mode.
            // Gated by --no-embed or MX_SKIP_WRITE_EMBED (see write_embed_enabled).
            if write_embed_enabled(no_embed) {
                auto_embed(&id, db.as_ref())?;
            } else {
                println!("  (embed skipped)");
            }

            // Auto-generate anchors if in network SurrealDB mode
            // Pass explicitly removed anchors so auto_anchor respects user intent:
            // if the user did --anchors (full replacement) and removed some anchors,
            // auto_anchor should not re-add them.
            let removed = if explicitly_removed_anchors.is_empty() {
                None
            } else {
                Some(explicitly_removed_anchors.as_slice())
            };
            // Gated by --no-auto-anchor or MX_SKIP_WRITE_ANCHOR (see
            // write_anchor_enabled). The entry is already durable here, so
            // skipping anchoring is safe; the explicit `mx memory auto-anchor`
            // command is never gated and still anchors deferred writes.
            if write_anchor_enabled(no_auto_anchor) {
                auto_anchor(&id, db.as_ref(), removed)?;
            } else {
                println!("  (auto-anchor skipped)");
            }

            if json {
                println!(
                    "{}",
                    serde_json::to_string_pretty(&serde_json::json!({
                        "id": id,
                        "changes": changes,
                    }))?
                );
            } else {
                println!("Updated entry: {}", id);
                if changes.is_empty() {
                    println!("  No changes specified");
                } else {
                    for change in &changes {
                        println!("  {}", change);
                    }
                }
            }
        }

        MemoryCommands::Edit {
            id,
            find,
            replace,
            replace_all,
            nth,
            no_auto_anchor,
            no_embed,
            json,
        } => {
            let db = store::create_store_with_verbose(&config.db_path, verbose)?;
            let id = normalize_id(&id);

            // Use current agent context for private entry access
            let current_agent = std::env::var("MX_CURRENT_AGENT")
                .ok()
                .filter(|s| !s.is_empty());
            let ctx = match &current_agent {
                Some(agent) => store::AgentContext::for_agent(agent),
                None => store::AgentContext::public_only(),
            };

            // Backup before edit (Issue #206)
            if let Some(entry) = db.get(&id, &ctx)? {
                let _ = db
                    .backup_content(&entry, "edit", current_agent.as_deref())
                    .map_err(|e| eprintln!("Warning: failed to create backup: {}", e));
            }

            let result = db.edit_content(&id, &ctx, &find, &replace, replace_all, nth)?;

            // Auto-generate embedding if in network SurrealDB mode.
            // Gated by --no-embed or MX_SKIP_WRITE_EMBED (see write_embed_enabled).
            if write_embed_enabled(no_embed) {
                auto_embed(&id, db.as_ref())?;
            } else {
                println!("  (embed skipped)");
            }

            // Auto-generate anchors if in network SurrealDB mode.
            // Gated by --no-auto-anchor or MX_SKIP_WRITE_ANCHOR (see
            // write_anchor_enabled). The entry is already durable here, so
            // skipping anchoring is safe; the explicit `mx memory auto-anchor`
            // command is never gated and still anchors deferred writes.
            if write_anchor_enabled(no_auto_anchor) {
                auto_anchor(&id, db.as_ref(), None)?;
            } else {
                println!("  (auto-anchor skipped)");
            }

            if json {
                println!(
                    "{}",
                    serde_json::to_string_pretty(&serde_json::json!({
                        "id": id,
                        "replacements": result.replacements,
                    }))?
                );
            } else {
                println!("Edited entry: {}", id);
                println!(
                    "  {} replacement{}",
                    result.replacements,
                    if result.replacements == 1 { "" } else { "s" }
                );
            }
        }

        MemoryCommands::Append {
            id,
            content,
            file,
            no_auto_anchor,
            no_embed,
            json,
        } => {
            use std::io::{self, Read};

            let db = store::create_store_with_verbose(&config.db_path, verbose)?;
            let id = normalize_id(&id);

            // Use current agent context for private entry access
            let current_agent = std::env::var("MX_CURRENT_AGENT")
                .ok()
                .filter(|s| !s.is_empty());
            let ctx = match &current_agent {
                Some(agent) => store::AgentContext::for_agent(agent),
                None => store::AgentContext::public_only(),
            };

            // Get content from argument, file, or stdin
            let text = if let Some(c) = content {
                c
            } else if let Some(file_path) = file {
                std::fs::read_to_string(&file_path)
                    .with_context(|| format!("Failed to read file: {}", file_path))?
            } else {
                let mut buffer = String::new();
                io::stdin()
                    .read_to_string(&mut buffer)
                    .context("Failed to read from stdin")?;
                buffer.trim_end().to_string()
            };

            if text.is_empty() {
                bail!("No content provided");
            }

            // Backup before append (Issue #206)
            if let Some(entry) = db.get(&id, &ctx)? {
                let _ = db
                    .backup_content(&entry, "append", current_agent.as_deref())
                    .map_err(|e| eprintln!("Warning: failed to create backup: {}", e));
            }

            db.append_content(&id, &ctx, &text)?;

            // Auto-generate embedding if in network SurrealDB mode.
            // Gated by --no-embed or MX_SKIP_WRITE_EMBED (see write_embed_enabled).
            if write_embed_enabled(no_embed) {
                auto_embed(&id, db.as_ref())?;
            } else {
                println!("  (embed skipped)");
            }

            // Auto-generate anchors if in network SurrealDB mode.
            // Gated by --no-auto-anchor or MX_SKIP_WRITE_ANCHOR (see
            // write_anchor_enabled). The entry is already durable here, so
            // skipping anchoring is safe; the explicit `mx memory auto-anchor`
            // command is never gated and still anchors deferred writes.
            if write_anchor_enabled(no_auto_anchor) {
                auto_anchor(&id, db.as_ref(), None)?;
            } else {
                println!("  (auto-anchor skipped)");
            }

            if json {
                println!(
                    "{}",
                    serde_json::to_string_pretty(&serde_json::json!({
                        "id": id,
                        "bytes_added": text.len(),
                    }))?
                );
            } else {
                println!("Appended to entry: {}", id);
                println!("  {} bytes added", text.len());
            }
        }

        MemoryCommands::Prepend {
            id,
            content,
            file,
            no_auto_anchor,
            no_embed,
            json,
        } => {
            use std::io::{self, Read};

            let db = store::create_store_with_verbose(&config.db_path, verbose)?;
            let id = normalize_id(&id);

            // Use current agent context for private entry access
            let current_agent = std::env::var("MX_CURRENT_AGENT")
                .ok()
                .filter(|s| !s.is_empty());
            let ctx = match &current_agent {
                Some(agent) => store::AgentContext::for_agent(agent),
                None => store::AgentContext::public_only(),
            };

            // Get content from argument, file, or stdin
            let text = if let Some(c) = content {
                c
            } else if let Some(file_path) = file {
                std::fs::read_to_string(&file_path)
                    .with_context(|| format!("Failed to read file: {}", file_path))?
            } else {
                let mut buffer = String::new();
                io::stdin()
                    .read_to_string(&mut buffer)
                    .context("Failed to read from stdin")?;
                buffer.trim_end().to_string()
            };

            if text.is_empty() {
                bail!("No content provided");
            }

            // Backup before prepend (Issue #206)
            if let Some(entry) = db.get(&id, &ctx)? {
                let _ = db
                    .backup_content(&entry, "prepend", current_agent.as_deref())
                    .map_err(|e| eprintln!("Warning: failed to create backup: {}", e));
            }

            db.prepend_content(&id, &ctx, &text)?;

            // Auto-generate embedding if in network SurrealDB mode.
            // Gated by --no-embed or MX_SKIP_WRITE_EMBED (see write_embed_enabled).
            if write_embed_enabled(no_embed) {
                auto_embed(&id, db.as_ref())?;
            } else {
                println!("  (embed skipped)");
            }

            // Auto-generate anchors if in network SurrealDB mode.
            // Gated by --no-auto-anchor or MX_SKIP_WRITE_ANCHOR (see
            // write_anchor_enabled). The entry is already durable here, so
            // skipping anchoring is safe; the explicit `mx memory auto-anchor`
            // command is never gated and still anchors deferred writes.
            if write_anchor_enabled(no_auto_anchor) {
                auto_anchor(&id, db.as_ref(), None)?;
            } else {
                println!("  (auto-anchor skipped)");
            }

            if json {
                println!(
                    "{}",
                    serde_json::to_string_pretty(&serde_json::json!({
                        "id": id,
                        "bytes_added": text.len(),
                    }))?
                );
            } else {
                println!("Prepended to entry: {}", id);
                println!("  {} bytes added", text.len());
            }
        }

        MemoryCommands::Restore {
            id,
            list,
            no_auto_anchor,
            no_embed,
            json,
        } => {
            let db = store::create_store_with_verbose(&config.db_path, verbose)?;
            let id = normalize_id(&id);

            // Shared agent context (#10: read MX_CURRENT_AGENT once)
            let current_agent = std::env::var("MX_CURRENT_AGENT")
                .ok()
                .filter(|s| !s.is_empty());
            let ctx = match &current_agent {
                Some(agent) => store::AgentContext::for_agent(agent),
                None => store::AgentContext::public_only(),
            };

            if list {
                // List available backups
                // #7: filter by visibility — only show backups for entries the agent can see
                if db.get(&id, &ctx)?.is_none() {
                    if json {
                        println!("{}", serde_json::to_string_pretty(&serde_json::json!([]))?);
                    } else {
                        println!("No entry or backups found for {}", id);
                    }
                } else {
                    let backups = db.list_backups(&id)?;
                    if json {
                        println!("{}", serde_json::to_string_pretty(&backups)?);
                    } else if backups.is_empty() {
                        println!("No backups found for {}", id);
                    } else {
                        println!("Backups for {}:", id);
                        for b in &backups {
                            let body_len = b.body.as_ref().map(|s| s.len()).unwrap_or(0);
                            println!(
                                "  {} | {} | {} | {} bytes",
                                b.id,
                                b.created_at.as_deref().unwrap_or("unknown"),
                                b.operation,
                                body_len,
                            );
                        }
                    }
                }
            } else {
                let backup = db
                    .latest_backup(&id)?
                    .ok_or_else(|| anyhow::anyhow!("No backups found for {}", id))?;

                // #5: single fetch, #6: better error for deleted entries
                let mut entry = match db.get(&id, &ctx)? {
                    Some(entry) => {
                        // Backup current state before restoring
                        if let Err(e) =
                            db.backup_content(&entry, "update", current_agent.as_deref())
                        {
                            eprintln!(
                                "Warning: failed to backup current state before restore: {}",
                                e
                            );
                        }
                        entry
                    }
                    None => {
                        bail!(
                            "Entry '{}' not found (may have been deleted). \
                             Restore from backup after deletion is not yet supported.",
                            id
                        );
                    }
                };

                // Restore body from backup
                entry.body = backup.body.clone();

                // #4: set updated_at
                entry.updated_at = Some(chrono::Utc::now().to_rfc3339());

                // Recompute content hash
                let hash_body = entry.body.as_deref().unwrap_or("").to_string();
                entry.content_hash = Some(knowledge::KnowledgeEntry::compute_hash(&hash_body));

                db.upsert_knowledge(&entry)?;

                // #3: update embeddings and anchors like all other mutation paths.
                // Embedding gated by --no-embed or MX_SKIP_WRITE_EMBED (see
                // write_embed_enabled). The entry is already durable here.
                if write_embed_enabled(no_embed) {
                    auto_embed(&id, db.as_ref())?;
                } else {
                    println!("  (embed skipped)");
                }
                // Gated by --no-auto-anchor or MX_SKIP_WRITE_ANCHOR (see
                // write_anchor_enabled). The entry is already durable here, so
                // skipping anchoring is safe; the explicit `mx memory
                // auto-anchor` command is never gated and still anchors
                // deferred writes.
                if write_anchor_enabled(no_auto_anchor) {
                    auto_anchor(&id, db.as_ref(), None)?;
                } else {
                    println!("  (auto-anchor skipped)");
                }

                if json {
                    println!(
                        "{}",
                        serde_json::to_string_pretty(&serde_json::json!({
                            "restored": true,
                            "id": id,
                            "from_backup": backup.id,
                            "backup_created": backup.created_at,
                            "operation": backup.operation,
                        }))?
                    );
                } else {
                    println!("Restored entry: {}", id);
                    println!("  from backup: {}", backup.id);
                    println!(
                        "  backup created: {}",
                        backup.created_at.as_deref().unwrap_or("unknown")
                    );
                    println!("  original operation: {}", backup.operation);
                }
            }
        }

        MemoryCommands::AddBatch { file, no_embed } => {
            use std::io::BufRead;

            // Read JSONL lines from --file or stdin.
            let lines: Vec<String> = if let Some(ref path) = file {
                let f = std::fs::File::open(path)
                    .with_context(|| format!("Failed to open batch file: {}", path))?;
                std::io::BufReader::new(f)
                    .lines()
                    .collect::<std::result::Result<_, _>>()
                    .context("Failed to read batch file")?
            } else {
                let stdin = std::io::stdin();
                stdin
                    .lock()
                    .lines()
                    .collect::<std::result::Result<_, _>>()
                    .context("Failed to read from stdin")?
            };

            // Open store ONCE for the whole batch.
            let db = store::create_store_with_verbose(&config.db_path, verbose)?;

            let mut added_ids: Vec<String> = Vec::new();
            let mut entry_errors: Vec<(usize, String)> = Vec::new();

            for (line_idx, raw) in lines.iter().enumerate() {
                let raw = raw.trim();
                if raw.is_empty() || raw.starts_with('#') {
                    continue;
                }

                // Parse the JSON line.
                let v: serde_json::Value = match serde_json::from_str(raw) {
                    Ok(val) => val,
                    Err(e) => {
                        entry_errors.push((line_idx + 1, format!("JSON parse error: {}", e)));
                        continue;
                    }
                };

                // Extract fields matching the Add command signature.
                // Each JSONL line is a self-describing Add payload.
                let str_field = |key: &str| -> Option<String> {
                    v.get(key).and_then(|x| x.as_str()).map(|s| s.to_string())
                };
                let bool_field = |key: &str| -> bool {
                    v.get(key).and_then(|x| x.as_bool()).unwrap_or(false)
                };
                let int_field = |key: &str| -> Option<i32> {
                    v.get(key).and_then(|x| x.as_i64()).map(|n| n as i32)
                };

                // Resolve agent: source_agent field or MX_CURRENT_AGENT env.
                let agent_id = match str_field("source_agent").filter(|s| !s.is_empty()) {
                    Some(a) => a,
                    None => match std::env::var("MX_CURRENT_AGENT") {
                        Ok(a) if !a.is_empty() => a,
                        _ => {
                            entry_errors.push((
                                line_idx + 1,
                                "source_agent not provided and MX_CURRENT_AGENT not set"
                                    .to_string(),
                            ));
                            continue;
                        }
                    },
                };

                // Determine the fact-type path vs standard path.
                if let Some(fact_type) = str_field("type") {
                    // Fact-type routing path.
                    let body = match str_field("content") {
                        Some(b) if !b.is_empty() => b,
                        _ => {
                            entry_errors.push((
                                line_idx + 1,
                                "fact entries require a 'content' field".to_string(),
                            ));
                            continue;
                        }
                    };

                    let routing = match route_fact_type(&fact_type) {
                        Ok(r) => r,
                        Err(e) => {
                            entry_errors
                                .push((line_idx + 1, format!("invalid fact type: {}", e)));
                            continue;
                        }
                    };

                    let session = str_field("session");
                    let session_hint = session.as_deref().unwrap_or("fact");
                    let truncated_title = crate::display::safe_truncate(&body, 60);
                    let fact_title = format!("{}: {}", fact_type, truncated_title);
                    let id = knowledge::KnowledgeEntry::generate_id(session_hint, &fact_title);

                    let now = chrono::Utc::now().to_rfc3339();
                    let trigger_list: Vec<String> = str_field("triggers")
                        .map(|t| knowledge::normalize_triggers(t.split(',')))
                        .unwrap_or_default();
                    let mut tag_list: Vec<String> = routing.tags.iter().map(|s| s.to_string()).collect();
                    if let Some(t) = str_field("tags") {
                        tag_list.extend(
                            t.split(',')
                                .map(|s| s.trim().to_string())
                                .filter(|s| !s.is_empty()),
                        );
                    }

                    let mut metadata = serde_json::Map::new();
                    metadata.insert(
                        "fact_type".to_string(),
                        serde_json::Value::String(fact_type.clone()),
                    );
                    metadata.insert(
                        "agent".to_string(),
                        serde_json::Value::String(agent_id.clone()),
                    );
                    metadata.insert(
                        "date".to_string(),
                        serde_json::Value::String(chrono::Local::now().format("%Y-%m-%d").to_string()),
                    );
                    if routing.category == "thread" {
                        metadata.insert(
                            "state".to_string(),
                            serde_json::Value::String("open".to_string()),
                        );
                    }
                    let summary_json = serde_json::Value::Object(metadata).to_string();

                    let entry = knowledge::KnowledgeEntry {
                        id: id.clone(),
                        category_id: routing.category.to_string(),
                        title: fact_title,
                        body: Some(body.clone()),
                        summary: Some(summary_json),
                        applicability: vec![],
                        source_project_id: str_field("project"),
                        source_agent_id: Some(format!("agent:{}", agent_id)),
                        file_path: None,
                        tags: tag_list,
                        created_at: Some(now.clone()),
                        updated_at: Some(now),
                        content_hash: Some(knowledge::KnowledgeEntry::compute_hash(&body)),
                        source_type_id: Some("source_type:agent_session".to_string()),
                        entry_type_id: Some("entry_type:primary".to_string()),
                        session_id: session.clone(),
                        ephemeral: true,
                        content_type_id: Some("content_type:text".to_string()),
                        owner: Some(format!("agent:{}", agent_id)),
                        visibility: "public".to_string(),
                        resonance: int_field("resonance").unwrap_or(3),
                        resonance_type: Some("ephemeral".to_string()),
                        last_activated: None,
                        activation_count: 0,
                        decay_rate: 0.0,
                        anchors: vec![],
                        wake_phrases: vec![],
                        triggers: trigger_list,
                        wake_order: None,
                        wake_phrase: None,
                        embedding: None,
                        embedding_model: None,
                        embedded_at: None,
                        chunk_count: 0,
                        format: "markdown".to_string(),
                        effective_resonance: None,
                    };

                    match db.upsert_knowledge(&entry) {
                        Ok(_) => {}
                        Err(e) => {
                            entry_errors.push((line_idx + 1, format!("db write error: {}", e)));
                            continue;
                        }
                    }

                    // Read-back verify (same as single-add path)
                    let ctx = store::AgentContext::for_agent(&agent_id);
                    match db.get(&id, &ctx) {
                        Ok(Some(_)) => {}
                        Ok(None) => {
                            entry_errors.push((
                                line_idx + 1,
                                format!("write rejected: fact '{}' not persisted", id),
                            ));
                            continue;
                        }
                        Err(e) => {
                            entry_errors.push((line_idx + 1, format!("read-back error: {}", e)));
                            continue;
                        }
                    }

                    // EXTRACTED_FROM edge if session provided
                    if let Some(ref sess) = session {
                        let session_ref = if sess.starts_with("kn-") {
                            sess.clone()
                        } else {
                            format!("kn-{}", sess)
                        };
                        let pub_ctx = store::AgentContext::public_only();
                        match db.get(&session_ref, &pub_ctx) {
                            Ok(None) => {
                                eprintln!(
                                    "  line {}: Warning: Session {} not found - relationship not created",
                                    line_idx + 1,
                                    session_ref
                                );
                            }
                            Ok(Some(_)) => {
                                if let Err(e) = db.add_relationship(&id, &session_ref, "extracted_from") {
                                    eprintln!(
                                        "  line {}: Warning: EXTRACTED_FROM edge failed: {}",
                                        line_idx + 1,
                                        e
                                    );
                                }
                            }
                            Err(e) => {
                                eprintln!(
                                    "  line {}: Warning: session lookup error for {}: {}",
                                    line_idx + 1,
                                    session_ref,
                                    e
                                );
                            }
                        }
                    }

                    println!("  [{}] Added fact: {}", line_idx + 1, id);
                    added_ids.push(id);
                } else {
                    // Standard add path — delegate to add_one(), the shared single-
                    // entry write path. This ensures batch-added standard entries get
                    // the same insert, verify, edge, embed (deferred), and anchor
                    // logic as single adds, with no drift possible between the paths.
                    let category = match str_field("category") {
                        Some(c) if !c.is_empty() => c,
                        _ => {
                            entry_errors.push((
                                line_idx + 1,
                                "missing 'category' field (required when 'type' not set)"
                                    .to_string(),
                            ));
                            continue;
                        }
                    };
                    let title = match str_field("title") {
                        Some(t) if !t.is_empty() => t,
                        _ => {
                            entry_errors.push((
                                line_idx + 1,
                                "missing 'title' field (required when 'type' not set)".to_string(),
                            ));
                            continue;
                        }
                    };

                    // Resolve body from 'content' or 'file'.
                    let body = if let Some(c) = str_field("content") {
                        c
                    } else if let Some(file_path) = str_field("file") {
                        match std::fs::read_to_string(&file_path) {
                            Ok(s) => s,
                            Err(e) => {
                                entry_errors.push((
                                    line_idx + 1,
                                    format!("failed to read file '{}': {}", file_path, e),
                                ));
                                continue;
                            }
                        }
                    } else {
                        entry_errors.push((
                            line_idx + 1,
                            "missing 'content' or 'file' field".to_string(),
                        ));
                        continue;
                    };

                    // Validate category (done before add_one to provide skip-and-continue).
                    match db.get_category(&category) {
                        Ok(Some(_)) => {}
                        Ok(None) => {
                            match db.list_categories() {
                                Ok(cats) => {
                                    let valid: Vec<&str> =
                                        cats.iter().map(|c| c.id.as_str()).collect();
                                    entry_errors.push((
                                        line_idx + 1,
                                        format!(
                                            "invalid category '{}'. Valid: {}",
                                            category,
                                            valid.join(", ")
                                        ),
                                    ));
                                }
                                Err(e) => {
                                    entry_errors.push((
                                        line_idx + 1,
                                        format!("category lookup error: {}", e),
                                    ));
                                }
                            }
                            continue;
                        }
                        Err(e) => {
                            entry_errors
                                .push((line_idx + 1, format!("category lookup error: {}", e)));
                            continue;
                        }
                    }

                    let is_private = bool_field("private")
                        || str_field("visibility").as_deref() == Some("private");
                    let entry_visibility = if is_private {
                        "private".to_string()
                    } else {
                        "public".to_string()
                    };
                    let entry_owner: Option<String> = if is_private {
                        Some(str_field("owner").unwrap_or_else(|| agent_id.clone()))
                    } else {
                        str_field("owner")
                    };

                    let tag_list: Vec<String> = str_field("tags")
                        .map(|t| {
                            t.split(',')
                                .map(|s| s.trim().to_string())
                                .filter(|s| !s.is_empty())
                                .collect()
                        })
                        .unwrap_or_default();
                    let applicability_list: Vec<String> = str_field("applicability")
                        .map(|a| {
                            a.split(',')
                                .map(|s| s.trim().to_string())
                                .filter(|s| !s.is_empty())
                                .collect()
                        })
                        .unwrap_or_default();
                    let anchor_list: Vec<String> = str_field("anchors")
                        .map(|a| {
                            a.split(',')
                                .map(|s| s.trim().to_string())
                                .filter(|s| !s.is_empty())
                                .collect()
                        })
                        .unwrap_or_default();
                    let trigger_list: Vec<String> = str_field("triggers")
                        .map(|t| knowledge::normalize_triggers(t.split(',')))
                        .unwrap_or_default();
                    let wake_phrase_list: Vec<String> = str_field("wake_phrases")
                        .map(|p| {
                            p.split(',')
                                .map(|s| s.trim().to_string())
                                .filter(|s| !s.is_empty())
                                .collect()
                        })
                        .unwrap_or_else(|| {
                            str_field("wake_phrase")
                                .map(|p| vec![p])
                                .unwrap_or_default()
                        });

                    // Call the shared write path. Batch passes embed=false so the
                    // hoisted embedding pass at the end of the loop handles it once.
                    // no_auto_anchor mirrors the batch-level --no-embed: the batch
                    // contract is defer-and-hoist for embedding; anchoring runs
                    // per-entry here (same as single-add) because add_one handles it.
                    let entry_result = add_one(
                        AddOneArgs {
                            agent_id: agent_id.clone(),
                            category: category.clone(),
                            title: title.clone(),
                            body,
                            tag_list,
                            applicability_list,
                            anchor_list,
                            trigger_list,
                            wake_phrase_list,
                            wake_phrase: str_field("wake_phrase"),
                            wake_order: int_field("wake_order"),
                            entry_visibility,
                            entry_owner,
                            session_id: str_field("session_id"),
                            ephemeral: bool_field("ephemeral"),
                            source_type: str_field("source_type")
                                .unwrap_or_else(|| "manual".to_string()),
                            entry_type: str_field("entry_type")
                                .unwrap_or_else(|| "primary".to_string()),
                            content_type: str_field("content_type")
                                .unwrap_or_else(|| "text".to_string()),
                            domain: str_field("domain"),
                            resonance: int_field("resonance").unwrap_or(0),
                            resonance_type: str_field("resonance_type"),
                            project: str_field("project"),
                        },
                        db.as_ref(),
                        false, // embed=false — hoisted pass below embeds all at once
                        true,  // no_auto_anchor=true — batch anchoring via nightly run
                    );

                    match entry_result {
                        Ok(entry) => {
                            println!("  [{}] Added entry: {} ({})", line_idx + 1, entry.id, title);
                            added_ids.push(entry.id);
                        }
                        Err(e) => {
                            entry_errors
                                .push((line_idx + 1, format!("write error: {}", e)));
                            continue;
                        }
                    }
                }
            }

            // Report any per-entry errors (partial success: don't abort on them).
            if !entry_errors.is_empty() {
                eprintln!("\nBatch errors ({} of {} entries failed):", entry_errors.len(), lines.len());
                for (lineno, msg) in &entry_errors {
                    eprintln!("  line {}: {}", lineno, msg);
                }
            }

            // Single hoisted embedding pass — ONE model cold-load for all entries.
            // This is the entire point of add-batch: amortize the ~435 MB TractProvider
            // load across N entries rather than paying it N times.
            if !added_ids.is_empty() && !no_embed {
                use crate::embeddings::TractProvider;
                println!("\nEmbedding {} entr{}...", added_ids.len(), if added_ids.len() == 1 { "y" } else { "ies" });
                let provider = TractProvider::new()?;
                let chunking_tokenizer = crate::embeddings::load_tokenizer()?;
                for (i, entry_id) in added_ids.iter().enumerate() {
                    println!(
                        "  Embedding {}/{}: {}",
                        i + 1,
                        added_ids.len(),
                        entry_id
                    );
                    crate::helpers::auto_embed_with(
                        entry_id,
                        db.as_ref(),
                        &provider,
                        &chunking_tokenizer,
                    )?;
                }
                println!("Embedding complete.");
            } else if no_embed && !added_ids.is_empty() {
                println!("\n(embed skipped for {} entries — run `mx memory embed --all` to embed)", added_ids.len());
            }

            // Summary
            println!(
                "\nBatch complete: {} added, {} failed.",
                added_ids.len(),
                entry_errors.len()
            );

            // Exit non-zero if any entries failed.
            if !entry_errors.is_empty() {
                std::process::exit(1);
            }
        }

        MemoryCommands::Embed { id, all, long_only } => {
            let db = store::create_store_with_verbose(&config.db_path, verbose)?;

            // Use current agent context for private entry access
            let ctx = match std::env::var("MX_CURRENT_AGENT") {
                Ok(agent) if !agent.is_empty() => store::AgentContext::for_agent(agent),
                _ => store::AgentContext::public_only(),
            };

            if all {
                let entries = db.list_all(&ctx)?;
                let total = entries.len();

                // Hoist provider construction ONCE above the loop so the whole
                // --all batch pays one ~435 MB cold-load instead of one per entry.
                // Uses auto_embed_with (provider injected) rather than auto_embed
                // (which constructs internally on every call).
                use crate::embeddings::TractProvider;
                let provider = TractProvider::new()?;
                // Load the chunking tokenizer once. When --long-only is also set
                // this tokenizer serves double duty: token-count gate + chunking.
                let chunking_tokenizer = crate::embeddings::load_tokenizer()?;

                let mut embedded = 0;
                let mut skipped = 0;

                println!("Found {} entries to embed", total);
                for entry in &entries {
                    // Check token count if --long-only is specified
                    if let Some(min_tokens) = long_only {
                        let text = entry.embedding_text();
                        let encoding = chunking_tokenizer
                            .encode(text.as_str(), false)
                            .map_err(|e| anyhow::anyhow!("Tokenization failed: {}", e))?;
                        if encoding.get_ids().len() <= min_tokens {
                            skipped += 1;
                            continue;
                        }
                    }

                    embedded += 1;
                    println!(
                        "Embedding {}/{}: {}",
                        embedded,
                        total - skipped,
                        entry.title
                    );
                    crate::helpers::auto_embed_with(
                        &entry.id,
                        db.as_ref(),
                        &provider,
                        &chunking_tokenizer,
                    )?;
                }
                if long_only.is_some() {
                    println!(
                        "Embedded {} entries ({} skipped below threshold)",
                        embedded, skipped
                    );
                } else {
                    println!("All {} entries embedded!", total);
                }
            } else {
                let entry_id = id.ok_or_else(|| {
                    anyhow::anyhow!("Entry ID required (use --all to embed all entries)")
                })?;
                let entry = db
                    .get(&entry_id, &ctx)?
                    .ok_or_else(|| anyhow::anyhow!("Entry not found: {}", entry_id))?;
                println!("Generating embedding for '{}'...", entry.title);
                crate::helpers::auto_embed(&entry.id, db.as_ref())?;
                println!("Embedding generated and saved for: {}", entry_id);
            }
        }

        MemoryCommands::AutoAnchor {
            id,
            threshold,
            max_anchors,
            dry_run,
            detailed,
            fill,
        } => {
            let db = store::create_store_with_verbose(&config.db_path, verbose)?;

            // Use current agent context for private entry access
            let ctx = match std::env::var("MX_CURRENT_AGENT") {
                Ok(agent) if !agent.is_empty() => store::AgentContext::for_agent(agent),
                _ => store::AgentContext::public_only(),
            };

            // Get entries to process
            let entries = if let Some(entry_id) = id {
                // Process single entry
                let entry = db
                    .get(&entry_id, &ctx)?
                    .ok_or_else(|| anyhow::anyhow!("Entry not found: {}", entry_id))?;

                if entry.embedding.is_none() {
                    anyhow::bail!(
                        "Entry {} has no embedding. Run `mx memory embed {}` first.",
                        entry_id,
                        entry_id
                    );
                }

                vec![entry]
            } else {
                // Get all entries with embeddings
                let all_entries = db.list_all(&ctx)?;
                all_entries
                    .into_iter()
                    .filter(|e| e.embedding.is_some())
                    .collect()
            };

            if entries.is_empty() {
                println!("No entries with embeddings found.");
                return Ok(());
            }

            if fill {
                println!("Fill mode: only processing entries with no existing anchors");
            }
            println!("Processing {} entries...", entries.len());

            // Get ALL entries with embeddings for similarity comparison
            let all_candidates = db.list_all(&ctx)?;
            let candidates: Vec<_> = all_candidates
                .into_iter()
                .filter(|e| e.embedding.is_some())
                .collect();

            let mut total_added = 0;
            let mut total_pruned = 0;
            let mut total_skipped = 0;
            let entries_count = entries.len();

            for entry in entries {
                // In fill mode, skip entries that already have anchors
                if fill && !entry.anchors.is_empty() {
                    total_skipped += 1;
                    if detailed {
                        println!(
                            "  {} \"{}\" - Skipped (has {} anchors)",
                            entry.id,
                            entry.title,
                            entry.anchors.len()
                        );
                    } else {
                        println!("Entry {} already has anchors, skipping (--fill)", entry.id);
                    }
                    continue;
                }

                let entry_embedding = entry.embedding.as_ref().unwrap();

                // Calculate similarities
                let mut similarities: Vec<(String, String, f32)> = Vec::new();
                let mut stale_anchors: Vec<String> = Vec::new();

                for candidate in &candidates {
                    // Skip self
                    if candidate.id == entry.id {
                        continue;
                    }

                    // Re-evaluate existing anchors for staleness
                    if entry.anchors.contains(&candidate.id) {
                        let candidate_embedding = candidate.embedding.as_ref().unwrap();
                        let similarity = cosine_similarity(entry_embedding, candidate_embedding);
                        if similarity < threshold || similarity > NEAR_DUPLICATE_CEILING {
                            stale_anchors.push(candidate.id.clone());
                        }
                        continue; // don't consider existing anchors as new candidates
                    }

                    // Privacy check
                    let can_anchor = if entry.visibility == "private" {
                        // Private can anchor to same-owner private OR public
                        candidate.visibility == "public"
                            || (candidate.visibility == "private" && candidate.owner == entry.owner)
                    } else {
                        // Public can only anchor to public
                        candidate.visibility == "public"
                    };

                    if !can_anchor {
                        continue;
                    }

                    // Calculate cosine similarity
                    let candidate_embedding = candidate.embedding.as_ref().unwrap();
                    let similarity = cosine_similarity(entry_embedding, candidate_embedding);

                    // Filter by threshold, skip near-duplicates
                    if similarity >= threshold && similarity <= NEAR_DUPLICATE_CEILING {
                        similarities.push((
                            candidate.id.clone(),
                            candidate.title.clone(),
                            similarity,
                        ));
                    }
                }

                // Sort by similarity (descending) and take top N
                similarities
                    .sort_by(|a, b| b.2.partial_cmp(&a.2).unwrap_or(std::cmp::Ordering::Equal));
                let top_matches: Vec<_> = similarities.into_iter().take(max_anchors).collect();

                if top_matches.is_empty() && stale_anchors.is_empty() {
                    if detailed {
                        println!(
                            "  {} \"{}\" - No similar entries found",
                            entry.id, entry.title
                        );
                    }
                    continue;
                }

                println!("Processing {} \"{}\"...", entry.id, entry.title);

                if !stale_anchors.is_empty() {
                    println!("  Pruning {} stale anchor(s)", stale_anchors.len());
                    for stale_id in &stale_anchors {
                        println!("  x {}", stale_id);
                    }
                }

                for (match_id, match_title, score) in &top_matches {
                    if detailed {
                        println!("  → {} \"{}\" ({:.2})", match_id, match_title, score);
                    } else {
                        println!("  → {} \"{}\"", match_id, match_title);
                    }
                }

                if dry_run {
                    println!(
                        "[DRY RUN] Would add {} anchors and prune {} stale anchors on {}",
                        top_matches.len(),
                        stale_anchors.len(),
                        entry.id
                    );
                } else {
                    // Update the entry with new anchors, filtering out stale ones
                    let new_anchor_ids: Vec<String> =
                        top_matches.iter().map(|(id, _, _)| id.clone()).collect();

                    let mut updated_anchors: Vec<String> = entry
                        .anchors
                        .clone()
                        .into_iter()
                        .filter(|a| !stale_anchors.contains(a))
                        .collect();
                    updated_anchors.extend(new_anchor_ids);
                    updated_anchors.sort();
                    updated_anchors.dedup();

                    // Create updated entry
                    let mut updated_entry = entry.clone();
                    updated_entry.anchors = updated_anchors;
                    updated_entry.updated_at = Some(chrono::Utc::now().to_rfc3339());

                    // Save to database
                    db.upsert_knowledge(&updated_entry)?;

                    println!(
                        "Added {} anchors, pruned {} stale",
                        top_matches.len(),
                        stale_anchors.len()
                    );
                    total_added += top_matches.len();
                    total_pruned += stale_anchors.len();
                }
            }

            if dry_run {
                println!("\n[DRY RUN] Complete. No changes written.");
            } else {
                println!(
                    "\n✓ Added {} total anchors, pruned {} stale across {} entries",
                    total_added, total_pruned, entries_count
                );
            }
            if fill && total_skipped > 0 {
                println!(
                    "  Skipped {} entries that already had anchors (--fill)",
                    total_skipped
                );
            }
        }
        MemoryCommands::Agents { command } => handle_agents(command, &config)?,

        MemoryCommands::Projects { command } => handle_projects(command, &config)?,

        MemoryCommands::Applicability { command } => handle_applicability(command, &config)?,

        MemoryCommands::Sessions { command } => handle_sessions(command, &config)?,

        MemoryCommands::Categories { command } => handle_categories(command, &config)?,

        MemoryCommands::Tags { command } => handle_tags(command, &config)?,

        MemoryCommands::SourceTypes { command } => handle_source_types(command, &config)?,

        MemoryCommands::EntryTypes { command } => handle_entry_types(command, &config)?,

        MemoryCommands::SessionTypes { command } => handle_session_types(command, &config)?,

        MemoryCommands::RelationshipTypes { command } => {
            handle_relationship_types(command, &config)?
        }

        MemoryCommands::Relationships { command } => handle_relationships(command, &config)?,

        MemoryCommands::ContentTypes { command } => handle_content_types(command, &config)?,

        MemoryCommands::Export { format, output } => {
            let db = store::create_store(&config.db_path)?;

            match format.as_str() {
                "md" | "markdown" => {
                    // Markdown exports to directory
                    let output_dir = output.as_deref().unwrap_or("./memory-export");

                    let dir_path = std::path::PathBuf::from(output_dir);
                    export_markdown(db.as_ref(), &dir_path)?;
                    println!("Exported to directory: {}", output_dir);
                }
                "jsonl" => {
                    // JSONL exports to file or stdout
                    if let Some(ref path) = output {
                        export_jsonl(db.as_ref(), &std::path::PathBuf::from(path))?;
                        println!("Exported to {}", path);
                    } else {
                        export_jsonl(db.as_ref(), &std::path::PathBuf::from("/dev/stdout"))?;
                    }
                }
                "csv" => {
                    // CSV exports to file or stdout
                    if let Some(ref path) = output {
                        export_csv(db.as_ref(), &std::path::PathBuf::from(path))?;
                        println!("Exported to {}", path);
                    } else {
                        export_csv(db.as_ref(), &std::path::PathBuf::from("/dev/stdout"))?;
                    }
                }
                _ => {
                    bail!("Invalid format '{}'. Valid formats: md, jsonl, csv", format);
                }
            }
        }

        MemoryCommands::Wake {
            limit,
            min_resonance,
            days,
            no_activate,
            begin,
            bloom_id,
            respond,
            skip,
            session,
        } => {
            let db = store::create_store(&config.db_path)?;

            // Get current agent context - required for wake
            let current_agent = match std::env::var("MX_CURRENT_AGENT") {
                Ok(agent) if !agent.is_empty() => agent,
                _ => {
                    bail!("MX_CURRENT_AGENT not set. Cannot wake without identity.");
                }
            };

            let ctx = store::AgentContext::for_agent(current_agent.clone());

            // Run cascade
            let cascade = db.wake_cascade(&ctx, limit, min_resonance, days)?;

            // Increment activation counts for wake cascade entries.
            // We do NOT reset last_activated here — wake surfacing is passive, not
            // intentional access, and resetting the decay clock would create a feedback
            // loop where frequently-surfaced entries never decay.
            if !no_activate {
                let ids = cascade.all_ids();
                if !ids.is_empty() {
                    db.increment_activation_count(&ids)?;
                }
            }

            // Output
            if begin {
                // Start session-based ritual (state stored in DB)
                let output = wake_ritual::begin_ritual(db.as_ref(), &cascade)?;
                println!("{}", output);
            } else if let Some(phrase) = respond {
                // Submit wake phrase response
                let session_token =
                    session.ok_or_else(|| anyhow::anyhow!("--session required with --respond"))?;
                let id = bloom_id
                    .ok_or_else(|| anyhow::anyhow!("--bloom-id required with --respond"))?;

                let output =
                    wake_ritual::respond_ritual(db.as_ref(), &ctx, &id, &phrase, &session_token)?;
                println!("{}", output);
            } else if skip {
                // Skip a bloom
                let session_token =
                    session.ok_or_else(|| anyhow::anyhow!("--session required with --skip"))?;
                let id =
                    bloom_id.ok_or_else(|| anyhow::anyhow!("--bloom-id required with --skip"))?;

                let output = wake_ritual::skip_ritual(db.as_ref(), &ctx, &id, &session_token)?;
                println!("{}", output);
            } else {
                print_wake_cascade(&cascade);
            }
        }

        MemoryCommands::Recent {
            days,
            json,
            format,
            resonance_type,
            all_types,
            sort,
            limit,
        } => {
            let db = store::create_store_with_verbose(&config.db_path, verbose)?;

            // Note: Listing doesn't activate facts - bulk view != focused access
            // Auto-enable all_types when --resonance-type is set, otherwise the
            // default ephemeral-only query would silently return nothing for
            // non-ephemeral types (e.g. `--resonance-type foundational`).
            let all_types = all_types || resonance_type.is_some();

            // Decide which query to use:
            //   --all-types (or --resonance-type) => query all resonance types
            //   (default)                         => ephemeral only (backwards compatible)
            // --resonance-type filter is applied post-query in both cases.
            let mut facts = if all_types {
                db.query_recent_facts_all_types(days)?
            } else {
                db.query_recent_facts(days)?
            };

            // Filter by resonance_type if provided (works with both code paths)
            if let Some(ref rtype) = resonance_type {
                facts.retain(|f| f.resonance_type.as_deref() == Some(rtype.as_str()));
            }

            // Apply sort: "resonance" sorts by effective_resonance (decay-adjusted) highest-first.
            // DB already returns entries ORDER BY effective_resonance DESC; the default path
            // preserves that ordering rather than re-sorting, so a resonance-9 from 6 months
            // ago does not outrank a resonance-7 from yesterday.
            if matches!(sort, RecentSortOrder::Resonance) {
                facts.sort_by(|a, b| {
                    // Sort by effective_resonance (decay-adjusted) when available;
                    // fall back to raw resonance for entries that lack it.
                    let a_val = a.effective_resonance.unwrap_or(a.resonance as f64);
                    let b_val = b.effective_resonance.unwrap_or(b.resonance as f64);
                    b_val
                        .partial_cmp(&a_val)
                        .unwrap_or(std::cmp::Ordering::Equal)
                });
            }
            // Default: preserve DB ordering (effective_resonance DESC). No re-sort needed.

            // Apply limit
            facts.truncate(limit);

            // Support both --json flag and legacy --format json
            if json || format == "json" {
                let json_facts: Vec<serde_json::Value> = facts
                    .iter()
                    .map(|f| {
                        let fact_type = f
                            .summary
                            .as_ref()
                            .and_then(|s| serde_json::from_str::<serde_json::Value>(s).ok())
                            .and_then(|v: serde_json::Value| {
                                v.get("fact_type")
                                    .and_then(|t| t.as_str())
                                    .map(String::from)
                            });

                        serde_json::json!({
                            "id": f.id,
                            "type": fact_type,
                            "content": f.body.as_ref().unwrap_or(&"".to_string()),
                            "created_at": f.created_at.as_ref().unwrap_or(&"".to_string()),
                            "resonance": f.resonance,
                        })
                    })
                    .collect();
                println!("{}", serde_json::to_string_pretty(&json_facts)?);
            } else {
                for fact in facts {
                    let summary_json = fact
                        .summary
                        .as_ref()
                        .and_then(|s| serde_json::from_str::<serde_json::Value>(s).ok());

                    let fact_type = summary_json
                        .as_ref()
                        .and_then(|v: &serde_json::Value| {
                            v.get("fact_type")
                                .and_then(|t| t.as_str())
                                .map(String::from)
                        })
                        .unwrap_or_else(|| "unknown".to_string());

                    let state = fact.get_summary_state();

                    let date = fact
                        .created_at
                        .as_ref()
                        .and_then(|dt_str: &String| {
                            chrono::DateTime::parse_from_rfc3339(dt_str).ok()
                        })
                        .map(|dt| dt.format("%Y-%m-%d").to_string())
                        .unwrap_or_else(|| "unknown".to_string());

                    let content = fact.body.as_deref().unwrap_or("");
                    let preview = safe_truncate(content, 60);

                    if let Some(state) = state {
                        println!(
                            "[{}] {} ({}): {} ({}, resonance {})",
                            date, fact_type, state, preview, fact.id, fact.resonance
                        );
                    } else {
                        println!(
                            "[{}] {}: {} ({}, resonance {})",
                            date, fact_type, preview, fact.id, fact.resonance
                        );
                    }
                }
            }
        }

        MemoryCommands::WakeFetch {
            days,
            limit,
            exclude_tags,
        } => {
            if days <= 0 {
                bail!("--days must be a positive integer (got {days})");
            }

            // Parse --exclude-tags into a list of prefix strings.
            // Empty string segments (from trailing commas) are silently dropped.
            let exclude_prefixes = parse_exclude_prefixes(exclude_tags.as_deref());

            let db = store::create_store_with_verbose(&config.db_path, verbose)?;

            let mut facts = db.query_recent_facts_all_types(days)?;

            // Filter to resonance >= 3 AND extract fact_type in a single pass.
            // Collect (entry, fact_type) pairs so we don't re-parse summary JSON later.
            // Also apply tag-prefix exclusion: drop any entry whose tags include a value
            // that starts with any of the requested exclude prefixes.
            let mut typed_facts: Vec<(crate::knowledge::KnowledgeEntry, String)> = facts
                .drain(..)
                .filter(|f| f.resonance >= 3)
                .filter(|f| keep_after_exclude(&f.tags, &exclude_prefixes))
                .map(|f| {
                    let ft = resolve_fact_type(f.summary.as_deref(), &f.category_id);
                    (f, ft)
                })
                .collect();

            // Sort by effective resonance (decay-adjusted), highest first
            typed_facts.sort_by(|(a, _), (b, _)| {
                let a_val = a.effective_resonance.unwrap_or(a.resonance as f64);
                let b_val = b.effective_resonance.unwrap_or(b.resonance as f64);
                b_val
                    .partial_cmp(&a_val)
                    .unwrap_or(std::cmp::Ordering::Equal)
            });

            // Apply limit
            typed_facts.truncate(limit);

            if typed_facts.is_empty() {
                println!("(no memory entries returned)");
                return Ok(());
            }

            println!("<facts>");
            for (i, (fact, fact_type)) in typed_facts.iter().enumerate() {
                if i > 0 {
                    println!();
                }

                let date = fact
                    .created_at
                    .as_ref()
                    .and_then(|dt_str| chrono::DateTime::parse_from_rfc3339(dt_str).ok())
                    .map(|dt| dt.format("%Y-%m-%d").to_string())
                    .unwrap_or_else(|| "unknown".to_string());

                let content = fact.body.as_deref().unwrap_or("");

                println!(
                    "[{}] {} (resonance {}) {}",
                    date, fact_type, fact.resonance, fact.id
                );
                let escaped = content.replace("]]>", "]]]]><![CDATA[>");
                println!("<![CDATA[{}]]>", escaped);
            }
            println!("</facts>");
        }

        MemoryCommands::ForSession {
            session_id,
            json,
            format,
        } => {
            let db = store::create_store_with_verbose(&config.db_path, verbose)?;

            // Normalize session ID
            let session_ref = normalize_id(&session_id);

            // Get fact IDs
            let fact_ids = db.get_facts_for_session(&session_ref)?;

            if fact_ids.is_empty() {
                println!("No facts found for session: {}", session_ref);
                return Ok(());
            }

            // Increment activation counts for session facts — viewing a session is
            // passive bulk access, not intentional recall of any single entry.
            // Do NOT reset last_activated so decay continues normally.
            if !fact_ids.is_empty()
                && let Err(e) = db.increment_activation_count(&fact_ids)
            {
                eprintln!("Warning: failed to update activation counts: {}", e);
            }

            // Fetch full entries for each fact
            let ctx = match std::env::var("MX_CURRENT_AGENT") {
                Ok(agent) if !agent.is_empty() => store::AgentContext::for_agent(agent),
                _ => store::AgentContext::public_only(),
            };

            // Support both --json flag and legacy --format json
            if json || format == "json" {
                let mut json_facts = Vec::new();
                for fact_id in &fact_ids {
                    if let Some(fact) = db.get(fact_id, &ctx)? {
                        let fact_type = fact
                            .summary
                            .as_ref()
                            .and_then(|s| serde_json::from_str::<serde_json::Value>(s).ok())
                            .and_then(|v: serde_json::Value| {
                                v.get("fact_type")
                                    .and_then(|t| t.as_str())
                                    .map(String::from)
                            });

                        json_facts.push(serde_json::json!({
                            "id": fact.id,
                            "type": fact_type,
                            "content": fact.body.as_ref().unwrap_or(&"".to_string()),
                            "created_at": fact.created_at.as_ref().unwrap_or(&"".to_string()),
                            "resonance": fact.resonance,
                        }));
                    }
                }
                println!("{}", serde_json::to_string_pretty(&json_facts)?);
            } else {
                println!("Facts for session {}:", session_ref);
                for fact_id in fact_ids {
                    if let Some(fact) = db.get(&fact_id, &ctx)? {
                        let fact_type = fact
                            .summary
                            .as_ref()
                            .and_then(|s| serde_json::from_str::<serde_json::Value>(s).ok())
                            .and_then(|v: serde_json::Value| {
                                v.get("fact_type")
                                    .and_then(|t| t.as_str())
                                    .map(String::from)
                            })
                            .unwrap_or_else(|| "unknown".to_string());

                        let date = fact
                            .created_at
                            .as_ref()
                            .and_then(|dt_str: &String| {
                                chrono::DateTime::parse_from_rfc3339(dt_str).ok()
                            })
                            .map(|dt| dt.format("%Y-%m-%d").to_string())
                            .unwrap_or_else(|| "unknown".to_string());

                        let content = fact.body.as_deref().unwrap_or("");
                        let preview = safe_truncate(content, 60);

                        println!(
                            "[{}] {}: {} ({}, resonance {})",
                            date, fact_type, preview, fact.id, fact.resonance
                        );
                    }
                }
            }
        }

        MemoryCommands::FactSession {
            fact_id,
            json,
            format,
        } => {
            let db = store::create_store_with_verbose(&config.db_path, verbose)?;

            // Normalize fact ID
            let fact_ref = normalize_id(&fact_id);

            // Activate fact when fetching its session (going deeper)
            if let Err(e) = db.update_activations(std::slice::from_ref(&fact_ref)) {
                eprintln!("Warning: failed to update activation: {}", e);
            }

            // Get session ID
            // Support both --json flag and legacy --format json
            let use_json = json || format == "json";
            match db.get_session_for_fact(&fact_ref)? {
                Some(session_id) => {
                    if use_json {
                        println!(
                            "{}",
                            serde_json::to_string_pretty(&serde_json::json!({
                                "fact_id": fact_ref,
                                "session_id": session_id,
                            }))?
                        );
                    } else {
                        println!(
                            "Fact {} was extracted from session: {}",
                            fact_ref, session_id
                        );
                    }
                }
                None => {
                    if use_json {
                        println!(
                            "{}",
                            serde_json::to_string_pretty(&serde_json::json!({
                                "fact_id": fact_ref,
                                "session_id": null,
                            }))?
                        );
                    } else {
                        println!("No session found for fact: {}", fact_ref);
                    }
                }
            }
        }

        MemoryCommands::Reinforce {
            id,
            amount,
            cap,
            json,
            format,
        } => {
            let db = store::create_store_with_verbose(&config.db_path, verbose)?;

            // Normalize ID
            let normalized_id = normalize_id(&id);

            // Respect visibility: agents can only reinforce entries they can see
            let ctx = match std::env::var("MX_CURRENT_AGENT") {
                Ok(agent) if !agent.is_empty() => store::AgentContext::for_agent(agent),
                _ => store::AgentContext::public_only(),
            };

            // Call reinforce on the store
            if let Some(result) = db.reinforce(&normalized_id, amount, Some(cap), &ctx)? {
                // Output result - support both --json flag and legacy --format json
                if json || format == "json" {
                    println!("{}", serde_json::to_string_pretty(&result)?);
                } else {
                    println!("Reinforced entry: {}", result.id);
                    println!("  Old resonance: {}", result.old_resonance);
                    println!("  New resonance: {}", result.new_resonance);
                    println!("  Amount added: {}", result.amount_added);
                    if result.capped {
                        println!("  (Capped at {})", cap);
                    }
                    println!("  Last activated: {}", result.last_activated);
                    println!("  Activation count: {}", result.activation_count);
                }
            } else {
                bail!("Entry '{}' not found", normalized_id);
            }
        }

        MemoryCommands::SweepGhosts { dry_run, json } => {
            let db = store::create_store_with_verbose(&config.db_path, verbose)?;

            if dry_run {
                eprintln!("sweep-ghosts: DRY RUN — no changes will be made");
            } else {
                eprintln!(
                    "sweep-ghosts: WARNING — this will modify memory data. Use --dry-run to preview first."
                );
            }

            let result = db.sweep_ghost_anchors(dry_run)?;

            if json {
                println!("{}", serde_json::to_string_pretty(&result)?);
            } else {
                // Human-readable report
                println!(
                    "Ghost anchor sweep{}",
                    if dry_run { " (dry run)" } else { "" }
                );
                println!(
                    "  Entries scanned (with anchors): {}",
                    result.entries_scanned
                );
                println!("  Ghost references found:         {}", result.ghosts_found);
                if dry_run {
                    println!(
                        "  Ghost references to remove:     {} (dry run, no changes made)",
                        result.ghosts_found
                    );
                } else {
                    println!(
                        "  Ghost references removed:       {}",
                        result.ghosts_removed
                    );
                }
                println!(
                    "  Entries affected:               {}",
                    result.affected_entries.len()
                );

                if !result.affected_entries.is_empty() {
                    println!();
                    println!("Affected entries:");
                    for entry in &result.affected_entries {
                        let ghost_count = entry.ghost_anchors.len();
                        println!(
                            "  {} ({}) — {} ghost anchor{}",
                            entry.id,
                            entry.title,
                            ghost_count,
                            if ghost_count == 1 { "" } else { "s" }
                        );
                        // Show individual ghost IDs — useful for verifying before real run
                        if verbose {
                            for ghost in &entry.ghost_anchors {
                                println!("      ghost: {}", ghost);
                            }
                        }
                    }
                }

                if dry_run && result.ghosts_found > 0 {
                    println!();
                    println!("To apply: hearth mx memory sweep-ghosts (without --dry-run)");
                } else if !dry_run && result.ghosts_removed > 0 {
                    println!();
                    println!(
                        "Done. {} ghost reference{} removed.",
                        result.ghosts_removed,
                        if result.ghosts_removed == 1 { "" } else { "s" }
                    );
                } else if result.ghosts_found == 0 {
                    println!();
                    println!("Graph is clean. No ghost anchors found.");
                }
            }
        }
    }

    Ok(())
}

#[cfg(test)]
mod show_divert_tests {
    use super::*;
    use std::path::Path;

    const ID: &str = "kn-99e08808";

    #[test]
    fn under_threshold_prints_inline() {
        let content = "small memory body\n";
        let plan = plan_show_output(content, ID, Path::new("/tmp"));
        assert_eq!(plan, ShowOutput::Inline);
    }

    #[test]
    fn empty_content_is_inline() {
        let plan = plan_show_output("", ID, Path::new("/tmp"));
        assert_eq!(plan, ShowOutput::Inline);
    }

    #[test]
    fn over_threshold_writes_file_and_pointer() {
        // One byte over the threshold must divert.
        let content = "a".repeat(BASH_STDOUT_DIVERT_THRESHOLD + 1);
        let temp_dir = Path::new("/tmp");
        let plan = plan_show_output(&content, ID, temp_dir);
        // Derive the expected path the same way the code does (`temp_dir.join`)
        // so the assertion is platform-agnostic -- on Windows the joined
        // separator is a backslash, so a hardcoded `/tmp/...` literal can never
        // match. Comparing against the joined path proves the real derived path.
        let expected_path = temp_dir.join("mx-memory-kn-99e08808.md");
        match plan {
            ShowOutput::Divert { path, pointer } => {
                assert_eq!(path, expected_path);
                // Pointer reports the true BYTE length.
                assert!(
                    pointer.contains(&format!("{} bytes", BASH_STDOUT_DIVERT_THRESHOLD + 1)),
                    "pointer missing byte count: {pointer}"
                );
                // The pointer renders the path via `Path::display()`, so derive
                // the expected substring the same way rather than assuming a
                // POSIX separator.
                let expected_path_str = expected_path.display().to_string();
                assert!(
                    pointer.contains(&expected_path_str),
                    "pointer missing path {expected_path_str}: {pointer}"
                );
                assert!(pointer.contains("Read the file to see full content."));
                // The pointer itself must stay safely under the ceiling.
                assert!(pointer.len() < BASH_STDOUT_DIVERT_THRESHOLD);
            }
            ShowOutput::Inline => panic!("expected divert for oversized content"),
        }
    }

    #[test]
    fn boundary_exactly_at_threshold_is_inline() {
        // Exactly at the threshold stays inline (the check is `<=`).
        let content = "a".repeat(BASH_STDOUT_DIVERT_THRESHOLD);
        assert_eq!(content.len(), BASH_STDOUT_DIVERT_THRESHOLD);
        let plan = plan_show_output(&content, ID, Path::new("/tmp"));
        assert_eq!(plan, ShowOutput::Inline);
    }

    #[test]
    fn boundary_counts_bytes_not_chars() {
        // 'é' (U+00E9) is 2 bytes in UTF-8 but 1 char. A string whose CHAR
        // count is under the threshold but whose BYTE count is over it must
        // divert -- proving we measure bytes, not chars.
        let multibyte = "é".repeat(BASH_STDOUT_DIVERT_THRESHOLD - 1);
        assert!(multibyte.chars().count() < BASH_STDOUT_DIVERT_THRESHOLD);
        assert!(multibyte.len() > BASH_STDOUT_DIVERT_THRESHOLD);
        let plan = plan_show_output(&multibyte, ID, Path::new("/tmp"));
        assert!(
            matches!(plan, ShowOutput::Divert { .. }),
            "multibyte content over the byte ceiling must divert"
        );
    }

    #[test]
    fn pointer_line_count_is_accurate() {
        // 5 lines, each padded so the total exceeds the threshold.
        let line = "x".repeat(BASH_STDOUT_DIVERT_THRESHOLD);
        let content = format!("{line}\n{line}\n{line}\n{line}\n{line}\n");
        match plan_show_output(&content, ID, Path::new("/tmp")) {
            ShowOutput::Divert { pointer, .. } => {
                assert!(
                    pointer.contains("(5 lines)"),
                    "expected 5 lines in pointer: {pointer}"
                );
            }
            ShowOutput::Inline => panic!("expected divert"),
        }
    }

    #[test]
    fn emit_diverts_oversized_content_to_real_temp_file() {
        // End-to-end through the IO wrapper: file is written with full content.
        let id = "kn-emittest1";
        let content = "z".repeat(BASH_STDOUT_DIVERT_THRESHOLD + 100);
        emit_show_output(&content, id, false).expect("emit should succeed");

        let expected = std::env::temp_dir().join(format!("mx-memory-{id}.md"));
        let written = std::fs::read_to_string(&expected).expect("temp file should exist");
        assert_eq!(written, content, "diverted file must hold the full content");

        let _ = std::fs::remove_file(&expected);
    }
}

#[cfg(test)]
mod wake_fetch_filter_tests {
    use super::*;

    // ---- parse_exclude_prefixes / keep_after_exclude (pure) ----

    fn prefixes(raw: &str) -> Vec<String> {
        parse_exclude_prefixes(Some(raw))
    }

    fn tags(t: &[&str]) -> Vec<String> {
        t.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn single_prefix_drops_matching_tag_keeps_others() {
        let ex = prefixes("project/");
        // An entry tagged project/x is excluded.
        assert!(!keep_after_exclude(&tags(&["project/x"]), &ex));
        // An untagged entry survives.
        assert!(keep_after_exclude(&[], &ex));
        // An entry whose tags don't match the prefix survives.
        assert!(keep_after_exclude(&tags(&["scratch/y", "notes"]), &ex));
        // Mixed: one matching tag is enough to exclude the whole entry.
        assert!(!keep_after_exclude(&tags(&["notes", "project/deep"]), &ex));
    }

    #[test]
    fn multiple_prefixes_exclude_any_match() {
        let ex = prefixes("project/,scratch/");
        assert!(!keep_after_exclude(&tags(&["project/a"]), &ex));
        assert!(!keep_after_exclude(&tags(&["scratch/b"]), &ex));
        assert!(keep_after_exclude(&tags(&["docs/c"]), &ex));
    }

    #[test]
    fn empty_whitespace_and_trailing_comma_input_yields_no_prefixes() {
        // Each of these parses to an empty prefix list -> nothing is excluded.
        for raw in ["", "   ", ",", ",,", "project/,", " , project/ "] {
            let ex = parse_exclude_prefixes(Some(raw));
            // Trailing/empty segments are dropped; only real prefixes remain.
            let expected_excludes = raw.contains("project/");
            // A project/ tag is excluded only when a real prefix survived parsing.
            assert_eq!(
                !keep_after_exclude(&tags(&["project/x"]), &ex),
                expected_excludes,
                "raw input {raw:?} produced prefixes {ex:?}"
            );
        }
        // Pure empty/whitespace cases produce zero prefixes.
        assert!(parse_exclude_prefixes(Some("")).is_empty());
        assert!(parse_exclude_prefixes(Some("   ")).is_empty());
        assert!(parse_exclude_prefixes(Some(",,")).is_empty());
        // Trailing comma drops the empty segment but keeps the real one.
        assert_eq!(parse_exclude_prefixes(Some("project/,")), vec!["project/"]);
    }

    #[test]
    fn none_input_is_empty_prefix_list_and_keeps_everything() {
        let ex = parse_exclude_prefixes(None);
        assert!(ex.is_empty());
        // Empty prefix list keeps every entry, even ones with tags.
        assert!(keep_after_exclude(&tags(&["project/x", "anything"]), &ex));
        assert!(keep_after_exclude(&[], &ex));
    }

    // ---- resolve_fact_type fallback (pure) ----

    #[test]
    fn fact_type_falls_back_to_category_when_summary_is_none() {
        // No summary at all -> entry survives the gate, adopts category_id as label.
        let label = resolve_fact_type(None, "decision");
        assert_eq!(label, "decision");
    }

    #[test]
    fn fact_type_falls_back_when_summary_lacks_fact_type_key() {
        // Summary JSON present but without a fact_type key -> fall back to category.
        let label = resolve_fact_type(Some(r#"{"other":"value"}"#), "discovery");
        assert_eq!(label, "discovery");
        // Malformed JSON also falls back rather than dropping the entry.
        let label = resolve_fact_type(Some("not json"), "method");
        assert_eq!(label, "method");
    }

    #[test]
    fn fact_type_uses_summary_value_when_present() {
        // When the label IS present in summary JSON, it wins over the category.
        let label = resolve_fact_type(Some(r#"{"fact_type":"decree"}"#), "decision");
        assert_eq!(label, "decree");
    }
}

#[cfg(test)]
mod write_verification_tests {
    use super::*;
    use crate::knowledge::KnowledgeEntry;
    use crate::store::AgentContext;
    use crate::store::KnowledgeStore;
    use crate::surreal_db::SurrealDatabase;

    /// Build an entry the way the standard `memory add` arm does, with explicit
    /// owner/visibility so the read-back path can be exercised faithfully.
    fn entry_with(id: &str, owner: Option<&str>, visibility: &str) -> KnowledgeEntry {
        let now = chrono::Utc::now().to_rfc3339();
        KnowledgeEntry {
            id: id.to_string(),
            category_id: "test".to_string(),
            title: format!("Title {id}"),
            body: Some("body".to_string()),
            summary: None,
            applicability: vec![],
            source_project_id: None,
            source_agent_id: None,
            file_path: None,
            tags: vec![],
            created_at: Some(now.clone()),
            updated_at: Some(now.clone()),
            content_hash: Some("hash".to_string()),
            source_type_id: Some("manual".to_string()),
            entry_type_id: Some("primary".to_string()),
            session_id: None,
            ephemeral: false,
            content_type_id: Some("text".to_string()),
            owner: owner.map(String::from),
            visibility: visibility.to_string(),
            resonance: 5,
            resonance_type: Some("ephemeral".to_string()),
            last_activated: Some(now),
            activation_count: 0,
            decay_rate: 0.0,
            anchors: vec![],
            wake_phrases: vec![],
            triggers: vec![],
            wake_order: None,
            wake_phrase: None,
            embedding: None,
            embedding_model: None,
            embedded_at: None,
            chunk_count: 0,
            format: "markdown".to_string(),
            effective_resonance: None,
        }
    }

    #[test]
    fn public_write_reads_back_present_no_false_bail() {
        // A public write must verify present regardless of the acting agent: the
        // verification context for a public entry uses the acting agent, and a
        // public row is always visible.
        let db = SurrealDatabase::open_in_memory().unwrap();
        let acting = "agent:writer";
        let entry = entry_with("kn-pub-write", Some("agent:writer"), "public");
        db.upsert_knowledge(&entry).unwrap();

        let ctx = write_verification_ctx(&entry.visibility, entry.owner.as_deref(), acting);
        assert!(
            db.get(&entry.id, &ctx).unwrap().is_some(),
            "public write must read back present (no false rejection)"
        );
    }

    #[test]
    fn private_write_with_foreign_owner_reads_back_present() {
        // THE B1 REGRESSION: `mx memory add --private --owner someone-else` stores
        // the row with owner=someone-else. The visibility filter only admits a
        // private row when owner = $current_agent, so verifying with the ACTING
        // agent (writer) would falsely fail. The fix builds the verification
        // context from the entry's stored owner instead.
        //
        // Fail-before / pass-after: against the pre-fix code (which used
        // for_agent(&agent_id)), this assertion FAILS because the read-back binds
        // $current_agent=agent:writer while the row's owner is agent:someone-else.
        // With the fix it PASSES.
        let db = SurrealDatabase::open_in_memory().unwrap();
        let acting = "agent:writer";
        let owner = "agent:someone-else";
        let entry = entry_with("kn-priv-foreign", Some(owner), "private");
        db.upsert_knowledge(&entry).unwrap();

        // What the FIXED code does: verify against the written owner.
        let ctx = write_verification_ctx(&entry.visibility, entry.owner.as_deref(), acting);
        assert!(
            db.get(&entry.id, &ctx).unwrap().is_some(),
            "successful private write to a foreign owner must NOT be reported as rejected"
        );

        // Sanity: the pre-fix behavior (verify as the acting agent) is exactly what
        // produced the false bail — proving the test discriminates the bug.
        let pre_fix_ctx = AgentContext::for_agent(acting);
        assert!(
            db.get(&entry.id, &pre_fix_ctx).unwrap().is_none(),
            "verifying a foreign-owned private row as the acting agent finds nothing (the old bug)"
        );
    }

    #[test]
    fn private_write_owned_by_acting_agent_reads_back_present() {
        // The common private case (owner defaults to the acting agent) still
        // verifies present under the fix.
        let db = SurrealDatabase::open_in_memory().unwrap();
        let acting = "agent:writer";
        let entry = entry_with("kn-priv-self", Some(acting), "private");
        db.upsert_knowledge(&entry).unwrap();

        let ctx = write_verification_ctx(&entry.visibility, entry.owner.as_deref(), acting);
        assert!(
            db.get(&entry.id, &ctx).unwrap().is_some(),
            "private write owned by the acting agent must read back present"
        );
    }

    #[test]
    fn genuinely_absent_entry_is_reported_rejected() {
        // Preserve Geoff's intent: a row that truly is not present (simulating a
        // silently-rejected write) reads back as None and would bail loudly.
        let db = SurrealDatabase::open_in_memory().unwrap();
        let ctx = write_verification_ctx("private", Some("agent:writer"), "agent:writer");
        assert!(
            db.get("kn-never-written", &ctx).unwrap().is_none(),
            "an absent entry must read back None so the handler bails loudly"
        );
    }
}
