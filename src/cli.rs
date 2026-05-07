use clap::{Parser, Subcommand, ValueEnum};

#[derive(Parser)]
#[command(name = "mx")]
#[command(about = "Tsunderground CLI - memory, workflow, and identity tooling")]
#[command(version)]
pub struct Cli {
    /// Enable verbose output (show connection logs)
    #[arg(short = 'v', long, global = true)]
    pub verbose: bool,

    #[command(subcommand)]
    pub command: Commands,
}

#[derive(Subcommand)]
#[allow(clippy::large_enum_variant)]
pub enum Commands {
    /// Knowledge base operations (CRUD, search, wake, facts)
    Memory {
        #[command(subcommand)]
        command: MemoryCommands,
    },

    /// Create an encoded git commit
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

        /// Show the full encoded commit fields (Title/Body/Dejavu/Footer).
        /// Default output is just the footer line and `Committed.`
        #[arg(long, conflicts_with = "encode_only", conflicts_with_all = ["title", "body"])]
        show_encoded: bool,

        /// Preview what would be committed without mutating git state.
        /// Runs all encoding/validation logic but skips git commit and push.
        /// Output is prefixed with `[dry-run]`.
        #[arg(long, conflicts_with = "encode_only", conflicts_with_all = ["title", "body"])]
        dry_run: bool,
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

    /// Decoded git show (transparently decodes encoded commit messages)
    Show {
        /// Arguments passed to git show
        #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
        args: Vec<String>,
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

    /// Emotional state tensor operations
    State {
        #[command(subcommand)]
        command: StateCommands,
    },

    /// Local key-value store for fast agent state
    Kv {
        #[command(subcommand)]
        command: KvCommands,
    },
}

#[derive(Subcommand)]
pub enum ConvertCommands {
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
pub enum StateCommands {
    /// Encode state tensor from dimensional values
    Encode {
        /// Pipe-separated values (e.g., "0.3|0.2|0.7|0.8|0.4")
        values: Option<String>,

        /// Named dimension values (e.g., "temp=0.8 entropy=0.75 agency=0.4")
        #[arg(short = 'd', long, conflicts_with = "values", conflicts_with = "file")]
        dimensions: Option<String>,

        /// Read values from file (one value per line or pipe-separated)
        #[arg(short, long, conflicts_with = "values")]
        file: Option<String>,

        /// Schema ID or path (defaults to "tensor")
        #[arg(short, long)]
        schema: Option<String>,

        /// Interactive guided mode - walks through dimensions with anchors
        #[arg(short = 'g', long)]
        guided: bool,

        /// Output format: tensor (default), json, human, bootstrap
        #[arg(short = 'F', long, default_value = "tensor")]
        format: String,

        /// Include runes in output
        #[arg(long)]
        runes: bool,
    },

    /// Decode state tensor to human-readable format
    Decode {
        /// Encoded tensor string (e.g., "@state:crewu|0.3|0.2|...")
        input: Option<String>,

        /// Schema ID or path (inferred from input if not specified)
        #[arg(short, long)]
        schema: Option<String>,

        /// Output format: human (default), json, tensor, mood
        #[arg(short = 'F', long, default_value = "human")]
        format: String,
    },

    /// List available schemas
    Schemas {
        /// Output as JSON
        #[arg(long)]
        json: bool,
    },

    /// List moods for a schema
    Moods {
        /// Schema ID or path (defaults to "tensor")
        #[arg(short, long)]
        schema: Option<String>,

        /// Show specific mood details
        mood: Option<String>,

        /// Output as JSON
        #[arg(long)]
        json: bool,
    },

    /// Show schema information (dimensions, moods)
    Info {
        /// Schema ID or path (defaults to "tensor")
        #[arg(short, long)]
        schema: Option<String>,

        /// Output as JSON
        #[arg(long)]
        json: bool,
    },
}

#[derive(Subcommand)]
pub enum PrCommands {
    /// Merge a pull request with encoded commit message
    Merge {
        /// PR number
        number: u32,

        /// Use rebase merge instead of squash (mutually exclusive with --merge-commit)
        #[arg(long, conflicts_with = "merge")]
        rebase: bool,

        /// Use standard merge commit instead of squash (mutually exclusive with --rebase)
        #[arg(long, name = "merge", conflicts_with = "rebase")]
        merge_commit: bool,

        /// Skip post-merge cleanup (don't switch to target branch or delete local source branch)
        #[arg(long)]
        no_cleanup: bool,
    },
}

#[derive(Subcommand)]
pub enum SyncCommands {
    /// Pull issues/discussions from GitHub to local YAML
    Pull {
        /// Repository (owner/repo format)
        repo: String,

        /// Output directory (defaults to $MX_HOME/cache/sync/<repo>)
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

        /// Input directory (defaults to $MX_HOME/cache/sync/<repo>)
        #[arg(short, long)]
        input: Option<String>,

        /// Dry run - show what would be pushed
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

/// Shared filter flags for search/list commands (extracted from duplicated definitions)
#[derive(Debug, Clone, clap::Args)]
pub struct EntryFilter {
    /// Filter by category (comma-separated, see 'mx memory categories list' for valid names)
    #[arg(short, long, value_delimiter = ',')]
    pub category: Option<Vec<String>>,

    /// Output as JSON
    #[arg(long)]
    pub json: bool,

    /// Show only your private entries
    #[arg(long)]
    pub mine: bool,

    /// Include private entries (requires matching owner)
    #[arg(long)]
    pub include_private: bool,

    /// Minimum resonance level
    #[arg(long)]
    pub min_resonance: Option<i32>,

    /// Maximum resonance level
    #[arg(long)]
    pub max_resonance: Option<i32>,

    /// Filter to entries WITH wake phrase
    #[arg(long)]
    pub has_wake_phrase: bool,

    /// Filter to entries WITHOUT wake phrase
    #[arg(long, conflicts_with = "has_wake_phrase")]
    pub missing_wake_phrase: bool,

    /// Filter to entries WITH anchors
    #[arg(long)]
    pub has_anchors: bool,

    /// Filter to entries WITHOUT anchors
    #[arg(long, conflicts_with = "has_anchors")]
    pub missing_anchors: bool,

    /// Filter to entries WITH resonance type
    #[arg(long)]
    pub has_resonance_type: bool,

    /// Filter to entries WITHOUT resonance type
    #[arg(long, conflicts_with = "has_resonance_type")]
    pub missing_resonance_type: bool,

    /// Limit number of results
    #[arg(long)]
    pub limit: Option<usize>,

    /// Filter by tags (can specify multiple: focus,rust) (matches any)
    #[arg(long, value_delimiter = ',')]
    pub tags: Option<Vec<String>>,
}

/// Sort order for `memory recent` results.
#[derive(Clone, Debug, ValueEnum)]
pub enum RecentSortOrder {
    /// Sort by creation time (most recent first)
    Chronological,
    /// Sort by effective resonance (highest first, decay-adjusted)
    Resonance,
}

#[derive(Subcommand)]
pub enum MemoryCommands {
    /// Removed -- see follow-up for markdown ingest plans
    ///
    /// The export-then-edit-then-rebuild flow has no users on this codebase.
    /// A doctor command (`mx doctor memory rebuild`) covers the
    /// "export-wipe-reimport" recovery use case in a follow-up.
    /// TODO(legacy-state-cleanup): remove command stub after one release cycle.
    #[command(hide = true)]
    Rebuild,

    /// Seed memory from on-disk artifacts (agents, knowledge, ...)
    Seed {
        #[command(subcommand)]
        command: MemorySeedCommands,
    },

    /// Search knowledge entries
    Search {
        /// Search query
        query: String,

        #[command(flatten)]
        filter: EntryFilter,

        /// Use semantic (vector) search instead of keyword search
        #[arg(long)]
        semantic: bool,
    },

    /// List entries by category
    List {
        #[command(flatten)]
        filter: EntryFilter,
    },

    /// Show a specific entry
    Show {
        /// Entry ID
        id: String,

        /// Output as JSON
        #[arg(long)]
        json: bool,

        /// Output only the body content (for piping)
        #[arg(long)]
        content_only: bool,
    },

    /// Show index statistics
    Stats {
        /// Output as JSON
        #[arg(long)]
        json: bool,
    },

    /// Show graph health vitality percentages (embedding coverage, anchor coverage, stale high-res entries)
    Health {
        /// Output as JSON (default format for dashboard consumers)
        #[arg(long)]
        json: bool,
    },

    /// Show per-week entry growth over the last 8 weeks as a JSON array
    Growth {
        /// Output as JSON array of 8 integers (oldest to newest)
        #[arg(long)]
        json: bool,
    },

    /// List open threads (category:thread entries with state="open" or no state)
    OpenThreads {
        /// Output as JSON array (required for dashboard consumers)
        #[arg(long)]
        json: bool,
    },

    /// Delete an entry from the index
    Delete {
        /// Entry ID to delete
        id: String,

        /// Output as JSON
        #[arg(long)]
        json: bool,
    },

    /// [Removed] Import entries from JSONL file. Use `mx memory seed knowledge` instead.
    ///
    /// TODO(legacy-state-cleanup): remove stub after one release cycle.
    #[command(hide = true)]
    Import {
        /// Path to JSONL file
        path: Option<String>,
    },

    /// Add a new entry directly to the database
    Add {
        /// Category name (run 'mx memory categories list' to see available categories)
        /// When --type is provided, category is auto-determined from fact type routing
        #[arg(long, required_unless_present = "type")]
        category: Option<String>,

        /// Entry title (auto-generated from content when --type is provided)
        #[arg(short, long, required_unless_present = "type")]
        title: Option<String>,

        /// Content inline
        #[arg(long, conflicts_with = "file")]
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

        /// Source agent ID (defaults to MX_CURRENT_AGENT env var)
        #[arg(long)]
        source_agent: Option<String>,

        /// Source type (manual, ram, cache, agent_session)
        #[arg(long, default_value = "manual")]
        source_type: String,

        /// Entry type (primary, summary, synthesis)
        #[arg(long, default_value = "primary")]
        entry_type: String,

        /// Session ID (for regular entries)
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

        /// Mark as private (only visible to owner) - shorthand for --visibility private
        #[arg(long, conflicts_with = "visibility")]
        private: bool,

        /// Set visibility (public or private)
        #[arg(long, conflicts_with = "private")]
        visibility: Option<String>,

        /// Explicit owner (defaults to source_agent or MX_CURRENT_AGENT if private)
        #[arg(long)]
        owner: Option<String>,

        /// Output as JSON
        #[arg(long)]
        json: bool,

        /// Resonance level (1-10, or higher for transcendent)
        #[arg(long)]
        resonance: Option<i32>,

        /// Resonance type (foundational, transformative, relational, operational, ephemeral, session)
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

        /// Fact type for ephemeral knowledge (decision, insight, person, quote, thread_opened, commitment, thread_closed)
        /// Routes to appropriate category and sets resonance_type=ephemeral
        #[arg(long = "type")]
        r#type: Option<String>,

        /// Session to link fact to via EXTRACTED_FROM relationship (requires --type)
        #[arg(long, requires = "type")]
        session: Option<String>,

        /// Thread ID for thread_closed operations (requires --type=thread_closed)
        #[arg(long, requires = "type")]
        thread_id: Option<String>,
    },

    /// Update an existing entry in the database
    Update {
        /// Entry ID to update
        id: String,

        /// Update title
        #[arg(short, long)]
        title: Option<String>,

        /// Replace content inline (full replacement)
        #[arg(long, conflicts_with_all = ["file", "append_content", "append_file", "prepend_content", "prepend_file", "find"])]
        content: Option<String>,

        /// Replace content from file (full replacement)
        #[arg(short, long, visible_alias = "content-file", conflicts_with_all = ["content", "append_content", "append_file", "prepend_content", "prepend_file", "find"])]
        file: Option<String>,

        /// Append text to end of existing content
        #[arg(long, conflicts_with_all = ["content", "file", "append_file", "prepend_content", "prepend_file", "find"])]
        append_content: Option<String>,

        /// Append content from file to end of existing content
        #[arg(long, conflicts_with_all = ["content", "file", "append_content", "prepend_content", "prepend_file", "find"])]
        append_file: Option<String>,

        /// Prepend text to start of existing content
        #[arg(long, conflicts_with_all = ["content", "file", "append_content", "append_file", "prepend_file", "find"])]
        prepend_content: Option<String>,

        /// Prepend content from file to start of existing content
        #[arg(long, conflicts_with_all = ["content", "file", "append_content", "append_file", "prepend_content", "find"])]
        prepend_file: Option<String>,

        /// Find text in content (requires --replace)
        #[arg(long, requires = "replace", conflicts_with_all = ["content", "file", "append_content", "append_file", "prepend_content", "prepend_file"])]
        find: Option<String>,

        /// Replace text found by --find
        #[arg(long, requires = "find")]
        replace: Option<String>,

        /// Replace all occurrences (with --find/--replace)
        #[arg(long, requires = "find")]
        replace_all: bool,

        /// Replace only the Nth occurrence (1-indexed, with --find/--replace)
        #[arg(long, requires = "find", conflicts_with = "replace_all")]
        nth: Option<usize>,

        /// Update category
        #[arg(long)]
        category: Option<String>,

        /// Update tags (comma-separated, replaces all)
        #[arg(long, conflicts_with_all = ["add_tag", "remove_tag"])]
        tags: Option<String>,

        /// Add a single tag to existing tags
        #[arg(long, conflicts_with = "tags")]
        add_tag: Option<String>,

        /// Remove a specific tag
        #[arg(long, conflicts_with = "tags")]
        remove_tag: Option<String>,

        /// Update applicability (comma-separated, replaces all)
        #[arg(short = 'a', long)]
        applicability: Option<String>,

        /// Update content type
        #[arg(long)]
        content_type: Option<String>,

        /// Update resonance level (1-10, or higher for transcendent)
        #[arg(long)]
        resonance: Option<i32>,

        /// Update resonance type (foundational, transformative, relational, operational, ephemeral, session)
        #[arg(long)]
        resonance_type: Option<String>,

        /// Update anchors (comma-separated bloom IDs, replaces all)
        #[arg(long, conflicts_with_all = ["add_anchor", "remove_anchor"])]
        anchors: Option<String>,

        /// Add a single anchor to existing anchors
        #[arg(long, conflicts_with = "anchors")]
        add_anchor: Option<String>,

        /// Remove a specific anchor
        #[arg(long, conflicts_with = "anchors")]
        remove_anchor: Option<String>,

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

        /// Mark as private (shorthand for --visibility private)
        #[arg(long, conflicts_with = "visibility")]
        private: bool,

        /// Change visibility (public or private)
        #[arg(long, conflicts_with = "private")]
        visibility: Option<String>,

        /// Update owner (only valid when visibility is private)
        #[arg(long)]
        owner: Option<String>,

        /// Update session ID (for retrofitting entries written with wrong or missing session linkage)
        #[arg(long)]
        session_id: Option<String>,

        /// Force dangerous visibility changes (e.g., making blooms public)
        #[arg(long)]
        force: bool,

        /// Output as JSON
        #[arg(long)]
        json: bool,
    },

    /// Edit content by finding and replacing text (shortcut for: update <id> --find ... --replace ...)
    Edit {
        /// Entry ID to edit
        id: String,

        /// Text to find in the content
        #[arg(long, visible_alias = "old")]
        find: String,

        /// Replacement text
        #[arg(long, visible_alias = "new")]
        replace: String,

        /// Replace all occurrences (default: error if multiple matches)
        #[arg(long)]
        replace_all: bool,

        /// Replace only the Nth occurrence (1-indexed)
        #[arg(long, conflicts_with = "replace_all")]
        nth: Option<usize>,

        /// Output as JSON
        #[arg(long)]
        json: bool,
    },

    /// Append content to the end of an entry's body (shortcut for: update <id> --append-content ...)
    Append {
        /// Entry ID to append to
        id: String,

        /// Content to append (omit to read from stdin)
        #[arg(long, conflicts_with = "file")]
        content: Option<String>,

        /// Read content to append from file
        #[arg(
            short,
            long,
            visible_alias = "content-file",
            conflicts_with = "content"
        )]
        file: Option<String>,

        /// Output as JSON
        #[arg(long)]
        json: bool,
    },

    /// Prepend content to the start of an entry's body (shortcut for: update <id> --prepend-content ...)
    Prepend {
        /// Entry ID to prepend to
        id: String,

        /// Content to prepend (omit to read from stdin)
        #[arg(long, conflicts_with = "file")]
        content: Option<String>,

        /// Read content to prepend from file
        #[arg(
            short,
            long,
            visible_alias = "content-file",
            conflicts_with = "content"
        )]
        file: Option<String>,

        /// Output as JSON
        #[arg(long)]
        json: bool,
    },

    /// Restore entry content from a backup
    Restore {
        /// Entry ID to restore
        id: String,

        /// List available backups instead of restoring
        #[arg(long)]
        list: bool,

        /// Output as JSON
        #[arg(long)]
        json: bool,
    },

    /// Generate embedding for a knowledge entry
    Embed {
        /// Entry ID to embed (not used with --all)
        #[arg(required_unless_present = "all")]
        id: Option<String>,

        /// Embed all knowledge entries
        #[arg(short, long)]
        all: bool,
    },

    /// Automatically add anchors based on embedding similarity
    AutoAnchor {
        /// Entry ID to process (omit to process all entries with embeddings)
        id: Option<String>,

        /// Minimum cosine similarity threshold (0.0-1.0)
        #[arg(long, default_value = "0.75")]
        threshold: f32,

        /// Maximum anchors to add per entry
        #[arg(long, default_value = "5")]
        max_anchors: usize,

        /// Preview changes without writing
        #[arg(long)]
        dry_run: bool,

        /// Show similarity scores in output
        #[arg(long)]
        verbose: bool,
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

    /// Query tags used in memory entries
    Tags {
        #[command(subcommand)]
        command: TagsCommands,
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

    /// List recent ephemeral facts with decay
    Recent {
        /// Number of days to look back
        #[arg(long, default_value = "10")]
        days: i32,

        /// Output as JSON
        #[arg(long)]
        json: bool,

        /// Output format (text, json) [deprecated: use --json]
        #[arg(long, default_value = "text", hide = true)]
        format: String,

        /// Filter by resonance type (e.g., ephemeral). When omitted without --all-types, defaults to ephemeral only.
        #[arg(long)]
        resonance_type: Option<String>,

        /// Surface all resonance types (blooms, patterns, insights, decisions, ephemeral, etc.)
        /// instead of ephemeral-only. Can be combined with --resonance-type to filter within
        /// the broader set.
        #[arg(long)]
        all_types: bool,

        /// Sort order: "chronological" (default) or "resonance" (highest first)
        #[arg(long, value_enum, default_value_t = RecentSortOrder::Chronological)]
        sort: RecentSortOrder,

        /// Maximum number of results
        #[arg(long, default_value = "100")]
        limit: usize,
    },

    /// Fetch facts for the wake ritual (resonance >= 3, all types, sorted by resonance)
    WakeFetch {
        /// Number of days to look back
        #[arg(long, default_value = "15")]
        days: i32,

        /// Maximum number of results
        #[arg(long, default_value = "100")]
        limit: usize,
    },

    /// List facts extracted from a specific session
    ForSession {
        /// Session ID (with or without kn- prefix)
        session_id: String,

        /// Output as JSON
        #[arg(long)]
        json: bool,

        /// Output format (text, json) [deprecated: use --json]
        #[arg(long, default_value = "text", hide = true)]
        format: String,
    },

    /// Get the session a fact was extracted from
    FactSession {
        /// Fact ID (with or without kn- prefix)
        fact_id: String,

        /// Output as JSON
        #[arg(long)]
        json: bool,

        /// Output format (text, json) [deprecated: use --json]
        #[arg(long, default_value = "text", hide = true)]
        format: String,
    },

    /// Reinforce a knowledge entry (increment resonance, update last_activated, increment activation_count)
    Reinforce {
        /// Entry ID to reinforce
        id: String,

        /// Amount to increase resonance by (default: 1)
        #[arg(long, default_value = "1")]
        amount: i32,

        /// Maximum resonance cap (default: 10)
        #[arg(long, default_value = "10")]
        cap: i32,

        /// Output as JSON
        #[arg(long)]
        json: bool,

        /// Output format (text, json) [deprecated: use --json]
        #[arg(long, default_value = "text", hide = true)]
        format: String,
    },
}

#[derive(Subcommand)]
pub enum GithubCommands {
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
pub enum CommentCommands {
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
pub enum SessionCommands {
    /// Export a Claude session as markdown.
    ///
    /// DEPRECATED: use `mx codex export` instead. This subcommand is now
    /// a thin alias that forwards into the codex export pipeline and
    /// will be removed in a future release. The new command supports
    /// filtering by --session, --project, --date, multiple output
    /// formats (markdown / json / both), and inlines sub-agent
    /// transcripts by default.
    Export {
        /// Path to session JSONL file (defaults to most recent non-agent session).
        ///
        /// DEPRECATED: when set, the file stem (the session UUID) is
        /// extracted and routed to `mx codex export --session <uuid>`.
        path: Option<String>,

        /// Output file (defaults to stdout).
        #[arg(short, long)]
        output: Option<String>,
    },
}

/// Shared fields for `mx codex archive` and its deprecated `save` alias.
/// Extracted so both CLI variants use the same definition and neither can
/// drift out of sync with the other.
#[derive(Debug, Clone, clap::Args)]
pub struct ArchiveArgs {
    /// Path to session JSONL file (defaults to most recent non-agent session)
    #[arg(conflicts_with_all = ["all", "backfill"])]
    pub path: Option<String>,

    /// Archive all unarchived sessions
    #[arg(long, conflicts_with_all = ["backfill"])]
    pub all: bool,

    /// Save only conversation.md + manifest.json + images (no JSONL, no agent files)
    #[arg(long)]
    pub clean: bool,

    /// Include agent sub-session conversations in clean transcript.
    ///
    /// Requires `subagents` in `--include` (the default; if you pass
    /// `--include none` or any value without `subagents`, this flag
    /// will error at parse time rather than silently no-op).
    #[arg(long, requires = "clean")]
    pub include_agents: bool,

    /// Comma-separated list of optional source artifacts to capture.
    ///
    /// Recognized: `subagents`, `mcp`, `tool-output`, `history`, `all`,
    /// `none`. Today's default behavior corresponds to `--include
    /// subagents`. The other tokens enable forthcoming source walkers
    /// (MCP server logs, /tmp tool outputs, history.jsonl slice).
    ///
    /// Note: this flag governs which source files are *captured* into
    /// the archive sidecars. The separate `--include-agents` flag
    /// controls whether subagent transcripts are folded into the
    /// `conversation.md` rendering when `--clean` is set.
    #[arg(long, default_value = "subagents")]
    pub include: String,

    /// Ingest the legacy wonka vault snapshots into the codex.
    ///
    /// Walks every `session-*` snapshot under the supplied path
    /// (defaults to `~/.wonka/vault/archives/`) and feeds each
    /// session JSONL through the regular archive pipeline.
    /// Idempotent: re-running on the same vault produces the same
    /// codex state.
    ///
    /// Mutually exclusive with `--all` and the positional `[PATH]`.
    /// `--include` and `--clean` still apply (they govern what the
    /// per-session pipeline captures).
    #[arg(
        long,
        value_name = "VAULT_PATH",
        num_args = 0..=1,
        default_missing_value = "",
        conflicts_with_all = ["all", "path"],
    )]
    pub backfill: Option<String>,
}

#[derive(Subcommand)]
pub enum CodexCommands {
    /// Archive current session to permanent storage
    Archive {
        #[command(flatten)]
        args: ArchiveArgs,
    },

    /// Deprecated alias for `mx codex archive`. Use `archive` instead.
    #[command(hide = true)]
    Save {
        #[command(flatten)]
        args: ArchiveArgs,
    },

    /// Export an archived session as Markdown or structured JSON.
    ///
    /// The default with no flags is "the most recent codex session, as
    /// markdown to stdout, with sub-agent transcripts inlined and
    /// everything else stripped." Selectors are mutually exclusive: at
    /// most one of `--session`, `--project`, `--date` may be passed.
    Export {
        /// Session UUID (full or unique prefix).
        #[arg(long, group = "selector")]
        session: Option<String>,

        /// Project to filter by: absolute path, raw cwd-encoded slug
        /// (`-home-charlie-...`), or basename (`mx`). Ambiguous basenames
        /// list every colliding absolute path and exit non-zero.
        #[arg(long, group = "selector")]
        project: Option<String>,

        /// Date selector. Accepts `YYYY-MM-DD`, `YYYY-MM-DD..YYYY-MM-DD`,
        /// or `YYYY-MM`.
        #[arg(long, group = "selector")]
        date: Option<String>,

        /// Output format. `markdown` (default), `json`, or `both`.
        ///
        /// `both` requires `--output`: JSON is written to the supplied
        /// path and markdown is written to a sibling sidecar file
        /// (`<out>.json` + `<out>.md`, with the operator-supplied
        /// extension preserved if it's already `.json` or `.md`).
        #[arg(long, default_value = "markdown")]
        format: String,

        /// Comma-separated list of optional content to render. Default:
        /// `subagents`. Recognized: `subagents`, `tools`,
        /// `system-reminders`, `mcp`, `tool-output`, `history`, `all`,
        /// `none`.
        #[arg(long, default_value = "subagents")]
        include: String,

        /// Run `mx codex archive --all` before exporting and skip the
        /// unarchived-data warning. Useful when you know live
        /// `~/.claude/projects/` data hasn't been ingested yet.
        #[arg(long)]
        archive_first: bool,

        /// Output file path. Default: stdout (markdown / json).
        /// Required when `--format both` (writes `<out>.json` and
        /// `<out>.md` sidecar files).
        #[arg(short, long)]
        output: Option<String>,
    },

    /// List archived sessions
    List {
        /// Show all archives including incremental saves
        #[arg(long)]
        all: bool,

        /// Output as JSON
        #[arg(long)]
        json: bool,
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

        /// Output as JSON
        #[arg(long)]
        json: bool,

        /// Read the clean markdown transcript (conversation.md)
        #[arg(long, conflicts_with = "human")]
        clean: bool,
    },

    /// Search all archives for a pattern
    Search {
        /// Pattern to search for
        pattern: String,

        /// Output as JSON
        #[arg(long)]
        json: bool,
    },

    /// Migrate v1 archives to v2 (extract images to files)
    Migrate {
        /// Show what would be migrated without doing it
        #[arg(long)]
        dry_run: bool,

        /// Show detailed progress
        #[arg(long)]
        verbose: bool,

        /// Generate conversation.md for archives that have session.jsonl but no clean transcript
        #[arg(long)]
        clean: bool,

        /// Include agent sub-session conversations in clean transcript
        #[arg(long, requires = "clean")]
        include_agents: bool,
    },
}

#[derive(Subcommand)]
pub enum MemorySeedCommands {
    /// Seed agents from markdown files with YAML frontmatter
    ///
    /// Default location: `$MX_HOME/memory/seed/agents/`.
    /// Soft fallback: `$MX_HOME/agents/` (legacy) -- emits stderr warning.
    /// TODO(memory-seed-agents-migration): remove fallback after one release cycle.
    Agents {
        /// Path to agents directory (defaults to $MX_HOME/memory/seed/agents/)
        #[arg(short, long)]
        path: Option<String>,
    },

    /// Seed knowledge from JSONL files
    ///
    /// With no path: scans `$MX_HOME/memory/seed/knowledge/*.jsonl` and
    /// imports every file. With a path: imports just that file.
    /// Soft fallback: `$MX_HOME/memory/index.jsonl` (legacy) -- emits stderr warning.
    /// TODO(memory-seed-knowledge-migration): remove fallback after one release cycle.
    Knowledge {
        /// Path to a single .jsonl file (omit to scan the seed dir)
        path: Option<String>,
    },
}

#[derive(Subcommand)]
pub enum AgentsCommands {
    /// List all agents
    List {
        /// Output as JSON
        #[arg(long)]
        json: bool,
    },

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
    ///
    /// Removed in favor of `mx memory seed agents`. This stub remains
    /// only so misuse prints a helpful pointer.
    /// TODO(legacy-state-cleanup): remove after one release cycle.
    #[command(hide = true)]
    Seed {
        /// Path to agents directory (defaults to $MX_HOME/memory/seed/agents/)
        #[arg(short, long)]
        path: Option<String>,
    },
}

#[derive(Subcommand)]
pub enum ProjectsCommands {
    /// List all projects
    List {
        /// Output as JSON
        #[arg(long)]
        json: bool,
    },
    /// Add a new project
    Add {
        /// Unique project identifier
        #[arg(long)]
        id: String,
        /// Human-readable project name
        #[arg(long)]
        name: String,
        /// Local filesystem path to the project
        #[arg(long)]
        path: Option<String>,
        /// Git repository URL (e.g., owner/repo)
        #[arg(long)]
        repo_url: Option<String>,
        /// Project description
        #[arg(long)]
        description: Option<String>,
    },
}

#[derive(Subcommand)]
pub enum ApplicabilityCommands {
    /// List all applicability types
    List,
    /// Add a new applicability type
    Add {
        /// Unique identifier for the applicability type
        #[arg(long)]
        id: String,
        /// Description of when this applicability applies
        #[arg(long)]
        description: String,
        /// Scope constraint (e.g., project, global)
        #[arg(long)]
        scope: Option<String>,
    },
}

#[derive(Subcommand)]
pub enum SessionsCommands {
    /// List sessions
    List {
        /// Filter by project ID
        #[arg(long)]
        project: Option<String>,

        /// Output as JSON
        #[arg(long)]
        json: bool,
    },
    /// Create a new session
    Create {
        /// Session type (e.g., development, review, exploration)
        #[arg(long)]
        session_type: String,
        /// Associated project ID
        #[arg(long)]
        project: Option<String>,
    },
    /// Close a session
    Close {
        /// Session ID to close
        #[arg(long)]
        id: String,
    },
}

#[derive(Subcommand)]
pub enum CategoriesCommands {
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
pub enum TagsCommands {
    /// List all tags (optionally filter by category)
    List {
        /// Filter to tags used in a specific category
        #[arg(long)]
        category: Option<String>,

        /// Output as JSON
        #[arg(long)]
        json: bool,
    },
}

#[derive(Subcommand)]
pub enum SourceTypesCommands {
    /// List all source types
    List {
        /// Output as JSON
        #[arg(long)]
        json: bool,
    },
}

#[derive(Subcommand)]
pub enum EntryTypesCommands {
    /// List all entry types
    List {
        /// Output as JSON
        #[arg(long)]
        json: bool,
    },
}

#[derive(Subcommand)]
pub enum SessionTypesCommands {
    /// List all session types
    List {
        /// Output as JSON
        #[arg(long)]
        json: bool,
    },
}

#[derive(Subcommand)]
pub enum RelationshipTypesCommands {
    /// List all relationship types
    List {
        /// Output as JSON
        #[arg(long)]
        json: bool,
    },
}

#[derive(Subcommand)]
pub enum RelationshipsCommands {
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
pub enum ContentTypesCommands {
    /// List all content types
    List {
        /// Output as JSON
        #[arg(long)]
        json: bool,
    },
}

/// Output format for `kv dump`.
#[derive(Clone, Debug, ValueEnum)]
pub enum DumpFormat {
    /// JSON (default)
    Json,
    /// Compact key=value format
    Compact,
}

#[derive(Subcommand)]
pub enum KvCommands {
    /// Get the current value for a key
    Get {
        /// Key name
        key: String,

        /// Resolve and display linked memory entry (kn- reference)
        #[arg(long)]
        memory: bool,
    },

    /// Set a value (string/counter), or set a field on a state type
    Set {
        /// Key name
        key: String,

        /// Value to set (for string/counter), or field name (for state type)
        value: Option<String>,

        /// Field value (only for state type: mx kv set <key> <field> <value>)
        field_value: Option<String>,

        /// Link a memory entry (kn- ID) to this key, or "" to clear
        #[arg(long)]
        memory: Option<String>,
    },

    /// Increment a counter
    Inc {
        /// Key name
        key: String,

        /// Amount to increment by (default: 1)
        #[arg(long, default_value = "1")]
        by: i64,
    },

    /// Decrement a counter
    Dec {
        /// Key name
        key: String,

        /// Amount to decrement by (default: 1)
        #[arg(long, default_value = "1")]
        by: i64,
    },

    /// Push a value onto a history or list
    Push {
        /// Key name
        key: String,

        /// Value to push
        value: String,
    },

    /// Pop the last value from a list
    Pop {
        /// Key name
        key: String,
    },

    /// Get the last N entries from a history or list
    Last {
        /// Key name
        key: String,

        /// Number of entries (default: 1)
        #[arg(long, default_value = "1")]
        count: usize,

        /// Resolve and display linked memory entry (kn- reference)
        #[arg(long)]
        memory: bool,
    },

    /// Get history entries since a time reference (ISO-8601 or relative: 1h, 7d, 2w, 30m)
    Since {
        /// Key name
        key: String,

        /// Time reference (e.g., "1h", "7d", "2w", "30m", or ISO-8601)
        timeref: String,

        /// Resolve and display linked memory entry (kn- reference)
        #[arg(long)]
        memory: bool,
    },

    /// Dump all state
    Dump {
        /// Output format: compact or json
        #[arg(long, default_value = "json", value_enum)]
        format: DumpFormat,

        /// Resolve and display linked memory entries (kn- references)
        #[arg(long)]
        memory: bool,
    },

    /// Reset a key to its schema default
    Reset {
        /// Key name
        key: String,
    },

    /// Remove an entry by value or ID from a list/history
    Remove {
        /// Key name
        key: String,

        /// Value substring to match (omit if using --id)
        value: Option<String>,

        /// Remove by entry ID
        #[arg(long)]
        id: Option<u64>,

        /// Remove all matches (default: first match only)
        #[arg(long)]
        all: bool,
    },

    /// Search entries in a list/history by substring
    Search {
        /// Key name
        key: String,

        /// Search query (case-insensitive substring)
        query: String,

        /// Resolve and display linked memory entry (kn- reference)
        #[arg(long)]
        memory: bool,
    },

    /// Count entries in a list/history, optionally filtered
    Count {
        /// Key name
        key: String,

        /// Count only entries matching this substring
        value: Option<String>,
    },

    /// List all defined keys with their types
    Keys,
}

#[derive(Subcommand)]
pub enum WikiCommands {
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
