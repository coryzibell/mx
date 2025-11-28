mod commit;
mod db;
mod index;
mod knowledge;

use anyhow::Result;
use clap::{Parser, Subcommand};

use crate::db::Database;
use crate::index::{import_jsonl, rebuild_index, IndexConfig};

#[derive(Parser)]
#[command(name = "mx")]
#[command(about = "Matrix CLI - Knowledge indexing and task management")]
#[command(version)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Zion knowledge management
    Zion {
        #[command(subcommand)]
        command: ZionCommands,
    },

    /// Encoded commit (upload pattern)
    Commit {
        /// Commit message (human-readable, will be encoded)
        message: String,

        /// Stage all changes before committing
        #[arg(short = 'a', long)]
        all: bool,

        /// Push after committing
        #[arg(short, long)]
        push: bool,
    },
}

#[derive(Subcommand)]
enum ZionCommands {
    /// Rebuild the knowledge index
    Rebuild,

    /// Search knowledge entries
    Search {
        /// Search query
        query: String,

        /// Output as JSON
        #[arg(long)]
        json: bool,
    },

    /// List entries by category
    List {
        /// Category to list (pattern, technique, insight, ritual, project)
        #[arg(short, long)]
        category: Option<String>,

        /// Output as JSON
        #[arg(long)]
        json: bool,
    },

    /// Show a specific entry
    Show {
        /// Entry ID
        id: String,

        /// Output as JSON
        #[arg(long)]
        json: bool,
    },

    /// Show index statistics
    Stats,

    /// Delete an entry from the index
    Delete {
        /// Entry ID to delete
        id: String,
    },

    /// Import entries from JSONL file
    Import {
        /// Path to JSONL file (defaults to zion/index.jsonl)
        path: Option<String>,
    },

    /// Add a new entry directly to the database
    Add {
        /// Category (pattern, technique, insight, ritual, artifact, chronicle, project, future)
        #[arg(short, long)]
        category: String,

        /// Entry title
        #[arg(short, long)]
        title: String,

        /// Content inline
        #[arg(short = 'c', long, conflicts_with = "file")]
        content: Option<String>,

        /// Content from file
        #[arg(short, long, conflicts_with = "content")]
        file: Option<String>,

        /// Comma-separated tags
        #[arg(long)]
        tags: Option<String>,

        /// Associated project name
        #[arg(short, long)]
        project: Option<String>,

        /// Domain/subdomain path
        #[arg(short, long)]
        domain: Option<String>,
    },
}

fn main() -> Result<()> {
    let cli = Cli::parse();

    match cli.command {
        Commands::Zion { command } => handle_zion(command),
        Commands::Commit { message, all, push } => {
            commit::upload_commit(&message, all, push)?;
            Ok(())
        }
    }
}

fn handle_zion(cmd: ZionCommands) -> Result<()> {
    let config = IndexConfig::default();

    match cmd {
        ZionCommands::Rebuild => {
            println!("Rebuilding Zion index...");
            let stats = rebuild_index(&config)?;
            println!("{}", stats);
            println!("Index written to {:?}", config.jsonl_path);
        }

        ZionCommands::Search { query, json } => {
            let db = Database::open(&config.db_path)?;
            let entries = db.search(&query)?;

            if json {
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

        ZionCommands::List { category, json } => {
            let db = Database::open(&config.db_path)?;

            let entries = if let Some(cat) = &category {
                db.list_by_category(cat)?
            } else {
                // List all categories
                let mut all = Vec::new();
                for cat in &["pattern", "technique", "insight", "ritual", "project"] {
                    all.extend(db.list_by_category(cat)?);
                }
                all
            };

            if json {
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

        ZionCommands::Show { id, json } => {
            let db = Database::open(&config.db_path)?;

            match db.get(&id)? {
                Some(entry) => {
                    if json {
                        println!("{}", serde_json::to_string_pretty(&entry)?);
                    } else {
                        print_entry_full(&entry);
                    }
                }
                None => {
                    eprintln!("Entry '{}' not found", id);
                    std::process::exit(1);
                }
            }
        }

        ZionCommands::Stats => {
            let db = Database::open(&config.db_path)?;

            println!("Zion Index Statistics\n");
            println!("Total entries: {}", db.count()?);
            println!();

            for cat in &["pattern", "technique", "insight", "ritual", "project"] {
                let count = db.list_by_category(cat)?.len();
                println!("  {:12} {}", cat, count);
            }
        }

        ZionCommands::Delete { id } => {
            let db = Database::open(&config.db_path)?;

            if db.delete(&id)? {
                println!("Deleted entry '{}'", id);
            } else {
                eprintln!("Entry '{}' not found", id);
                std::process::exit(1);
            }
        }

        ZionCommands::Import { path } => {
            let db = Database::open(&config.db_path)?;
            let import_path = path
                .map(std::path::PathBuf::from)
                .unwrap_or_else(|| config.jsonl_path.clone());

            let count = import_jsonl(&db, &import_path)?;
            println!("Imported {} entries from {:?}", count, import_path);
        }

        ZionCommands::Add {
            category,
            title,
            content,
            file,
            tags,
            project,
            domain,
        } => {
            use anyhow::Context;
            use std::fs;

            // Validate category
            let valid_categories = [
                "pattern",
                "technique",
                "insight",
                "ritual",
                "artifact",
                "chronicle",
                "project",
                "future",
            ];

            if !valid_categories.contains(&category.as_str()) {
                eprintln!("Error: Invalid category '{}'", category);
                eprintln!("Valid categories: {}", valid_categories.join(", "));
                std::process::exit(1);
            }

            // Get content from either --content or --file
            let body = if let Some(text) = content {
                text
            } else if let Some(file_path) = file {
                fs::read_to_string(&file_path)
                    .with_context(|| format!("Failed to read file: {}", file_path))?
            } else {
                eprintln!("Error: Either --content or --file must be provided");
                std::process::exit(1);
            };

            // Parse tags
            let tag_list: Vec<String> = tags
                .map(|t| {
                    t.split(',')
                        .map(|s| s.trim().to_string())
                        .filter(|s| !s.is_empty())
                        .collect()
                })
                .unwrap_or_default();

            // Generate ID
            let path_hint = domain.unwrap_or_else(|| category.clone());
            let id = knowledge::KnowledgeEntry::generate_id(&path_hint, &title);

            // Create entry
            let now = chrono::Utc::now().to_rfc3339();
            let entry = knowledge::KnowledgeEntry {
                id: id.clone(),
                category: category.clone(),
                title: title.clone(),
                body: Some(body),
                summary: None,
                applicability: None,
                source_project: project,
                source_agent: None,
                file_path: None,
                tags: tag_list,
                created_at: Some(now.clone()),
                updated_at: Some(now),
                content_hash: Some(knowledge::KnowledgeEntry::compute_hash(&title)),
                source_type: Some("manual".to_string()),
                entry_type: Some("primary".to_string()),
                session_id: None,
                ephemeral: false,
            };

            // Insert into database
            let db = Database::open(&config.db_path)?;
            db.upsert_knowledge(&entry)?;

            println!("Added entry: {}", id);
            println!("  Category: {}", category);
            println!("  Title: {}", title);
            if !entry.tags.is_empty() {
                println!("  Tags: {}", entry.tags.join(", "));
            }
        }
    }

    Ok(())
}

fn print_entry_summary(entry: &knowledge::KnowledgeEntry) {
    println!("  {} [{}]", entry.id, entry.category);
    println!("  {}", entry.title);
    if let Some(summary) = &entry.summary {
        let short = if summary.len() > 80 {
            format!("{}...", &summary[..77])
        } else {
            summary.clone()
        };
        println!("  {}", short);
    }
    if !entry.tags.is_empty() {
        println!("  Tags: {}", entry.tags.join(", "));
    }
    println!();
}

fn print_entry_full(entry: &knowledge::KnowledgeEntry) {
    println!("ID:       {}", entry.id);
    println!("Category: {}", entry.category);
    println!("Title:    {}", entry.title);
    if let Some(path) = &entry.file_path {
        println!("File:     {}", path);
    }
    if !entry.tags.is_empty() {
        println!("Tags:     {}", entry.tags.join(", "));
    }
    if let Some(created) = &entry.created_at {
        println!("Created:  {}", created);
    }
    if let Some(updated) = &entry.updated_at {
        println!("Updated:  {}", updated);
    }
    println!();
    if let Some(body) = &entry.body {
        println!("{}", body);
    }
}
