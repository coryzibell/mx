mod commit;
mod convert;
mod db;
mod doctor;
mod github;
mod index;
mod knowledge;
mod session;
mod sync;

use anyhow::Result;
use clap::{Parser, Subcommand};

use crate::db::Database;
use crate::index::{
    export_csv, export_jsonl, export_markdown, import_jsonl, rebuild_index, IndexConfig,
};

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

    /// Generate encoded commit message (for MCP/API use)
    EncodeCommit {
        /// Title text (will be hashed and encoded)
        #[arg(short, long)]
        title: String,

        /// Body text (will be compressed and encoded)
        #[arg(short, long)]
        body: String,
    },

    /// Pull request operations
    Pr {
        #[command(subcommand)]
        command: PrCommands,
    },

    /// GitHub sync operations
    Sync {
        #[command(subcommand)]
        command: SyncCommands,
    },

    /// GitHub operations
    Github {
        #[command(subcommand)]
        command: GithubCommands,
    },

    /// Wiki operations
    Wiki {
        #[command(subcommand)]
        command: WikiCommands,
    },

    /// Session export operations
    Session {
        #[command(subcommand)]
        command: SessionCommands,
    },

    /// Conversion utilities
    Convert {
        #[command(subcommand)]
        command: ConvertCommands,
    },

    /// Environment health check
    Doctor,
}

#[derive(Subcommand)]
enum ConvertCommands {
    /// Convert markdown to YAML for GitHub sync
    Md2yaml {
        /// Input file or directory
        input: String,

        /// Output directory (defaults to current directory)
        #[arg(short, long)]
        output: Option<String>,

        /// Dry run - show what would be created
        #[arg(long)]
        dry_run: bool,
    },

    /// Convert YAML to markdown for human reading
    Yaml2md {
        /// Input file or directory
        input: String,

        /// Output directory (defaults to current directory)
        #[arg(short, long)]
        output: Option<String>,

        /// Repository in owner/repo format (for GitHub URLs)
        #[arg(short, long)]
        repo: Option<String>,

        /// Dry run - show what would be created
        #[arg(long)]
        dry_run: bool,
    },
}

#[derive(Subcommand)]
enum PrCommands {
    /// Merge a pull request with encoded commit message
    Merge {
        /// PR number
        number: u32,

        /// Use rebase merge
        #[arg(long)]
        rebase: bool,

        /// Use standard merge commit (instead of squash)
        #[arg(long, name = "merge")]
        merge_commit: bool,
    },
}

#[derive(Subcommand)]
pub enum SyncCommands {
    /// Pull issues/discussions from GitHub to local YAML
    Pull {
        /// Repository (owner/repo format)
        repo: String,

        /// Output directory (defaults to ~/.matrix/cache/sync/<repo>)
        #[arg(short, long)]
        output: Option<String>,

        /// Dry run - show what would be pulled
        #[arg(long)]
        dry_run: bool,
    },

    /// Push local changes to GitHub
    Push {
        /// Repository (owner/repo format)
        repo: String,

        /// Input directory (defaults to ~/.matrix/cache/sync/<repo>)
        #[arg(short, long)]
        input: Option<String>,

        /// Dry run - show what would be pushed
        #[arg(long)]
        dry_run: bool,
    },

    /// Sync identity labels to repository
    Labels {
        /// Repository (owner/repo format)
        repo: String,

        /// Dry run - show what would be synced
        #[arg(long)]
        dry_run: bool,
    },

    /// Sync issues bidirectionally
    Issues {
        /// Repository (owner/repo format)
        repo: String,

        /// Dry run - show what would be synced
        #[arg(long)]
        dry_run: bool,
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

        /// Applicability contexts (comma-separated)
        #[arg(short = 'a', long)]
        applicability: Option<String>,

        /// Source project ID
        #[arg(short, long)]
        project: Option<String>,

        /// Source agent ID
        #[arg(long)]
        source_agent: Option<String>,

        /// Source type (manual, ram, cache, agent_session)
        #[arg(long, default_value = "manual")]
        source_type: String,

        /// Entry type (primary, summary, synthesis)
        #[arg(long, default_value = "primary")]
        entry_type: String,

        /// Session ID
        #[arg(long)]
        session_id: Option<String>,

        /// Mark as ephemeral
        #[arg(long)]
        ephemeral: bool,

        /// Domain/subdomain path
        #[arg(short, long)]
        domain: Option<String>,
    },

    /// Apply database schema migrations
    Migrate {
        /// Show migration status (list tables)
        #[arg(long)]
        status: bool,
    },

    /// Manage agents registry
    Agents {
        #[command(subcommand)]
        command: AgentsCommands,
    },

    /// Export knowledge database
    Export {
        /// Output format (md, jsonl, csv)
        #[arg(short, long, default_value = "md")]
        format: String,

        /// Output directory for md format (defaults to ./zion-export), or file for jsonl/csv (defaults to stdout)
        #[arg(short, long)]
        output: Option<String>,
    },

    /// Manage projects
    Projects {
        #[command(subcommand)]
        command: ProjectsCommands,
    },

    /// Manage applicability types
    Applicability {
        #[command(subcommand)]
        command: ApplicabilityCommands,
    },

    /// Manage sessions
    Sessions {
        #[command(subcommand)]
        command: SessionsCommands,
    },
}

#[derive(Subcommand)]
enum GithubCommands {
    /// Clean up GitHub issues and discussions
    Cleanup {
        /// Repository (owner/repo format)
        repo: String,

        /// Issue numbers to close (comma-separated)
        #[arg(long)]
        issues: Option<String>,

        /// Discussion numbers to delete (comma-separated)
        #[arg(long)]
        discussions: Option<String>,

        /// Dry run - show what would be done
        #[arg(long)]
        dry_run: bool,
    },

    /// Post comments to issues or discussions
    Comment {
        #[command(subcommand)]
        command: CommentCommands,
    },
}

#[derive(Subcommand)]
enum CommentCommands {
    /// Post comment to an issue
    Issue {
        /// Repository (owner/repo format)
        repo: String,

        /// Issue number
        number: u64,

        /// Comment message
        message: String,

        /// Identity signature (e.g., "smith", "neo")
        #[arg(long)]
        identity: Option<String>,
    },

    /// Post comment to a discussion
    Discussion {
        /// Repository (owner/repo format)
        repo: String,

        /// Discussion number
        number: u64,

        /// Comment message
        message: String,

        /// Identity signature (e.g., "smith", "neo")
        #[arg(long)]
        identity: Option<String>,
    },
}

#[derive(Subcommand)]
enum SessionCommands {
    /// Export session to markdown
    Export {
        /// Path to session JSONL file (defaults to most recent non-agent session)
        path: Option<String>,

        /// Output file (defaults to stdout)
        #[arg(short, long)]
        output: Option<String>,
    },
}

#[derive(Subcommand)]
enum AgentsCommands {
    /// List all agents
    List,

    /// Add a new agent
    Add {
        /// Agent ID (e.g., smith, neo, trinity)
        id: String,

        /// Agent description
        #[arg(short, long)]
        description: String,

        /// Agent domain/responsibility
        #[arg(short = 'D', long)]
        domain: String,
    },

    /// Show agent details
    Show {
        /// Agent ID
        id: String,
    },

    /// Seed agents from markdown files with YAML frontmatter
    Seed {
        /// Path to agents directory (defaults to ~/.matrix/agents/)
        #[arg(short, long)]
        path: Option<String>,
    },
}

#[derive(Subcommand)]
enum ProjectsCommands {
    /// List all projects
    List,
    /// Add a new project
    Add {
        #[arg(long)]
        id: String,
        #[arg(long)]
        name: String,
        #[arg(long)]
        path: Option<String>,
        #[arg(long)]
        repo_url: Option<String>,
        #[arg(long)]
        description: Option<String>,
    },
}

#[derive(Subcommand)]
enum ApplicabilityCommands {
    /// List all applicability types
    List,
    /// Add a new applicability type
    Add {
        #[arg(long)]
        id: String,
        #[arg(long)]
        description: String,
        #[arg(long)]
        scope: Option<String>,
    },
}

#[derive(Subcommand)]
enum SessionsCommands {
    /// List sessions
    List {
        #[arg(long)]
        project: Option<String>,
    },
    /// Create a new session
    Create {
        #[arg(long)]
        session_type: String,
        #[arg(long)]
        project: Option<String>,
    },
    /// Close a session
    Close {
        #[arg(long)]
        id: String,
    },
}

#[derive(Subcommand)]
enum WikiCommands {
    /// Sync markdown files to GitHub wiki
    Sync {
        /// Repository (owner/repo format)
        repo: String,

        /// Source file or directory
        source: String,

        /// Custom page name (single file only)
        #[arg(long)]
        page_name: Option<String>,

        /// Dry run - show what would be synced
        #[arg(long)]
        dry_run: bool,
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
        Commands::EncodeCommit { title, body } => {
            let message = commit::encode_commit_message(&title, &body)?;
            println!("{}", message);
            Ok(())
        }
        Commands::Pr { command } => handle_pr(command),
        Commands::Sync { command } => sync::handle_sync(command),
        Commands::Github { command } => handle_github(command),
        Commands::Wiki { command } => handle_wiki(command),
        Commands::Session { command } => handle_session(command),
        Commands::Convert { command } => handle_convert(command),
        Commands::Doctor => doctor::run_checks(),
    }
}

fn handle_zion(cmd: ZionCommands) -> Result<()> {
    let config = IndexConfig::default();

    match cmd {
        ZionCommands::Rebuild => {
            println!("Rebuilding Zion index...");
            let stats = rebuild_index(&config)?;
            println!("{}", stats);
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
                // List all categories from database
                let mut all = Vec::new();
                let categories = db.list_categories()?;
                for cat in categories {
                    all.extend(db.list_by_category(&cat.id)?);
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

            let categories = db.list_categories()?;
            for cat in categories {
                let count = db.list_by_category(&cat.id)?.len();
                println!("  {:12} {}", cat.id, count);
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
            applicability,
            project,
            source_agent,
            source_type,
            entry_type,
            session_id,
            ephemeral,
            domain,
        } => {
            use anyhow::Context;
            use std::fs;

            let db = Database::open(&config.db_path)?;

            // Validate category against database
            if db.get_category(&category)?.is_none() {
                let categories = db.list_categories()?;
                let valid_ids: Vec<&str> = categories.iter().map(|c| c.id.as_str()).collect();
                eprintln!("Error: Invalid category '{}'", category);
                eprintln!("Valid categories: {}", valid_ids.join(", "));
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

            // Parse applicability CSV
            let applicability_list: Vec<String> = applicability
                .map(|a| {
                    a.split(',')
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
                category_id: category.clone(),
                title: title.clone(),
                body: Some(body),
                summary: None,
                applicability: applicability_list.clone(),
                source_project_id: project,
                source_agent_id: source_agent,
                file_path: None,
                tags: tag_list,
                created_at: Some(now.clone()),
                updated_at: Some(now),
                content_hash: Some(knowledge::KnowledgeEntry::compute_hash(&title)),
                source_type_id: Some(source_type),
                entry_type_id: Some(entry_type),
                session_id,
                ephemeral,
            };

            // Insert into database
            db.upsert_knowledge(&entry)?;

            // Set applicability if provided
            if !applicability_list.is_empty() {
                db.set_applicability_for_entry(&entry.id, &applicability_list)?;
            }

            println!("Added entry: {}", id);
            println!("  Category: {}", category);
            println!("  Title: {}", title);
            if !entry.tags.is_empty() {
                println!("  Tags: {}", entry.tags.join(", "));
            }
            if !applicability_list.is_empty() {
                println!("  Applicability: {}", applicability_list.join(", "));
            }
        }

        ZionCommands::Migrate { status } => {
            let db = Database::open(&config.db_path)?;

            if status {
                // Show current tables
                let tables: Vec<String> = db.list_tables()?;
                println!("Database tables:");
                for table in tables {
                    println!("  {}", table);
                }
            } else {
                // Apply migrations (schema is applied in Database::open via init_schema)
                println!("Applying migrations to {:?}...", config.db_path);
                println!("Schema applied successfully");

                // Show what exists now
                let tables = db.list_tables()?;
                println!("\nCurrent tables:");
                for table in tables {
                    println!("  {}", table);
                }
            }
        }

        ZionCommands::Agents { command } => handle_agents(command, &config)?,

        ZionCommands::Projects { command } => handle_projects(command, &config)?,

        ZionCommands::Applicability { command } => handle_applicability(command, &config)?,

        ZionCommands::Sessions { command } => handle_sessions(command, &config)?,

        ZionCommands::Export { format, output } => {
            let db = Database::open(&config.db_path)?;

            match format.as_str() {
                "md" | "markdown" => {
                    // Markdown exports to directory
                    let output_dir = output.as_deref().unwrap_or("./zion-export");

                    let dir_path = std::path::PathBuf::from(output_dir);
                    export_markdown(&db, &dir_path)?;
                    println!("Exported to directory: {}", output_dir);
                }
                "jsonl" => {
                    // JSONL exports to file or stdout
                    if let Some(ref path) = output {
                        export_jsonl(&db, &std::path::PathBuf::from(path))?;
                        println!("Exported to {}", path);
                    } else {
                        export_jsonl(&db, &std::path::PathBuf::from("/dev/stdout"))?;
                    }
                }
                "csv" => {
                    // CSV exports to file or stdout
                    if let Some(ref path) = output {
                        export_csv(&db, &std::path::PathBuf::from(path))?;
                        println!("Exported to {}", path);
                    } else {
                        export_csv(&db, &std::path::PathBuf::from("/dev/stdout"))?;
                    }
                }
                _ => {
                    eprintln!(
                        "Error: Invalid format '{}'. Valid formats: md, jsonl, csv",
                        format
                    );
                    std::process::exit(1);
                }
            }
        }
    }

    Ok(())
}

fn handle_agents(cmd: AgentsCommands, config: &IndexConfig) -> Result<()> {
    let db = Database::open(&config.db_path)?;

    match cmd {
        AgentsCommands::List => {
            let agents = db.list_agents()?;
            if agents.is_empty() {
                println!("No agents registered");
            } else {
                println!("Registered agents:\n");
                for agent in agents {
                    println!(
                        "  {} - {}",
                        agent.id,
                        agent.description.as_deref().unwrap_or("")
                    );
                    if let Some(domain) = &agent.domain {
                        println!("    Domain: {}", domain);
                    }
                }
            }
        }

        AgentsCommands::Add {
            id,
            description,
            domain,
        } => {
            let now = chrono::Utc::now().to_rfc3339();
            let agent = db::Agent {
                id: id.clone(),
                description: Some(description.clone()),
                domain: Some(domain.clone()),
                created_at: Some(now.clone()),
                updated_at: Some(now),
            };

            db.upsert_agent(&agent)?;
            println!("Added agent: {}", id);
            println!("  Description: {}", description);
            println!("  Domain: {}", domain);
        }

        AgentsCommands::Show { id } => match db.get_agent(&id)? {
            Some(agent) => {
                println!("Agent: {}", agent.id);
                if let Some(desc) = &agent.description {
                    println!("Description: {}", desc);
                }
                if let Some(domain) = &agent.domain {
                    println!("Domain: {}", domain);
                }
                if let Some(created) = &agent.created_at {
                    println!("Created: {}", created);
                }
                if let Some(updated) = &agent.updated_at {
                    println!("Updated: {}", updated);
                }
            }
            None => {
                eprintln!("Agent '{}' not found", id);
                std::process::exit(1);
            }
        },

        AgentsCommands::Seed { path } => {
            use anyhow::Context;
            use std::fs;
            use std::path::PathBuf;

            // Determine agents directory
            let agents_dir = if let Some(p) = path {
                PathBuf::from(p)
            } else {
                // Default: ~/.matrix/agents/
                let home = dirs::home_dir().context("Could not determine home directory")?;
                home.join(".matrix").join("agents")
            };

            if !agents_dir.exists() {
                eprintln!("Agents directory does not exist: {:?}", agents_dir);
                std::process::exit(1);
            }

            // Scan for .md files
            let entries = fs::read_dir(&agents_dir)
                .with_context(|| format!("Failed to read directory: {:?}", agents_dir))?;

            let mut seeded = Vec::new();
            let now = chrono::Utc::now().to_rfc3339();

            for entry in entries {
                let entry = entry?;
                let path = entry.path();

                // Skip if not a markdown file
                if path.extension().and_then(|s| s.to_str()) != Some("md") {
                    continue;
                }

                // Skip files starting with _
                if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
                    if name.starts_with('_') {
                        continue;
                    }
                }

                // Read file
                let content = fs::read_to_string(&path)
                    .with_context(|| format!("Failed to read file: {:?}", path))?;

                // Parse frontmatter
                if let Some((frontmatter, _body)) = parse_frontmatter(&content) {
                    if let Ok(agent_data) = serde_yaml::from_str::<AgentFrontmatter>(&frontmatter) {
                        let agent = db::Agent {
                            id: agent_data.name.clone(),
                            description: Some(agent_data.description.clone()),
                            domain: agent_data.domain,
                            created_at: Some(now.clone()),
                            updated_at: Some(now.clone()),
                        };

                        db.upsert_agent(&agent)?;
                        seeded.push(agent_data.name);
                    }
                }
            }

            if seeded.is_empty() {
                println!("No agents seeded from {:?}", agents_dir);
            } else {
                println!("Seeded {} agents:", seeded.len());
                for name in &seeded {
                    println!("  {}", name);
                }
            }
        }
    }

    Ok(())
}

fn handle_projects(cmd: ProjectsCommands, config: &IndexConfig) -> Result<()> {
    let db = Database::open(&config.db_path)?;

    match cmd {
        ProjectsCommands::List => {
            let projects = db.list_projects(false)?;
            if projects.is_empty() {
                println!("No projects registered");
            } else {
                println!("Registered projects:\n");
                for project in projects {
                    println!("  {} - {}", project.id, project.name);
                    if let Some(path) = &project.path {
                        println!("    Path: {}", path);
                    }
                    if let Some(url) = &project.repo_url {
                        println!("    Repo: {}", url);
                    }
                    if let Some(desc) = &project.description {
                        println!("    Description: {}", desc);
                    }
                    println!();
                }
            }
        }

        ProjectsCommands::Add {
            id,
            name,
            path,
            repo_url,
            description,
        } => {
            let now = chrono::Utc::now().to_rfc3339();
            let project = db::Project {
                id: id.clone(),
                name: name.clone(),
                path,
                repo_url,
                description,
                active: true,
                created_at: now.clone(),
                updated_at: now,
            };

            db.upsert_project(&project)?;
            println!("Added project: {}", id);
            println!("  Name: {}", name);
        }
    }

    Ok(())
}

fn handle_applicability(cmd: ApplicabilityCommands, config: &IndexConfig) -> Result<()> {
    let db = Database::open(&config.db_path)?;

    match cmd {
        ApplicabilityCommands::List => {
            let types = db.list_applicability_types()?;
            if types.is_empty() {
                println!("No applicability types registered");
            } else {
                println!("Registered applicability types:\n");
                for atype in types {
                    println!("  {} - {}", atype.id, atype.description);
                    if let Some(scope) = &atype.scope {
                        println!("    Scope: {}", scope);
                    }
                    println!();
                }
            }
        }

        ApplicabilityCommands::Add {
            id,
            description,
            scope,
        } => {
            let now = chrono::Utc::now().to_rfc3339();
            let atype = db::ApplicabilityType {
                id: id.clone(),
                description: description.clone(),
                scope,
                created_at: now,
            };

            db.upsert_applicability_type(&atype)?;
            println!("Added applicability type: {}", id);
            println!("  Description: {}", description);
        }
    }

    Ok(())
}

fn handle_sessions(cmd: SessionsCommands, config: &IndexConfig) -> Result<()> {
    let db = Database::open(&config.db_path)?;

    match cmd {
        SessionsCommands::List { project } => {
            let sessions = db.list_sessions(project.as_deref())?;
            if sessions.is_empty() {
                println!("No sessions found");
            } else {
                println!("Sessions:\n");
                for session in sessions {
                    println!("  ID: {}", session.id);
                    println!("    Type: {}", session.session_type_id);
                    if let Some(proj) = &session.project_id {
                        println!("    Project: {}", proj);
                    }
                    println!("    Started: {}", session.started_at);
                    if let Some(ended) = &session.ended_at {
                        println!("    Ended: {}", ended);
                    }
                    println!();
                }
            }
        }

        SessionsCommands::Create {
            session_type,
            project,
        } => {
            let now = chrono::Utc::now().to_rfc3339();
            let id = format!("sess-{}", chrono::Utc::now().format("%Y%m%d-%H%M%S"));
            let session = db::Session {
                id: id.clone(),
                session_type_id: session_type,
                project_id: project,
                started_at: now,
                ended_at: None,
                metadata: None,
            };

            db.upsert_session(&session)?;
            println!("Created session: {}", id);
        }

        SessionsCommands::Close { id } => {
            if let Some(mut session) = db.get_session(&id)? {
                session.ended_at = Some(chrono::Utc::now().to_rfc3339());
                db.upsert_session(&session)?;
                println!("Closed session: {}", id);
            } else {
                eprintln!("Session '{}' not found", id);
                std::process::exit(1);
            }
        }
    }

    Ok(())
}

fn handle_pr(cmd: PrCommands) -> Result<()> {
    match cmd {
        PrCommands::Merge {
            number,
            rebase,
            merge_commit,
        } => {
            commit::pr_merge(number, rebase, merge_commit)?;
            Ok(())
        }
    }
}

fn handle_github(cmd: GithubCommands) -> Result<()> {
    match cmd {
        GithubCommands::Cleanup {
            repo,
            issues,
            discussions,
            dry_run,
        } => {
            github::cleanup(&repo, issues, discussions, dry_run)?;
            Ok(())
        }
        GithubCommands::Comment { command } => {
            handle_comment(command)?;
            Ok(())
        }
    }
}

fn handle_comment(cmd: CommentCommands) -> Result<()> {
    match cmd {
        CommentCommands::Issue {
            repo,
            number,
            message,
            identity,
        } => {
            let url = github::post_issue_comment(&repo, number, &message, identity.as_deref())?;
            println!("Comment posted: {}", url);
        }
        CommentCommands::Discussion {
            repo,
            number,
            message,
            identity,
        } => {
            let url = github::post_discussion_comment(&repo, number, &message, identity.as_deref())?;
            println!("Comment posted: {}", url);
        }
    }
    Ok(())
}

fn handle_session(cmd: SessionCommands) -> Result<()> {
    match cmd {
        SessionCommands::Export { path, output } => {
            session::export_session(path, output)?;
            Ok(())
        }
    }
}

fn handle_convert(cmd: ConvertCommands) -> Result<()> {
    use std::path::PathBuf;

    match cmd {
        ConvertCommands::Md2yaml {
            input,
            output,
            dry_run,
        } => {
            let input_path = PathBuf::from(&input);
            let output_dir = output
                .map(PathBuf::from)
                .unwrap_or_else(|| std::env::current_dir().unwrap());

            if input_path.is_file() {
                convert::convert_file(&input_path, &output_dir, dry_run)?;
            } else if input_path.is_dir() {
                convert::convert_directory(&input_path, &output_dir, dry_run)?;
            } else {
                eprintln!("Error: Input path does not exist: {:?}", input_path);
                std::process::exit(1);
            }

            Ok(())
        }

        ConvertCommands::Yaml2md {
            input,
            output,
            repo,
            dry_run,
        } => {
            let input_path = PathBuf::from(&input);
            let output_dir = output
                .map(PathBuf::from)
                .unwrap_or_else(|| std::env::current_dir().unwrap());

            if input_path.is_file() {
                convert::yaml_to_markdown_file(&input_path, &output_dir, repo.as_deref(), dry_run)?;
            } else if input_path.is_dir() {
                convert::yaml_to_markdown_directory(&input_path, &output_dir, repo.as_deref(), dry_run)?;
            } else {
                eprintln!("Error: Input path does not exist: {:?}", input_path);
                std::process::exit(1);
            }

            Ok(())
        }
    }
}

fn handle_wiki(cmd: WikiCommands) -> Result<()> {
    match cmd {
        WikiCommands::Sync {
            repo,
            source,
            page_name,
            dry_run,
        } => {
            sync::wiki::sync(&repo, &source, page_name.as_deref(), dry_run)?;
            Ok(())
        }
    }
}

#[derive(serde::Deserialize)]
struct AgentFrontmatter {
    name: String,
    description: String,
    #[serde(default)]
    domain: Option<String>,
}

fn parse_frontmatter(content: &str) -> Option<(String, String)> {
    let lines: Vec<&str> = content.lines().collect();

    // Check if starts with ---
    if lines.first()? != &"---" {
        return None;
    }

    // Find closing ---
    let end_idx = lines.iter().skip(1).position(|&line| line == "---")?;

    let frontmatter = lines[1..=end_idx].join("\n");
    let body = lines[end_idx + 2..].join("\n");

    Some((frontmatter, body))
}

fn print_entry_summary(entry: &knowledge::KnowledgeEntry) {
    println!("  {} [{}]", entry.id, entry.category_id);
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
    println!("Category: {}", entry.category_id);
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
