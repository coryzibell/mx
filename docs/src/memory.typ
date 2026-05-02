#import "lib.typ": *

#page-header(
  "Memory",
  "Knowledge graph with SurrealDB-backed persistent memory."
)

The memory subsystem is the largest command surface in mx. It provides a
persistent knowledge graph backed by SurrealDB (embedded SurrealKV or networked
WebSocket), with categories, tags, resonance levels, embeddings for semantic
search, relationships between entries, and a wake ritual for identity bootstrap.

Every entry in the graph has a unique ID (prefixed `kn-`), a category, a title,
body content, optional tags, a resonance level (1--10+), and timestamps. Entries
can be linked via typed relationships, anchored to each other by embedding
similarity, and surfaced through keyword or semantic search.

== Table of contents

- #link(<adding>)[Adding entries]
- #link(<reading>)[Reading entries]
- #link(<updating>)[Updating entries]
- #link(<deleting>)[Deleting entries]
- #link(<wake>)[Wake system]
- #link(<embeddings>)[Embeddings and anchoring]
- #link(<relationships>)[Relationships]
- #link(<seeding>)[Seeding]
- #link(<health>)[Health and statistics]
- #link(<export>)[Export]
- #link(<reinforcement>)[Reinforcement]
- #link(<metadata>)[Metadata management]
- #link(<sessions>)[Session tracking]

// ═══════════════════════════════════════════════════════════════════════
// ADDING ENTRIES
// ═══════════════════════════════════════════════════════════════════════

== Adding entries <adding>

#command(
  "mx memory add",
  [Create a new entry in the knowledge graph. At minimum, provide a category
  and title (or a `--type` for ephemeral facts, which auto-routes the category
  and generates a title from content).],
  flags: (
    ([`--category`],   [`string`], [Category name (run `mx memory categories list` for valid names). Required unless `--type` is provided.]),
    ([`-t, --title`],  [`string`], [Entry title. Required unless `--type` is provided.]),
    ([`--content`],    [`string`], [Inline content. Conflicts with `--file`.]),
    ([`-f, --file`],   [`path`],   [Read content from a file. Also accepts `--content-file`.]),
    ([`--tags`],       [`string`], [Comma-separated tags.]),
    ([`-a, --applicability`], [`string`], [Comma-separated applicability contexts.]),
    ([`-p, --project`], [`string`], [Source project ID.]),
    ([`--source-agent`], [`string`], [Source agent ID. Defaults to `MX_CURRENT_AGENT` env var.]),
    ([`--source-type`], [`string`], [Source type: `manual`, `ram`, `cache`, `agent_session`. Default: `manual`.]),
    ([`--entry-type`], [`string`], [Entry type: `primary`, `summary`, `synthesis`. Default: `primary`.]),
    ([`--session-id`], [`string`], [Session ID to associate with this entry.]),
    ([`--ephemeral`],  [`flag`],   [Mark entry as ephemeral.]),
    ([`-d, --domain`], [`string`], [Domain/subdomain path.]),
    ([`--content-type`], [`string`], [Content type: `text`, `code`, `config`, `data`, `binary`. Default: `text`.]),
    ([`--private`],    [`flag`],   [Mark as private (only visible to owner). Shorthand for `--visibility private`.]),
    ([`--visibility`], [`string`], [Set visibility: `public` or `private`.]),
    ([`--owner`],      [`string`], [Explicit owner. Defaults to `source_agent` or `MX_CURRENT_AGENT` if private.]),
    ([`--resonance`],  [`int`],    [Resonance level (1--10, or higher for transcendent).]),
    ([`--resonance-type`], [`string`], [Resonance type: `foundational`, `transformative`, `relational`, `operational`, `ephemeral`, `session`.]),
    ([`--wake-phrase`], [`string`], [Wake phrase for memory ritual verification.]),
    ([`--wake-phrases`], [`string`], [Multiple wake phrases (comma-separated).]),
    ([`--wake-order`], [`int`],    [Custom wake order (lower = earlier in sequence).]),
    ([`--anchors`],    [`string`], [Comma-separated bloom IDs this entry connects to.]),
    ([`--type`],       [`string`], [Fact type for ephemeral knowledge: `decision`, `insight`, `person`, `quote`, `thread_opened`, `commitment`, `thread_closed`. Auto-routes category and sets `resonance_type=ephemeral`.]),
    ([`--session`],    [`string`], [Session to link fact to via EXTRACTED_FROM relationship. Requires `--type`.]),
    ([`--thread-id`],  [`string`], [Thread ID for `thread_closed` operations. Requires `--type`.]),
    ([`--json`],       [`flag`],   [Output as JSON.]),
  ),
  examples: (
    "mx memory add --category recipe --title \"Retry with backoff\" \\\n  --content \"Use exponential backoff with jitter...\" \\\n  --tags \"reliability,networking\" --source-agent whistledown",
    "mx memory add --category discovery --title \"SurrealDB needs explicit NS\" \\\n  --content \"Always set namespace before queries\" \\\n  --resonance 7 --resonance-type operational",
    "# Ephemeral fact (auto-routes category, generates title)\nmx memory add --type decision \\\n  --content \"Chose Typst over mdBook for docs\" \\\n  --session abc-123",
    "# Content from file\nmx memory add --category ingredient -t \"API reference\" -f api-notes.md",
  ),
)

#tip[When `--type` is provided, `--category` and `--title` become optional. The
fact type routes to an appropriate category and generates a title from the
content automatically.]


// ═══════════════════════════════════════════════════════════════════════
// READING ENTRIES
// ═══════════════════════════════════════════════════════════════════════

== Reading entries <reading>

=== Shared filter flags

Several read commands (`search`, `list`) share a common set of filter
flags. These are documented once here and referenced below.

#table(
  columns: (auto, auto, auto),
  table.header([*Flag*], [*Type*], [*Description*]),
  [`-c, --category`], [`string`], [Filter by category (comma-separated).],
  [`--json`],          [`flag`],   [Output as JSON.],
  [`--mine`],          [`flag`],   [Show only your private entries.],
  [`--include-private`], [`flag`], [Include private entries (requires matching owner).],
  [`--min-resonance`], [`int`],    [Minimum resonance level.],
  [`--max-resonance`], [`int`],    [Maximum resonance level.],
  [`--has-wake-phrase`], [`flag`],  [Filter to entries WITH a wake phrase.],
  [`--missing-wake-phrase`], [`flag`], [Filter to entries WITHOUT a wake phrase.],
  [`--has-anchors`],   [`flag`],   [Filter to entries WITH anchors.],
  [`--missing-anchors`], [`flag`], [Filter to entries WITHOUT anchors.],
  [`--has-resonance-type`], [`flag`], [Filter to entries WITH a resonance type.],
  [`--missing-resonance-type`], [`flag`], [Filter to entries WITHOUT a resonance type.],
  [`--limit`],         [`int`],    [Limit number of results.],
  [`--tags`],          [`string`], [Filter by tags (comma-separated, matches any).],
)

#command(
  "mx memory show",
  [Display a single entry by ID.],
  flags: (
    ([`--json`],         [`flag`], [Output as JSON.]),
    ([`--content-only`], [`flag`], [Output only the body content (useful for piping).]),
  ),
  examples: (
    "mx memory show kn-abc123",
    "mx memory show kn-abc123 --content-only | pbcopy",
  ),
)

#command(
  "mx memory list",
  [List entries, optionally filtered by category, tags, resonance, and other
  shared filter flags.],
  flags: (),
  examples: (
    "mx memory list -c recipe",
    "mx memory list -c discovery,decree --min-resonance 5",
    "mx memory list --missing-wake-phrase --limit 20",
  ),
)

#note[`list` accepts all shared filter flags documented above.]

#command(
  "mx memory search",
  [Search entries by keyword or semantic similarity. Keyword search is the
  default; add `--semantic` to use vector embeddings.],
  flags: (
    ([`--semantic`], [`flag`], [Use semantic (vector) search instead of keyword search.]),
  ),
  examples: (
    "mx memory search \"retry pattern\"",
    "mx memory search \"how to handle timeouts\" --semantic",
    "mx memory search \"agent bootstrap\" -c recipe,method --limit 5",
  ),
)

#note[`search` accepts all shared filter flags. Semantic search requires entries
to have embeddings generated via `mx memory embed`.]

#command(
  "mx memory recent",
  [List recent ephemeral facts with decay. By default shows only ephemeral
  entries from the last 10 days. Use `--all-types` to surface all resonance
  types.],
  flags: (
    ([`--days`],           [`int`],    [Number of days to look back. Default: `10`.]),
    ([`--json`],           [`flag`],   [Output as JSON.]),
    ([`--resonance-type`], [`string`], [Filter by resonance type. Defaults to ephemeral only when `--all-types` is omitted.]),
    ([`--all-types`],      [`flag`],   [Surface all resonance types instead of ephemeral only.]),
    ([`--sort`],           [`enum`],   [Sort order: `chronological` (default) or `resonance` (highest first).]),
    ([`--limit`],          [`int`],    [Maximum number of results. Default: `100`.]),
  ),
  examples: (
    "mx memory recent",
    "mx memory recent --days 30 --all-types --sort resonance",
    "mx memory recent --resonance-type foundational --limit 10",
  ),
)


// ═══════════════════════════════════════════════════════════════════════
// UPDATING ENTRIES
// ═══════════════════════════════════════════════════════════════════════

== Updating entries <updating>

#command(
  "mx memory update",
  [Update an existing entry. Supports replacing content entirely, appending,
  prepending, find-and-replace, and modifying any metadata field. Content
  mutation modes are mutually exclusive.],
  flags: (
    ([`-t, --title`],     [`string`], [Update the title.]),
    ([`--content`],       [`string`], [Replace content entirely (inline).]),
    ([`-f, --file`],      [`path`],   [Replace content entirely from file.]),
    ([`--append-content`], [`string`], [Append text to end of existing content.]),
    ([`--append-file`],   [`path`],   [Append content from file to end.]),
    ([`--prepend-content`], [`string`], [Prepend text to start of existing content.]),
    ([`--prepend-file`],  [`path`],   [Prepend content from file to start.]),
    ([`--find`],          [`string`], [Find text in content (requires `--replace`).]),
    ([`--replace`],       [`string`], [Replace text found by `--find`.]),
    ([`--replace-all`],   [`flag`],   [Replace all occurrences (with `--find`/`--replace`).]),
    ([`--nth`],           [`int`],    [Replace only the Nth occurrence (1-indexed).]),
    ([`--category`],      [`string`], [Update category.]),
    ([`--tags`],          [`string`], [Replace all tags (comma-separated).]),
    ([`--add-tag`],       [`string`], [Add a single tag to existing tags.]),
    ([`--remove-tag`],    [`string`], [Remove a specific tag.]),
    ([`-a, --applicability`], [`string`], [Update applicability (comma-separated, replaces all).]),
    ([`--content-type`],  [`string`], [Update content type.]),
    ([`--resonance`],     [`int`],    [Update resonance level (1--10+).]),
    ([`--resonance-type`], [`string`], [Update resonance type.]),
    ([`--anchors`],       [`string`], [Replace all anchors (comma-separated bloom IDs).]),
    ([`--add-anchor`],    [`string`], [Add a single anchor.]),
    ([`--remove-anchor`], [`string`], [Remove a specific anchor.]),
    ([`--wake-phrase`],   [`string`], [Update wake phrase.]),
    ([`--wake-phrases`],  [`string`], [Replace all wake phrases (comma-separated).]),
    ([`--add-wake-phrase`], [`string`], [Add a single wake phrase.]),
    ([`--remove-wake-phrase`], [`string`], [Remove a specific wake phrase.]),
    ([`--wake-order`],    [`string`], [Update wake order. Use `'-'` to clear.]),
    ([`--private`],       [`flag`],   [Mark as private (shorthand for `--visibility private`).]),
    ([`--visibility`],    [`string`], [Change visibility: `public` or `private`.]),
    ([`--owner`],         [`string`], [Update owner (only valid when visibility is private).]),
    ([`--session-id`],    [`string`], [Update session ID (for retrofitting entries with wrong or missing session linkage).]),
    ([`--force`],         [`flag`],   [Force dangerous visibility changes (e.g., making blooms public).]),
    ([`--json`],          [`flag`],   [Output as JSON.]),
  ),
  examples: (
    "mx memory update kn-abc123 --title \"Better title\"",
    "mx memory update kn-abc123 --add-tag reliability",
    "mx memory update kn-abc123 --find \"old text\" --replace \"new text\"",
    "mx memory update kn-abc123 --append-content \"\\n\\nUpdate: confirmed working\"",
    "mx memory update kn-abc123 --resonance 8 --resonance-type foundational",
  ),
)

#command(
  "mx memory edit",
  [Find-and-replace shortcut. Equivalent to
  `mx memory update <id> --find ... --replace ...` with a simpler interface.],
  flags: (
    ([`--find`],        [`string`], [Text to find in content. Also accepts `--old`.]),
    ([`--replace`],     [`string`], [Replacement text. Also accepts `--new`.]),
    ([`--replace-all`], [`flag`],   [Replace all occurrences (default: error if multiple matches).]),
    ([`--nth`],         [`int`],    [Replace only the Nth occurrence (1-indexed).]),
    ([`--json`],        [`flag`],   [Output as JSON.]),
  ),
  examples: (
    "mx memory edit kn-abc123 --find \"old pattern\" --replace \"new pattern\"",
    "mx memory edit kn-abc123 --old \"v1\" --new \"v2\" --replace-all",
  ),
)

#command(
  "mx memory append",
  [Append content to the end of an entry's body. Shortcut for
  `mx memory update <id> --append-content ...`.],
  flags: (
    ([`--content`], [`string`], [Content to append (omit to read from stdin).]),
    ([`-f, --file`], [`path`],  [Read content from file. Also accepts `--content-file`.]),
    ([`--json`],    [`flag`],   [Output as JSON.]),
  ),
  examples: (
    "mx memory append kn-abc123 --content \"\\n\\nAdditional note here.\"",
    "mx memory append kn-abc123 -f addendum.md",
  ),
)

#command(
  "mx memory prepend",
  [Prepend content to the start of an entry's body. Shortcut for
  `mx memory update <id> --prepend-content ...`.],
  flags: (
    ([`--content`], [`string`], [Content to prepend (omit to read from stdin).]),
    ([`-f, --file`], [`path`],  [Read content from file. Also accepts `--content-file`.]),
    ([`--json`],    [`flag`],   [Output as JSON.]),
  ),
  examples: (
    "mx memory prepend kn-abc123 --content \"IMPORTANT: \"",
  ),
)

#command(
  "mx memory restore",
  [Restore entry content from a backup. Use `--list` to see available backups
  before restoring.],
  flags: (
    ([`--list`], [`flag`], [List available backups instead of restoring.]),
    ([`--json`], [`flag`], [Output as JSON.]),
  ),
  examples: (
    "mx memory restore kn-abc123 --list",
    "mx memory restore kn-abc123",
  ),
)


// ═══════════════════════════════════════════════════════════════════════
// DELETING ENTRIES
// ═══════════════════════════════════════════════════════════════════════

== Deleting entries <deleting>

#command(
  "mx memory delete",
  [Remove an entry from the knowledge graph.],
  flags: (
    ([`--json`], [`flag`], [Output as JSON.]),
  ),
  examples: (
    "mx memory delete kn-abc123",
  ),
)


// ═══════════════════════════════════════════════════════════════════════
// WAKE SYSTEM
// ═══════════════════════════════════════════════════════════════════════

== Wake system <wake>

The wake system provides identity bootstrap for agents. It retrieves
high-resonance entries ("blooms") and presents them through a ritual that
verifies the agent's connection to its knowledge. There are several output
modes and an interactive engage flow.

#command(
  "mx memory wake",
  [Wake up with resonant identity cascade. Retrieves high-resonance blooms
  and presents them in the requested format.],
  flags: (
    ([`-l, --limit`],   [`int`],    [Number of blooms to return. Default: `20`.]),
    ([`--min-resonance`], [`int`],   [Minimum resonance threshold -- get ALL blooms >= this value (overrides `--limit`).]),
    ([`-d, --days`],    [`int`],    [Include memories activated in last N days. Default: `7`.]),
    ([`--json`],        [`flag`],   [Output as JSON.]),
    ([`--ritual`],      [`flag`],   [Output as bash ritual script (sequential reading).]),
    ([`--index`],       [`flag`],   [Output as compact markdown index (for identity loading).]),
    ([`--no-activate`], [`flag`],   [Do not update activation counts.]),
    ([`-e, --engage`],  [`flag`],   [Interactive engage mode -- verify wake phrases (requires TTY).]),
    ([`-s, --set-missing`], [`flag`], [Prompt to set missing wake phrases during engage mode. Requires `--engage`.]),
    ([`--begin`],       [`flag`],   [Start token-based wake ritual. Returns first bloom and session token.]),
    ([`--bloom-id`],    [`string`], [Bloom ID for `--respond` or `--skip` operations.]),
    ([`--respond`],     [`string`], [Submit wake phrase response for a bloom.]),
    ([`--skip`],        [`flag`],   [Skip a bloom without wake phrase.]),
    ([`--session`],     [`string`], [Session token for chained ritual (required with `--respond` or `--skip`).]),
  ),
  examples: (
    "# Default wake -- top 20 blooms, text output\nmx memory wake",
    "# Compact index for agent identity loading\nmx memory wake --index",
    "# All blooms with resonance >= 7\nmx memory wake --min-resonance 7",
    "# Interactive engage mode with wake phrase verification\nmx memory wake --engage",
    "# Token-based ritual (for non-TTY / programmatic use)\nmx memory wake --begin\nmx memory wake --bloom-id kn-abc --respond \"the phrase\" --session tok-xyz\nmx memory wake --bloom-id kn-def --skip --session tok-xyz",
  ),
)

#note[`MX_CURRENT_AGENT` must be set for wake to function. The wake ritual
reads blooms ordered by resonance and wake order, then optionally verifies
the agent can produce each bloom's wake phrase.]

=== Wake modes

- *Default* (`mx memory wake`): plain text output, blooms listed with titles and content.
- *JSON* (`--json`): structured output for programmatic consumption.
- *Ritual* (`--ritual`): bash script that presents blooms sequentially.
- *Index* (`--index`): compact markdown suitable for loading into agent context.
- *Engage* (`--engage`): interactive TTY mode where the agent verifies each bloom's wake phrase. Add `--set-missing` to be prompted for phrases on blooms that lack them.
- *Token-based* (`--begin`, `--respond`, `--skip`): stateless chained ritual for non-interactive environments. Start with `--begin`, then loop with `--respond` or `--skip` using the returned session token and bloom ID.

#command(
  "mx memory wake-fetch",
  [Fetch facts for the wake ritual. Returns entries with resonance >= 3
  across all types, sorted by resonance (highest first). Designed as a
  data source for wake ritual presentation.],
  flags: (
    ([`--days`],  [`int`], [Number of days to look back. Default: `15`.]),
    ([`--limit`], [`int`], [Maximum number of results. Default: `100`.]),
  ),
  examples: (
    "mx memory wake-fetch",
    "mx memory wake-fetch --days 30 --limit 50",
  ),
)


// ═══════════════════════════════════════════════════════════════════════
// EMBEDDINGS & ANCHORING
// ═══════════════════════════════════════════════════════════════════════

== Embeddings and anchoring <embeddings>

Embeddings enable semantic search and automatic relationship discovery.
Each entry can have a vector embedding generated from its title and content.
Anchors are connections between entries discovered via embedding similarity.

#command(
  "mx memory embed",
  [Generate a vector embedding for one or all entries. Embeddings power
  semantic search (`--semantic` flag on `search`) and automatic anchoring.],
  flags: (
    ([`-a, --all`], [`flag`], [Embed all knowledge entries (instead of a single ID).]),
  ),
  examples: (
    "mx memory embed kn-abc123",
    "mx memory embed --all",
  ),
)

#command(
  "mx memory auto-anchor",
  [Automatically add anchors between entries based on embedding similarity.
  Processes a single entry or all entries that have embeddings.],
  flags: (
    ([`--threshold`],   [`float`], [Minimum cosine similarity (0.0--1.0). Default: `0.75`.]),
    ([`--max-anchors`], [`int`],   [Maximum anchors to add per entry. Default: `5`.]),
    ([`--dry-run`],     [`flag`],  [Preview changes without writing.]),
    ([`--verbose`],     [`flag`],  [Show similarity scores in output.]),
  ),
  examples: (
    "mx memory auto-anchor",
    "mx memory auto-anchor kn-abc123 --threshold 0.8 --max-anchors 3",
    "mx memory auto-anchor --dry-run --verbose",
  ),
)

#tip[A typical workflow: run `mx memory embed --all` to generate embeddings,
then `mx memory auto-anchor --dry-run --verbose` to preview anchor
candidates, then `mx memory auto-anchor` to write them.]


// ═══════════════════════════════════════════════════════════════════════
// RELATIONSHIPS
// ═══════════════════════════════════════════════════════════════════════

== Relationships <relationships>

Explicit typed edges between entries. While anchors are discovered
automatically via embedding similarity, relationships are manually declared
semantic connections.

#command(
  "mx memory relationships list",
  [List all relationships for an entry.],
  flags: (
    ([`--json`], [`flag`], [Output as JSON.]),
  ),
  examples: (
    "mx memory relationships list kn-abc123",
  ),
)

#command(
  "mx memory relationships add",
  [Add a typed relationship between two entries.],
  flags: (
    ([`--from`], [`string`], [Source entry ID.]),
    ([`--to`],   [`string`], [Target entry ID.]),
    ([`--type`], [`string`], [Relationship type: `related`, `supersedes`, `extends`, `implements`, `contradicts`.]),
  ),
  examples: (
    "mx memory relationships add --from kn-abc --to kn-def --type extends",
    "mx memory relationships add --from kn-abc --to kn-ghi --type supersedes",
  ),
)

#command(
  "mx memory relationships delete",
  [Delete a relationship by its ID.],
  flags: (),
  examples: (
    "mx memory relationships delete rel-abc123",
  ),
)


// ═══════════════════════════════════════════════════════════════════════
// SEEDING
// ═══════════════════════════════════════════════════════════════════════

== Seeding <seeding>

Seed commands populate the knowledge graph from on-disk artifacts. Used for
initial setup and bulk import.

#command(
  "mx memory seed agents",
  [Seed agents from markdown files with YAML frontmatter. Reads from
  `$MX_HOME/memory/seed/agents/` by default.],
  flags: (
    ([`-p, --path`], [`path`], [Path to agents directory. Defaults to `$MX_HOME/memory/seed/agents/`.]),
  ),
  examples: (
    "mx memory seed agents",
    "mx memory seed agents --path /data/agents/",
  ),
)

#note[Legacy fallback: if `$MX_HOME/memory/seed/agents/` does not exist, mx
checks `$MX_HOME/agents/` and emits a stderr warning. This fallback will be
removed in a future release.]

#command(
  "mx memory seed knowledge",
  [Seed knowledge from JSONL files. With no path, scans
  `$MX_HOME/memory/seed/knowledge/*.jsonl` and imports every file found. With
  a path, imports just that single file.],
  flags: (),
  examples: (
    "mx memory seed knowledge",
    "mx memory seed knowledge /data/knowledge/bootstrap.jsonl",
  ),
)


// ═══════════════════════════════════════════════════════════════════════
// HEALTH & STATISTICS
// ═══════════════════════════════════════════════════════════════════════

== Health and statistics <health>

#command(
  "mx memory stats",
  [Show index statistics -- entry counts, category breakdown, and other
  aggregate metrics.],
  flags: (
    ([`--json`], [`flag`], [Output as JSON.]),
  ),
  examples: (
    "mx memory stats",
    "mx memory stats --json",
  ),
)

#command(
  "mx memory health",
  [Show graph health vitality percentages: embedding coverage, anchor
  coverage, and stale high-resonance entries.],
  flags: (
    ([`--json`], [`flag`], [Output as JSON (default format for dashboard consumers).]),
  ),
  examples: (
    "mx memory health",
    "mx memory health --json",
  ),
)

#command(
  "mx memory growth",
  [Show per-week entry growth over the last 8 weeks.],
  flags: (
    ([`--json`], [`flag`], [Output as JSON array of 8 integers (oldest to newest).]),
  ),
  examples: (
    "mx memory growth",
    "mx memory growth --json",
  ),
)

#command(
  "mx memory open-threads",
  [List open threads (`category:thread` entries with `state=\"open\"` or no
  state).],
  flags: (
    ([`--json`], [`flag`], [Output as JSON array (required for dashboard consumers).]),
  ),
  examples: (
    "mx memory open-threads",
    "mx memory open-threads --json",
  ),
)


// ═══════════════════════════════════════════════════════════════════════
// EXPORT
// ═══════════════════════════════════════════════════════════════════════

== Export <export>

#command(
  "mx memory export",
  [Export the entire knowledge database to a file or directory.],
  flags: (
    ([`-f, --format`], [`string`], [Output format: `md`, `jsonl`, `csv`. Default: `md`.]),
    ([`-o, --output`], [`path`],   [Output directory for `md` format (defaults to `./memory-export`), or file for `jsonl`/`csv` (defaults to stdout).]),
  ),
  examples: (
    "mx memory export",
    "mx memory export -f jsonl -o backup.jsonl",
    "mx memory export -f csv -o entries.csv",
    "mx memory export -f md -o /data/export/",
  ),
)


// ═══════════════════════════════════════════════════════════════════════
// REINFORCEMENT
// ═══════════════════════════════════════════════════════════════════════

== Reinforcement <reinforcement>

#command(
  "mx memory reinforce",
  [Reinforce a knowledge entry by incrementing its resonance, updating
  `last_activated`, and incrementing `activation_count`. Used to signal
  that an entry remains relevant.],
  flags: (
    ([`--amount`], [`int`], [Amount to increase resonance by. Default: `1`.]),
    ([`--cap`],    [`int`], [Maximum resonance cap. Default: `10`.]),
    ([`--json`],   [`flag`], [Output as JSON.]),
  ),
  examples: (
    "mx memory reinforce kn-abc123",
    "mx memory reinforce kn-abc123 --amount 2 --cap 8",
  ),
)


// ═══════════════════════════════════════════════════════════════════════
// METADATA MANAGEMENT
// ═══════════════════════════════════════════════════════════════════════

== Metadata management <metadata>

The knowledge graph has several registries for typed metadata. These
commands manage the registries themselves -- the types, categories, and
agent identities that entries reference.

=== Agents

#command(
  "mx memory agents list",
  [List all registered agents.],
  flags: (
    ([`--json`], [`flag`], [Output as JSON.]),
  ),
  examples: (
    "mx memory agents list",
  ),
)

#command(
  "mx memory agents add",
  [Register a new agent.],
  flags: (
    ([`-d, --description`], [`string`], [Agent description.]),
    ([`-D, --domain`],      [`string`], [Agent domain/responsibility.]),
  ),
  examples: (
    "mx memory agents add whistledown -d \"Round-trip builder\" -D \"development\"",
  ),
)

#command(
  "mx memory agents show",
  [Show details for a specific agent.],
  flags: (),
  examples: (
    "mx memory agents show whistledown",
  ),
)

=== Projects

#command(
  "mx memory projects list",
  [List all registered projects.],
  flags: (
    ([`--json`], [`flag`], [Output as JSON.]),
  ),
  examples: (
    "mx memory projects list",
  ),
)

#command(
  "mx memory projects add",
  [Register a new project.],
  flags: (
    ([`--id`],          [`string`], [Unique project identifier.]),
    ([`--name`],        [`string`], [Human-readable project name.]),
    ([`--path`],        [`path`],   [Local filesystem path to the project.]),
    ([`--repo-url`],    [`string`], [Git repository URL (e.g., `owner/repo`).]),
    ([`--description`], [`string`], [Project description.]),
  ),
  examples: (
    "mx memory projects add --id mx --name \"mx CLI\" \\\n  --repo-url coryzibell/mx --path ~/recipes/coryzibell/mx",
  ),
)

=== Categories

#command(
  "mx memory categories list",
  [List all categories.],
  flags: (
    ([`--json`], [`flag`], [Output as JSON.]),
  ),
  examples: (
    "mx memory categories list",
  ),
)

#command(
  "mx memory categories add",
  [Add a new category.],
  flags: (),
  examples: (
    "mx memory categories add pitfall \"Things that went wrong and why\"",
  ),
)

#command(
  "mx memory categories remove",
  [Remove a category (only if no entries use it).],
  flags: (),
  examples: (
    "mx memory categories remove pitfall",
  ),
)

=== Applicability

#command(
  "mx memory applicability list",
  [List all applicability types.],
  flags: (),
  examples: (
    "mx memory applicability list",
  ),
)

#command(
  "mx memory applicability add",
  [Add a new applicability type.],
  flags: (
    ([`--id`],          [`string`], [Unique identifier.]),
    ([`--description`], [`string`], [Description of when this applicability applies.]),
    ([`--scope`],       [`string`], [Scope constraint (e.g., `project`, `global`).]),
  ),
  examples: (
    "mx memory applicability add --id rust-only \\\n  --description \"Applies only to Rust projects\" --scope project",
  ),
)

=== Type registries

These are read-only registries listing the valid values for typed fields.
Each supports `list` with an optional `--json` flag.

#table(
  columns: (auto, auto),
  table.header([*Command*], [*Lists valid values for*]),
  [`mx memory tags list`],              [Tags used across entries. Supports `--category` filter.],
  [`mx memory source-types list`],      [Source types (`manual`, `ram`, `cache`, `agent_session`).],
  [`mx memory entry-types list`],       [Entry types (`primary`, `summary`, `synthesis`).],
  [`mx memory session-types list`],     [Session types (e.g., `development`, `review`, `exploration`).],
  [`mx memory relationship-types list`], [Relationship types (`related`, `supersedes`, `extends`, `implements`, `contradicts`).],
  [`mx memory content-types list`],     [Content types (`text`, `code`, `config`, `data`, `binary`).],
)

All type registry `list` commands accept `--json` for structured output.
`tags list` also accepts `--category` to filter tags to a specific category.


// ═══════════════════════════════════════════════════════════════════════
// SESSION TRACKING
// ═══════════════════════════════════════════════════════════════════════

== Session tracking <sessions>

Sessions group entries created during a work period. Entries can be linked
to sessions, and facts can be queried by their source session.

#command(
  "mx memory sessions list",
  [List sessions, optionally filtered by project.],
  flags: (
    ([`--project`], [`string`], [Filter by project ID.]),
    ([`--json`],    [`flag`],   [Output as JSON.]),
  ),
  examples: (
    "mx memory sessions list",
    "mx memory sessions list --project mx",
  ),
)

#command(
  "mx memory sessions create",
  [Create a new session.],
  flags: (
    ([`--session-type`], [`string`], [Session type (e.g., `development`, `review`, `exploration`).]),
    ([`--project`],      [`string`], [Associated project ID.]),
  ),
  examples: (
    "mx memory sessions create --session-type development --project mx",
  ),
)

#command(
  "mx memory sessions close",
  [Close an active session.],
  flags: (
    ([`--id`], [`string`], [Session ID to close.]),
  ),
  examples: (
    "mx memory sessions close --id ses-abc123",
  ),
)

#command(
  "mx memory for-session",
  [List facts extracted from a specific session. The session ID can be
  provided with or without the `kn-` prefix.],
  flags: (
    ([`--json`], [`flag`], [Output as JSON.]),
  ),
  examples: (
    "mx memory for-session ses-abc123",
  ),
)

#command(
  "mx memory fact-session",
  [Get the session a fact was extracted from. The fact ID can be provided
  with or without the `kn-` prefix.],
  flags: (
    ([`--json`], [`flag`], [Output as JSON.]),
  ),
  examples: (
    "mx memory fact-session kn-abc123",
  ),
)
