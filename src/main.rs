mod db;
mod index;
mod knowledge;

use anyhow::Result;
use clap::{Parser, Subcommand};

use crate::db::Database;
use crate::index::{rebuild_index, IndexConfig};

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
}

fn main() -> Result<()> {
    let cli = Cli::parse();

    match cli.command {
        Commands::Zion { command } => handle_zion(command),
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
            } else {
                if entries.is_empty() {
                    println!("No results for '{}'", query);
                } else {
                    println!("Found {} results:\n", entries.len());
                    for entry in entries {
                        print_entry_summary(&entry);
                    }
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
            } else {
                if entries.is_empty() {
                    println!("No entries found");
                } else {
                    println!("Found {} entries:\n", entries.len());
                    for entry in entries {
                        print_entry_summary(&entry);
                    }
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
