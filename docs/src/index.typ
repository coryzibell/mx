#import "lib.typ": *

#page-header(
  "mx",
  "A Swiss army knife for Claude Code and multi-agent toolkits."
)

mx is a Rust CLI providing encoded git operations, a SurrealDB-backed knowledge
graph, session archival, GitHub sync, and emotional state tensors. Designed for
use with Claude Code, but works with any multi-agent workflow that needs
persistent memory, encoded commits, or session management.

== Quick links

- #link("getting-started.html")[Getting Started] -- install mx and make your first encoded commit
- #link("commit.html")[Commit] -- encoded git commits
- #link("log.html")[Log] -- decoded git log
- #link("memory.html")[Memory] -- knowledge graph operations
- #link("codex.html")[Codex] -- session archival
- #link("kv.html")[KV] -- local key-value store
- #link("state.html")[State] -- emotional state tensors
- #link("sync.html")[Sync] -- GitHub sync
- #link("pr.html")[PR] -- pull request merge
- #link("github.html")[GitHub] -- GitHub operations
- #link("convert.html")[Convert] -- conversion utilities

== Features

=== Encoded commits

`mx commit` wraps `git commit` but encodes the message using
#link("base-d.html")[base-d]. The commit title is hashed and the body is
compressed, each encoded through a randomly selected dictionary. The result
looks like hieroglyphs in `git log` but decodes cleanly with `mx log`.

```bash
mx commit "fix session export crash on empty JSONL" -a
mx log
```

=== Knowledge graph

The #link("memory.html")[memory] system is a knowledge graph backed by SurrealDB
(or embedded SurrealKV). Entries have categories, tags, resonance levels,
embeddings, and relationships.

```bash
mx memory search "session bootstrap"
mx memory search "how to handle state" --semantic
mx memory add --category pattern --title "Retry pattern" \
  --content "Use exponential backoff..." --tags "reliability"
```

=== Session archival

The #link("codex.html")[codex] archives Claude session JSONL files to permanent
storage with transcripts, extracted images, and manifests.

```bash
mx codex archive
mx codex list
mx codex read <archive-id> --clean
```

=== Local key-value store

#link("kv.html")[KV] provides fast per-agent state: counters, strings, lists,
and history with time-based queries and structured data filtering.

```bash
mx kv set session.goal "ship the docs"
mx kv get session.goal
mx kv push decisions "chose Typst over markdown"
mx kv get shipped --id 35-64
mx kv push projects "palmtop DSI fix" \
  --data '{"tags":["palmtop","i915"],"status":"active"}'
mx kv search projects --where status=active
```

== Installation

From #link("https://crates.io/crates/mx")[crates.io]:

```bash
cargo install mx
```

Or from source:

```bash
git clone https://github.com/coryzibell/mx.git
cd mx
cargo install --path .
```

Requires Rust 2024 edition.

== Configuration

Everything mx writes lives under a single base directory: `$MX_HOME`, which
defaults to `~/.mx/`. See #link("paths.html")[Filesystem Layout] for the full
reference.

== Start here

New to mx? Start with #link("getting-started.html")[Getting Started] for a
hands-on walkthrough of installation, your first encoded commit, and a tour
of the subsystems.
