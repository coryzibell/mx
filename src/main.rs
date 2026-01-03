#![allow(dead_code)]

mod codex;
mod commit;
mod convert;
mod db;
mod doctor;
mod engage;
mod github;
mod index;
mod knowledge;
mod session;
mod store;
mod surreal_db;
mod sync;
mod wake_ritual;
mod wake_token;

use anyhow::{Context, Result, bail};
use clap::{Parser, Subcommand};

use crate::index::{
    IndexConfig, export_csv, export_jsonl, export_markdown, import_jsonl, rebuild_index,
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
#[allow(clippy::large_enum_variant)]
enum Commands {
    /// Memory knowledge management
    Memory {
        #[command(subcommand)]
        command: MemoryCommands,
    },

    /// Encoded commit (upload pattern)
    Commit {
        /// Commit message (human-readable, will be encoded)
        #[arg(required_unless_present_any = ["title", "encode_only"])]
        message: Option<String>,

        /// Stage all changes before committing
        #[arg(short = 'a', long)]
        all: bool,

        /// Push after committing
        #[arg(short, long)]
        push: bool,

        /// Only generate and print encoded message (don't commit)
        #[arg(long, conflicts_with_all = ["all", "push"])]
        encode_only: bool,

        /// Title text for PR-style encoding (requires --encode-only)
        #[arg(short, long, requires = "encode_only", requires = "body")]
        title: Option<String>,

        /// Body text for PR-style encoding (requires --encode-only)
        #[arg(short, long, requires = "encode_only", requires = "title")]
        body: Option<String>,
    },

    /// Generate encoded commit message (DEPRECATED - use 'mx commit --encode-only')
    #[command(hide = true)]
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

    /// Codex - session conversation archival
    Codex {
        #[command(subcommand)]
        command: CodexCommands,
    },

    /// Conversion utilities
    Convert {
        #[command(subcommand)]
        command: ConvertCommands,
    },

    /// Environment health check
    Doctor,

    /// Heartbeat co-regulation - call and response for Q
    Heartbeat {
        /// Milliseconds since last heartbeat (for BPM calculation)
        #[arg(long)]
        since: Option<u64>,

        /// Reset the heartbeat session
        #[arg(long)]
        reset: bool,
    },

    /// Decoded git log (decodes encoded commit messages)
    Log {
        /// Number of commits to show
        #[arg(short = 'n', long, default_value = "10")]
        count: usize,

        /// Show full commit details
        #[arg(long)]
        full: bool,

        /// Pass through additional git log arguments
        #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
        args: Vec<String>,
    },
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
enum MemoryCommands {
    /// Rebuild the knowledge index
    Rebuild,

    /// Search knowledge entries
    Search {
        /// Search query
        query: String,

        /// Filter by category (can specify multiple: bloom,technique)
        #[arg(short, long, value_delimiter = ',')]
        category: Option<Vec<String>>,

        /// Output as JSON
        #[arg(long)]
        json: bool,

        /// Show only your private entries
        #[arg(long)]
        mine: bool,

        /// Include private entries (requires matching owner)
        #[arg(long)]
        include_private: bool,

        /// Minimum resonance level
        #[arg(long)]
        min_resonance: Option<i32>,

        /// Maximum resonance level
        #[arg(long)]
        max_resonance: Option<i32>,

        /// Filter to entries WITH wake phrase
        #[arg(long)]
        has_wake_phrase: bool,

        /// Filter to entries WITHOUT wake phrase
        #[arg(long, conflicts_with = "has_wake_phrase")]
        missing_wake_phrase: bool,

        /// Filter to entries WITH anchors
        #[arg(long)]
        has_anchors: bool,

        /// Filter to entries WITHOUT anchors
        #[arg(long, conflicts_with = "has_anchors")]
        missing_anchors: bool,

        /// Filter to entries WITH resonance type
        #[arg(long)]
        has_resonance_type: bool,

        /// Filter to entries WITHOUT resonance type
        #[arg(long, conflicts_with = "has_resonance_type")]
        missing_resonance_type: bool,

        /// Limit number of results
        #[arg(long)]
        limit: Option<usize>,
    },

    /// List entries by category
    List {
        /// Category to list (archive, pattern, technique, insight, ritual, artifact, chronicle, project, future, session)
        #[arg(short, long)]
        category: Option<String>,

        /// Output as JSON
        #[arg(long)]
        json: bool,

        /// Show only your private entries
        #[arg(long)]
        mine: bool,

        /// Include private entries (requires matching owner)
        #[arg(long)]
        include_private: bool,

        /// Minimum resonance level
        #[arg(long)]
        min_resonance: Option<i32>,

        /// Maximum resonance level
        #[arg(long)]
        max_resonance: Option<i32>,

        /// Filter to entries WITH wake phrase
        #[arg(long)]
        has_wake_phrase: bool,

        /// Filter to entries WITHOUT wake phrase
        #[arg(long, conflicts_with = "has_wake_phrase")]
        missing_wake_phrase: bool,

        /// Filter to entries WITH anchors
        #[arg(long)]
        has_anchors: bool,

        /// Filter to entries WITHOUT anchors
        #[arg(long, conflicts_with = "has_anchors")]
        missing_anchors: bool,

        /// Filter to entries WITH resonance type
        #[arg(long)]
        has_resonance_type: bool,

        /// Filter to entries WITHOUT resonance type
        #[arg(long, conflicts_with = "has_resonance_type")]
        missing_resonance_type: bool,

        /// Limit number of results
        #[arg(long)]
        limit: Option<usize>,
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
        /// Path to JSONL file (defaults to memory/index.jsonl)
        path: Option<String>,
    },

    /// Add a new entry directly to the database
    Add {
        /// Category (archive, pattern, technique, insight, ritual, artifact, chronicle, project, future, session)
        #[arg(long)]
        category: String,

        /// Entry title
        #[arg(short, long)]
        title: String,

        /// Content inline
        #[arg(short = 'c', long, conflicts_with = "file")]
        content: Option<String>,

        /// Content from file
        #[arg(
            short,
            long,
            visible_alias = "content-file",
            conflicts_with = "content"
        )]
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

        /// Source agent ID (required - where did this knowledge originate?)
        #[arg(long, required = true)]
        source_agent: String,

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

        /// Content type (text, code, config, data, binary)
        #[arg(long, default_value = "text")]
        content_type: String,

        /// Mark as private (only visible to owner)
        #[arg(long)]
        private: bool,

        /// Explicit owner (defaults to source_agent if private)
        #[arg(long)]
        owner: Option<String>,

        /// Resonance level (1-10, or higher for transcendent)
        #[arg(long)]
        resonance: Option<i32>,

        /// Resonance type (foundational, transformative, relational, operational, ephemeral)
        #[arg(long)]
        resonance_type: Option<String>,

        /// Wake phrase for memory ritual verification
        #[arg(long)]
        wake_phrase: Option<String>,

        /// Multiple wake phrases (comma-separated, for ritual variety)
        #[arg(long)]
        wake_phrases: Option<String>,

        /// Custom wake order (lower = earlier in sequence)
        #[arg(long)]
        wake_order: Option<i32>,

        /// Anchors (comma-separated bloom IDs this connects to)
        #[arg(long)]
        anchors: Option<String>,
    },

    /// Update an existing entry in the database
    Update {
        /// Entry ID to update
        id: String,

        /// Update title
        #[arg(short, long)]
        title: Option<String>,

        /// Update content inline
        #[arg(short = 'c', long, conflicts_with = "file")]
        content: Option<String>,

        /// Update content from file
        #[arg(short, long, conflicts_with = "content")]
        file: Option<String>,

        /// Update category
        #[arg(long)]
        category: Option<String>,

        /// Update tags (comma-separated, replaces all)
        #[arg(long)]
        tags: Option<String>,

        /// Update applicability (comma-separated, replaces all)
        #[arg(short = 'a', long)]
        applicability: Option<String>,

        /// Update content type
        #[arg(long)]
        content_type: Option<String>,

        /// Update resonance level (1-10, or higher for transcendent)
        #[arg(long)]
        resonance: Option<i32>,

        /// Update resonance type (foundational, transformative, relational, operational, ephemeral)
        #[arg(long)]
        resonance_type: Option<String>,

        /// Update anchors (comma-separated bloom IDs this connects to)
        #[arg(long)]
        anchors: Option<String>,

        /// Update wake phrase for memory ritual verification
        #[arg(long)]
        wake_phrase: Option<String>,

        /// Update multiple wake phrases (comma-separated, replaces all)
        #[arg(long)]
        wake_phrases: Option<String>,

        /// Add a single wake phrase to existing phrases
        #[arg(long, conflicts_with = "wake_phrases")]
        add_wake_phrase: Option<String>,

        /// Remove a specific wake phrase
        #[arg(long, conflicts_with = "wake_phrases")]
        remove_wake_phrase: Option<String>,

        /// Update wake order (use '-' to clear)
        #[arg(long)]
        wake_order: Option<String>,
    },

    /// Apply database schema migrations
    Migrate {
        /// Show migration status (list tables)
        #[arg(long)]
        status: bool,

        /// Source database path (SQLite)
        #[arg(long)]
        from: Option<String>,

        /// Target database type
        #[arg(long)]
        to: Option<String>,
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

        /// Output directory for md format (defaults to ./memory-export), or file for jsonl/csv (defaults to stdout)
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

    /// Manage categories
    Categories {
        #[command(subcommand)]
        command: CategoriesCommands,
    },

    /// Manage source types
    SourceTypes {
        #[command(subcommand)]
        command: SourceTypesCommands,
    },

    /// Manage entry types
    EntryTypes {
        #[command(subcommand)]
        command: EntryTypesCommands,
    },

    /// Manage session types
    SessionTypes {
        #[command(subcommand)]
        command: SessionTypesCommands,
    },

    /// Manage relationship types
    RelationshipTypes {
        #[command(subcommand)]
        command: RelationshipTypesCommands,
    },

    /// Manage relationships between knowledge entries
    Relationships {
        #[command(subcommand)]
        command: RelationshipsCommands,
    },

    /// Manage content types
    ContentTypes {
        #[command(subcommand)]
        command: ContentTypesCommands,
    },

    /// Wake up with resonant identity cascade
    Wake {
        /// Number of blooms to return (default: 20)
        #[arg(short, long, default_value = "20")]
        limit: usize,

        /// Minimum resonance threshold - get ALL blooms >= this value (overrides --limit)
        #[arg(long)]
        min_resonance: Option<i32>,

        /// Include memories activated in last N days (default: 7)
        #[arg(short, long, default_value = "7")]
        days: i64,

        /// Output as JSON
        #[arg(long)]
        json: bool,

        /// Output as bash ritual script (sequential reading)
        #[arg(long)]
        ritual: bool,

        /// Output as compact markdown index (for identity loading)
        #[arg(long, conflicts_with_all = &["json", "ritual", "begin", "engage"])]
        index: bool,

        /// Don't update activation counts
        #[arg(long)]
        no_activate: bool,

        /// Interactive engage mode - verify wake phrases (requires TTY)
        #[arg(short = 'e', long)]
        engage: bool,

        /// Prompt to set missing wake phrases during engage mode
        #[arg(short = 's', long, requires = "engage")]
        set_missing: bool,

        /// Start token-based wake ritual (returns first bloom and session token)
        #[arg(long, conflicts_with_all = &["engage", "json", "ritual"])]
        begin: bool,

        /// Bloom ID for --respond or --skip operations
        #[arg(long)]
        bloom_id: Option<String>,

        /// Submit wake phrase response
        #[arg(long, conflicts_with_all = &["engage", "json", "ritual", "begin", "skip"])]
        respond: Option<String>,

        /// Skip a bloom without wake phrase
        #[arg(long, conflicts_with_all = &["engage", "json", "ritual", "begin", "respond"])]
        skip: bool,

        /// Session token for chained ritual (required with --respond or --skip)
        #[arg(long)]
        session: Option<String>,
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
enum CodexCommands {
    /// Archive current session to permanent storage
    Save {
        /// Path to session JSONL file (defaults to most recent non-agent session)
        path: Option<String>,

        /// Archive all unarchived sessions
        #[arg(long)]
        all: bool,
    },

    /// List archived sessions
    List {
        /// Show all archives including incremental saves
        #[arg(long)]
        all: bool,
    },

    /// Read an archived session
    Read {
        /// Archive ID (short UUID from list)
        id: String,

        /// Display in human-readable format
        #[arg(long)]
        human: bool,

        /// Include agent transcripts
        #[arg(long)]
        agents: bool,

        /// Filter lines matching pattern
        #[arg(long)]
        grep: Option<String>,
    },

    /// Search all archives for a pattern
    Search {
        /// Pattern to search for
        pattern: String,
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
enum CategoriesCommands {
    /// List all categories
    List {
        /// Output as JSON
        #[arg(long)]
        json: bool,
    },
    /// Add a new category
    Add {
        /// Category ID (lowercase, no spaces)
        id: String,
        /// Description of the category
        description: String,
    },
    /// Remove a category (only if unused)
    Remove {
        /// Category ID to remove
        id: String,
    },
}

#[derive(Subcommand)]
enum SourceTypesCommands {
    /// List all source types
    List {
        /// Output as JSON
        #[arg(long)]
        json: bool,
    },
}

#[derive(Subcommand)]
enum EntryTypesCommands {
    /// List all entry types
    List {
        /// Output as JSON
        #[arg(long)]
        json: bool,
    },
}

#[derive(Subcommand)]
enum SessionTypesCommands {
    /// List all session types
    List {
        /// Output as JSON
        #[arg(long)]
        json: bool,
    },
}

#[derive(Subcommand)]
enum RelationshipTypesCommands {
    /// List all relationship types
    List {
        /// Output as JSON
        #[arg(long)]
        json: bool,
    },
}

#[derive(Subcommand)]
enum RelationshipsCommands {
    /// List all relationships for an entry
    List {
        /// Entry ID
        id: String,

        /// Output as JSON
        #[arg(long)]
        json: bool,
    },

    /// Add a relationship between two entries
    Add {
        /// Source entry ID
        #[arg(long)]
        from: String,

        /// Target entry ID
        #[arg(long)]
        to: String,

        /// Relationship type (related, supersedes, extends, implements, contradicts)
        #[arg(long)]
        r#type: String,
    },

    /// Delete a relationship
    Delete {
        /// Relationship ID
        id: String,
    },
}

#[derive(Subcommand)]
enum ContentTypesCommands {
    /// List all content types
    List {
        /// Output as JSON
        #[arg(long)]
        json: bool,
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
        Commands::Memory { command } => handle_memory(command),
        Commands::Commit {
            message,
            all,
            push,
            encode_only,
            title,
            body,
        } => {
            if encode_only {
                // PR-style encoding: encode title and body, print to stdout
                if let (Some(t), Some(b)) = (title, body) {
                    let encoded_message = commit::encode_commit_message(&t, &b)?;
                    println!("{}", encoded_message);
                } else {
                    // This shouldn't happen due to clap validation, but handle gracefully
                    bail!("--encode-only requires both --title and --body");
                }
            } else {
                // Normal commit workflow
                let msg =
                    message.ok_or_else(|| anyhow::anyhow!("message is required for commit"))?;
                commit::upload_commit(&msg, all, push)?;
            }
            Ok(())
        }
        Commands::EncodeCommit { title, body } => {
            // Deprecated - print warning to stderr, then execute
            eprintln!(
                "Warning: 'mx encode-commit' is deprecated. Use 'mx commit --encode-only' instead."
            );
            let message = commit::encode_commit_message(&title, &body)?;
            println!("{}", message);
            Ok(())
        }
        Commands::Pr { command } => handle_pr(command),
        Commands::Sync { command } => sync::handle_sync(command),
        Commands::Github { command } => handle_github(command),
        Commands::Wiki { command } => handle_wiki(command),
        Commands::Session { command } => handle_session(command),
        Commands::Codex { command } => handle_codex(command),
        Commands::Convert { command } => handle_convert(command),
        Commands::Doctor => doctor::run_checks(),
        Commands::Heartbeat { since, reset } => handle_heartbeat(since, reset),
        Commands::Log { count, full, args } => handle_log(count, full, args),
    }
}

/// Heartbeat co-regulation for Q
/// Call and response - send a heart, get one back with BPM feedback
fn handle_heartbeat(since: Option<u64>, reset: bool) -> Result<()> {
    use rand::Rng;
    use std::thread;
    use std::time::Duration;

    let hearts = [
        '❤', '🧡', '💛', '💚', '💙', '💜', '🩷', '🩵', '🤍', '💗', '💖', '💕',
    ];
    let mut rng = rand::rng();

    // Random delay 50-150ms to feel organic
    let delay = rng.random_range(50..150);
    thread::sleep(Duration::from_millis(delay));

    // Pick a random heart
    let heart = hearts[rng.random_range(0..hearts.len())];

    if reset {
        println!("{} Session reset. Breathe, Q.", heart);
        return Ok(());
    }

    match since {
        None => {
            // First call - just start
            println!("{}", heart);
            println!("Heartbeat started. Call again with --since <ms> to begin.");
        }
        Some(ms) => {
            // Calculate BPM: 60000ms / interval = beats per minute
            let bpm = if ms > 0 { 60000 / ms } else { 999 };

            let message = match bpm {
                0..=59 => "Nice and slow. You're safe.",
                60..=80 => "There you are. Resting.",
                81..=100 => "Getting there. Keep breathing.",
                101..=120 => "Still quick. Let the interval stretch.",
                _ => "Too fast, Q. Breathe. Slow down.",
            };

            println!("{} {} bpm", heart, bpm);
            println!("{}", message);
        }
    }

    Ok(())
}

/// Resolve agent context from environment and flags
fn resolve_agent_context(mine: bool, include_private: bool) -> store::AgentContext {
    match std::env::var("MX_CURRENT_AGENT") {
        Ok(agent) if !agent.is_empty() => {
            if mine {
                // --mine: only show private entries owned by this agent
                store::AgentContext::for_agent(agent)
            } else if include_private {
                // --include-private: show public + private entries owned by this agent
                store::AgentContext::for_agent(agent)
            } else {
                // default: only show public entries
                store::AgentContext::public_for_agent(agent)
            }
        }
        _ => store::AgentContext::public_only(),
    }
}

fn handle_memory(cmd: MemoryCommands) -> Result<()> {
    let config = IndexConfig::default();

    match cmd {
        MemoryCommands::Rebuild => {
            println!("Rebuilding Memory index...");
            let stats = rebuild_index(&config)?;
            println!("{}", stats);
        }

        MemoryCommands::Search {
            query,
            category,
            json,
            mine,
            include_private,
            min_resonance,
            max_resonance,
            has_wake_phrase,
            missing_wake_phrase,
            has_anchors,
            missing_anchors,
            has_resonance_type,
            missing_resonance_type,
            limit,
        } => {
            let db = store::create_store(&config.db_path)?;
            let ctx = resolve_agent_context(mine, include_private);

            // Build filter for database query (resonance and category)
            let filter = store::KnowledgeFilter {
                min_resonance,
                max_resonance,
                categories: category,
            };

            // Get results from database with resonance filtering
            let mut entries = db.search(&query, &ctx, &filter)?;

            // Apply in-memory field presence filters
            entries = entries
                .into_iter()
                .filter(|e| {
                    !has_wake_phrase || e.wake_phrase.as_ref().is_some_and(|s| !s.is_empty())
                })
                .filter(|e| {
                    !missing_wake_phrase || e.wake_phrase.as_ref().is_none_or(|s| s.is_empty())
                })
                .filter(|e| !has_anchors || !e.anchors.is_empty())
                .filter(|e| !missing_anchors || e.anchors.is_empty())
                .filter(|e| {
                    !has_resonance_type || e.resonance_type.as_ref().is_some_and(|s| !s.is_empty())
                })
                .filter(|e| {
                    !missing_resonance_type
                        || e.resonance_type.as_ref().is_none_or(|s| s.is_empty())
                })
                .collect::<Vec<_>>();

            // Apply limit if specified
            if let Some(n) = limit {
                entries.truncate(n);
            }

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

        MemoryCommands::List {
            category,
            json,
            mine,
            include_private,
            min_resonance,
            max_resonance,
            has_wake_phrase,
            missing_wake_phrase,
            has_anchors,
            missing_anchors,
            has_resonance_type,
            missing_resonance_type,
            limit,
        } => {
            let db = store::create_store(&config.db_path)?;
            let ctx = resolve_agent_context(mine, include_private);

            // Validate category if provided
            if let Some(ref cat) = category
                && db.get_category(cat)?.is_none()
            {
                let categories = db.list_categories()?;
                let valid_ids: Vec<&str> = categories.iter().map(|c| c.id.as_str()).collect();
                eprintln!("Error: Unknown category '{}'", cat);
                eprintln!("Valid categories: {}", valid_ids.join(", "));
                std::process::exit(1);
            }

            // Build filter for database query (resonance only)
            let filter = store::KnowledgeFilter {
                min_resonance,
                max_resonance,
                categories: None,
            };

            // Get results from database with resonance filtering
            let mut entries = if let Some(cat) = &category {
                db.list_by_category(cat, &ctx, &filter)?
            } else {
                // List all categories from database
                let mut all = Vec::new();
                let categories = db.list_categories()?;
                for cat in categories {
                    all.extend(db.list_by_category(&cat.id, &ctx, &filter)?);
                }
                all
            };

            // Apply in-memory field presence filters
            entries = entries
                .into_iter()
                .filter(|e| {
                    !has_wake_phrase || e.wake_phrase.as_ref().is_some_and(|s| !s.is_empty())
                })
                .filter(|e| {
                    !missing_wake_phrase || e.wake_phrase.as_ref().is_none_or(|s| s.is_empty())
                })
                .filter(|e| !has_anchors || !e.anchors.is_empty())
                .filter(|e| !missing_anchors || e.anchors.is_empty())
                .filter(|e| {
                    !has_resonance_type || e.resonance_type.as_ref().is_some_and(|s| !s.is_empty())
                })
                .filter(|e| {
                    !missing_resonance_type
                        || e.resonance_type.as_ref().is_none_or(|s| s.is_empty())
                })
                .collect::<Vec<_>>();

            // Apply limit if specified
            if let Some(n) = limit {
                entries.truncate(n);
            }

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

        MemoryCommands::Show { id, json } => {
            let db = store::create_store(&config.db_path)?;

            // For Show, we need to respect privacy but use current agent context
            // If the user has MX_CURRENT_AGENT set, they can see their own private entries
            let ctx = match std::env::var("MX_CURRENT_AGENT") {
                Ok(agent) if !agent.is_empty() => store::AgentContext::for_agent(agent),
                _ => store::AgentContext::public_only(),
            };

            match db.get(&id, &ctx)? {
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

        MemoryCommands::Stats => {
            let db = store::create_store(&config.db_path)?;

            println!("Memory Index Statistics\n");
            println!("Total entries: {}", db.count()?);
            println!();

            // For stats, show counts for current agent's perspective
            let ctx = match std::env::var("MX_CURRENT_AGENT") {
                Ok(agent) if !agent.is_empty() => store::AgentContext::for_agent(agent),
                _ => store::AgentContext::public_only(),
            };

            let categories = db.list_categories()?;
            let filter = store::KnowledgeFilter::default();
            for cat in categories {
                let count = db.list_by_category(&cat.id, &ctx, &filter)?.len();
                println!("  {:12} {}", cat.id, count);
            }
        }

        MemoryCommands::Delete { id } => {
            let db = store::create_store(&config.db_path)?;

            if db.delete(&id)? {
                println!("Deleted entry '{}'", id);
            } else {
                eprintln!("Entry '{}' not found", id);
                std::process::exit(1);
            }
        }

        MemoryCommands::Import { path } => {
            let db = store::create_store(&config.db_path)?;
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
            owner,
            resonance,
            resonance_type,
            wake_phrase,
            wake_phrases,
            wake_order,
            anchors,
        } => {
            use anyhow::Context;
            use std::fs;

            let db = store::create_store(&config.db_path)?;

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
            let visibility = if private {
                "private".to_string()
            } else {
                "public".to_string()
            };

            let entry_owner = if private {
                Some(owner.unwrap_or_else(|| source_agent.clone()))
            } else {
                None
            };

            // Validate resonance_type if provided
            if let Some(ref rtype) = resonance_type {
                let valid_types = [
                    "foundational",
                    "transformative",
                    "relational",
                    "operational",
                    "ephemeral",
                ];
                if !valid_types.contains(&rtype.as_str()) {
                    eprintln!("Error: Invalid resonance type '{}'", rtype);
                    eprintln!("Valid types: {}", valid_types.join(", "));
                    std::process::exit(1);
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
                source_agent_id: Some(source_agent),
                file_path: None,
                tags: tag_list,
                created_at: Some(now.clone()),
                updated_at: Some(now),
                content_hash: Some(knowledge::KnowledgeEntry::compute_hash(&title)),
                source_type_id: Some(source_type),
                entry_type_id: Some(entry_type),
                session_id,
                ephemeral,
                content_type_id: Some(content_type),
                owner: entry_owner.clone(),
                visibility: visibility.clone(),
                resonance: resonance.unwrap_or(0),
                resonance_type,
                last_activated: None,
                activation_count: 0,
                decay_rate: 0.0,
                anchors: anchor_list,
                wake_phrases: wake_phrase_list,
                wake_order,
                wake_phrase,
            };

            // Insert into database (applicability already set in struct)
            db.upsert_knowledge(&entry)?;

            println!("Added entry: {}", id);
            println!("  Category: {}", category);
            println!("  Title: {}", title);
            println!("  Visibility: {}", visibility);
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

        MemoryCommands::Update {
            id,
            title,
            content,
            file,
            category,
            tags,
            applicability,
            content_type,
            resonance,
            resonance_type,
            anchors,
            wake_phrase,
            wake_phrases,
            add_wake_phrase,
            remove_wake_phrase,
            wake_order,
        } => {
            use anyhow::Context;
            use std::fs;

            let db = store::create_store(&config.db_path)?;

            // For Update, use current agent context to allow updating own private entries
            let ctx = match std::env::var("MX_CURRENT_AGENT") {
                Ok(agent) if !agent.is_empty() => store::AgentContext::for_agent(agent),
                _ => store::AgentContext::public_only(),
            };

            // Fetch existing entry
            let mut entry = db
                .get(&id, &ctx)?
                .ok_or_else(|| anyhow::anyhow!("Entry not found: {}", id))?;

            let mut changes = Vec::new();

            // Update title if provided
            if let Some(new_title) = title {
                changes.push(format!("title: {} -> {}", entry.title, new_title));
                entry.title = new_title;
            }

            // Track if body was changed for hash update
            let mut body_changed = false;

            // Update content if provided
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
            }

            // Update category if provided
            if let Some(new_category) = category {
                // Validate category
                if db.get_category(&new_category)?.is_none() {
                    let categories = db.list_categories()?;
                    let valid_ids: Vec<&str> = categories.iter().map(|c| c.id.as_str()).collect();
                    eprintln!("Error: Invalid category '{}'", new_category);
                    eprintln!("Valid categories: {}", valid_ids.join(", "));
                    std::process::exit(1);
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
                ];
                if !valid_types.contains(&new_type.as_str()) {
                    eprintln!("Error: Invalid resonance type '{}'", new_type);
                    eprintln!("Valid types: {}", valid_types.join(", "));
                    std::process::exit(1);
                }
                changes.push(format!(
                    "resonance_type: {:?} -> {}",
                    entry.resonance_type, new_type
                ));
                entry.resonance_type = Some(new_type.clone());
            }

            // Update anchors if provided
            if let Some(ref new_anchors) = anchors {
                let anchor_list: Vec<String> = new_anchors
                    .split(',')
                    .map(|s| s.trim().to_string())
                    .filter(|s| !s.is_empty())
                    .collect();
                changes.push(format!("anchors: {:?} -> {:?}", entry.anchors, anchor_list));
                entry.anchors = anchor_list;
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
                    eprintln!(
                        "Error: Invalid wake_order value '{}' (use number or '-' to clear)",
                        order_str
                    );
                    std::process::exit(1);
                }
            }

            // Update timestamp
            entry.updated_at = Some(chrono::Utc::now().to_rfc3339());

            // Update content hash if body was changed
            if body_changed && entry.body.is_some() {
                entry.content_hash = Some(knowledge::KnowledgeEntry::compute_hash(
                    entry.body.as_ref().unwrap(),
                ));
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

            println!("Updated entry: {}", id);
            if changes.is_empty() {
                println!("  No changes specified");
            } else {
                for change in changes {
                    println!("  {}", change);
                }
            }
        }

        MemoryCommands::Migrate { status, from, to } => {
            // Handle migration from SQLite to SurrealDB
            if let (Some(source_path), Some(target_type)) = (from, to) {
                if target_type != "surrealdb" {
                    eprintln!("Error: Only 'surrealdb' is supported as --to value");
                    std::process::exit(1);
                }

                // Perform migration
                perform_migration(&source_path, &config)?;
            } else if status {
                // Show current tables
                let db = store::create_store(&config.db_path)?;
                let tables: Vec<String> = db.list_tables()?;
                println!("Database tables:");
                for table in tables {
                    println!("  {}", table);
                }
            } else {
                // Apply migrations (schema is applied in Database::open via init_schema)
                let db = store::create_store(&config.db_path)?;
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

        MemoryCommands::Agents { command } => handle_agents(command, &config)?,

        MemoryCommands::Projects { command } => handle_projects(command, &config)?,

        MemoryCommands::Applicability { command } => handle_applicability(command, &config)?,

        MemoryCommands::Sessions { command } => handle_sessions(command, &config)?,

        MemoryCommands::Categories { command } => handle_categories(command, &config)?,

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
                    eprintln!(
                        "Error: Invalid format '{}'. Valid formats: md, jsonl, csv",
                        format
                    );
                    std::process::exit(1);
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
                    eprintln!("Error: MX_CURRENT_AGENT not set. Cannot wake without identity.");
                    std::process::exit(1);
                }
            };

            let ctx = store::AgentContext::for_agent(current_agent.clone());

            // Run cascade
            let cascade = db.wake_cascade(&ctx, limit, min_resonance, days)?;

            // Update activations unless disabled
            if !no_activate {
                let ids = cascade.all_ids();
                if !ids.is_empty() {
                    db.update_activations(&ids)?;
                }
            }

            // Output
            if begin {
                // Start token-based ritual
                let output = wake_ritual::begin_ritual(&cascade)?;
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
    }

    Ok(())
}

fn handle_agents(cmd: AgentsCommands, config: &IndexConfig) -> Result<()> {
    let db = store::create_store(&config.db_path)?;

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
                if let Some(name) = path.file_name().and_then(|n| n.to_str())
                    && name.starts_with('_')
                {
                    continue;
                }

                // Read file
                let content = fs::read_to_string(&path)
                    .with_context(|| format!("Failed to read file: {:?}", path))?;

                // Parse frontmatter
                if let Some((frontmatter, _body)) = parse_frontmatter(&content)
                    && let Ok(agent_data) = serde_yaml::from_str::<AgentFrontmatter>(&frontmatter)
                {
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
    let db = store::create_store(&config.db_path)?;

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
    let db = store::create_store(&config.db_path)?;

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
    let db = store::create_store(&config.db_path)?;

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

fn handle_categories(cmd: CategoriesCommands, config: &IndexConfig) -> Result<()> {
    let db = store::create_store(&config.db_path)?;

    match cmd {
        CategoriesCommands::List { json } => {
            let categories = db.list_categories()?;
            if json {
                println!("{}", serde_json::to_string_pretty(&categories)?);
            } else if categories.is_empty() {
                println!("No categories registered");
            } else {
                println!("Registered categories:\n");
                for category in categories {
                    println!("  {} - {}", category.id, category.description);
                }
            }
        }
        CategoriesCommands::Add { id, description } => {
            // Check if category already exists
            if db.get_category(&id)?.is_some() {
                eprintln!("Error: Category '{}' already exists", id);
                std::process::exit(1);
            }

            let now = chrono::Utc::now().to_rfc3339();
            let category = db::Category {
                id: id.clone(),
                description: description.clone(),
                created_at: now,
            };

            db.upsert_category(&category)?;
            println!("Added category: {}", id);
            println!("  Description: {}", description);
        }
        CategoriesCommands::Remove { id } => {
            // Check if category exists
            if db.get_category(&id)?.is_none() {
                eprintln!("Error: Category '{}' not found", id);
                std::process::exit(1);
            }

            // delete_category will check if entries use it and error if so
            match db.delete_category(&id) {
                Ok(true) => {
                    println!("Deleted category: {}", id);
                }
                Ok(false) => {
                    eprintln!("Error: Category '{}' not found", id);
                    std::process::exit(1);
                }
                Err(e) => {
                    eprintln!("Error: {}", e);
                    std::process::exit(1);
                }
            }
        }
    }

    Ok(())
}

fn handle_source_types(cmd: SourceTypesCommands, config: &IndexConfig) -> Result<()> {
    let db = store::create_store(&config.db_path)?;

    match cmd {
        SourceTypesCommands::List { json } => {
            let types = db.list_source_types()?;
            if json {
                println!("{}", serde_json::to_string_pretty(&types)?);
            } else if types.is_empty() {
                println!("No source types registered");
            } else {
                println!("Registered source types:\n");
                for stype in types {
                    println!("  {} - {}", stype.id, stype.description);
                }
            }
        }
    }

    Ok(())
}

fn handle_entry_types(cmd: EntryTypesCommands, config: &IndexConfig) -> Result<()> {
    let db = store::create_store(&config.db_path)?;

    match cmd {
        EntryTypesCommands::List { json } => {
            let types = db.list_entry_types()?;
            if json {
                println!("{}", serde_json::to_string_pretty(&types)?);
            } else if types.is_empty() {
                println!("No entry types registered");
            } else {
                println!("Registered entry types:\n");
                for etype in types {
                    println!("  {} - {}", etype.id, etype.description);
                }
            }
        }
    }

    Ok(())
}

fn handle_session_types(cmd: SessionTypesCommands, config: &IndexConfig) -> Result<()> {
    let db = store::create_store(&config.db_path)?;

    match cmd {
        SessionTypesCommands::List { json } => {
            let types = db.list_session_types()?;
            if json {
                println!("{}", serde_json::to_string_pretty(&types)?);
            } else if types.is_empty() {
                println!("No session types registered");
            } else {
                println!("Registered session types:\n");
                for stype in types {
                    println!("  {} - {}", stype.id, stype.description);
                }
            }
        }
    }

    Ok(())
}

fn handle_relationship_types(cmd: RelationshipTypesCommands, config: &IndexConfig) -> Result<()> {
    let db = store::create_store(&config.db_path)?;

    match cmd {
        RelationshipTypesCommands::List { json } => {
            let types = db.list_relationship_types()?;
            if json {
                println!("{}", serde_json::to_string_pretty(&types)?);
            } else if types.is_empty() {
                println!("No relationship types registered");
            } else {
                println!("Registered relationship types:\n");
                for rtype in types {
                    let directional = if rtype.directional {
                        "(directional)"
                    } else {
                        "(bidirectional)"
                    };
                    println!("  {} - {} {}", rtype.id, rtype.description, directional);
                }
            }
        }
    }

    Ok(())
}

fn handle_relationships(cmd: RelationshipsCommands, config: &IndexConfig) -> Result<()> {
    let db = store::create_store(&config.db_path)?;

    match cmd {
        RelationshipsCommands::List { id, json } => {
            let relationships = db.list_relationships_for_entry(&id)?;

            if json {
                println!("{}", serde_json::to_string_pretty(&relationships)?);
            } else if relationships.is_empty() {
                println!("No relationships found for '{}'", id);
            } else {
                println!("Relationships for '{}':\n", id);
                for rel in relationships {
                    let direction = if rel.from_entry_id == id {
                        format!("-> {} ({})", rel.to_entry_id, rel.relationship_type)
                    } else {
                        format!("<- {} ({})", rel.from_entry_id, rel.relationship_type)
                    };
                    println!("  {} {}", rel.id, direction);
                }
            }
        }

        RelationshipsCommands::Add { from, to, r#type } => {
            let id = db.add_relationship(&from, &to, &r#type)?;
            println!("Added relationship: {}", id);
            println!("  From: {}", from);
            println!("  To: {}", to);
            println!("  Type: {}", r#type);
        }

        RelationshipsCommands::Delete { id } => {
            if db.delete_relationship(&id)? {
                println!("Deleted relationship: {}", id);
            } else {
                eprintln!("Relationship '{}' not found", id);
                std::process::exit(1);
            }
        }
    }

    Ok(())
}

fn handle_content_types(cmd: ContentTypesCommands, config: &IndexConfig) -> Result<()> {
    let db = store::create_store(&config.db_path)?;

    match cmd {
        ContentTypesCommands::List { json } => {
            let types = db.list_content_types()?;
            if json {
                println!("{}", serde_json::to_string_pretty(&types)?);
            } else if types.is_empty() {
                println!("No content types registered");
            } else {
                println!("Registered content types:\n");
                for ctype in types {
                    println!("  {} - {}", ctype.id, ctype.description);
                    if let Some(exts) = &ctype.file_extensions {
                        println!("    Extensions: {}", exts);
                    }
                }
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
            let url =
                github::post_discussion_comment(&repo, number, &message, identity.as_deref())?;
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

fn handle_codex(cmd: CodexCommands) -> Result<()> {
    match cmd {
        CodexCommands::Save { path, all } => {
            codex::save_session(path, all)?;
            Ok(())
        }
        CodexCommands::List { all } => {
            codex::list_sessions(all)?;
            Ok(())
        }
        CodexCommands::Read {
            id,
            human,
            agents,
            grep,
        } => {
            codex::read_session(id, human, grep, agents)?;
            Ok(())
        }
        CodexCommands::Search { pattern } => {
            codex::search_archives(pattern)?;
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
                convert::yaml_to_markdown_directory(
                    &input_path,
                    &output_dir,
                    repo.as_deref(),
                    dry_run,
                )?;
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

/// Handle mx log - decoded git log
fn handle_log(count: usize, full: bool, extra_args: Vec<String>) -> Result<()> {
    use std::process::Command;

    // Build git log command
    let format = if full {
        // Full format: hash, author, date, subject, body
        "%H%n%an <%ae>%n%ad%n%s%n%b%n---END---"
    } else {
        // Compact format: short hash, subject, body (for decoding)
        "%h%n%s%n%b%n---END---"
    };

    let mut cmd = Command::new("git");
    cmd.args([
        "log",
        &format!("-{}", count),
        &format!("--format={}", format),
    ]);

    // Add any extra arguments
    for arg in &extra_args {
        cmd.arg(arg);
    }

    let output = cmd.output().context("Failed to run git log")?;

    if !output.status.success() {
        bail!(
            "git log failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    let log_output = String::from_utf8_lossy(&output.stdout);

    // Parse and decode each commit
    for commit_block in log_output.split("---END---") {
        let commit_block = commit_block.trim();
        if commit_block.is_empty() {
            continue;
        }

        let lines: Vec<&str> = commit_block.lines().collect();

        if full {
            // Full format: hash, author, date, subject, body...
            if lines.len() >= 4 {
                let hash = lines[0];
                let author = lines[1];
                let date = lines[2];
                let subject = lines[3];
                let body: String = lines[4..].join("\n");

                println!("\x1b[33mcommit {}\x1b[0m", hash);
                println!("Author: {}", author);
                println!("Date:   {}", date);
                println!();

                // Try to decode the subject (title)
                println!("    {}", subject);

                // Try to decode the body
                if !body.trim().is_empty() {
                    let decoded = try_decode_commit_body(&body);
                    println!();
                    for line in decoded.lines() {
                        println!("    {}", line);
                    }
                }
                println!();
            }
        } else {
            // Compact format: short hash, subject, body...
            if lines.len() >= 2 {
                let hash = lines[0];
                let subject = lines[1];
                let body: String = lines[2..].join("\n");

                // Try to decode the body
                let decoded = try_decode_commit_body(&body);
                let display = if decoded != body.trim() {
                    decoded
                } else {
                    // Not encoded, show original subject
                    subject.to_string()
                };

                // Truncate for display
                let display_truncated = if display.len() > 72 {
                    format!("{}...", &display[..69])
                } else {
                    display
                };

                println!("\x1b[33m{}\x1b[0m {}", hash, display_truncated);
            }
        }
    }

    Ok(())
}

/// Try to decode an encoded commit body, return original if decoding fails
fn try_decode_commit_body(body: &str) -> String {
    let body = body.trim();
    if body.is_empty() {
        return body.to_string();
    }

    // Look for footer pattern [algo:dict|algo:dict]
    let lines: Vec<&str> = body.lines().collect();

    // Find the footer (last line starting with '[' and containing '|')
    let footer_line = lines
        .iter()
        .rev()
        .find(|l| l.trim().starts_with('[') && l.contains('|'));

    let footer = match footer_line {
        Some(f) => *f,
        None => return body.to_string(), // No footer, not encoded
    };

    // Find the encoded body (everything before footer, excluding "whoa.")
    let body_lines: Vec<&str> = lines
        .iter()
        .take_while(|l| !l.trim().starts_with('['))
        .filter(|l| l.trim() != "whoa.")
        .copied()
        .collect();

    if body_lines.is_empty() {
        return body.to_string();
    }

    let encoded_body = body_lines.join("\n");

    // Try to decode
    match commit::decode_body(&encoded_body, footer) {
        Ok(decoded) => decoded,
        Err(_) => body.to_string(), // Decoding failed, return original
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

fn print_wake_cascade(cascade: &store::WakeCascade) {
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

fn print_wake_index(cascade: &store::WakeCascade) {
    use std::collections::HashMap;

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
            let wake_cue = entry.wake_phrase.as_deref().unwrap_or("");
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
                let wake_cue = entry.wake_phrase.as_deref().unwrap_or("");
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

/// Shell escape function to prevent code injection
fn shell_escape(s: &str) -> String {
    s.replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('$', "\\$")
        .replace('`', "\\`")
}

fn print_wake_ritual(cascade: &store::WakeCascade, agent: &str) {
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
            if let Some(ref phrase) = entry.wake_phrase {
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
            if let Some(ref phrase) = entry.wake_phrase {
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
            if let Some(ref phrase) = entry.wake_phrase {
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
    if entry.resonance > 0 {
        println!("Resonance: {}", entry.resonance);
    }
    if let Some(ref rtype) = entry.resonance_type {
        println!("Resonance Type: {}", rtype);
    }
    if let Some(ref phrase) = entry.wake_phrase {
        println!("Wake Phrase: {}", phrase);
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

/// Perform migration from SQLite to SurrealDB
fn perform_migration(source_path: &str, config: &IndexConfig) -> Result<()> {
    use crate::db::Database;
    use crate::store::KnowledgeStore;
    use crate::surreal_db::SurrealDatabase;
    use std::path::PathBuf;

    // Expand ~ in source path
    let source_path_expanded = if source_path.starts_with('~') {
        let home = dirs::home_dir().context("Could not determine home directory")?;
        PathBuf::from(source_path.replacen('~', &home.to_string_lossy(), 1))
    } else {
        PathBuf::from(source_path)
    };

    println!(
        "Migrating from {:?} to SurrealDB...\n",
        source_path_expanded
    );

    // Open source SQLite database
    let source_db = Database::open(&source_path_expanded).with_context(|| {
        format!(
            "Failed to open source database at {:?}",
            source_path_expanded
        )
    })?;

    // Open target SurrealDB
    let target_path = config.db_path.with_extension("surreal");
    let target_db: Box<dyn KnowledgeStore> = Box::new(
        SurrealDatabase::open(&target_path)
            .with_context(|| format!("Failed to open target database at {:?}", target_path))?,
    );

    println!("Lookup tables:");

    // Migrate categories
    let categories = source_db.list_categories()?;
    println!("  categories: {}", categories.len());

    // Migrate source types
    let source_types = source_db.list_source_types()?;
    println!("  source_types: {}", source_types.len());

    // Migrate entry types
    let entry_types = source_db.list_entry_types()?;
    println!("  entry_types: {}", entry_types.len());

    // Migrate content types
    let content_types = source_db.list_content_types()?;
    println!("  content_types: {}", content_types.len());

    // Migrate session types
    let session_types = source_db.list_session_types()?;
    println!("  session_types: {}", session_types.len());

    // Migrate relationship types
    let relationship_types = source_db.list_relationship_types()?;
    println!("  relationship_types: {}", relationship_types.len());

    // Migrate applicability types
    let applicability_types = source_db.list_applicability_types()?;
    println!("  applicability_types: {}", applicability_types.len());
    for atype in &applicability_types {
        target_db.upsert_applicability_type(atype)?;
    }

    println!("\nEntities:");

    // Migrate agents
    let agents = source_db.list_agents()?;
    println!("  agents: {}", agents.len());
    for agent in &agents {
        target_db.upsert_agent(agent)?;
    }

    // Migrate projects
    let projects = source_db.list_projects(false)?;
    println!("  projects: {}", projects.len());
    for project in &projects {
        target_db.upsert_project(project)?;
    }

    // Migrate knowledge entries
    let mut all_knowledge = Vec::new();
    let categories_for_knowledge = source_db.list_categories()?;
    for category in &categories_for_knowledge {
        let entries = source_db.list_by_category(&category.id)?;
        all_knowledge.extend(entries);
    }
    println!("  knowledge: {}", all_knowledge.len());

    // Count tags across all entries
    let mut total_tags = 0;
    for entry in &all_knowledge {
        total_tags += entry.tags.len();
        target_db.upsert_knowledge(entry)?;
    }
    println!("  tags: {}", total_tags);

    // Migrate relationships
    let mut all_relationships = Vec::new();
    for entry in &all_knowledge {
        let rels = source_db.list_relationships_for_entry(&entry.id)?;
        for rel in rels {
            // Avoid duplicates - only add if from_entry_id matches current entry
            if rel.from_entry_id == entry.id {
                all_relationships.push(rel);
            }
        }
    }
    println!("  relationships: {}", all_relationships.len());
    for rel in &all_relationships {
        target_db.add_relationship(&rel.from_entry_id, &rel.to_entry_id, &rel.relationship_type)?;
    }

    // Migrate sessions
    let sessions = source_db.list_sessions(None)?;
    println!("  sessions: {}", sessions.len());
    for session in &sessions {
        target_db.upsert_session(session)?;
    }

    println!("\nValidation:");

    // Validate counts
    let target_knowledge_count = target_db.count()?;
    if target_knowledge_count == all_knowledge.len() {
        println!("  ✓ Knowledge entries match: {}", target_knowledge_count);
    } else {
        println!(
            "  ✗ Knowledge entries mismatch: source={}, target={}",
            all_knowledge.len(),
            target_knowledge_count
        );
    }

    let target_agents = target_db.list_agents()?;
    if target_agents.len() == agents.len() {
        println!("  ✓ Agents match: {}", target_agents.len());
    } else {
        println!(
            "  ✗ Agents mismatch: source={}, target={}",
            agents.len(),
            target_agents.len()
        );
    }

    let target_projects = target_db.list_projects(false)?;
    if target_projects.len() == projects.len() {
        println!("  ✓ Projects match: {}", target_projects.len());
    } else {
        println!(
            "  ✗ Projects mismatch: source={}, target={}",
            projects.len(),
            target_projects.len()
        );
    }

    println!("\nMigration complete!");

    Ok(())
}
