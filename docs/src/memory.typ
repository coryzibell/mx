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

#note[The database schema is applied automatically on every connection, in both
embedded and network mode. All schema statements are idempotent
(`IF NOT EXISTS` / `UPSERT`), so no manual setup is required. Set `MX_SKIP_SCHEMA=1` to skip
auto-apply in environments with restricted DB permissions. Run `mx migrate` to
explicitly apply the schema (it ignores `MX_SKIP_SCHEMA`).]

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
    ([`--no-auto-anchor`], [`flag`], [Skip automatic anchor generation.]),
    ([`--no-embed`],   [`flag`],   [Skip synchronous embedding generation on write.]),
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

#command(
  "mx memory add-batch",
  [Add multiple entries in a single invocation from a JSONL file or stdin.
  Each line is a JSON object whose fields match `mx memory add` arguments
  (`category`, `title`, `content`, `tags`, `source_agent`, `type`, etc.).
  The store is opened once and the ~435 MB embedding model is loaded once for
  the whole batch, so a 30-entry bulk caller pays one cold-load instead of
  thirty. Malformed lines are skipped and reported at the end; one bad entry
  does not abort the rest (partial-success semantics). Exits non-zero when any
  entry failed.

  *Recommended delivery:* pass a JSONL file via `--file` when calling through
  a `sudo` wrapper (e.g. the hearth `_secret-exec` proxy) — a file path
  survives wrapper layers where a piped stdin may not.],
  flags: (
    ([`-f, --file`], [`path`],  [Path to a JSONL file (one JSON object per line). Reads from stdin when omitted.]),
    ([`--no-embed`], [`flag`],  [Skip embedding for the whole batch. Defers to the next `mx memory embed --all` run (e.g. a nightly cron). Keyword and tag search are unaffected; only `--semantic` search misses un-embedded entries until then.]),
  ),
  examples: (
    "# Stdin pipe\nprintf '{\"category\":\"insight\",\"title\":\"T1\",\"content\":\"C1\",\"source_agent\":\"soren\"}\\n{\"type\":\"decision\",\"content\":\"chose Rust\",\"source_agent\":\"soren\"}\\n' \\\n  | mx memory add-batch",
    "# File (safer through sudo wrappers)\nmx memory add-batch --file /tmp/pocket-entries.jsonl",
    "# Skip embedding — defer to nightly embed --all\nmx memory add-batch --file entries.jsonl --no-embed",
  ),
)

#note[Each JSONL line for `add-batch` is a self-describing payload: include all
per-entry fields (`category`, `title`, `content`, `source_agent`, `tags`,
`private`, `resonance`, `type`, etc.) on each line. There are no batch-wide
field overrides except `--no-embed`. This design supports heterogeneous batches
(facts, person nodes, summaries, blooms) in a single invocation --- which is
the primary pocket use-case.]


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
  [`--min-resonance`], [`int`],    [Minimum resonance level. Filtered on the time-decayed *effective* resonance (see note below on the basis divergence with `wake`).],
  [`--max-resonance`], [`int`],    [Maximum resonance level. Filtered on the time-decayed *effective* resonance.],
  [`--has-wake-phrase`], [`flag`],  [Filter to entries WITH a wake phrase.],
  [`--missing-wake-phrase`], [`flag`], [Filter to entries WITHOUT a wake phrase.],
  [`--has-anchors`],   [`flag`],   [Filter to entries WITH anchors.],
  [`--missing-anchors`], [`flag`], [Filter to entries WITHOUT anchors.],
  [`--has-resonance-type`], [`flag`], [Filter to entries WITH a resonance type.],
  [`--missing-resonance-type`], [`flag`], [Filter to entries WITHOUT a resonance type.],
  [`--limit`],         [`int`],    [Limit number of results.],
  [`--tags`],          [`string`], [Filter by tags (comma-separated, matches any).],
)

#note[*Visibility default:* `list` and `search` run *public-only* by default.
Your own private entries are omitted unless you pass `--include-private` (or
`--mine`, which shows only your private entries). When a public-only query
matches private entries you own but hides them, `mx` prints a best-effort
*stderr* nudge --- see the note below on the hidden-private hint. (`wake`, by
contrast, includes owned-private blooms in its cascade.)]

#note[*Hidden-private hint:* When `MX_CURRENT_AGENT` is set and a public-only
`list`/`search` query matches private entries you own, `mx` writes a hint to
*stderr* pointing at `--include-private`, e.g.:

```
note: 3 private entries of yours matched but are hidden; use --include-private to see them
```

The hint is *stderr only* --- it never touches `stdout`, `--json` output, or the
exit code, and any error computing it is swallowed silently. It fires only in
the public-only case: passing `--include-private` or `--mine` silences it (they
already show your private entries), and it is *suppressed under `search
--semantic`* because the underlying count uses the keyword (BM25) predicate,
which does not agree with vector similarity.]

#note[*Resonance basis divergence (Issue \#404):* `list`/`search --min-resonance`
and `--max-resonance` filter on the time-decayed *effective* resonance, whereas
`wake --min-resonance` filters on the *raw* stored value. The same numeric
threshold can therefore admit an entry under `wake` but exclude it under
`list`/`search` once decay is applied. `foundational` and `transformative`
resonance types are decay-exempt, so they filter identically under both. This
divergence is intentional for now; an explicit `--resonance-basis raw|decayed`
flag is tracked in \#404.]

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

#note[`list` accepts all shared filter flags documented above, including the
public-only visibility default and the hidden-private stderr hint.]

#command(
  "mx memory search",
  [Search entries by keyword or semantic similarity. Keyword search is the
  default; add `--semantic` to use vector embeddings.],
  flags: (
    ([`--semantic`], [`flag`], [Use semantic (vector) search instead of keyword search.]),
    ([`--activate`], [`flag`], [Activate all returned results: resets `last_activated` (decay clock) and increments `activation_count`. Marks results as intentionally consumed rather than just browsed.]),
  ),
  examples: (
    "mx memory search \"retry pattern\"",
    "mx memory search \"how to handle timeouts\" --semantic",
    "mx memory search \"agent bootstrap\" -c recipe,method --limit 5",
    "# Search and activate results (mark as consumed)\nmx memory search \"retry pattern\" --activate",
  ),
)

#note[`search` accepts all shared filter flags, including the public-only
visibility default and the hidden-private stderr hint (the hint is suppressed
under `--semantic`). Semantic search requires entries to have embeddings
generated via `mx memory embed`.]

#tip[By default, search does not activate results -- browsing is not the same as
engagement. Use `--activate` when you are intentionally consuming the results
(e.g., loading context for a task), not just exploring.]

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
    ([`--no-auto-anchor`], [`flag`],  [Skip automatic anchor generation.]),
    ([`--no-embed`],      [`flag`],  [Skip synchronous embedding generation on write.]),
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
    ([`--no-auto-anchor`], [`flag`], [Skip automatic anchor generation.]),
    ([`--no-embed`],    [`flag`],   [Skip synchronous embedding generation on write.]),
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
    ([`--no-auto-anchor`], [`flag`], [Skip automatic anchor generation.]),
    ([`--no-embed`], [`flag`],  [Skip synchronous embedding generation on write.]),
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
    ([`--no-auto-anchor`], [`flag`], [Skip automatic anchor generation.]),
    ([`--no-embed`], [`flag`],  [Skip synchronous embedding generation on write.]),
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
    ([`--no-auto-anchor`], [`flag`], [Skip automatic anchor generation.]),
    ([`--no-embed`], [`flag`], [Skip synchronous embedding generation on write.]),
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
high-resonance entries ("blooms") and presents them through a cascade that
reconnects the agent to its knowledge. The default output is a plain-text
cascade; a token-based ritual flow is available for programmatic use.

#command(
  "mx memory wake",
  [Wake up with resonant identity cascade. Retrieves high-resonance blooms
  and presents them in the requested format.],
  flags: (
    ([`-l, --limit`],   [`int`],    [Number of blooms to return. Default: `20`.]),
    ([`--min-resonance`], [`int`],   [Minimum resonance threshold -- get ALL blooms >= this value (overrides `--limit`). Filtered on the *raw* stored resonance, unlike `list`/`search`, which use the decayed *effective* value (Issue \#404).]),
    ([`-d, --days`],    [`int`],    [Include memories activated in last N days. Default: `7`.]),
    ([`--no-activate`], [`flag`],   [Do not update activation counts.]),
    ([`--begin`],       [`flag`],   [Start token-based wake ritual. Returns first bloom and session token.]),
    ([`--bloom-id`],    [`string`], [Bloom ID for `--respond` or `--skip` operations.]),
    ([`--respond`],     [`string`], [Submit wake phrase response for a bloom.]),
    ([`--skip`],        [`flag`],   [Skip a bloom without wake phrase.]),
    ([`--session`],     [`string`], [Session token for chained ritual (required with `--respond` or `--skip`).]),
  ),
  examples: (
    "# Default wake -- top 20 blooms, text output\nmx memory wake",
    "# All blooms with resonance >= 7\nmx memory wake --min-resonance 7",
    "# Token-based ritual (for non-TTY / programmatic use)\nmx memory wake --begin\nmx memory wake --bloom-id kn-abc --respond \"the phrase\" --session tok-xyz\nmx memory wake --bloom-id kn-def --skip --session tok-xyz",
  ),
)

#note[`MX_CURRENT_AGENT` must be set for wake to function. The wake system
reads blooms ordered by resonance and wake order.]

=== Wake modes

- *Default* (`mx memory wake`): plain text cascade output, blooms listed with titles and content.
- *Token-based* (`--begin`, `--respond`, `--skip`): stateless chained ritual for non-interactive environments. Start with `--begin`, then loop with `--respond` or `--skip` using the returned session token and bloom ID.

#command(
  "mx memory wake-fetch",
  [Fetch facts for the wake ritual. Returns entries with resonance >= 3
  across all types, sorted by resonance (highest first). Designed as a
  data source for wake ritual presentation.],
  flags: (
    ([`--days`],  [`int`], [Number of days to look back. Default: `15`.]),
    ([`--limit`], [`int`], [Maximum number of results. Default: `100`.]),
    ([`--exclude-tags`], [`string`], [Comma-separated list of tag prefixes. Drops any entry that carries at least one tag prefix-matching at least one of these values -- useful for excluding whole tag namespaces (e.g. `project/` removes every entry tagged `project/<anything>`). Matching is OR across both the entry's tags and the supplied prefixes. Empty segments from trailing commas are ignored.]),
  ),
  examples: (
    "mx memory wake-fetch",
    "mx memory wake-fetch --days 30 --limit 50",
    "# Exclude an entire tag namespace from the wake set\nmx memory wake-fetch --exclude-tags 'project/'",
    "# Exclude multiple namespaces at once\nmx memory wake-fetch --exclude-tags 'project/,scratch/'",
  ),
)


// ═══════════════════════════════════════════════════════════════════════
// EMBEDDINGS & ANCHORING
// ═══════════════════════════════════════════════════════════════════════

== Embeddings and anchoring <embeddings>

Embeddings enable semantic search and automatic relationship discovery.
Each entry can have a vector embedding generated from its title and content.
Anchors are connections between entries discovered via embedding similarity.

=== Chunked embeddings <chunked-embeddings>

Entries longer than 400 tokens are automatically split into overlapping chunks
before embedding. This ensures semantic search covers the full content of long
entries, not just the first 400 tokens.

*How it works:*

+ The entry's embedding text (title + body/summary + tags) is tokenized using
  the BGE-Base-EN-v1.5 tokenizer.
+ If the text fits within 400 tokens, a single embedding is generated and
  stored on the entry --- exactly as before. No chunks are created.
+ If the text exceeds 400 tokens, it is split into overlapping chunks with a
  sliding window: 400 tokens per chunk, 100-token overlap (stride 300).
+ Each chunk is embedded separately and stored in the `embedding_chunk` table.
+ A normalized mean vector of all chunk embeddings is stored on the entry's
  `embedding` field for `auto-anchor` compatibility.
+ The entry's `chunk_count` field records how many chunks were created (0 for
  unchunked entries).

*Semantic search with chunks:*

When `mx memory search --semantic` runs, it queries both unchunked entry
embeddings and chunk embeddings in parallel. Results are merged by taking the
maximum similarity score per entry --- if a chunk from entry X scores 0.92 and
the entry's mean vector scores 0.85, the entry's final score is 0.92. This
ensures long entries surface when any section is relevant, not just when the
overall average is relevant.

#tip[Short entries (≤400 tokens) behave exactly as before --- single embedding,
no chunks, no behavior change. Chunking only activates for entries that exceed
the 400-token threshold.]

#note[The `embedding_text()` method on entries no longer truncates body content.
The chunker handles length management, ensuring no content is lost during
embedding.]

#command(
  "mx memory embed",
  [Generate a vector embedding for one or all entries. Embeddings power
  semantic search (`--semantic` flag on `search`) and automatic anchoring.
  Long entries (>400 tokens) are automatically split into overlapping chunks,
  with each chunk embedded separately. Short entries get a single embedding.],
  flags: (
    ([`-a, --all`], [`flag`], [Embed all knowledge entries (instead of a single ID).]),
    ([`--long-only`], [`int`], [Only re-embed entries whose `embedding_text()` exceeds this many tokens. Entries at or below the threshold are skipped entirely. Use with `--all`. Useful for selectively re-embedding long entries that were previously truncated at a smaller token limit (e.g., 512).]),
  ),
  examples: (
    "mx memory embed kn-abc123",
    "mx memory embed --all",
    "# Re-embed only entries that exceed 512 tokens\nmx memory embed --all --long-only 512",
  ),
)

#command(
  "mx memory auto-anchor",
  [Automatically add anchors between entries based on embedding similarity.
  Processes a single entry or all entries that have embeddings.

  Also re-evaluates existing anchors: any anchor whose cosine similarity has
  fallen below the threshold (default 0.75) or risen above the near-duplicate
  ceiling (0.95) is pruned. This keeps the anchor graph self-cleaning --
  anchors that made sense once but no longer do are removed automatically.],
  flags: (
    ([`--threshold`],   [`float`], [Minimum cosine similarity (0.0--1.0). Default: `0.75`.]),
    ([`--max-anchors`], [`int`],   [Maximum anchors to add per entry. Default: `5`.]),
    ([`--dry-run`],     [`flag`],  [Preview changes without writing.]),
    ([`--detailed`],    [`flag`],  [Show similarity scores in output.]),
    ([`--fill`],        [`flag`],  [Only process entries with zero existing anchors. Fills gaps in the graph without touching already-anchored entries.]),
  ),
  examples: (
    "mx memory auto-anchor",
    "mx memory auto-anchor kn-abc123 --threshold 0.8 --max-anchors 3",
    "mx memory auto-anchor --dry-run --detailed",
    "mx memory auto-anchor --fill",
  ),
)

#tip[A typical workflow: run `mx memory embed --all` to generate embeddings,
then `mx memory auto-anchor --dry-run --detailed` to preview anchor
candidates, then `mx memory auto-anchor` to write them.]

#note[Anchors are also maintained automatically on every write operation
(`add`, `update`, `edit`, `append`, `prepend`, `restore`). After each write,
mx re-evaluates anchors and prunes stale ones using the same similarity
thresholds. Pass `--no-auto-anchor` on any of these commands to skip this
step -- useful for bulk operations or cleanup scripts where the overhead is
unwanted.]

#note[*Deferred embedding:* Pass `--no-embed` (or set `MX_SKIP_WRITE_EMBED=1`)
on any write command (`add`, `update`, `edit`, `append`, `prepend`, `restore`,
`add-batch`) to skip synchronous embedding generation on that write. The entry
is written and fully durable immediately; only the vector embedding is deferred.

*Trade-off:* entries written with `--no-embed` are absent from `--semantic`
(vector) search results until a later `mx memory embed --all` run fills the
gap. Keyword search (`mx memory search <query>`) and tag-based filters are
unaffected.

This flag exists to amortize the ~435 MB embedding model cold-load: use it with
`add-batch` (which does its own single hoisted embed pass at the end), or set
`MX_SKIP_WRITE_EMBED=1` globally in a deployment where a nightly
`mx memory embed --all` (e.g. via Cinder's `embed.nix` timer) keeps the graph
fresh.]


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
  [Add a typed relationship between two entries. By default, the target entry
  (`--to`) is automatically reinforced by +1 (capped at 10) when the
  relationship is created -- being linked to means the fact proved relevant.
  The `contradicts` and `supersedes` types are excluded from auto-reinforcement
  because boosting an outdated or contradicted entry works against intent.],
  flags: (
    ([`--from`],         [`string`], [Source entry ID.]),
    ([`--to`],           [`string`], [Target entry ID.]),
    ([`--type`],         [`string`], [Relationship type: `related`, `supersedes`, `extends`, `implements`, `contradicts`.]),
    ([`--no-reinforce`], [`flag`],   [Skip automatic reinforcement of the target entry.]),
  ),
  examples: (
    "mx memory relationships add --from kn-abc --to kn-def --type extends",
    "mx memory relationships add --from kn-abc --to kn-ghi --type supersedes",
    "# Add a relationship without auto-reinforcing the target\nmx memory relationships add --from kn-abc --to kn-def --type related --no-reinforce",
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

Reinforcement is the mechanism by which the knowledge graph breathes in --
entries that are used, referenced, or linked gain resonance, counteracting
the natural decay of the exhale. There are three reinforcement paths:

+ *Explicit reinforcement* via `mx memory reinforce` -- directly boost an
  entry's resonance.
+ *Auto-reinforce on relationship creation* -- when
  `mx memory relationships add` links to a target entry, the target is
  reinforced by +1 (capped at 10). The `contradicts` and `supersedes`
  types are excluded. Use `--no-reinforce` to opt out.
+ *Search activation* via `mx memory search --activate` -- marks returned
  results as intentionally consumed, resetting their decay clock and
  incrementing their activation count.

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
