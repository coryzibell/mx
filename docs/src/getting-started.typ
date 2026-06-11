#import "lib.typ": *

#page-header(
  "Getting Started",
  "Install mx, make your first encoded commit, and explore the subsystems."
)

== Installation

=== From crates.io

```bash
cargo install mx
```

=== From source

```bash
git clone https://github.com/coryzibell/mx.git
cd mx
cargo install --path .
```

Requires Rust 2024 edition. The binary is named `mx`.

#tip[Run `mx --version` to verify the installation.]

== Your first encoded commit

The core workflow that makes mx unique is encoded commits. Every commit message
is hashed and compressed through randomly selected dictionaries, producing
output that looks like hieroglyphs in raw `git log` but decodes cleanly with
`mx log`.

=== Make a change and commit

```bash
echo "hello" > test.txt
mx commit "add test file" -a
```

The `-a` flag stages all changes before committing, just like `git commit -a`.
You will see a footer line showing which algorithms and dictionaries were used,
something like `[sha256:ocean|zstd:forest]`.

=== Read it back

```bash
mx log
```

This shows the last 10 commits with decoded messages. `mx log` has full parity
with `git log` -- use any display or filter flag you already know:

```bash
mx log -3                          # last 3 commits (-N shorthand)
mx log --oneline                   # one-line format with ref decorations
mx log --stat                      # include diffstat per commit
mx log -n 5 --full                 # full details for the last 5
mx log --format=fuller -3          # git's fuller format, decoded
mx log --author="charlie" -p       # filter by author, show patches
```

To inspect a single commit (decoded replacement for `git show`):

```bash
mx show
mx show abc1234 --stat
```

=== Preview without committing

If you want to see what the encoding produces without actually committing:

```bash
mx commit "your message" --dry-run
```

Or to test title/body encoding separately:

```bash
mx commit --encode-only --title "refactor store" --body "split backends"
```

#note[Always use `mx log` and `mx show` to read commit history. Raw `git log`
and `git show` show encoded output that is intentionally unreadable. See
#link("base-d.html")[base-d] for how the encoding works.]

== Setting up MX_HOME

By default, mx stores everything under `~/.mx/`. To move the entire tree:

```bash
export MX_HOME=/data/mx
```

Add this to your shell profile (`.bashrc`, `.zshrc`, etc.) to make it
permanent. Individual subsystems can be overridden separately -- see
#link("paths.html")[Filesystem Layout] for the full reference.

== Subsystems at a glance

=== Memory

The #link("memory.html")[memory] system is a knowledge graph backed by SurrealDB.
Store patterns, insights, decisions, and reference material with categories,
tags, resonance levels, and semantic search via embeddings.

```bash
mx memory search "retry pattern" --semantic
mx memory add --category insight --title "Always check timeouts" \
  --content "Connection pools need explicit timeout config" \
  --tags "reliability,networking"
mx memory stats
```

=== Codex

The #link("codex.html")[codex] archives Claude Code sessions to permanent storage.
Clean markdown transcripts, extracted images, and searchable manifests.

```bash
mx codex archive           # archive current session
mx codex archive --all     # archive everything unarchived
mx codex list              # see what you have
mx codex search "migration"
```

=== KV

The #link("kv.html")[kv] store provides fast local state per agent. Counters,
strings, lists, and history with time-based queries and structured data
filtering. Schema-driven with defaults.

```bash
mx kv set session.goal "ship the docs"
mx kv inc builds
mx kv push decisions "chose Typst for docs"  # prints: kv-A3fB (1)
mx kv last decisions --count 5
mx kv last decisions --since 1w
mx kv count decisions --day 2026-05-07
mx kv get decisions --id kv-A3fB              # look up by entry ID

# Define, list, edit, and remove keys with the schema subcommands
mx kv schema add puns --type history --max-entries 500
mx kv schema list                             # (replaces 'kv keys')
mx kv schema update puns --description "the good ones"
mx kv schema drop puns --force                # removes definition + data

# Auto-create a key in the schema and push in one step (inline shortcut)
mx kv push puns "the joke" --create history
mx kv push ideas "wild thought" --create list --max-entries 500

# Batch set multiple state fields at once
mx kv set context goal="done" phase="writing"
mx kv set context --json '{"goal":"done","phase":"writing"}'
mx kv set mytensor --json '[0.4, 0.6, 0.5]'
echo '{"goal":"done"}' | mx kv set context --json -

# Link entries to the memory graph
mx kv push decisions "adopted memory links" --memory kn-abc123
mx kv set decisions --id 17 --memory kn-abc123
mx kv last decisions --count 3 --memory       # resolves linked entries

# Attach structured data and query it
mx kv push projects "palmtop DSI fix" \
  --data '{"tags":["palmtop","i915"],"status":"active"}'
mx kv search projects --where status=active
mx kv search projects "DSI" --where tags=palmtop

# Update an existing entry's value or data in-place
mx kv update projects "palmtop DSI fix (v2)" --id kv-A3fB
mx kv update projects --id 42 --data '{"status":"done"}'

# Migrate entries to match current schema data definitions
mx kv migrate projects --dry-run
mx kv migrate projects --prune

# Rename a key (preserves all entries and data)
mx kv schema update old_decisions --name archived_decisions

# JSON output for scripting and jq piping
mx kv last projects --count 5 --json | jq '.[].data.status'
mx kv count shipped --json | jq '.count'
```

=== State (deprecated)

#deprecated[
  `mx state` is deprecated and will be removed in a future release. Use `mx kv`
  with structured data (`--data`) instead.
]

The #link("state.html")[state] system encodes emotional state tensors -- multi-
dimensional values compressed into a compact string format. Used for agent
co-regulation and identity tracking.

```bash
mx state encode -d "temp=0.8 entropy=0.75 agency=0.4"
mx state decode "@state:tensor|0.8|0.75|0.4"
mx state schemas
```

=== PR

#link("pr.html")[PR merge] handles pull request merging with encoded commit
messages. Supports squash (default), rebase, and standard merge commits.

```bash
mx pr merge 42             # squash merge
mx pr merge 42 --rebase    # rebase merge
```

=== Sync

#link("sync.html")[Sync] pulls and pushes GitHub issues and discussions as local
YAML files for offline editing and batch operations.

```bash
mx sync pull owner/repo
mx sync push owner/repo --dry-run
```

=== Migrate

`mx migrate` explicitly applies the database schema to SurrealDB. The schema is
normally applied automatically on every connection (both embedded and network
mode), but `migrate` is useful after upgrading mx, or to apply the schema on an
instance where `MX_SKIP_SCHEMA=1` is set.

```bash
mx migrate            # apply schema (ignores MX_SKIP_SCHEMA)
mx migrate -v         # verbose: see connection and schema details
```

== Environment variable reference <env-vars>

Key environment variables recognized by mx. All boolean opt-outs follow the
same convention: `"1"` or `"true"` (case-insensitive) to disable; any other
value leaves the default behavior on.

SurrealDB connection variables (`MX_SURREAL_MODE`, `MX_SURREAL_URL`,
`MX_SURREAL_NS`, `MX_SURREAL_DB`, etc.) are documented in full with their
defaults in #link("paths.html#env-surreal")[filesystem layout → SurrealDB connection].

#table(
  columns: (auto, auto, auto, 1fr),
  [*Variable*], [*Type*], [*Default*], [*Description*],
  [`MX_HOME`], [path], [`~/.mx/`], [Root directory for all mx data. Override to move the whole tree.],
  [`MX_CURRENT_AGENT`], [string], [—], [Active agent identifier. Used as default `source_agent` on writes and to scope private entry visibility.],
  [`MX_SKIP_SCHEMA`], [bool], [`false`], [Skip automatic schema application on connection. Use `mx migrate` to apply the schema explicitly when set.],
  [`MX_SKIP_WRITE_ANCHOR`], [bool], [`false`], [Skip synchronous `auto-anchor` on every write. Equivalent to passing `--no-auto-anchor` globally. Useful when a nightly `mx memory auto-anchor` handles anchoring in batch.],
  [`MX_SKIP_WRITE_EMBED`], [bool], [`false`], [Skip synchronous embedding generation on every write. Equivalent to passing `--no-embed` globally. Entries written with this set are absent from `--semantic` search until `mx memory embed --all` runs. Intended for deployments where a nightly embed timer (e.g. Cinder's `embed.nix`) keeps the graph fresh.],
)

== What's next

- Read the #link("commit.html")[commit], #link("log.html")[log], and
  #link("show.html")[show] reference pages for the full flag reference
- Explore the #link("memory.html")[memory] system for persistent knowledge
- Check #link("paths.html")[filesystem layout] for configuration options
- See #link("architecture.html")[architecture] for how mx is built internally
