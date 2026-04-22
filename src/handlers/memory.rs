use anyhow::{Context, Result, bail};

use crate::cli::*;
use crate::content_ops;
use crate::display::*;
use crate::engage;
use crate::helpers::*;
use crate::index::{
    IndexConfig, export_csv, export_jsonl, export_markdown, import_jsonl, rebuild_index,
};
use crate::knowledge;
use crate::store;
use crate::wake_ritual;

use super::metadata::*;

pub(crate) fn handle_memory(cmd: MemoryCommands, verbose: bool) -> Result<()> {
    let config = IndexConfig::default();

    match cmd {
        MemoryCommands::Rebuild => {
            println!("Rebuilding Memory index...");
            let stats = rebuild_index(&config)?;
            println!("{}", stats);
        }

        MemoryCommands::Search {
            query,
            filter,
            semantic,
        } => {
            let db = store::create_store_with_verbose(&config.db_path, verbose)?;
            let ctx = resolve_agent_context(filter.mine, filter.include_private);

            // Note: Search doesn't activate facts - discovery != engagement
            // Build filter for database query (resonance and category)
            let db_filter = store::KnowledgeFilter {
                min_resonance: filter.min_resonance,
                max_resonance: filter.max_resonance,
                categories: filter.category.clone(),
            };

            // Get results from database with resonance filtering
            let entries = if semantic {
                use crate::embeddings::{EmbeddingProvider, FastEmbedProvider};

                eprintln!("Initializing semantic search...");
                let mut provider = FastEmbedProvider::new()?;
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
                for entry in entries {
                    print_entry_summary(&entry);
                }
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

                    if content_only {
                        if let Some(body) = &entry.body {
                            print!("{}", body);
                        }
                    } else if json {
                        println!("{}", serde_json::to_string_pretty(&entry)?);
                    } else {
                        print_entry_full(&entry);
                    }
                }
                None => {
                    bail!("Entry '{}' not found", id);
                }
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
            let db = store::create_store_with_verbose(&config.db_path, verbose)?;
            let import_path = path
                .map(std::path::PathBuf::from)
                .unwrap_or_else(|| config.jsonl_path.clone());

            let count = import_jsonl(db.as_ref(), &import_path)?;
            println!("Imported {} entries from {:?}", count, import_path);
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
            anchors,
            r#type,
            session,
            thread_id,
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
                        if db.update_summary(
                            &tid,
                            &new_summary,
                            &store::AgentContext::for_agent(&agent_id),
                        )? {
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
                    wake_order: None,
                    wake_phrase: None,
                    embedding: None,
                    embedding_model: None,
                    embedded_at: None,
                    format: "markdown".to_string(),
                    effective_resonance: None,
                };

                // Insert the fact
                db.upsert_knowledge(&entry)?;

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

                // Auto-generate embedding if in network SurrealDB mode
                auto_embed(&id, db.as_ref())?;

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

            // Generate ID
            let path_hint = domain.unwrap_or_else(|| category.clone());
            let id = knowledge::KnowledgeEntry::generate_id(&path_hint, &title);

            // Create entry
            let now = chrono::Utc::now().to_rfc3339();
            let entry = knowledge::KnowledgeEntry {
                id: id.clone(),
                category_id: category.clone(),
                title: title.clone(),
                body: Some(body),
                summary: None,
                applicability: applicability_list.clone(),
                source_project_id: project,
                source_agent_id: Some(agent_id.clone()),
                file_path: None,
                tags: tag_list,
                created_at: Some(now.clone()),
                updated_at: Some(now),
                content_hash: Some(knowledge::KnowledgeEntry::compute_hash(&title)),
                source_type_id: Some(source_type),
                entry_type_id: Some(entry_type),
                session_id: session_id.clone(),
                ephemeral,
                content_type_id: Some(content_type),
                owner: entry_owner.clone(),
                visibility: entry_visibility.clone(),
                resonance: resonance.unwrap_or(0),
                resonance_type,
                last_activated: None,
                activation_count: 0,
                decay_rate: 0.0,
                anchors: anchor_list,
                wake_phrases: wake_phrase_list,
                wake_order,
                wake_phrase,
                embedding: None,
                embedding_model: None,
                embedded_at: None,
                format: "markdown".to_string(),
                effective_resonance: None,
            };

            // Insert into database (applicability already set in struct)
            db.upsert_knowledge(&entry)?;

            // Create EXTRACTED_FROM edge when --session-id is provided.
            // Standard mode stores session_id as a field but the for-session query
            // traverses the relates_to edge — wire both paths for consistency.
            if let Some(ref sess_id) = session_id {
                let session_ref = normalize_id(sess_id);
                let ctx = crate::store::AgentContext::public_only();
                if db.get(&session_ref, &ctx)?.is_none() {
                    eprintln!(
                        "Warning: Session {} not found - EXTRACTED_FROM edge not created",
                        session_ref
                    );
                } else {
                    db.add_relationship(&id, &session_ref, "extracted_from")?;
                }
            }

            // Auto-generate embedding if in network SurrealDB mode
            auto_embed(&id, db.as_ref())?;

            // Auto-generate anchors if in network SurrealDB mode
            auto_anchor(&id, db.as_ref(), None)?;

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
            wake_order,
            private,
            visibility,
            owner,
            session_id,
            force,
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

            // Auto-generate embedding if in network SurrealDB mode
            auto_embed(&id, db.as_ref())?;

            // Auto-generate anchors if in network SurrealDB mode
            // Pass explicitly removed anchors so auto_anchor respects user intent:
            // if the user did --anchors (full replacement) and removed some anchors,
            // auto_anchor should not re-add them.
            let removed = if explicitly_removed_anchors.is_empty() {
                None
            } else {
                Some(explicitly_removed_anchors.as_slice())
            };
            auto_anchor(&id, db.as_ref(), removed)?;

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

            // Auto-generate embedding if in network SurrealDB mode
            auto_embed(&id, db.as_ref())?;

            // Auto-generate anchors if in network SurrealDB mode
            auto_anchor(&id, db.as_ref(), None)?;

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

            // Auto-generate embedding if in network SurrealDB mode
            auto_embed(&id, db.as_ref())?;

            // Auto-generate anchors if in network SurrealDB mode
            auto_anchor(&id, db.as_ref(), None)?;

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

            // Auto-generate embedding if in network SurrealDB mode
            auto_embed(&id, db.as_ref())?;

            // Auto-generate anchors if in network SurrealDB mode
            auto_anchor(&id, db.as_ref(), None)?;

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

        MemoryCommands::Restore { id, list, json } => {
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

                // #3: update embeddings and anchors like all other mutation paths
                auto_embed(&id, db.as_ref())?;
                auto_anchor(&id, db.as_ref(), None)?;

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

        MemoryCommands::Embed { id, all } => {
            use crate::embeddings::{EmbeddingProvider, FastEmbedProvider};

            let db = store::create_store_with_verbose(&config.db_path, verbose)?;

            // Use current agent context for private entry access
            let ctx = match std::env::var("MX_CURRENT_AGENT") {
                Ok(agent) if !agent.is_empty() => store::AgentContext::for_agent(agent),
                _ => store::AgentContext::public_only(),
            };

            // Initialize embedding provider once
            println!("Initializing FastEmbed model...");
            let mut provider = FastEmbedProvider::new()?;

            if all {
                // Embed ALL entries
                let entries = db.list_all(&ctx)?;
                let total = entries.len();

                println!("Found {} entries to embed", total);

                for (idx, mut entry) in entries.into_iter().enumerate() {
                    // Construct embedding text from title + summary/body + tags
                    let mut parts = vec![entry.title.clone()];

                    if let Some(summary) = &entry.summary {
                        parts.push(summary.clone());
                    } else if let Some(body) = &entry.body {
                        parts.push(body.chars().take(2000).collect());
                    }

                    if !entry.tags.is_empty() {
                        parts.push(format!("Tags: {}", entry.tags.join(", ")));
                    }

                    let embedding_text = parts.join("\n\n");

                    // Generate embedding
                    println!("Embedded {}/{}: {}", idx + 1, total, entry.title);
                    let embedding = provider.embed(&embedding_text)?;

                    // Update entry with embedding
                    entry.embedding = Some(embedding);
                    entry.embedding_model = Some(provider.model_id().to_string());
                    entry.embedded_at = Some(chrono::Utc::now().to_rfc3339());
                    entry.updated_at = Some(chrono::Utc::now().to_rfc3339());

                    // Save to database
                    db.upsert_knowledge(&entry)?;
                }

                println!("✓ All {} entries embedded successfully!", total);
                println!("  Model: {}", provider.model_id());
                println!("  Dimensions: {}", provider.dimensions());
            } else {
                // Embed single entry
                let entry_id = id.ok_or_else(|| {
                    anyhow::anyhow!("Entry ID required (use --all to embed all entries)")
                })?;

                // Fetch entry
                let mut entry = db
                    .get(&entry_id, &ctx)?
                    .ok_or_else(|| anyhow::anyhow!("Entry not found: {}", entry_id))?;

                // Construct embedding text from title + summary/body + tags
                let mut parts = vec![entry.title.clone()];

                if let Some(summary) = &entry.summary {
                    parts.push(summary.clone());
                } else if let Some(body) = &entry.body {
                    parts.push(body.chars().take(2000).collect());
                }

                if !entry.tags.is_empty() {
                    parts.push(format!("Tags: {}", entry.tags.join(", ")));
                }

                let embedding_text = parts.join("\n\n");

                // Generate embedding
                println!("Generating embedding for '{}'...", entry.title);
                let embedding = provider.embed(&embedding_text)?;

                // Update entry with embedding
                entry.embedding = Some(embedding);
                entry.embedding_model = Some(provider.model_id().to_string());
                entry.embedded_at = Some(chrono::Utc::now().to_rfc3339());
                entry.updated_at = Some(chrono::Utc::now().to_rfc3339());

                // Save to database
                db.upsert_knowledge(&entry)?;

                println!("✓ Embedding generated and saved!");
                println!("  Entry: {}", entry_id);
                println!("  Model: {}", provider.model_id());
                println!("  Dimensions: {}", provider.dimensions());
            }
        }

        MemoryCommands::AutoAnchor {
            id,
            threshold,
            max_anchors,
            dry_run,
            verbose,
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

            println!("Processing {} entries...", entries.len());

            // Get ALL entries with embeddings for similarity comparison
            let all_candidates = db.list_all(&ctx)?;
            let candidates: Vec<_> = all_candidates
                .into_iter()
                .filter(|e| e.embedding.is_some())
                .collect();

            let mut total_added = 0;
            let entries_count = entries.len();

            for entry in entries {
                let entry_embedding = entry.embedding.as_ref().unwrap();

                // Calculate similarities
                let mut similarities: Vec<(String, String, f32)> = Vec::new();

                for candidate in &candidates {
                    // Skip self
                    if candidate.id == entry.id {
                        continue;
                    }

                    // Skip if already an anchor
                    if entry.anchors.contains(&candidate.id) {
                        continue;
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
                    if similarity >= threshold && similarity <= 0.95 {
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

                if top_matches.is_empty() {
                    if verbose {
                        println!(
                            "  {} \"{}\" - No similar entries found",
                            entry.id, entry.title
                        );
                    }
                    continue;
                }

                println!("Processing {} \"{}\"...", entry.id, entry.title);

                for (match_id, match_title, score) in &top_matches {
                    if verbose {
                        println!("  → {} \"{}\" ({:.2})", match_id, match_title, score);
                    } else {
                        println!("  → {} \"{}\"", match_id, match_title);
                    }
                }

                if dry_run {
                    println!(
                        "[DRY RUN] Would add {} anchors to {}",
                        top_matches.len(),
                        entry.id
                    );
                } else {
                    // Update the entry with new anchors
                    let new_anchor_ids: Vec<String> =
                        top_matches.iter().map(|(id, _, _)| id.clone()).collect();

                    // Merge with existing anchors
                    let mut updated_anchors = entry.anchors.clone();
                    updated_anchors.extend(new_anchor_ids);
                    updated_anchors.sort();
                    updated_anchors.dedup();

                    // Create updated entry
                    let mut updated_entry = entry.clone();
                    updated_entry.anchors = updated_anchors;
                    updated_entry.updated_at = Some(chrono::Utc::now().to_rfc3339());

                    // Save to database
                    db.upsert_knowledge(&updated_entry)?;

                    println!("Added {} anchors", top_matches.len());
                    total_added += top_matches.len();
                }
            }

            if dry_run {
                println!("\n[DRY RUN] Complete. No changes written.");
            } else {
                println!(
                    "\n✓ Added {} total anchors across {} entries",
                    total_added, entries_count
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
            json,
            ritual,
            index,
            no_activate,
            engage,
            set_missing,
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
            } else if engage {
                // Interactive engage mode
                engage::run_engage_ritual(&cascade, db.as_ref(), set_missing)?;
            } else if json {
                println!("{}", serde_json::to_string_pretty(&cascade)?);
            } else if index {
                print_wake_index(&cascade);
            } else if ritual {
                print_wake_ritual(&cascade, &current_agent);
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

        MemoryCommands::WakeFetch { days, limit } => {
            if days <= 0 {
                bail!("--days must be a positive integer (got {days})");
            }

            let db = store::create_store_with_verbose(&config.db_path, verbose)?;

            let mut facts = db.query_recent_facts_all_types(days)?;

            // Filter to resonance >= 3 AND extract fact_type in a single pass.
            // Collect (entry, fact_type) pairs so we don't re-parse summary JSON later.
            let mut typed_facts: Vec<(crate::knowledge::KnowledgeEntry, String)> = facts
                .drain(..)
                .filter(|f| f.resonance >= 3)
                .filter_map(|f| {
                    let ft = f
                        .summary
                        .as_ref()
                        .and_then(|s| serde_json::from_str::<serde_json::Value>(s).ok())
                        .and_then(|v| v.get("fact_type")?.as_str().map(String::from))?;
                    Some((f, ft))
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
    }

    Ok(())
}
