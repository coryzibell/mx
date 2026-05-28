# mx

A Swiss army knife for Claude Code and multi-agent toolkits.

------------------------------------------------------------------------

mx is a Rust CLI providing encoded git operations, a SurrealDB-backed
knowledge graph, session archival, GitHub sync, and emotional state
tensors. Designed for use with Claude Code, but works with any
multi-agent workflow that needs persistent memory, encoded commits, or
session management.

## Quick links

- Getting Started -- install mx and make your first encoded commit

- Commit -- encoded git commits

- Log -- decoded git log

- Memory -- knowledge graph operations

- Codex -- session archival

- KV -- local key-value store

- State -- emotional state tensors *(deprecated)*

- Sync -- GitHub sync

- PR -- pull request merge

- GitHub -- GitHub operations

- Convert -- conversion utilities

## Features

### Encoded commits

`mx commit` wraps `git commit` but encodes the message using base-d. The
commit title is hashed and the body is compressed, each encoded through
a randomly selected dictionary. The result looks like hieroglyphs in
`git log` but decodes cleanly with `mx log`.

``` bash
mx commit "fix session export crash on empty JSONL" -a
mx log
```

### Knowledge graph

The memory system is a knowledge graph backed by SurrealDB (or embedded
SurrealKV). Entries have categories, tags, resonance levels, embeddings,
and relationships.

``` bash
mx memory search "session bootstrap"
mx memory search "how to handle state" --semantic
mx memory add --category pattern --title "Retry pattern" \
  --content "Use exponential backoff..." --tags "reliability"
```

### Session archival

The codex archives Claude session JSONL files to permanent storage with
transcripts, extracted images, and manifests.

``` bash
mx codex archive
mx codex list
mx codex read <archive-id> --clean
```

### Local key-value store

KV provides fast per-agent state: counters, strings, lists, and history
with time-based queries and structured data filtering.

``` bash
mx kv set session.goal "ship the docs"
mx kv get session.goal
mx kv push decisions "chose Typst over markdown"  # prints: kv-A3fB (1)
mx kv get shipped --id kv-A3fB
mx kv get shipped --id 35-64
mx kv push puns "the joke" --create history        # auto-adds key to schema
mx kv push projects "palmtop DSI fix" \
  --data '{"tags":["palmtop","i915"],"status":"active"}'
mx kv search projects --where status=active
mx kv push decisions "adopted memory links" --memory kn-abc123
mx kv set decisions --id 17 --memory kn-abc123
mx kv last projects --count 5 --json | jq '.[].data.status'
```

## Installation

From crates.io:

``` bash
cargo install mx
```

Or from source:

``` bash
git clone https://github.com/coryzibell/mx.git
cd mx
cargo install --path .
```

Requires Rust 2024 edition.

## Configuration

Everything mx writes lives under a single base directory: `$MX_HOME`,
which defaults to `~/.mx/`. See Filesystem Layout for the full
reference.

## Start here

New to mx? Start with Getting Started for a hands-on walkthrough of
installation, your first encoded commit, and a tour of the subsystems.

# Getting Started

Install mx, make your first encoded commit, and explore the subsystems.

------------------------------------------------------------------------

## Installation

### From crates.io

``` bash
cargo install mx
```

### From source

``` bash
git clone https://github.com/coryzibell/mx.git
cd mx
cargo install --path .
```

Requires Rust 2024 edition. The binary is named `mx`.

::: {.admonition .tip}
**TIP:** Run `mx --version` to verify the installation.
:::

## Your first encoded commit

The core workflow that makes mx unique is encoded commits. Every commit
message is hashed and compressed through randomly selected dictionaries,
producing output that looks like hieroglyphs in raw `git log` but
decodes cleanly with `mx log`.

### Make a change and commit

``` bash
echo "hello" > test.txt
mx commit "add test file" -a
```

The `-a` flag stages all changes before committing, just like
`git commit -a`. You will see a footer line showing which algorithms and
dictionaries were used, something like `[sha256:ocean|zstd:forest]`.

### Read it back

``` bash
mx log
```

This shows the last 10 commits with decoded messages. `mx log` has full
parity with `git log` -- use any display or filter flag you already
know:

``` bash
mx log -3                          # last 3 commits (-N shorthand)
mx log --oneline                   # one-line format with ref decorations
mx log --stat                      # include diffstat per commit
mx log -n 5 --full                 # full details for the last 5
mx log --format=fuller -3          # git's fuller format, decoded
mx log --author="charlie" -p       # filter by author, show patches
```

To inspect a single commit (decoded replacement for `git show`):

``` bash
mx show
mx show abc1234 --stat
```

### Preview without committing

If you want to see what the encoding produces without actually
committing:

``` bash
mx commit "your message" --dry-run
```

Or to test title/body encoding separately:

``` bash
mx commit --encode-only --title "refactor store" --body "split backends"
```

::: {.admonition .note}
**NOTE:** Always use `mx log` and `mx show` to read commit history. Raw
`git log` and `git show` show encoded output that is intentionally
unreadable. See base-d for how the encoding works.
:::

## Setting up MX_HOME

By default, mx stores everything under `~/.mx/`. To move the entire
tree:

``` bash
export MX_HOME=/data/mx
```

Add this to your shell profile (`.bashrc`, `.zshrc`, etc.) to make it
permanent. Individual subsystems can be overridden separately -- see
Filesystem Layout for the full reference.

## Subsystems at a glance

### Memory

The memory system is a knowledge graph backed by SurrealDB. Store
patterns, insights, decisions, and reference material with categories,
tags, resonance levels, and semantic search via embeddings.

``` bash
mx memory search "retry pattern" --semantic
mx memory add --category insight --title "Always check timeouts" \
  --content "Connection pools need explicit timeout config" \
  --tags "reliability,networking"
mx memory stats
```

### Codex

The codex archives Claude Code sessions to permanent storage. Clean
markdown transcripts, extracted images, and searchable manifests.

``` bash
mx codex archive           # archive current session
mx codex archive --all     # archive everything unarchived
mx codex list              # see what you have
mx codex search "migration"
```

### KV

The kv store provides fast local state per agent. Counters, strings,
lists, and history with time-based queries and structured data
filtering. Schema-driven with defaults.

``` bash
mx kv set session.goal "ship the docs"
mx kv inc builds
mx kv push decisions "chose Typst for docs"  # prints: kv-A3fB (1)
mx kv last decisions --count 5
mx kv last decisions --since 1w
mx kv count decisions --day 2026-05-07
mx kv get decisions --id kv-A3fB              # look up by entry ID

# Auto-create a key in the schema and push in one step
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
mx kv rename old_decisions archived_decisions

# JSON output for scripting and jq piping
mx kv last projects --count 5 --json | jq '.[].data.status'
mx kv count shipped --json | jq '.count'
```

### State (deprecated)

::: {.admonition .deprecated}
**DEPRECATED:** `mx state` is deprecated and will be removed in a future
release. Use `mx kv` with structured data (`--data`) instead.

The state system encodes emotional state tensors -- multi- dimensional
values compressed into a compact string format. Used for agent
co-regulation and identity tracking.
:::

``` bash
mx state encode -d "temp=0.8 entropy=0.75 agency=0.4"
mx state decode "@state:tensor|0.8|0.75|0.4"
mx state schemas
```

### PR

PR merge handles pull request merging with encoded commit messages.
Supports squash (default), rebase, and standard merge commits.

``` bash
mx pr merge 42             # squash merge
mx pr merge 42 --rebase    # rebase merge
```

### Sync

Sync pulls and pushes GitHub issues and discussions as local YAML files
for offline editing and batch operations.

``` bash
mx sync pull owner/repo
mx sync push owner/repo --dry-run
```

## What's next

- Read the commit, log, and show reference pages for the full flag
  reference

- Explore the memory system for persistent knowledge

- Check filesystem layout for configuration options

- See architecture for how mx is built internally

# commit

Encoded git commits with base-d compression.

------------------------------------------------------------------------

## Overview

`mx commit` wraps `git commit` with automatic encoding. Your
human-readable commit message is compressed and encoded through a
randomly selected base-d dictionary, and the diff is hashed through a
second (also random) dictionary. The result is a three-part commit:

- **Title** -- a hash of the staged diff, encoded through a random
  dictionary.

- **Body** -- your message, compressed and encoded through a random
  dictionary.

- **Footer** -- a tag identifying the hash algorithm, compression
  algorithm, and both dictionary names:
  `[hash:title_dict|compress:body_dict]`.

Raw `git log` and `git show` display encoded glyphs. `mx log` and
`mx show` decode them back to plain text.

When both the title and body randomly land on the *same* dictionary, a
dejavu marker (`whoa.`) is appended to the footer -- a small easter egg
that emerges from pure chance.

::: {.admonition .note}
**NOTE:** Always use `mx log` to read commit history and `mx show` to
inspect individual commits. Raw `git log` and `git show` output is
intentionally unreadable.
:::

## Basic usage

Stage your changes and commit with a message:

``` bash
mx commit "fix session export crash on empty JSONL"
```

This commits whatever is already staged (via `git add`). If nothing is
staged, the command fails with an error.

To stage all changes automatically before committing:

``` bash
mx commit "fix session export crash on empty JSONL" -a
```

To commit and push in one step:

``` bash
mx commit "fix session export crash on empty JSONL" -p
```

Both flags compose:

``` bash
mx commit "fix session export crash on empty JSONL" -a -p
```

## Flags reference

## `mx commit [message]`

Create an encoded git commit. The message is required unless
`--encode-only` is used with `--title` and `--body`.

### Flags

  **Flag**           **Type**     **Description**
  ------------------ ------------ ----------------------------------------------------------------------------------------------------------------------------------
  `message`          positional   Human-readable commit message. Will be compressed and encoded as the commit body.
  `-a`, `--all`      flag         Stage all changes before committing (runs `git add -A`). Skipped during dry-run.
  `-p`, `--push`     flag         Push to the remote after committing. Pulls with rebase first to handle CI version bumps. Sets upstream automatically if needed.
  `--encode-only`    flag         Only generate and print the encoded message to stdout. Does not create a commit. Conflicts with `-a` and `-p`.
  `-t`, `--title`    string       Title text for PR-style encoding. Requires `--encode-only` and `--body`.
  `-b`, `--body`     string       Body text for PR-style encoding. Requires `--encode-only` and `--title`.
  `--show-encoded`   flag         Print the full encoded fields (Title, Body, Dejavu, Footer) instead of just the footer line. Conflicts with `--encode-only`.
  `--dry-run`        flag         Preview encoding and validation without mutating git state. Output is prefixed with `[dry-run]`. Conflicts with `--encode-only`.

## Dry run mode

The `--dry-run` flag runs the full encoding and validation pipeline but
skips all git mutations. No staging, no commit, no push. The output is
prefixed with `[dry-run]` on every line so it can never be confused with
real output.

``` bash
mx commit "add retry logic" --dry-run
```

Output (default):

    [dry-run] Footer: [sha384:base62|lzma:uuencode]
    [dry-run] Would commit.

With `--show-encoded`:

``` bash
mx commit "add retry logic" --dry-run --show-encoded
```

    [dry-run] Title:  <encoded glyphs>
    [dry-run] Body:   <encoded glyphs>
    [dry-run] Footer: [sha384:base62|lzma:uuencode]
    [dry-run] Would commit.

If `-p` is also set, the preview includes `Would push.`:

``` bash
mx commit "add retry logic" --dry-run -p
```

    [dry-run] Footer: [sha384:base62|lzma:uuencode]
    [dry-run] Would commit.
    [dry-run] Would push.

Dry run still validates that staged changes exist. If there are no
staged changes, it exits with an error (also prefixed with `[dry-run]`).

::: {.admonition .tip}
**TIP:** Use `--dry-run` to verify your commit will encode cleanly
before actually committing. Useful when testing unfamiliar dictionary
configurations.
:::

## Encode-only mode

The `--encode-only` flag generates encoded output without touching git
at all. It requires both `--title` and `--body` and prints the full
three-part encoded message (title, body, footer) to stdout.

``` bash
mx commit --encode-only --title "refactor store" --body "split read/write backends"
```

This is useful for:

- Testing what base-d encoding produces for a given input.

- Generating encoded messages for use outside of git (scripts, PR
  bodies, etc.).

- Verifying dictionary behavior without needing staged changes.

`--encode-only` conflicts with `-a`, `-p`, `--show-encoded`, and
`--dry-run` because it has its own output path that does not involve git
state.

## Show encoded

By default, `mx commit` prints only the footer line and `Committed.`
(plus `Pushed.` if `-p` is set). The encoded title and body are
random-glyph noise from a freshly-rolled dictionary, so they are not
useful to read.

The `--show-encoded` flag opts into the full dump:

``` bash
mx commit "add retry logic" -a --show-encoded
```

    Title:  <encoded glyphs>
    Body:   <encoded glyphs>
    Footer: [sha384:base62|lzma:uuencode]
    Committed.

When dejavu occurs (both title and body randomly got the same
dictionary), an extra line appears:

    Title:  <encoded glyphs>
    Body:   <encoded glyphs>
    Dejavu: true (both used base62)
    Footer: [sha384:base62|lzma:base62]
    whoa.
    Committed.

## How encoding works

The encoding uses base-d, a dictionary-based encoding system that maps
binary data to tokens from randomly selected dictionaries.

1.  The staged diff is hashed (e.g. SHA-384) and the hash is encoded
    through a random dictionary. This becomes the commit **title**.

2.  The commit message is compressed (e.g. LZMA, Zstd, Brotli, Gzip,
    LZ4, or Snappy) and the compressed bytes are encoded through a
    second random dictionary. This becomes the commit **body**.

3.  A footer tag records the algorithms and dictionaries used:
    `[hash_algo:title_dict|compress_algo:body_dict]`.

4.  `mx log` and `mx show` read the footer, look up the dictionaries,
    and reverse the process to recover the original message.

If the encoded output contains NUL bytes or control characters (which
would break git), the encoder retries with a different random
dictionary, up to 5 attempts. Failed attempts are logged to stderr with
the dictionary that produced unsafe output.

When pushing (`-p`), mx pulls with rebase first to handle CI-pushed
version bumps, then pushes. If no upstream branch is set, it
automatically runs `git push -u origin <branch>`.

For the full encoding specification, see base-d.

# log

Decoded git log with full git-log parity.

------------------------------------------------------------------------

## Overview

`mx log` decodes the commit history that `mx commit` encodes. Because
`mx commit` compresses and encodes every commit message through a
randomly selected base-d dictionary, raw `git log` output is unreadable
glyphs. `mx log` reverses the encoding and displays your original
messages.

The command has full parity with `git log`. Every display flag, format
preset, and filter option you know from git works here, with transparent
decoding applied to every commit message. If you know `git log`, you
know `mx log`.

The round-trip works because each encoded commit carries a footer tag
that identifies the dictionary and compression algorithm used. `mx log`
reads the footer, looks up the dictionary, decompresses the body, and
prints the human-readable message.

::: {.admonition .note}
**NOTE:** Always use `mx log` to read commit history. Raw `git log` will
show encoded noise.
:::

## Basic usage

Show the last 10 commits (the default):

``` bash
mx log
```

Show the last 3 commits using the `-N` shorthand:

``` bash
mx log -3
```

Show the last 20 commits:

``` bash
mx log -n 20
```

Show full commit details (hash, author, date, decoded message):

``` bash
mx log --full
```

## Output formats

`mx log` supports several display modes. All of them decode encoded
messages transparently.

### Compact (default)

One line per commit: short hash and decoded subject, truncated to 72
characters. This is the default when no display flag is given.

    a1b2c3d fix session export crash on empty JSONL
    e4f5g6h add retry logic to sync pull

### Full (`--full`)

Full hash, author, date, and decoded message, styled like `git log`. If
the commit has trailing post-footer content (e.g. a dejavu marker), it
is rendered in dim text beneath the decoded message. This is an
mx-specific display mode preserved for backward compatibility.

### Oneline (`--oneline`)

One line per commit with short hash, ref decorations (branch/tag names),
and decoded subject. Matches git's `--oneline` output but with decoded
messages.

    a1b2c3d (HEAD -> main, origin/main) fix session export crash on empty JSONL
    e4f5g6h add retry logic to sync pull

Use `--no-decorate` to suppress the ref decorations:

``` bash
mx log --oneline --no-decorate
```

### Format presets

The standard git format presets all work with decoded messages:

- `--format=short` -- commit hash, author, decoded subject.

- `--format=medium` -- commit hash, author, date, decoded subject and
  body. This matches git's default format.

- `--format=full` -- commit hash, author, committer, decoded subject and
  body.

- `--format=fuller` -- commit hash, author with date, committer with
  date, decoded subject and body.

These can also be specified with `--pretty`:

``` bash
mx log --pretty=fuller -3
```

## Diff output

Attach diff information below each decoded commit header:

``` bash
# Diffstat (files changed, insertions, deletions)
mx log --stat

# One-line summary of changes
mx log --shortstat

# Full patch output
mx log -p
mx log --patch
```

Diff flags compose with any display mode:

``` bash
mx log --oneline --stat -5
mx log --format=short -p -3
```

## Filtering

All git log filter flags pass through to the underlying `git log` call.
This lets you filter by path, author, date range, or any other git-log
option:

``` bash
# Commits touching a specific file
mx log -- src/handlers/mod.rs

# Commits by a specific author
mx log --author="charlie"

# Commits in a date range
mx log --since="2026-04-01" --until="2026-05-01"

# All branches
mx log --all

# Reverse chronological order
mx log --reverse

# Combine filters with display options
mx log -5 --full -- docs/
mx log --oneline --author="charlie" --since="1 week ago"
```

## Ref decorations

By default, ref decorations (branch names, tags, `HEAD`) are shown in
`--oneline` mode. You can control this explicitly:

``` bash
mx log --oneline --decorate       # show decorations (default)
mx log --oneline --no-decorate    # hide decorations
```

## Count

Several syntaxes are accepted for limiting the number of commits:

``` bash
mx log -3              # -N shorthand (like git log -3)
mx log -n 5            # -n with space
mx log -n5             # -n without space
mx log --max-count=7   # git's long form
```

When no count is specified, `mx log` defaults to 10 commits. This
differs from `git log` (which defaults to unlimited) and is intentional
-- it keeps the default output concise.

## Passthrough modes

In two cases, `mx log` skips decoding entirely and falls through to raw
`git log` with a stderr note:

### `--graph`

Graph rendering requires line-level control that the four-phase
architecture cannot replicate without reimplementing git's graph layout.
When `--graph` is present, the command passes through to raw `git log`.
A note is printed to stderr:

    note: --graph bypasses message decoding

### Custom `--format` strings

When `--format` or `--pretty` is set to a custom format string (anything
other than the named presets `oneline`, `short`, `medium`, `full`,
`fuller`), the command passes through to raw `git log`. A note is
printed to stderr:

    note: custom --format bypasses message decoding

In both passthrough modes, the count, diff flags, and filter args are
still forwarded.

## Flags reference

## `mx log`

Display decoded git log. Commits encoded by `mx commit` are decoded back
to their original messages. Non-encoded commits pass through unchanged.

### Flags

  **Flag**          **Type**    **Description**
  ----------------- ----------- -----------------------------------------------------------------------------------------------------------------------------------
  `-N`              shorthand   Number of commits to show, as a bare number after the dash. Example: `-3`, `-10`. Equivalent to git's `-N` shorthand.
  `-n`              integer     Number of commits to show. Accepts `-n 5` or `-n5`. Defaults to `10`.
  `--max-count`     integer     Number of commits to show (git's long form). Example: `--max-count=7`.
  `--full`          flag        Show full commit details: full hash, author, date, and complete decoded message. An mx-specific display mode.
  `--oneline`       flag        One line per commit: short hash, ref decorations, decoded subject.
  `--stat`          flag        Show diffstat below each commit.
  `--shortstat`     flag        Show a one-line summary of changes below each commit.
  `-p`, `--patch`   flag        Show the full patch below each commit.
  `--decorate`      flag        Show ref decorations (branch, tag, HEAD). On by default in `--oneline` mode.
  `--no-decorate`   flag        Suppress ref decorations.
  `--format`        preset      Format preset: `short`, `medium`, `full`, `fuller`. Named presets decode messages; custom format strings pass through to raw git.
  `--pretty`        preset      Alias for `--format`.
  `--graph`         flag        Passthrough to raw `git log` with graph rendering. Decoding is skipped.

### Filter passthrough

Any additional arguments not listed above are passed through to the
underlying `git log` call. Common examples:

+-----------------------------------+-----------------------------------+
| **Argument**                      | **Description**                   |
+===================================+===================================+
| `--author=<pattern>`              | Filter by author name or email.   |
+-----------------------------------+-----------------------------------+
| `--since=<date>`                  | Show commits after a date.        |
+-----------------------------------+-----------------------------------+
| `--until=<date>`                  | Show commits before a date.       |
+-----------------------------------+-----------------------------------+
| `--all`                           | Show commits from all refs, not   |
|                                   | just the current branch.          |
+-----------------------------------+-----------------------------------+
| `--reverse`                       | Show commits in reverse           |
|                                   | chronological order.              |
+-----------------------------------+-----------------------------------+
| `-- <path>`                       | Filter to commits touching the    |
|                                   | given path(s).                    |
+-----------------------------------+-----------------------------------+

## Architecture

`mx log` uses a four-phase architecture:

1.  **Parse** -- raw CLI arguments are parsed into a structured
    `LogOptions` with separate fields for count, display mode, diff
    mode, decorate preference, and filter arguments. Custom `--format`
    strings and `--graph` are detected here and trigger passthrough.

2.  **Harvest** -- a single `git log` call with a structured format
    string retrieves commit metadata (hashes, author, dates,
    decorations) and the encoded message body. Each commit is parsed and
    decoded.

3.  **Attach diffs** -- if `--stat`, `--shortstat`, or `-p` was
    requested, a second `git log` call retrieves the diff output. Each
    diff block is attached to its corresponding commit.

4.  **Render** -- the display mode selects a renderer (compact, full,
    oneline, or a format preset). Each renderer prints the decoded
    message with the appropriate header format, followed by any attached
    diff output.

This architecture ensures that decoding is always applied before
rendering, and that diff output appears in the correct position
regardless of the display format.

## Relationship to mx commit and mx show

`mx commit`, `mx log`, and `mx show` form the encoding round-trip:

1.  `mx commit` compresses your message, encodes it through a random
    dictionary, and writes the encoded result as the git commit body
    with a footer tag.

2.  `mx log` reads the footer tag, reverses the encoding, decompresses,
    and displays your original message across the commit history.

3.  `mx show` does the same decoding for individual commits, replacing
    `git show`.

Both `mx log` and `mx show` have full parity with their git
counterparts. Every flag that `git log` or `git show` accepts works with
the mx versions, with transparent decoding applied to encoded messages.
Non-encoded commits (e.g. commits made with raw `git commit`) pass
through unchanged.

For the full encoding specification, see commit.

# show

Decoded git show for encoded commits.

------------------------------------------------------------------------

## Overview

`mx show` decodes the output of `git show` the same way `mx log` decodes
`git log`. Because `mx commit` encodes every commit message through a
randomly selected base-d dictionary, raw `git show` displays unreadable
glyphs where the commit message should be. `mx show` reverses the
encoding and displays your original message while passing everything
else -- diffs, stats, file content -- through unchanged.

It is a drop-in replacement for `git show`. Every flag that `git show`
accepts works with `mx show`.

::: {.admonition .note}
**NOTE:** Always use `mx show` to inspect commits. Raw `git show` will
show encoded noise for any commit made with `mx commit`.
:::

## Basic usage

Show the most recent commit with its diff:

``` bash
mx show
```

Show a specific commit:

``` bash
mx show abc1234
```

Show a commit with diffstat instead of the full diff:

``` bash
mx show --stat
```

Show only the commit message (no diff):

``` bash
mx show --no-patch
```

Show only filenames changed:

``` bash
mx show --name-only
```

## How it works

`mx show` uses a two-pass approach:

1.  **Pass 1** runs `git show` with `--no-patch` and a structured format
    to retrieve commit metadata (hash, author, date, parent hashes) and
    the encoded message body. The body is decoded using the same
    pipeline as `mx log` -- the footer tag identifies the dictionary and
    compression algorithm, and the body is decompressed back to your
    original message.

2.  **Pass 2** runs `git show` with an empty format string to retrieve
    just the diff output. This is streamed to your terminal as-is,
    identical to what `git show` would produce.

The result looks exactly like `git show` output, except the commit
message is readable.

### Passthrough modes

In certain cases, `mx show` skips decoding entirely and runs raw
`git show`:

- **File content** (`ref:path` syntax) -- when you use
  `mx show HEAD:src/main.rs` to view a file at a specific revision,
  there is no commit message to decode. The command passes through to
  `git show` directly.

- **Custom format** (`--format` or `--pretty`) -- when you control the
  output format yourself, decoding would interfere. The command passes
  through unchanged.

### Fallback behavior

If decoding fails for any reason -- the commit was not made with
`mx commit`, the footer is missing, or the dictionary lookup fails --
`mx show` falls back to displaying the raw message exactly as `git show`
would. It is always safe to use `mx show` in place of `git show`, even
in repositories with a mix of encoded and non-encoded commits.

## Merge commits

Merge commits display a `Merge:` line showing the parent hashes,
matching the default `git show` format:

    commit abc1234def5678...
    Merge:  aaa1111 bbb2222
    Author: Charlie <charlie@example.com>
    Date:   Wed May 7 2026

        the decoded merge commit message

## Multiple refs

You can pass multiple refs and `mx show` will decode each one:

``` bash
mx show HEAD HEAD~1 HEAD~2
```

## Tags

When showing a tag, `mx show` displays the tag metadata followed by the
decoded commit it points to. If the tag object itself is not a commit
(e.g. an annotated tag preamble), its content is printed as-is.

## Flags reference

`mx show` accepts all flags that `git show` accepts. There are no
mx-specific flags -- the command is designed to be a transparent
wrapper.

Common flags:

+-----------------------------------+-----------------------------------+
| **Flag**                          | **Description**                   |
+===================================+===================================+
| `--stat`                          | Show a diffstat summary instead   |
|                                   | of the full diff.                 |
+-----------------------------------+-----------------------------------+
| `--no-patch`                      | Show only the commit header and   |
|                                   | message, no diff.                 |
+-----------------------------------+-----------------------------------+
| `-s`                              | Shorthand for `--no-patch`.       |
+-----------------------------------+-----------------------------------+
| `--name-only`                     | Show only the names of changed    |
|                                   | files.                            |
+-----------------------------------+-----------------------------------+
| `--name-status`                   | Show names and status (added,     |
|                                   | modified, deleted) of changed     |
|                                   | files.                            |
+-----------------------------------+-----------------------------------+
| `--raw`                           | Show the diff in raw format.      |
+-----------------------------------+-----------------------------------+
| `--format=<fmt>`                  | Custom format string (passthrough |
|                                   | -- skips decoding).               |
+-----------------------------------+-----------------------------------+
| `--pretty=<fmt>`                  | Alias for `--format` (passthrough |
|                                   | -- skips decoding).               |
+-----------------------------------+-----------------------------------+

Any arguments not listed here are passed directly to `git show`.

## Relationship to mx log and mx commit

`mx commit`, `mx log`, and `mx show` form a complete encoding
round-trip:

1.  `mx commit` encodes your message and writes it as an encoded git
    commit.

2.  `mx log` decodes the commit history (replaces `git log`).

3.  `mx show` decodes individual commit details (replaces `git show`).

Both `mx log` and `mx show` have full parity with their git
counterparts. Every flag that `git log` or `git show` accepts works with
the mx versions, with transparent decoding applied to encoded messages.
`mx log` supports `--oneline`, `--stat`, `-p`, format presets (`short`,
`medium`, `full`, `fuller`), and all git log filter flags (`--author`,
`--since`, `-- <path>`, etc.). Non-encoded commits pass through
unchanged in all three commands.

For the full encoding specification, see commit. For the full flag
reference and architecture details, see log.

# Memory

Knowledge graph with SurrealDB-backed persistent memory.

------------------------------------------------------------------------

The memory subsystem is the largest command surface in mx. It provides a
persistent knowledge graph backed by SurrealDB (embedded SurrealKV or
networked WebSocket), with categories, tags, resonance levels,
embeddings for semantic search, relationships between entries, and a
wake ritual for identity bootstrap.

Every entry in the graph has a unique ID (prefixed `kn-`), a category, a
title, body content, optional tags, a resonance level (1--10+), and
timestamps. Entries can be linked via typed relationships, anchored to
each other by embedding similarity, and surfaced through keyword or
semantic search.

## Table of contents

- Adding entries

- Reading entries

- Updating entries

- Deleting entries

- Wake system

- Embeddings and anchoring

- Relationships

- Seeding

- Health and statistics

- Export

- Reinforcement

- Metadata management

- Session tracking

## Adding entries {#adding}

## `mx memory add`

Create a new entry in the knowledge graph. At minimum, provide a
category and title (or a `--type` for ephemeral facts, which auto-routes
the category and generates a title from content).

### Flags

  **Flag**                **Type**   **Description**
  ----------------------- ---------- ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------
  `--category`            `string`   Category name (run `mx memory categories list` for valid names). Required unless `--type` is provided.
  `-t, --title`           `string`   Entry title. Required unless `--type` is provided.
  `--content`             `string`   Inline content. Conflicts with `--file`.
  `-f, --file`            `path`     Read content from a file. Also accepts `--content-file`.
  `--tags`                `string`   Comma-separated tags.
  `-a, --applicability`   `string`   Comma-separated applicability contexts.
  `-p, --project`         `string`   Source project ID.
  `--source-agent`        `string`   Source agent ID. Defaults to `MX_CURRENT_AGENT` env var.
  `--source-type`         `string`   Source type: `manual`, `ram`, `cache`, `agent_session`. Default: `manual`.
  `--entry-type`          `string`   Entry type: `primary`, `summary`, `synthesis`. Default: `primary`.
  `--session-id`          `string`   Session ID to associate with this entry.
  `--ephemeral`           `flag`     Mark entry as ephemeral.
  `-d, --domain`          `string`   Domain/subdomain path.
  `--content-type`        `string`   Content type: `text`, `code`, `config`, `data`, `binary`. Default: `text`.
  `--private`             `flag`     Mark as private (only visible to owner). Shorthand for `--visibility private`.
  `--visibility`          `string`   Set visibility: `public` or `private`.
  `--owner`               `string`   Explicit owner. Defaults to `source_agent` or `MX_CURRENT_AGENT` if private.
  `--resonance`           `int`      Resonance level (1--10, or higher for transcendent).
  `--resonance-type`      `string`   Resonance type: `foundational`, `transformative`, `relational`, `operational`, `ephemeral`, `session`.
  `--wake-phrase`         `string`   Wake phrase for memory ritual verification.
  `--wake-phrases`        `string`   Multiple wake phrases (comma-separated).
  `--wake-order`          `int`      Custom wake order (lower = earlier in sequence).
  `--anchors`             `string`   Comma-separated bloom IDs this entry connects to.
  `--type`                `string`   Fact type for ephemeral knowledge: `decision`, `insight`, `person`, `quote`, `thread_opened`, `commitment`, `thread_closed`. Auto-routes category and sets `resonance_type=ephemeral`.
  `--session`             `string`   Session to link fact to via EXTRACTED_FROM relationship. Requires `--type`.
  `--thread-id`           `string`   Thread ID for `thread_closed` operations. Requires `--type`.
  `--no-auto-anchor`      `flag`     Skip automatic anchor generation.
  `--json`                `flag`     Output as JSON.

### Examples

``` bash
mx memory add --category recipe --title "Retry with backoff" \
  --content "Use exponential backoff with jitter..." \
  --tags "reliability,networking" --source-agent whistledown
```

``` bash
mx memory add --category discovery --title "SurrealDB needs explicit NS" \
  --content "Always set namespace before queries" \
  --resonance 7 --resonance-type operational
```

``` bash
# Ephemeral fact (auto-routes category, generates title)
mx memory add --type decision \
  --content "Chose Typst over mdBook for docs" \
  --session abc-123
```

``` bash
# Content from file
mx memory add --category ingredient -t "API reference" -f api-notes.md
```

::: {.admonition .tip}
**TIP:** When `--type` is provided, `--category` and `--title` become
optional. The fact type routes to an appropriate category and generates
a title from the content automatically.
:::

## Reading entries {#reading}

### Shared filter flags

Several read commands (`search`, `list`) share a common set of filter
flags. These are documented once here and referenced below.

  **Flag**                     **Type**   **Description**
  ---------------------------- ---------- ----------------------------------------------------
  `-c, --category`             `string`   Filter by category (comma-separated).
  `--json`                     `flag`     Output as JSON.
  `--mine`                     `flag`     Show only your private entries.
  `--include-private`          `flag`     Include private entries (requires matching owner).
  `--min-resonance`            `int`      Minimum resonance level.
  `--max-resonance`            `int`      Maximum resonance level.
  `--has-wake-phrase`          `flag`     Filter to entries WITH a wake phrase.
  `--missing-wake-phrase`      `flag`     Filter to entries WITHOUT a wake phrase.
  `--has-anchors`              `flag`     Filter to entries WITH anchors.
  `--missing-anchors`          `flag`     Filter to entries WITHOUT anchors.
  `--has-resonance-type`       `flag`     Filter to entries WITH a resonance type.
  `--missing-resonance-type`   `flag`     Filter to entries WITHOUT a resonance type.
  `--limit`                    `int`      Limit number of results.
  `--tags`                     `string`   Filter by tags (comma-separated, matches any).

## `mx memory show`

Display a single entry by ID.

### Flags

  **Flag**           **Type**   **Description**
  ------------------ ---------- ---------------------------------------------------
  `--json`           `flag`     Output as JSON.
  `--content-only`   `flag`     Output only the body content (useful for piping).

### Examples

``` bash
mx memory show kn-abc123
```

``` bash
mx memory show kn-abc123 --content-only | pbcopy
```

## `mx memory list`

List entries, optionally filtered by category, tags, resonance, and
other shared filter flags.

### Examples

``` bash
mx memory list -c recipe
```

``` bash
mx memory list -c discovery,decree --min-resonance 5
```

``` bash
mx memory list --missing-wake-phrase --limit 20
```

::: {.admonition .note}
**NOTE:** `list` accepts all shared filter flags documented above.
:::

## `mx memory search`

Search entries by keyword or semantic similarity. Keyword search is the
default; add `--semantic` to use vector embeddings.

### Flags

  **Flag**       **Type**   **Description**
  -------------- ---------- ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------
  `--semantic`   `flag`     Use semantic (vector) search instead of keyword search.
  `--activate`   `flag`     Activate all returned results: resets `last_activated` (decay clock) and increments `activation_count`. Marks results as intentionally consumed rather than just browsed.

### Examples

``` bash
mx memory search "retry pattern"
```

``` bash
mx memory search "how to handle timeouts" --semantic
```

``` bash
mx memory search "agent bootstrap" -c recipe,method --limit 5
```

``` bash
# Search and activate results (mark as consumed)
mx memory search "retry pattern" --activate
```

::: {.admonition .note}
**NOTE:** `search` accepts all shared filter flags. Semantic search
requires entries to have embeddings generated via `mx memory embed`.
:::

::: {.admonition .tip}
**TIP:** By default, search does not activate results -- browsing is not
the same as engagement. Use `--activate` when you are intentionally
consuming the results (e.g., loading context for a task), not just
exploring.
:::

## `mx memory recent`

List recent ephemeral facts with decay. By default shows only ephemeral
entries from the last 10 days. Use `--all-types` to surface all
resonance types.

### Flags

  **Flag**             **Type**   **Description**
  -------------------- ---------- -------------------------------------------------------------------------------------
  `--days`             `int`      Number of days to look back. Default: `10`.
  `--json`             `flag`     Output as JSON.
  `--resonance-type`   `string`   Filter by resonance type. Defaults to ephemeral only when `--all-types` is omitted.
  `--all-types`        `flag`     Surface all resonance types instead of ephemeral only.
  `--sort`             `enum`     Sort order: `chronological` (default) or `resonance` (highest first).
  `--limit`            `int`      Maximum number of results. Default: `100`.

### Examples

``` bash
mx memory recent
```

``` bash
mx memory recent --days 30 --all-types --sort resonance
```

``` bash
mx memory recent --resonance-type foundational --limit 10
```

## Updating entries {#updating}

## `mx memory update`

Update an existing entry. Supports replacing content entirely,
appending, prepending, find-and-replace, and modifying any metadata
field. Content mutation modes are mutually exclusive.

### Flags

  **Flag**                 **Type**   **Description**
  ------------------------ ---------- -------------------------------------------------------------------------------------
  `-t, --title`            `string`   Update the title.
  `--content`              `string`   Replace content entirely (inline).
  `-f, --file`             `path`     Replace content entirely from file.
  `--append-content`       `string`   Append text to end of existing content.
  `--append-file`          `path`     Append content from file to end.
  `--prepend-content`      `string`   Prepend text to start of existing content.
  `--prepend-file`         `path`     Prepend content from file to start.
  `--find`                 `string`   Find text in content (requires `--replace`).
  `--replace`              `string`   Replace text found by `--find`.
  `--replace-all`          `flag`     Replace all occurrences (with `--find`/`--replace`).
  `--nth`                  `int`      Replace only the Nth occurrence (1-indexed).
  `--category`             `string`   Update category.
  `--tags`                 `string`   Replace all tags (comma-separated).
  `--add-tag`              `string`   Add a single tag to existing tags.
  `--remove-tag`           `string`   Remove a specific tag.
  `-a, --applicability`    `string`   Update applicability (comma-separated, replaces all).
  `--content-type`         `string`   Update content type.
  `--resonance`            `int`      Update resonance level (1--10+).
  `--resonance-type`       `string`   Update resonance type.
  `--anchors`              `string`   Replace all anchors (comma-separated bloom IDs).
  `--add-anchor`           `string`   Add a single anchor.
  `--remove-anchor`        `string`   Remove a specific anchor.
  `--wake-phrase`          `string`   Update wake phrase.
  `--wake-phrases`         `string`   Replace all wake phrases (comma-separated).
  `--add-wake-phrase`      `string`   Add a single wake phrase.
  `--remove-wake-phrase`   `string`   Remove a specific wake phrase.
  `--wake-order`           `string`   Update wake order. Use `'-'` to clear.
  `--private`              `flag`     Mark as private (shorthand for `--visibility private`).
  `--visibility`           `string`   Change visibility: `public` or `private`.
  `--owner`                `string`   Update owner (only valid when visibility is private).
  `--session-id`           `string`   Update session ID (for retrofitting entries with wrong or missing session linkage).
  `--force`                `flag`     Force dangerous visibility changes (e.g., making blooms public).
  `--no-auto-anchor`       `flag`     Skip automatic anchor generation.
  `--json`                 `flag`     Output as JSON.

### Examples

``` bash
mx memory update kn-abc123 --title "Better title"
```

``` bash
mx memory update kn-abc123 --add-tag reliability
```

``` bash
mx memory update kn-abc123 --find "old text" --replace "new text"
```

``` bash
mx memory update kn-abc123 --append-content "\n\nUpdate: confirmed working"
```

``` bash
mx memory update kn-abc123 --resonance 8 --resonance-type foundational
```

## `mx memory edit`

Find-and-replace shortcut. Equivalent to
`mx memory update <id> --find ... --replace ...` with a simpler
interface.

### Flags

  **Flag**             **Type**   **Description**
  -------------------- ---------- ---------------------------------------------------------------
  `--find`             `string`   Text to find in content. Also accepts `--old`.
  `--replace`          `string`   Replacement text. Also accepts `--new`.
  `--replace-all`      `flag`     Replace all occurrences (default: error if multiple matches).
  `--nth`              `int`      Replace only the Nth occurrence (1-indexed).
  `--no-auto-anchor`   `flag`     Skip automatic anchor generation.
  `--json`             `flag`     Output as JSON.

### Examples

``` bash
mx memory edit kn-abc123 --find "old pattern" --replace "new pattern"
```

``` bash
mx memory edit kn-abc123 --old "v1" --new "v2" --replace-all
```

## `mx memory append`

Append content to the end of an entry's body. Shortcut for
`mx memory update <id> --append-content ...`.

### Flags

  **Flag**             **Type**   **Description**
  -------------------- ---------- --------------------------------------------------------
  `--content`          `string`   Content to append (omit to read from stdin).
  `-f, --file`         `path`     Read content from file. Also accepts `--content-file`.
  `--no-auto-anchor`   `flag`     Skip automatic anchor generation.
  `--json`             `flag`     Output as JSON.

### Examples

``` bash
mx memory append kn-abc123 --content "\n\nAdditional note here."
```

``` bash
mx memory append kn-abc123 -f addendum.md
```

## `mx memory prepend`

Prepend content to the start of an entry's body. Shortcut for
`mx memory update <id> --prepend-content ...`.

### Flags

  **Flag**             **Type**   **Description**
  -------------------- ---------- --------------------------------------------------------
  `--content`          `string`   Content to prepend (omit to read from stdin).
  `-f, --file`         `path`     Read content from file. Also accepts `--content-file`.
  `--no-auto-anchor`   `flag`     Skip automatic anchor generation.
  `--json`             `flag`     Output as JSON.

### Examples

``` bash
mx memory prepend kn-abc123 --content "IMPORTANT: "
```

## `mx memory restore`

Restore entry content from a backup. Use `--list` to see available
backups before restoring.

### Flags

  **Flag**             **Type**   **Description**
  -------------------- ---------- ----------------------------------------------
  `--list`             `flag`     List available backups instead of restoring.
  `--no-auto-anchor`   `flag`     Skip automatic anchor generation.
  `--json`             `flag`     Output as JSON.

### Examples

``` bash
mx memory restore kn-abc123 --list
```

``` bash
mx memory restore kn-abc123
```

## Deleting entries {#deleting}

## `mx memory delete`

Remove an entry from the knowledge graph.

### Flags

  **Flag**   **Type**   **Description**
  ---------- ---------- -----------------
  `--json`   `flag`     Output as JSON.

### Examples

``` bash
mx memory delete kn-abc123
```

## Wake system {#wake}

The wake system provides identity bootstrap for agents. It retrieves
high-resonance entries ("blooms") and presents them through a ritual
that verifies the agent's connection to its knowledge. There are several
output modes and an interactive engage flow.

## `mx memory wake`

Wake up with resonant identity cascade. Retrieves high-resonance blooms
and presents them in the requested format.

### Flags

  **Flag**              **Type**   **Description**
  --------------------- ---------- -------------------------------------------------------------------------------------
  `-l, --limit`         `int`      Number of blooms to return. Default: `20`.
  `--min-resonance`     `int`      Minimum resonance threshold -- get ALL blooms \>= this value (overrides `--limit`).
  `-d, --days`          `int`      Include memories activated in last N days. Default: `7`.
  `--json`              `flag`     Output as JSON.
  `--ritual`            `flag`     Output as bash ritual script (sequential reading).
  `--index`             `flag`     Output as compact markdown index (for identity loading).
  `--no-activate`       `flag`     Do not update activation counts.
  `-e, --engage`        `flag`     Interactive engage mode -- verify wake phrases (requires TTY).
  `-s, --set-missing`   `flag`     Prompt to set missing wake phrases during engage mode. Requires `--engage`.
  `--begin`             `flag`     Start token-based wake ritual. Returns first bloom and session token.
  `--bloom-id`          `string`   Bloom ID for `--respond` or `--skip` operations.
  `--respond`           `string`   Submit wake phrase response for a bloom.
  `--skip`              `flag`     Skip a bloom without wake phrase.
  `--session`           `string`   Session token for chained ritual (required with `--respond` or `--skip`).

### Examples

``` bash
# Default wake -- top 20 blooms, text output
mx memory wake
```

``` bash
# Compact index for agent identity loading
mx memory wake --index
```

``` bash
# All blooms with resonance >= 7
mx memory wake --min-resonance 7
```

``` bash
# Interactive engage mode with wake phrase verification
mx memory wake --engage
```

``` bash
# Token-based ritual (for non-TTY / programmatic use)
mx memory wake --begin
mx memory wake --bloom-id kn-abc --respond "the phrase" --session tok-xyz
mx memory wake --bloom-id kn-def --skip --session tok-xyz
```

::: {.admonition .note}
**NOTE:** `MX_CURRENT_AGENT` must be set for wake to function. The wake
ritual reads blooms ordered by resonance and wake order, then optionally
verifies the agent can produce each bloom's wake phrase.
:::

### Wake modes

- **Default** (`mx memory wake`): plain text output, blooms listed with
  titles and content.

- **JSON** (`--json`): structured output for programmatic consumption.

- **Ritual** (`--ritual`): bash script that presents blooms
  sequentially.

- **Index** (`--index`): compact markdown suitable for loading into
  agent context.

- **Engage** (`--engage`): interactive TTY mode where the agent verifies
  each bloom's wake phrase. Add `--set-missing` to be prompted for
  phrases on blooms that lack them.

- **Token-based** (`--begin`, `--respond`, `--skip`): stateless chained
  ritual for non-interactive environments. Start with `--begin`, then
  loop with `--respond` or `--skip` using the returned session token and
  bloom ID.

## `mx memory wake-fetch`

Fetch facts for the wake ritual. Returns entries with resonance \>= 3
across all types, sorted by resonance (highest first). Designed as a
data source for wake ritual presentation.

### Flags

  **Flag**    **Type**   **Description**
  ----------- ---------- ---------------------------------------------
  `--days`    `int`      Number of days to look back. Default: `15`.
  `--limit`   `int`      Maximum number of results. Default: `100`.

### Examples

``` bash
mx memory wake-fetch
```

``` bash
mx memory wake-fetch --days 30 --limit 50
```

## Embeddings and anchoring {#embeddings}

Embeddings enable semantic search and automatic relationship discovery.
Each entry can have a vector embedding generated from its title and
content. Anchors are connections between entries discovered via
embedding similarity.

### Chunked embeddings

Entries longer than 400 tokens are automatically split into overlapping
chunks before embedding. This ensures semantic search covers the full
content of long entries, not just the first 400 tokens.

**How it works:**

1.  The entry's embedding text (title + body/summary + tags) is
    tokenized using the BGE-Base-EN-v1.5 tokenizer.

2.  If the text fits within 400 tokens, a single embedding is generated
    and stored on the entry --- exactly as before. No chunks are
    created.

3.  If the text exceeds 400 tokens, it is split into overlapping chunks
    with a sliding window: 400 tokens per chunk, 100-token overlap
    (stride 300).

4.  Each chunk is embedded separately and stored in the
    `embedding_chunk` table.

5.  A normalized mean vector of all chunk embeddings is stored on the
    entry's `embedding` field for `auto-anchor` compatibility.

6.  The entry's `chunk_count` field records how many chunks were created
    (0 for unchunked entries).

**Semantic search with chunks:**

When `mx memory search --semantic` runs, it queries both unchunked entry
embeddings and chunk embeddings in parallel. Results are merged by
taking the maximum similarity score per entry --- if a chunk from entry
X scores 0.92 and the entry's mean vector scores 0.85, the entry's final
score is 0.92. This ensures long entries surface when any section is
relevant, not just when the overall average is relevant.

::: {.admonition .tip}
**TIP:** Short entries (≤400 tokens) behave exactly as before --- single
embedding, no chunks, no behavior change. Chunking only activates for
entries that exceed the 400-token threshold.
:::

::: {.admonition .note}
**NOTE:** The `embedding_text()` method on entries no longer truncates
body content. The chunker handles length management, ensuring no content
is lost during embedding.
:::

## `mx memory embed`

Generate a vector embedding for one or all entries. Embeddings power
semantic search (`--semantic` flag on `search`) and automatic anchoring.
Long entries (\>400 tokens) are automatically split into overlapping
chunks, with each chunk embedded separately. Short entries get a single
embedding.

### Flags

  **Flag**        **Type**   **Description**
  --------------- ---------- --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------
  `-a, --all`     `flag`     Embed all knowledge entries (instead of a single ID).
  `--long-only`   `int`      Only re-embed entries whose `embedding_text()` exceeds this many tokens. Entries at or below the threshold are skipped entirely. Use with `--all`. Useful for selectively re-embedding long entries that were previously truncated at a smaller token limit (e.g., 512).

### Examples

``` bash
mx memory embed kn-abc123
```

``` bash
mx memory embed --all
```

``` bash
# Re-embed only entries that exceed 512 tokens
mx memory embed --all --long-only 512
```

## `mx memory auto-anchor`

Automatically add anchors between entries based on embedding similarity.
Processes a single entry or all entries that have embeddings.

Also re-evaluates existing anchors: any anchor whose cosine similarity
has fallen below the threshold (default 0.75) or risen above the
near-duplicate ceiling (0.95) is pruned. This keeps the anchor graph
self-cleaning -- anchors that made sense once but no longer do are
removed automatically.

### Flags

  **Flag**          **Type**   **Description**
  ----------------- ---------- ---------------------------------------------------------------------------------------------------------------------
  `--threshold`     `float`    Minimum cosine similarity (0.0--1.0). Default: `0.75`.
  `--max-anchors`   `int`      Maximum anchors to add per entry. Default: `5`.
  `--dry-run`       `flag`     Preview changes without writing.
  `--detailed`      `flag`     Show similarity scores in output.
  `--fill`          `flag`     Only process entries with zero existing anchors. Fills gaps in the graph without touching already-anchored entries.

### Examples

``` bash
mx memory auto-anchor
```

``` bash
mx memory auto-anchor kn-abc123 --threshold 0.8 --max-anchors 3
```

``` bash
mx memory auto-anchor --dry-run --detailed
```

``` bash
mx memory auto-anchor --fill
```

::: {.admonition .tip}
**TIP:** A typical workflow: run `mx memory embed --all` to generate
embeddings, then `mx memory auto-anchor --dry-run --detailed` to preview
anchor candidates, then `mx memory auto-anchor` to write them.
:::

::: {.admonition .note}
**NOTE:** Anchors are also maintained automatically on every write
operation (`add`, `update`, `edit`, `append`, `prepend`, `restore`).
After each write, mx re-evaluates anchors and prunes stale ones using
the same similarity thresholds. Pass `--no-auto-anchor` on any of these
commands to skip this step -- useful for bulk operations or cleanup
scripts where the overhead is unwanted.
:::

## Relationships

Explicit typed edges between entries. While anchors are discovered
automatically via embedding similarity, relationships are manually
declared semantic connections.

## `mx memory relationships list`

List all relationships for an entry.

### Flags

  **Flag**   **Type**   **Description**
  ---------- ---------- -----------------
  `--json`   `flag`     Output as JSON.

### Examples

``` bash
mx memory relationships list kn-abc123
```

## `mx memory relationships add`

Add a typed relationship between two entries. By default, the target
entry (`--to`) is automatically reinforced by +1 (capped at 10) when the
relationship is created -- being linked to means the fact proved
relevant. The `contradicts` and `supersedes` types are excluded from
auto-reinforcement because boosting an outdated or contradicted entry
works against intent.

### Flags

  **Flag**           **Type**   **Description**
  ------------------ ---------- -------------------------------------------------------------------------------------
  `--from`           `string`   Source entry ID.
  `--to`             `string`   Target entry ID.
  `--type`           `string`   Relationship type: `related`, `supersedes`, `extends`, `implements`, `contradicts`.
  `--no-reinforce`   `flag`     Skip automatic reinforcement of the target entry.

### Examples

``` bash
mx memory relationships add --from kn-abc --to kn-def --type extends
```

``` bash
mx memory relationships add --from kn-abc --to kn-ghi --type supersedes
```

``` bash
# Add a relationship without auto-reinforcing the target
mx memory relationships add --from kn-abc --to kn-def --type related --no-reinforce
```

## `mx memory relationships delete`

Delete a relationship by its ID.

### Examples

``` bash
mx memory relationships delete rel-abc123
```

## Seeding

Seed commands populate the knowledge graph from on-disk artifacts. Used
for initial setup and bulk import.

## `mx memory seed agents`

Seed agents from markdown files with YAML frontmatter. Reads from
`$MX_HOME/memory/seed/agents/` by default.

### Flags

  **Flag**       **Type**   **Description**
  -------------- ---------- -----------------------------------------------------------------------
  `-p, --path`   `path`     Path to agents directory. Defaults to `$MX_HOME/memory/seed/agents/`.

### Examples

``` bash
mx memory seed agents
```

``` bash
mx memory seed agents --path /data/agents/
```

::: {.admonition .note}
**NOTE:** Legacy fallback: if `$MX_HOME/memory/seed/agents/` does not
exist, mx checks `$MX_HOME/agents/` and emits a stderr warning. This
fallback will be removed in a future release.
:::

## `mx memory seed knowledge`

Seed knowledge from JSONL files. With no path, scans
`$MX_HOME/memory/seed/knowledge/*.jsonl` and imports every file found.
With a path, imports just that single file.

### Examples

``` bash
mx memory seed knowledge
```

``` bash
mx memory seed knowledge /data/knowledge/bootstrap.jsonl
```

## Health and statistics {#health}

## `mx memory stats`

Show index statistics -- entry counts, category breakdown, and other
aggregate metrics.

### Flags

  **Flag**   **Type**   **Description**
  ---------- ---------- -----------------
  `--json`   `flag`     Output as JSON.

### Examples

``` bash
mx memory stats
```

``` bash
mx memory stats --json
```

## `mx memory health`

Show graph health vitality percentages: embedding coverage, anchor
coverage, and stale high-resonance entries.

### Flags

  **Flag**   **Type**   **Description**
  ---------- ---------- ----------------------------------------------------------
  `--json`   `flag`     Output as JSON (default format for dashboard consumers).

### Examples

``` bash
mx memory health
```

``` bash
mx memory health --json
```

## `mx memory growth`

Show per-week entry growth over the last 8 weeks.

### Flags

  **Flag**   **Type**   **Description**
  ---------- ---------- --------------------------------------------------------
  `--json`   `flag`     Output as JSON array of 8 integers (oldest to newest).

### Examples

``` bash
mx memory growth
```

``` bash
mx memory growth --json
```

## `mx memory open-threads`

List open threads (`category:thread` entries with `state=\"open\"` or no
state).

### Flags

  **Flag**   **Type**   **Description**
  ---------- ---------- ----------------------------------------------------------
  `--json`   `flag`     Output as JSON array (required for dashboard consumers).

### Examples

``` bash
mx memory open-threads
```

``` bash
mx memory open-threads --json
```

## Export

## `mx memory export`

Export the entire knowledge database to a file or directory.

### Flags

  **Flag**         **Type**   **Description**
  ---------------- ---------- -------------------------------------------------------------------------------------------------------------------
  `-f, --format`   `string`   Output format: `md`, `jsonl`, `csv`. Default: `md`.
  `-o, --output`   `path`     Output directory for `md` format (defaults to `./memory-export`), or file for `jsonl`/`csv` (defaults to stdout).

### Examples

``` bash
mx memory export
```

``` bash
mx memory export -f jsonl -o backup.jsonl
```

``` bash
mx memory export -f csv -o entries.csv
```

``` bash
mx memory export -f md -o /data/export/
```

## Reinforcement

Reinforcement is the mechanism by which the knowledge graph breathes in
-- entries that are used, referenced, or linked gain resonance,
counteracting the natural decay of the exhale. There are three
reinforcement paths:

1.  **Explicit reinforcement** via `mx memory reinforce` -- directly
    boost an entry's resonance.

2.  **Auto-reinforce on relationship creation** -- when
    `mx memory relationships add` links to a target entry, the target is
    reinforced by +1 (capped at 10). The `contradicts` and `supersedes`
    types are excluded. Use `--no-reinforce` to opt out.

3.  **Search activation** via `mx memory search --activate` -- marks
    returned results as intentionally consumed, resetting their decay
    clock and incrementing their activation count.

## `mx memory reinforce`

Reinforce a knowledge entry by incrementing its resonance, updating
`last_activated`, and incrementing `activation_count`. Used to signal
that an entry remains relevant.

### Flags

  **Flag**     **Type**   **Description**
  ------------ ---------- ------------------------------------------------
  `--amount`   `int`      Amount to increase resonance by. Default: `1`.
  `--cap`      `int`      Maximum resonance cap. Default: `10`.
  `--json`     `flag`     Output as JSON.

### Examples

``` bash
mx memory reinforce kn-abc123
```

``` bash
mx memory reinforce kn-abc123 --amount 2 --cap 8
```

## Metadata management {#metadata}

The knowledge graph has several registries for typed metadata. These
commands manage the registries themselves -- the types, categories, and
agent identities that entries reference.

### Agents

## `mx memory agents list`

List all registered agents.

### Flags

  **Flag**   **Type**   **Description**
  ---------- ---------- -----------------
  `--json`   `flag`     Output as JSON.

### Examples

``` bash
mx memory agents list
```

## `mx memory agents add`

Register a new agent.

### Flags

  **Flag**              **Type**   **Description**
  --------------------- ---------- ------------------------------
  `-d, --description`   `string`   Agent description.
  `-D, --domain`        `string`   Agent domain/responsibility.

### Examples

``` bash
mx memory agents add whistledown -d "Round-trip builder" -D "development"
```

## `mx memory agents show`

Show details for a specific agent.

### Examples

``` bash
mx memory agents show whistledown
```

### Projects

## `mx memory projects list`

List all registered projects.

### Flags

  **Flag**   **Type**   **Description**
  ---------- ---------- -----------------
  `--json`   `flag`     Output as JSON.

### Examples

``` bash
mx memory projects list
```

## `mx memory projects add`

Register a new project.

### Flags

  **Flag**          **Type**   **Description**
  ----------------- ---------- ------------------------------------------
  `--id`            `string`   Unique project identifier.
  `--name`          `string`   Human-readable project name.
  `--path`          `path`     Local filesystem path to the project.
  `--repo-url`      `string`   Git repository URL (e.g., `owner/repo`).
  `--description`   `string`   Project description.

### Examples

``` bash
mx memory projects add --id mx --name "mx CLI" \
  --repo-url coryzibell/mx --path ~/recipes/coryzibell/mx
```

### Categories

## `mx memory categories list`

List all categories.

### Flags

  **Flag**   **Type**   **Description**
  ---------- ---------- -----------------
  `--json`   `flag`     Output as JSON.

### Examples

``` bash
mx memory categories list
```

## `mx memory categories add`

Add a new category.

### Examples

``` bash
mx memory categories add pitfall "Things that went wrong and why"
```

## `mx memory categories remove`

Remove a category (only if no entries use it).

### Examples

``` bash
mx memory categories remove pitfall
```

### Applicability

## `mx memory applicability list`

List all applicability types.

### Examples

``` bash
mx memory applicability list
```

## `mx memory applicability add`

Add a new applicability type.

### Flags

  **Flag**          **Type**   **Description**
  ----------------- ---------- -------------------------------------------------
  `--id`            `string`   Unique identifier.
  `--description`   `string`   Description of when this applicability applies.
  `--scope`         `string`   Scope constraint (e.g., `project`, `global`).

### Examples

``` bash
mx memory applicability add --id rust-only \
  --description "Applies only to Rust projects" --scope project
```

### Type registries

These are read-only registries listing the valid values for typed
fields. Each supports `list` with an optional `--json` flag.

  **Command**                           **Lists valid values for**
  ------------------------------------- ---------------------------------------------------------------------------------------
  `mx memory tags list`                 Tags used across entries. Supports `--category` filter.
  `mx memory source-types list`         Source types (`manual`, `ram`, `cache`, `agent_session`).
  `mx memory entry-types list`          Entry types (`primary`, `summary`, `synthesis`).
  `mx memory session-types list`        Session types (e.g., `development`, `review`, `exploration`).
  `mx memory relationship-types list`   Relationship types (`related`, `supersedes`, `extends`, `implements`, `contradicts`).
  `mx memory content-types list`        Content types (`text`, `code`, `config`, `data`, `binary`).

All type registry `list` commands accept `--json` for structured output.
`tags list` also accepts `--category` to filter tags to a specific
category.

## Session tracking {#sessions}

Sessions group entries created during a work period. Entries can be
linked to sessions, and facts can be queried by their source session.

## `mx memory sessions list`

List sessions, optionally filtered by project.

### Flags

  **Flag**      **Type**   **Description**
  ------------- ---------- -----------------------
  `--project`   `string`   Filter by project ID.
  `--json`      `flag`     Output as JSON.

### Examples

``` bash
mx memory sessions list
```

``` bash
mx memory sessions list --project mx
```

## `mx memory sessions create`

Create a new session.

### Flags

  **Flag**           **Type**   **Description**
  ------------------ ---------- --------------------------------------------------------------
  `--session-type`   `string`   Session type (e.g., `development`, `review`, `exploration`).
  `--project`        `string`   Associated project ID.

### Examples

``` bash
mx memory sessions create --session-type development --project mx
```

## `mx memory sessions close`

Close an active session.

### Flags

  **Flag**   **Type**   **Description**
  ---------- ---------- ----------------------
  `--id`     `string`   Session ID to close.

### Examples

``` bash
mx memory sessions close --id ses-abc123
```

## `mx memory for-session`

List facts extracted from a specific session. The session ID can be
provided with or without the `kn-` prefix.

### Flags

  **Flag**   **Type**   **Description**
  ---------- ---------- -----------------
  `--json`   `flag`     Output as JSON.

### Examples

``` bash
mx memory for-session ses-abc123
```

## `mx memory fact-session`

Get the session a fact was extracted from. The fact ID can be provided
with or without the `kn-` prefix.

### Flags

  **Flag**   **Type**   **Description**
  ---------- ---------- -----------------
  `--json`   `flag`     Output as JSON.

### Examples

``` bash
mx memory fact-session kn-abc123
```

# Codex

Session archival, export, and retrieval.

------------------------------------------------------------------------

## Overview

The codex is the permanent archive for Claude Code session transcripts.
Every time you interact with Claude, the conversation is recorded as a
JSONL file under `~/.claude/projects/`. Those files are ephemeral --
Claude can overwrite or rotate them at any time. The codex captures them
into a stable, searchable archive before they disappear.

Each archived session produces a directory containing:

- `manifest.json` -- metadata (timestamps, message count, agent count,
  project path, checksum)

- `session.jsonl` -- the raw transcript (omitted in `--clean` mode)

- `conversation.md` -- a clean markdown rendering of the conversation
  (generated in `--clean` mode, or via `migrate --clean`)

- `agents/` -- sub-agent JSONL files (when sub-agents were used)

- `images/` -- extracted base64 images (pulled out during migration or
  archive)

Archives are stored under `$MX_CODEX_PATH` (defaults to
`$MX_HOME/codex/`), one directory per session, named with a timestamp
and short session ID.

## Archiving sessions

## `mx codex archive`

Archive session transcripts to permanent storage. With no arguments,
archives the most recent non-agent session. With `--all`, walks
`~/.claude/projects/` and archives every session not already in the
codex. Already-archived sessions are skipped (idempotent).

After archiving, the by-project index is rebuilt so subsequent reads can
locate sessions by project name.

### Flags

  **Flag**                    **Type**        **Description**
  --------------------------- --------------- -----------------------------------------------------------------------------------------------------------------------------------------------------------
  `[PATH]`                    positional      Path to a specific session JSONL file. Conflicts with `--all` and `--backfill`.
  `--all`                     flag            Archive all unarchived sessions. Conflicts with `--backfill`.
  `--clean`                   flag            Save only `conversation.md`, `manifest.json`, and extracted images. Omits the raw JSONL and agent files. Produces a smaller, human-readable archive.
  `--include-agents`          flag            Fold sub-agent transcripts into `conversation.md`. Requires `--clean` and requires `subagents` in `--include` (the default).
  `--include <LIST>`          string          Comma-separated list of source artifacts to capture. Recognized tokens: `subagents` (default), `mcp`, `tool-output`, `history`, `all`, `none`. See below.
  `--backfill [VAULT_PATH]`   optional path   Ingest legacy vault snapshots into the codex. See *Backfill* section. Conflicts with `--all` and `[PATH]`.

### Examples

``` bash
mx codex archive
```

``` bash
mx codex archive --all
```

``` bash
mx codex archive --clean --include-agents
```

``` bash
mx codex archive --all --clean --include subagents,mcp
```

``` bash
mx codex archive /path/to/specific-session.jsonl
```

### The --include flag

The `--include` flag controls which optional source artifacts are
captured alongside the session transcript. Tokens are case-insensitive
and comma-separated.

  **Token**       **Default**   **Description**
  --------------- ------------- -----------------------------------------------
  `subagents`     ON            Copy sub-agent JSONL files into `agents/`.
  `mcp`           OFF           Capture MCP server logs.
  `tool-output`   OFF           Capture `/tmp` tool output snapshots.
  `history`       OFF           Capture a slice of `~/.claude/history.jsonl`.
  `all`           --            Enable all of the above.
  `none`          --            Disable all of the above.

Passing both `all` and `none` in the same value is rejected as a user
error. Unknown tokens print a warning and are skipped. The hyphenated
`tool-output` is the canonical spelling; `tool_output` (with underscore)
is not recognized.

::: {.admonition .note}
**NOTE:** The `--include` flag on `archive` governs which source files
are *captured* into the archive. The separate `--include` flag on
`export` governs which captured artifacts are *rendered* into the
output. They share token names but serve different purposes.
:::

### Clean mode

When `--clean` is passed, the archive stores a rendered
`conversation.md` instead of the raw session JSONL. The transcript is
generated by:

1.  Extracting `user` and `assistant` messages from the JSONL

2.  Stripping `<system-reminder>` blocks from user messages

3.  Dropping tool-use blocks, tool results, and non-conversation message
    types

4.  Labeling speakers with configurable names (`MX_USER_NAME` env var,
    or git `user.name`, falling back to "User"; `MX_ASSISTANT_NAME` env
    var, falling back to "Orchestrator")

With `--include-agents`, sub-agent transcripts are appended to
`conversation.md` under `## Agent: <name>` headings, separated by
horizontal rules. Agent names are resolved from the parent session's
`subagent_type` field when available, falling back to the hex ID from
the filename.

## Backfill

The `--backfill` flag migrates historical session data from the legacy
vault (`~/.wonka/vault/archives/`) into the codex.

## `mx codex archive --backfill`

Walk every `session-*` snapshot under the vault path and feed each
session JSONL through the standard archive pipeline. The vault path
defaults to `~/.wonka/vault/archives/` when no value is given.

Backfill is idempotent: re-running against the same vault produces the
same codex state. Sessions already present in the codex (matched by
session ID derived from the JSONL filename) are skipped. Per-session
failures are non-fatal -- errors are accumulated and reported at the end
so a single corrupt file does not abort the entire run.

The `--clean` and `--include` flags still apply during backfill,
governing what each per-session archive captures.

### Flags

  **Flag**                    **Type**        **Description**
  --------------------------- --------------- -------------------------------------------------------------------------------
  `--backfill [VAULT_PATH]`   optional path   Path to the vault archives directory. Defaults to `~/.wonka/vault/archives/`.

### Examples

``` bash
mx codex archive --backfill
```

``` bash
mx codex archive --backfill /custom/vault/path
```

``` bash
mx codex archive --backfill --clean --include-agents
```

The expected vault layout is:

    <vault_path>/
      session-YYYYMMDD-HHMMSS-NNNNNN/
        projects/
          <project-slug>/
            <session-uuid>.jsonl
            <session-uuid>/
              subagents/
                agent-*.jsonl

::: {.admonition .tip}
**TIP:** When a vault exists at the default path, every `mx codex`
command prints a reminder to run `mx codex archive --backfill`. The
reminder is suppressed when you are already running backfill.
:::

## Exporting

## `mx codex export`

Export archived sessions as Markdown or structured JSON. Content is read
exclusively from the codex -- live `~/.claude/projects/` data is never
ingested directly. If unarchived sessions are detected, a warning is
printed to stderr (unless `--archive-first` is passed).

With no selector flags, exports the most recent archived session.
Selectors are mutually exclusive: at most one of `--session`,
`--project`, or `--date` may be passed.

### Flags

  **Flag**                **Type**   **Description**
  ----------------------- ---------- ------------------------------------------------------------------------------------------------------------------------------------------------------------
  `--session <UUID>`      string     Select by session UUID (full or unique prefix).
  `--project <QUERY>`     string     Filter by project: absolute path, cwd-encoded slug, or basename. Ambiguous basenames list collisions and exit non-zero.
  `--date <RANGE>`        string     Date selector. Accepts `YYYY-MM-DD`, `YYYY-MM-DD..YYYY-MM-DD`, or `YYYY-MM`.
  `--format <FMT>`        string     Output format: `markdown` (default), `json`, or `both`. `both` requires `--output`.
  `--include <LIST>`      string     Comma-separated content to render. Default: `subagents`. Tokens: `subagents`, `tools`, `system-reminders`, `mcp`, `tool-output`, `history`, `all`, `none`.
  `--archive-first`       flag       Run `mx codex archive --all` before exporting. Suppresses the unarchived-data warning.
  `-o, --output <PATH>`   path       Output file. Default: stdout. Required when `--format both`.

### Examples

``` bash
mx codex export
```

``` bash
mx codex export --session abc123
```

``` bash
mx codex export --project mx --format json
```

``` bash
mx codex export --date 2026-04 -o april.md
```

``` bash
mx codex export --date 2026-04-01..2026-04-15 --format both -o sessions
```

``` bash
mx codex export --archive-first --include all -o full.md
```

### Format: both

When `--format both` is used with `--output`, two sidecar files are
written:

- If the path ends in `.json`: JSON goes to the path, markdown goes to
  `<path>.md`

- If the path ends in `.md`: markdown goes to the path, JSON goes to
  `<path>.json`

- Otherwise: both extensions are appended (`<path>.json` and
  `<path>.md`)

Using `--format both` without `--output` is rejected -- there is no
clean way to multiplex two formats on stdout.

### Export --include tokens

The export `--include` set is distinct from the archive `--include` set.
It controls which content is *rendered*, not which is *captured*. Two
additional tokens are available for export only:

  **Token**            **Description**
  -------------------- --------------------------------------------------------
  `tools`              Render `tool_use` blocks (assistant tool invocations).
  `system-reminders`   Render `<system-reminder>` blocks.

The remaining tokens (`subagents`, `mcp`, `tool-output`, `history`,
`all`, `none`) behave identically to the archive side.

## Browsing

### List

## `mx codex list`

List archived sessions. By default, shows only the latest version of
each session (filtering out incremental re-archives). Output is a table
with columns: archive ID, archived timestamp, message count, agent
count, and size.

### Flags

  **Flag**   **Type**   **Description**
  ---------- ---------- ------------------------------------------------
  `--all`    flag       Show all archives including incremental saves.
  `--json`   flag       Output as JSON array.

### Examples

``` bash
mx codex list
```

``` bash
mx codex list --all
```

``` bash
mx codex list --json
```

### Read

## `mx codex read <ID>`

Read an archived session by its short archive ID (from `mx codex list`).
The ID is matched as a substring against archive directory names.

By default, outputs the raw transcript. With `--clean`, outputs the
rendered `conversation.md` (errors if no clean transcript exists -- use
`mx codex migrate --clean` to generate one). With `--human`,
pretty-prints JSONL as labeled User/Assistant blocks.

### Flags

  **Flag**             **Type**     **Description**
  -------------------- ------------ -----------------------------------------------------------------------------------
  `<ID>`               positional   Archive ID (short UUID from `list`).
  `--human`            flag         Pretty-print JSONL in human-readable format. Conflicts with `--clean`.
  `--agents`           flag         Include agent transcripts in the output.
  `--grep <PATTERN>`   string       Filter output to lines matching the pattern.
  `--json`             flag         Output the manifest as JSON.
  `--clean`            flag         Read the clean markdown transcript (`conversation.md`). Conflicts with `--human`.

### Examples

``` bash
mx codex read abc12345
```

``` bash
mx codex read abc12345 --clean
```

``` bash
mx codex read abc12345 --clean --agents
```

``` bash
mx codex read abc12345 --human
```

``` bash
mx codex read abc12345 --grep "migration"
```

``` bash
mx codex read abc12345 --json
```

### Search

## `mx codex search <PATTERN>`

Search all archived sessions for a text pattern. Scans both
`conversation.md` (preferred) and `session.jsonl` (fallback) in each
archive. Reports matching archive IDs and the lines that contain the
pattern. Archives with no transcript file are skipped with a count
reported to stderr.

### Flags

  **Flag**      **Type**     **Description**
  ------------- ------------ -----------------------------
  `<PATTERN>`   positional   Text pattern to search for.
  `--json`      flag         Output matches as JSON.

### Examples

``` bash
mx codex search "retry logic"
```

``` bash
mx codex search migration --json
```

## Migration

## `mx codex migrate`

Upgrade archive formats. Archives below the current schema version are
migrated forward.

Without `--clean`, the primary migration is image extraction: v1
archives have base64 images embedded in the JSONL. Migration extracts
them to `images/` files and rewrites the JSONL with references. Older
archives also receive a metadata-only version bump to the current schema
(v5). Original files are backed up as `*.bak`.

With `--clean`, generates `conversation.md` for archives that have
`session.jsonl` but no clean transcript. This is useful for
retroactively adding human-readable transcripts to archives created
before clean mode existed.

### Flags

  **Flag**             **Type**   **Description**
  -------------------- ---------- -----------------------------------------------------------------------------------
  `--dry-run`          flag       Show what would be migrated without making changes.
  `--detailed`         flag       Show detailed progress for each archive.
  `--clean`            flag       Generate `conversation.md` for archives missing a clean transcript.
  `--include-agents`   flag       Include sub-agent transcripts in generated clean transcripts. Requires `--clean`.

### Examples

``` bash
mx codex migrate --dry-run
```

``` bash
mx codex migrate --detailed
```

``` bash
mx codex migrate --clean
```

``` bash
mx codex migrate --clean --include-agents --detailed
```

## Deprecated alias

::: {.admonition .deprecated}
**DEPRECATED:** `mx codex save` was renamed to `mx codex archive` in PR
#284 (issue #273). The `save` subcommand still works and accepts all the
same flags, but it prints a deprecation notice to stderr on every
invocation:
:::

    note: `mx codex save` is deprecated; use `mx codex archive` instead.

The `save` alias is hidden from `--help` output. It will be removed in a
future release. Update scripts and muscle memory to use
`mx codex archive`.

# KV Store

Fast local key-value state per agent.

------------------------------------------------------------------------

The KV subsystem gives each agent a lightweight, schema-driven key-value
store for operational state that needs to be fast and local. Counters,
strings, lists, timestamped history, and structured state fields -- all
backed by a TOML schema file and a JSON data file. No networking, no
database. Reads and writes are direct file operations with atomic saves
(serialize to tmp, fsync, rename). History and list entries can carry
structured JSON data for queryable metadata.

Use KV for state that lives within a single agent session or across
sessions: build counters, track decisions as a history log, maintain a
todo list, or store the current goal as a string. For cross-agent
knowledge that needs search, tagging, and relationships, use Memory
instead.

## Concepts

### Data types

Every key has a type declared in the schema. Five types are supported:

string

:   A single text value. Has an optional `default`.

counter

:   An integer with optional `min`, `max`, and `default`. Clamped on
    every write.

history

:   A timestamped append-only log. Newest entries first. Has an optional
    `max_entries` cap that drops the oldest entries on overflow. Each
    entry gets a numeric index and a stable ID. Entries can carry
    optional structured JSON data.

list

:   An ordered collection with timestamps. Supports push and pop. Also
    has an optional `max_entries` cap. Each entry gets a numeric index
    and a stable ID. Entries can carry optional structured JSON data.

state

:   A structured record with named fields. Fields are declared in the
    schema and validated on write.

### Schema files

Each agent has a TOML schema file that declares every valid key, its
type, and any constraints. The schema lives at:

    $MX_HOME/kv/schema/{agent}.toml

The data file (JSON, auto-created on first write) lives at:

    $MX_HOME/kv/data/{agent}.json

The active agent is determined by the `MX_CURRENT_AGENT` environment
variable.

You can override the paths with `MX_KV_SCHEMA` and `MX_KV_DATA`
environment variables. Both support an `{agent}` placeholder that
expands to the current agent name.

### Schema format

A schema file is TOML with a `[keys.<name>]` section per key:

``` toml
[keys.builds]
type = "counter"
min = 0
default = "0"

[keys.session_goal]
type = "string"
default = ""

[keys.decisions]
type = "history"
max_entries = 50

[keys.ideas]
type = "list"

[keys.todos]
type = "list"
max_entries = 20
description = "Pending work items"

[keys.context]
type = "state"
fields = ["goal", "phase", "blocker"]
```

Schema fields:

`type`

:   Required. One of `string`, `counter`, `history`, `list`, `state`.

`default`

:   Optional. Initial value for string and counter types.

`min`

:   Optional. Minimum value for counters (clamped, never errors).

`max`

:   Optional. Maximum value for counters (clamped, never errors).

`max_entries`

:   Optional. Maximum entries for history and list types. Oldest entries
    are dropped when exceeded. Omit to allow unbounded growth.

`description`

:   Optional. Human-readable description of the key's purpose. Displayed
    as a third column by `mx kv keys`.

`fields`

:   Optional. List of valid field names for state types. Writes to
    unlisted fields are rejected.

### Auto-creating keys

Keys can be added to the schema on the fly with `mx kv push --create`.
When a key does not exist in the schema, `--create history` or
`--create list` appends a new `[keys.<name>]` block to the TOML file and
reloads the in-memory schema. This avoids manual schema editing for
simple cases. See push for details and validation rules.

### Agent keying

All KV operations require `MX_CURRENT_AGENT` to be set. Each agent gets
its own schema and data file -- there is no cross-agent state leakage.
Two agents can define entirely different schemas with different keys.

### Exit codes

KV commands use structured exit codes for scripting:

`0`

:   Success.

`1`

:   Key not found (or no data yet for that key).

`2`

:   Type mismatch (e.g., `inc` on a string key, or `get --id` on a
    non-history/list key).

`3`

:   Schema file not found.

`4`

:   Invalid input (e.g., reversed range, empty spec, empty ID after
    `kv-` prefix, entry not found by ID, ambiguous ID prefix).

## Basic operations

## `mx kv get <key>`

Get the current value of a key, or look up specific entries by ID.

Without `--id`, prints the full current value: raw text for strings and
counters, all entries with indexes and timestamps for history and list
types, and fields as JSON for state types.

With `--id`, retrieves specific entries from a history or list by
numeric index or entry ID. Four formats are supported:

Single numeric index

:   `--id 35` -- returns exactly one entry.

Single entry ID

:   `--id kv-A3fB` -- returns the entry matching that ID. ID matching is
    prefix-based: `kv-A3f` will match if the prefix is unambiguous.

Numeric range

:   `--id 35-64` -- returns all entries with indexes 35 through 64
    inclusive. Maximum range size is 10,000 entries. Ranges are numeric
    only.

Comma-separated

:   `--id 1,kv-A3fB,12` -- returns the listed entries. Numeric indexes
    and entry IDs can be mixed freely in comma lists.

If any requested IDs are not found, a note listing the missing IDs is
printed to stderr. The found entries are still printed to stdout.

The `--id` flag only works on history and list types. Using it on a
string, counter, or state key returns exit code 2 (type mismatch). Parse
failures (reversed ranges, empty specs) return exit code 4 (invalid
input).

### Flags

  **Flag**        **Type**   **Description**
  --------------- ---------- ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------
  `--id <spec>`   string     Entry identifier: numeric index (`35`), entry ID (`kv-A3fB`), range (`35-64`), or comma-separated (`1,kv-A3fB,12`)
  `--memory`      flag       Resolve and display any linked memory entry
  `--json`        flag       Output as JSON. Collections emit a JSON array of entry objects. Scalars emit `{"value": "..."}`. Memory resolution is skipped in JSON mode; the raw `kn-` ID is included in each entry's `memory` field.

### Examples

``` bash
mx kv get session_goal
```

``` bash
mx kv get builds
```

``` bash
mx kv get decisions
```

``` bash
mx kv get context --memory
```

``` bash
mx kv get shipped --id 35
```

``` bash
mx kv get shipped --id kv-A3fB
```

``` bash
mx kv get shipped --id 35-64
```

``` bash
mx kv get shipped --id 1,kv-A3fB,12
```

``` bash
mx kv get shipped --id 35 --memory
```

``` bash
mx kv get shipped --id 42 --json
```

``` bash
mx kv get shipped --json
```

## `mx kv set <key> [args...] [--json <value>]`

Set a value for a string, counter, or state key, or link a specific
entry to a memory node. Supports four input modes for state keys:
single-field, inline batch, JSON object, and JSON array (tensor).

For **string** keys: `mx kv set <key> <value>` sets the value directly.

For **counter** keys: `mx kv set <key> <value>` parses the value as an
integer and clamps to min/max.

For **state** keys, four input modes are available:

Single field (legacy)

:   `mx kv set <key> <field> <value>` -- sets one field. Backward
    compatible with pre-batch syntax.

Inline batch

:   `mx kv set <key> field1=value1 field2=value2 ...` -- sets multiple
    fields atomically. All field names are validated against the schema
    before any writes. Unmentioned fields are preserved (partial
    update).

JSON object

:   `mx kv set <key> --json '{"field1":"value1","field2":"value2"}'` --
    sets multiple fields from a JSON object. Non-string values (numbers,
    booleans, null) are coerced to strings. Same atomic validation as
    inline batch.

JSON array (tensor)

:   `mx kv set <key> --json '[0.4, 0.6, 0.5]'` -- sets all fields by
    position. The number of array elements must exactly match the number
    of fields declared in the schema. Values are mapped to schema fields
    in declaration order.

The `--json` flag accepts `"-"` to read JSON from stdin:
`echo '{"goal":"done"}' | mx kv set context --json -`

The `--json` flag and positional arguments cannot be combined -- use one
or the other.

Batch operations (inline and JSON) are all-or-nothing: if any field name
is invalid, no fields are written. Duplicate field names in a single
batch are rejected. All validation errors are reported, not just the
first.

With `--id` and `--memory`, links an existing history or list entry to a
memory knowledge node. The `--id` flag accepts a numeric index (`17`) or
an entry ID (`kv-A3fB`). ID matching is prefix-based -- an ambiguous
prefix returns an error asking for more characters. `--id` requires
`--memory`; it cannot be used alone. Pass an empty string
(`--memory ""`) to clear a per-entry link.

### Flags

  **Flag**             **Type**   **Description**
  -------------------- ---------- --------------------------------------------------------------------------------------------------------
  `--json <value>`     string     JSON input: object for state batch set, array for tensor positional set. Use `"-"` to read from stdin.
  `--memory <kn-id>`   string     Link a memory entry (kn- ID) to this key or entry, or `""` to clear
  `--id <spec>`        string     Target a specific entry by numeric index or entry ID (requires `--memory`)

### Examples

``` bash
mx kv set session_goal "ship the docs"
```

``` bash
mx kv set builds 0
```

``` bash
mx kv set context goal "finish KV docs"
```

``` bash
mx kv set context phase "writing"
```

``` bash
mx kv set context goal="done" phase="writing"
```

``` bash
mx kv set context goal="done" phase="writing" blocker="none"
```

``` bash
mx kv set context --json '{"goal":"done","phase":"writing"}'
```

``` bash
mx kv set mytensor --json '[0.4, 0.6, 0.5]'
```

``` bash
echo '{"goal":"done"}' | mx kv set context --json -
```

``` bash
mx kv set decisions --memory kn-abc123
```

``` bash
mx kv set decisions --memory ""
```

``` bash
mx kv set decisions --id 17 --memory kn-abc123
```

``` bash
mx kv set decisions --id kv-A3fB --memory kn-def456
```

``` bash
mx kv set decisions --id 17 --memory ""
```

## `mx kv keys`

List all keys defined in the schema with their types. Output is two
columns: key name (left-aligned, 30 chars) and type. When a key has a
`description` in the schema, it is shown as a third column.

### Examples

``` bash
mx kv keys
```

## Counters

## `mx kv inc <key>`

Increment a counter key. Returns the new value after incrementing. The
result is clamped to the schema's min/max bounds -- it never errors on
overflow, it just stops at the limit.

### Flags

  **Flag**     **Type**   **Description**
  ------------ ---------- -------------------------------------
  `--by <n>`   integer    Amount to increment by (default: 1)

### Examples

``` bash
mx kv inc builds
```

``` bash
mx kv inc builds --by 5
```

## `mx kv dec <key>`

Decrement a counter key. Returns the new value after decrementing. Like
`inc`, the result is clamped to schema bounds.

### Flags

  **Flag**     **Type**   **Description**
  ------------ ---------- -------------------------------------
  `--by <n>`   integer    Amount to decrement by (default: 1)

### Examples

``` bash
mx kv dec retries
```

``` bash
mx kv dec retries --by 3
```

## Lists & History

History and list types both store timestamped entries with auto-assigned
IDs. The difference is semantic: history is append-only (newest first,
no pop), while lists support push/pop and maintain insertion order.

Every entry gets two identifiers: a numeric index (sequential, per-key)
and a stable entry ID (a base58 string prefixed with `kv-`, e.g.
`kv-A3fB`). Both can be used anywhere an ID is accepted. See Entry IDs
for details.

Both types support `push`, `last`, `search`, `count`, `random`,
`remove`, `update`, `migrate`, and entry lookup by ID via `get --id`.
Both support structured data on entries (`--data` on push/update) and
structured data filtering (`--where` on queries). Only lists support
`pop`. Only history supports `since` (time-based queries).

### push

## `mx kv push <key> <value>`

Push a value onto a history or list key. The entry is automatically
timestamped and assigned both a numeric index and a stable entry ID.

On success, prints the new entry's identifiers:

    kv-A3fB (42)
      

Entry ID first (the primary stable identifier), numeric index in
parentheses.

For **history** keys, new entries are inserted at the front (newest
first). If the key has a `max_entries` schema constraint, the oldest
entries are truncated after the push.

For **list** keys, new entries are appended to the end. The same
`max_entries` truncation applies, dropping from the front.

Use `--data` to attach a JSON object to the entry. The data is stored
alongside the value and timestamp, and is displayed inline in output.
See Structured data for details and query examples.

Use `--memory` to link the new entry to a knowledge node in the memory
graph. This sets a per-entry memory pointer (a `kn-` ID) that is
resolved when `--memory` is passed to read commands. See Per-entry
memory links for the full resolution hierarchy.

Use `--create` to auto-add the key to the schema if it does not already
exist. Pass the type as the value: `--create history` or
`--create list`. Only `history` and `list` types are accepted (those are
the types that support `push`). If the key already exists in the schema,
`--create` is silently ignored -- this makes it safe to use
unconditionally in scripts without checking whether the key has been
defined yet.

The optional `--max-entries` flag sets the entry cap for the new key. It
requires `--create` and has no effect if the key already exists.

Key names are validated on creation: alphanumeric characters,
underscores, and hyphens only, maximum 128 characters. Dots are rejected
because they conflict with TOML key quoting. The new key block is
appended to the schema file without reformatting existing content.

### Flags

  **Flag**              **Type**   **Description**
  --------------------- ---------- ------------------------------------------------------------------------------------------------------------------------------
  `--data <json>`       string     Attach a JSON object to the entry. Must be a valid JSON object (not an array, string, or other type).
  `--memory <kn-id>`    string     Link this entry to a memory knowledge node (e.g. `kn-abc123`). Resolved when `--memory` is passed on read commands.
  `--create <type>`     enum       Auto-create the key in the schema if missing. Accepted types: `history`, `list`. Silently ignored if the key already exists.
  `--max-entries <n>`   integer    Maximum entries for the new key (only valid with `--create`). Oldest entries are dropped when exceeded.

### Examples

``` bash
mx kv push decisions "chose Typst for docs"
```

``` bash
mx kv push todos "write tests for kv handler"
```

``` bash
mx kv push projects "palmtop DSI fix" --data '{"tags":["palmtop","i915"],"status":"active"}'
```

``` bash
mx kv push shipped "v0.1.156" --data '{"pr":305,"scope":"kv"}'
```

``` bash
mx kv push decisions "adopted per-entry memory links" --memory kn-abc123
```

``` bash
mx kv push puns "the joke" --create history
```

``` bash
mx kv push ideas "wild thought" --create list --max-entries 500
```

### pop

## `mx kv pop <key>`

Pop the last item from a list key. Prints the removed entry with its
numeric index, entry ID, value, and timestamp. Returns silently if the
list is empty.

Only works on list types. History keys are append-only and do not
support pop.

### Examples

``` bash
mx kv pop todos
```

### last

## `mx kv last <key>`

Get the last N entries from a history or list key. Entries are printed
with their numeric index, entry ID, value, and timestamp.

For history keys, "last" means the most recent (entries are stored
newest first). For list keys, "last" means the tail of the list.

Time-range flags narrow the result set before `--count` is applied. See
Time-range queries for details and examples.

The `--where` flag filters entries by structured data fields. Multiple
`--where` flags are ANDed. See Structured data for filtering semantics.

### Flags

  **Flag**                      **Type**   **Description**
  ----------------------------- ---------- ---------------------------------------------------------------------------------------
  `--count <n>`                 integer    Number of entries to return (default: 1)
  `--memory`                    flag       Resolve and display any linked memory entry
  `--json`                      flag       Output as a JSON array of entry objects
  `--where <key=value>`         string     Filter by structured data field (repeatable, ANDed). Top-level fields only.
  `--day <YYYY-MM-DD>`          string     Entries from a specific day (UTC)
  `--month <YYYY-MM>`           string     Entries from a specific month (UTC)
  `--week <YYYY-Www>`           string     Entries from an ISO week, Monday to Sunday
  `--from <YYYY-MM-DD>`         string     Start of date range, inclusive (UTC)
  `--to <YYYY-MM-DD>`           string     End of date range, inclusive (UTC)
  `--since <relative-or-iso>`   string     Filter entries since a relative time (`30d`, `1w`, `2h`, `30m`) or ISO-8601 timestamp

### Examples

``` bash
mx kv last decisions
```

``` bash
mx kv last decisions --count 5
```

``` bash
mx kv last todos --count 3 --memory
```

``` bash
mx kv last shipped --day 2026-04-25
```

``` bash
mx kv last shipped --month 2026-04
```

``` bash
mx kv last shipped --month 2026-04 --count 5
```

``` bash
mx kv last shipped --since 1w
```

``` bash
mx kv last projects --where status=active
```

``` bash
mx kv last projects --where status=active --count 3
```

``` bash
mx kv last projects --count 5 --json
```

### since

## `mx kv since <key> <timeref>`

Get history entries since a time reference. Only works on history keys.

The time reference can be relative or absolute:

- Relative: `30m` (minutes), `1h` (hours), `7d` (days), `2w` (weeks)

- Absolute: ISO-8601 format (e.g., `2025-01-15T10:00:00Z`)

Entries are printed with their numeric index, entry ID, value, and
timestamp.

### Flags

  **Flag**     **Type**   **Description**
  ------------ ---------- ---------------------------------------------
  `--memory`   flag       Resolve and display any linked memory entry
  `--json`     flag       Output as a JSON array of entry objects

### Examples

``` bash
mx kv since decisions 1h
```

``` bash
mx kv since decisions 7d
```

``` bash
mx kv since decisions 2w --memory
```

``` bash
mx kv since decisions 2025-01-15T10:00:00Z
```

``` bash
mx kv since shipped 1w --json
```

### search

## `mx kv search <key> [query]`

Search entries in a list or history by case-insensitive substring match
and/or structured data filters. Prints matching entries with their
numeric index, entry ID, value, timestamp, and any attached data.

The text query is optional when `--where` filters are provided. You can
search by text alone, by structured data alone, or by both. At least one
of a text query or `--where` filter must be given.

Multiple `--where` flags are ANDed. See Structured data for filtering
semantics.

Time-range flags narrow the search to entries within the specified
period. See Time-range queries for details.

### Flags

  **Flag**                      **Type**   **Description**
  ----------------------------- ---------- -------------------------------------------------------------------------------
  `--memory`                    flag       Resolve and display any linked memory entry
  `--json`                      flag       Output as a JSON array of entry objects
  `--where <key=value>`         string     Filter by structured data field (repeatable, ANDed). Top-level fields only.
  `--day <YYYY-MM-DD>`          string     Search within a specific day (UTC)
  `--month <YYYY-MM>`           string     Search within a specific month (UTC)
  `--week <YYYY-Www>`           string     Search within an ISO week, Monday to Sunday
  `--from <YYYY-MM-DD>`         string     Start of date range, inclusive (UTC)
  `--to <YYYY-MM-DD>`           string     End of date range, inclusive (UTC)
  `--since <relative-or-iso>`   string     Search since a relative time (`30d`, `1w`, `2h`, `30m`) or ISO-8601 timestamp

### Examples

``` bash
mx kv search decisions "typst"
```

``` bash
mx kv search todos "test"
```

``` bash
mx kv search shipped "feature" --month 2026-04
```

``` bash
mx kv search shipped "feature" --since 30d
```

``` bash
mx kv search projects --where status=active
```

``` bash
mx kv search projects "DSI" --where status=active
```

``` bash
mx kv search projects --where tags=palmtop --where status=active
```

``` bash
mx kv search projects --where status=active --json
```

### count

## `mx kv count <key> [value]`

Count entries in a list or history. Without a value filter or `--where`,
prints the total count. With a value filter, `--where`, or both, prints
the matched count, total, and percentage.

Unfiltered output: `<count>` or `<count> (latest: <timestamp>)`.

Filtered output: `<matched>/<total> (<pct>%) --- latest: <timestamp>`.

The percentage display makes it easy to gauge ratios at a glance -- for
example, what fraction of your decisions mentioned a particular topic,
or how many entries have `status=active` in their structured data.

Multiple `--where` flags are ANDed. See Structured data for filtering
semantics.

Time-range flags restrict the count to entries within the specified
period. See Time-range queries for details.

### Flags

  **Flag**                      **Type**   **Description**
  ----------------------------- ---------- ------------------------------------------------------------------------------------------------------
  `--json`                      flag       Output as JSON: `{"count": N}` with optional `total` and `latest_ts` fields when filtering is active
  `--where <key=value>`         string     Filter by structured data field (repeatable, ANDed). Top-level fields only.
  `--day <YYYY-MM-DD>`          string     Count within a specific day (UTC)
  `--month <YYYY-MM>`           string     Count within a specific month (UTC)
  `--week <YYYY-Www>`           string     Count within an ISO week, Monday to Sunday
  `--from <YYYY-MM-DD>`         string     Start of date range, inclusive (UTC)
  `--to <YYYY-MM-DD>`           string     End of date range, inclusive (UTC)
  `--since <relative-or-iso>`   string     Count since a relative time (`30d`, `1w`, `2h`, `30m`) or ISO-8601 timestamp

### Examples

``` bash
mx kv count decisions
```

``` bash
mx kv count decisions "typst"
```

``` bash
mx kv count todos "blocked"
```

``` bash
mx kv count shipped --day 2026-05-07
```

``` bash
mx kv count shipped --from 2026-04-01 --to 2026-04-15
```

``` bash
mx kv count shipped --since 1w
```

``` bash
mx kv count projects --where status=active
```

``` bash
mx kv count projects --where status=active --since 30d
```

``` bash
mx kv count shipped --json
```

### random

## `mx kv random <key>`

Get N random entries from a history or list key. Entries are printed
with their numeric index, entry ID, value, and timestamp.

Useful for inspiration (pick a random idea), spot-checking (sample from
a large history), or building variety into automated workflows.

When fewer entries are available than requested, all matching entries
are returned and a note is printed to stderr. If a time range or
`--where` filter is specified, entries are filtered first, then random
sampling is applied to the filtered set.

Multiple `--where` flags are ANDed. See Structured data for filtering
semantics.

### Flags

  **Flag**                      **Type**   **Description**
  ----------------------------- ---------- --------------------------------------------------------------------------------------------
  `--count <n>`                 integer    Number of random entries to return (default: 1, must be \>= 1)
  `--memory`                    flag       Resolve and display any linked memory entry
  `--json`                      flag       Output as a JSON array of entry objects
  `--where <key=value>`         string     Filter by structured data field (repeatable, ANDed). Top-level fields only.
  `--day <YYYY-MM-DD>`          string     Sample from entries on a specific day (UTC)
  `--month <YYYY-MM>`           string     Sample from entries in a specific month (UTC)
  `--week <YYYY-Www>`           string     Sample from entries in an ISO week, Monday to Sunday
  `--from <YYYY-MM-DD>`         string     Start of date range, inclusive (UTC)
  `--to <YYYY-MM-DD>`           string     End of date range, inclusive (UTC)
  `--since <relative-or-iso>`   string     Sample from entries since a relative time (`30d`, `1w`, `2h`, `30m`) or ISO-8601 timestamp

### Examples

``` bash
mx kv random shipped
```

``` bash
mx kv random shipped --count 5
```

``` bash
mx kv random ideas --count 1
```

``` bash
mx kv random shipped --count 3 --since 30d
```

``` bash
mx kv random decisions --month 2026-04 --count 3
```

``` bash
mx kv random projects --where status=active --count 3
```

``` bash
mx kv random ideas --count 3 --json
```

### remove

## `mx kv remove <key> [value]`

Remove entries from a list or history by value substring or by ID. You
must provide either a value substring or `--id`.

The `--id` flag accepts a numeric index (`7`) or an entry ID
(`kv-A3fB`). ID matching is prefix-based -- if the prefix is ambiguous
(matches multiple entries), an error is returned asking for more
characters.

By default, only the first match is removed. Use `--all` to remove every
matching entry.

### Flags

  **Flag**        **Type**   **Description**
  --------------- ---------- ------------------------------------------------------------------
  `--id <spec>`   string     Remove the entry with this numeric index or entry ID (`kv-XXXX`)
  `--all`         flag       Remove all matching entries (default: first match only)

### Examples

``` bash
mx kv remove todos "write tests"
```

``` bash
mx kv remove todos --id 7
```

``` bash
mx kv remove todos --id kv-A3fB
```

``` bash
mx kv remove decisions "typo" --all
```

### update

## `mx kv update <key> [value] --id <spec>`

Update an existing entry's value and/or structured data in-place.
Preserves the entry's ID, position, and timestamp.

Requires `--id` to target a specific entry by numeric index or entry ID.
ID matching is prefix-based -- if the prefix is ambiguous (matches
multiple entries), an error is returned asking for more characters. The
value argument is optional -- you can update only `--data`, only the
value, or both.

When `--data` is provided, the JSON object is shallow-merged into the
entry's existing structured data. Fields in the patch overwrite existing
fields. Null values in the patch *delete* that field from the merged
result (they do not set it to null). If the key has a `[data]` schema
section, validation runs on the merged result before the write commits.

At least one of a value argument or `--data` must be provided -- calling
update with neither is rejected.

On success, prints the updated entry's identifiers:

    Updated entry 42 (kv-A3fB)
      

Works on both history and list types.

### Flags

  **Flag**          **Type**   **Description**
  ----------------- ---------- --------------------------------------------------------------------------------------------------------------------
  `--id <spec>`     string     Target entry by numeric index (`42`) or entry ID (`kv-A3fB`). Required.
  `--data <json>`   string     JSON object to merge into the entry's structured data. Null field values delete that field from the merged result.

### Examples

``` bash
mx kv update projects "palmtop DSI fix (v2)" --id kv-A3fB
```

``` bash
mx kv update projects --id 42 --data '{"status":"done"}'
```

``` bash
mx kv update projects "renamed" --id kv-A3fB --data '{"status":"closed"}'
```

``` bash
mx kv update projects --id 42 --data '{"obsolete_field":null}'
```

### migrate

## `mx kv migrate <key>`

Migrate existing entries to match current schema data definitions.
Operates on all entries in a key.

Compares each entry's structured data against the `[data]` section in
the key's schema. Missing fields that have a default value in the schema
are added. Required fields without defaults produce a warning. Type
mismatches between existing data and schema declarations are reported as
warnings.

Entries without any structured data get a new data object populated from
schema defaults.

With `--prune`, fields present in entries but not declared in the schema
are removed. Without `--prune`, undeclared fields are left untouched.

With `--dry-run`, reports what would change without modifying any data.
The output lists each affected entry with its added and pruned fields.

If the key has no `[data]` section in the schema, nothing is migrated
and a warning is printed.

Works on both history and list types.

### Flags

  **Flag**      **Type**   **Description**
  ------------- ---------- --------------------------------------------------
  `--prune`     flag       Remove fields not declared in the current schema
  `--dry-run`   flag       Show what would change without modifying data

### Examples

``` bash
mx kv migrate projects
```

``` bash
mx kv migrate projects --dry-run
```

``` bash
mx kv migrate projects --prune
```

``` bash
mx kv migrate projects --prune --dry-run
```

## Time-range queries

The `last`, `search`, `count`, and `random` subcommands accept
time-range flags that filter entries by their timestamp before any other
processing. This lets you answer questions like "what did I ship last
Tuesday?" or "how many decisions were recorded in April?" without
scanning the full history.

### Available flags

All time-range flags are mutually exclusive -- you can use one shorthand
(`--day`, `--month`, `--week`, `--since`) or one explicit range
(`--from`/`--to`), but not both.

+-----------------------+-----------------------+-----------------------+
| **Flag**              | **Format**            | **Selects**           |
+=======================+=======================+=======================+
| `--day`               | `YYYY-MM-DD`          | All entries from that |
|                       |                       | calendar day (00:00   |
|                       |                       | to 23:59 UTC)         |
+-----------------------+-----------------------+-----------------------+
| `--month`             | `YYYY-MM`             | All entries from that |
|                       |                       | calendar month (first |
|                       |                       | day to last day, UTC) |
+-----------------------+-----------------------+-----------------------+
| `--week`              | `YYYY-Www`            | All entries from that |
|                       |                       | ISO week (Monday      |
|                       |                       | 00:00 to Sunday 23:59 |
|                       |                       | UTC)                  |
+-----------------------+-----------------------+-----------------------+
| `--since`             | relative or ISO-8601  | All entries from the  |
|                       |                       | given point in time   |
|                       |                       | until now. Relative   |
|                       |                       | formats: `30d`        |
|                       |                       | (days), `1w` (weeks), |
|                       |                       | `2h` (hours), `30m`   |
|                       |                       | (minutes). Also       |
|                       |                       | accepts full ISO-8601 |
|                       |                       | timestamps.           |
+-----------------------+-----------------------+-----------------------+
| `--from`              | `YYYY-MM-DD`          | Start of range,       |
|                       |                       | inclusive (midnight   |
|                       |                       | UTC). Can be used     |
|                       |                       | alone (implies "to    |
|                       |                       | now")                 |
+-----------------------+-----------------------+-----------------------+
| `--to`                | `YYYY-MM-DD`          | End of range,         |
|                       |                       | inclusive (end of day |
|                       |                       | UTC). Can be used     |
|                       |                       | alone (implies "from  |
|                       |                       | the beginning")       |
+-----------------------+-----------------------+-----------------------+

All dates are interpreted as UTC. The `--to` date is inclusive --
entries from any time on that day are included.

### Interaction with `--count`

When both a time range and `--count` are specified, the time range is
applied first, then `--count` limits the result. This applies to both
`last` (which takes the N most recent from the filtered set) and
`random` (which samples N entries from the filtered set).

``` bash
# The 5 most recent entries from April 2026
mx kv last shipped --month 2026-04 --count 5

# 3 random entries from the last 30 days
mx kv random shipped --since 30d --count 3
```

### Examples

``` bash
# Everything shipped on a specific day
mx kv last shipped --day 2026-04-25

# Everything shipped in April
mx kv last shipped --month 2026-04

# Everything shipped in ISO week 17
mx kv last shipped --week 2026-W17

# Everything shipped in the first half of April
mx kv last shipped --from 2026-04-01 --to 2026-04-15

# Everything shipped in the last week
mx kv last shipped --since 1w

# Search within a time window
mx kv search shipped "feature" --month 2026-04

# Count entries on a specific day
mx kv count shipped --day 2026-05-07

# Count entries from the last 30 days
mx kv count shipped --since 30d

# Random entry from the last 2 hours
mx kv random shipped --since 2h
```

### Relationship to `since` subcommand

The `since` subcommand (`mx kv since <key> <timeref>`) is a standalone
command that returns all history entries since a time reference. It only
works on history keys and predates the time-range flag system.

The `--since` flag brings relative time filtering to all
time-range-aware subcommands (`last`, `search`, `count`, `random`) and
works on both history and list types. It accepts the same relative
formats (`30d`, `1w`, `2h`, `30m`) and ISO-8601 timestamps.

Use the `since` subcommand when you want a quick "everything since X"
dump from a history key. Use the `--since` flag when you want to combine
relative time filtering with other operations like counting, searching,
or random sampling, or when you need it on a list key.

::: {.admonition .note}
**NOTE:** Time-range flags (`--day`, `--month`, `--week`, `--since`,
`--from`/`--to`) are available on `last`, `search`, `count`, and
`random`. The `since` subcommand is unchanged and continues to work for
history keys.
:::

## Structured data

History and list entries can carry structured JSON data alongside their
text value. This turns each entry from a plain string into a string with
queryable metadata -- tags, status, priority, or any key-value pairs
relevant to the domain.

### Pushing data

Use `--data` on `push` to attach a JSON object to the entry:

``` bash
mx kv push projects "palmtop DSI fix" \
  --data '{"tags":["palmtop","i915"],"status":"active"}'

mx kv push shipped "v0.1.156" \
  --data '{"pr":305,"scope":"kv"}'
```

The data must be a valid JSON object. Arrays, strings, numbers, and
other non-object JSON types are rejected. If `--data` is omitted, the
entry has no structured data (backward compatible with all existing
entries).

### Output format

Entries display the numeric index, entry ID in brackets, value,
timestamp, and any structured data:

    42 [kv-A3fB]: palmtop DSI fix (2026-05-08T14:30:00Z) {"tags":["palmtop","i915"],"status":"active"}
    43 [kv-B7xQ]: display rotation patch (2026-05-08T15:00:00Z)

Entries without data omit the trailing JSON. This format appears on all
commands that display entries: `get`, `last`, `search`, `since`, `pop`,
`random`, and `dump`.

### Filtering with `--where`

The `--where` flag queries entries by their structured data fields. It
is available on `search`, `last`, `random`, and `count`.

``` bash
# Exact match on a string field
mx kv search projects --where status=active

# Array-contains: matches if the array includes the value
mx kv search projects --where tags=palmtop

# Combine text search with structured data filter
mx kv search projects "DSI" --where status=active

# Multiple --where flags are ANDed
mx kv search projects --where tags=palmtop --where status=active

# Works on last, random, and count too
mx kv last projects --where status=active --count 5
mx kv random projects --where status=active --count 3
mx kv count projects --where status=active
```

### Matching semantics

Each `--where` clause has the form `key=value` (split on the first `=`).
The match is evaluated against the top-level fields of the entry's JSON
data:

String field

:   The field value must equal the clause value exactly.

Array field

:   The array must contain a string element equal to the clause value.

Number field

:   The field's string representation must equal the clause value (e.g.,
    `--where pr=305`).

Boolean field

:   Matches against `true` or `false` as strings.

Missing field

:   Does not match. Entries without data never match any `--where`
    clause.

Only top-level fields are supported. Dot-path traversal (e.g.,
`--where nested.field=value`) is not available.

When multiple `--where` clauses are given, ALL must match (AND logic).
There is no OR operator -- use separate queries if you need union
semantics.

### Combining with other filters

The `--where` flag composes with both text queries and time-range flags.
All filters are applied together:

``` bash
# Text + where + time range: all three must match
mx kv search projects "DSI" --where status=active --since 30d
```

Filter application order: time range first, then `--where`, then text
query. The `--count` limit is applied last.

### Backward compatibility

Structured data is fully backward compatible. Existing data files
written before this feature was added continue to work without
migration. Entries without data are simply treated as having no
structured fields -- they will not match any `--where` clause, but they
are otherwise unaffected.

## Entry IDs

Every history and list entry has a stable entry ID in addition to its
numeric index. Entry IDs are short base58 strings (4--6 characters)
prefixed with `kv-` for visual identification, e.g. `kv-A3fB`.

### Generation

The ID is generated from `blake3(key + timestamp + index)`, with the
first 4 bytes encoded as base58 via base-d. This produces  11 million
unique addresses per key -- sufficient for typical KV usage. The ID is
deterministic: the same key, timestamp, and numeric index always produce
the same ID.

### Push output

`mx kv push` prints the new entry's identifiers on success:

    kv-A3fB (42)

Entry ID first (the primary stable identifier), numeric index in
parentheses. This makes it easy to capture the ID for later use in
scripts or follow-up commands.

### Dual addressing

Anywhere a numeric index is accepted, an entry ID also works:

``` bash
# Get by entry ID
mx kv get shipped --id kv-A3fB

# Get by numeric index (still works)
mx kv get shipped --id 42

# Mix in comma lists
mx kv get shipped --id 42,kv-A3fB,15

# Remove by entry ID
mx kv remove shipped --id kv-A3fB
```

Numeric ranges remain numeric only (`35-64`). Entry IDs cannot be used
in ranges because they are not ordered.

### Prefix matching

ID lookups are prefix-based: `kv-A3f` will match an entry with ID
`A3fBx2` as long as the prefix uniquely identifies one entry. If the
prefix is ambiguous (matches multiple entries), an error is returned:

    Error: ID prefix 'kv-A3' is ambiguous: matches 3 entries, provide more characters

### Backward compatibility

Old data files written before entry IDs existed are back-filled
automatically on first load. The store generates IDs for all entries
that lack one, saves the file, and continues normally. This is a
one-time migration -- no manual action is needed.

## Management

## `mx kv dump`

Dump all KV state. Defaults to JSON output (the full data file, pretty-
printed). Compact format shows one line per key in `key=value` notation,
designed for embedding in wake prompts or status lines.

Compact format examples:

- Counters: `builds=42`

- Strings: `session_goal=ship the docs`

- History: `decisions=[chose Typst\@14:30,fixed bug\@13:15]`

- Lists: `todos=[write tests\@14:30,review PR\@13:15]`

- State: `context={finish KV docs,writing,}`

- Memory links appended: `decisions=[...](kn-abc123)`

### Flags

  **Flag**           **Type**   **Description**
  ------------------ ---------- -----------------------------------------------
  `--format <fmt>`   enum       Output format: `json` (default) or `compact`
  `--memory`         flag       Resolve and display all linked memory entries

### Examples

``` bash
mx kv dump
```

``` bash
mx kv dump --format compact
```

``` bash
mx kv dump --memory
```

## `mx kv reset <key>`

Reset a key to its schema default value. Counters return to their
default (or 0). Strings return to their default (or empty). History and
list keys are cleared to empty. State keys reset all fields to empty
strings.

### Examples

``` bash
mx kv reset builds
```

``` bash
mx kv reset decisions
```

``` bash
mx kv reset context
```

## `mx kv rename <old-key> <new-key>`

Rename a key, preserving all entries and data. The old key is removed
from both the schema (TOML) and data (JSON) files, and all its content
-- type definition, constraints, entries, timestamps, structured data,
memory links -- is moved to the new key name. Entry IDs are stable and
do not change.

The new key name is validated with the same rules as `push --create`:
alphanumeric characters, underscores, and hyphens only, maximum 128
characters, no dots.

Persistence order: the data file is written first (higher-value file),
then the schema file. If the data write fails, in-memory state is rolled
back and no files are modified.

### Examples

``` bash
mx kv rename session_goal current_goal
```

``` bash
mx kv rename old_decisions archived_decisions
```

## Memory linking

History, list, and state keys can be linked to a memory graph entry via
the `--memory` flag. This creates a pointer from the KV key to a
knowledge entry (a `kn-` ID), bridging fast local state with the
persistent knowledge graph.

When a memory link is set, commands that read the key (`get`, `last`,
`since`, `search`, `random`, `dump`) can resolve the link with
`--memory`, which fetches the linked entry from SurrealDB and prints its
title, category, and body.

### Key-level memory links

``` bash
# Link a key to a memory entry
mx kv set decisions --memory kn-abc123

# Clear a memory link (pass empty string)
mx kv set decisions --memory ""
```

Key-level memory links are stored in the JSON data file alongside the
key's entries. They survive resets -- `mx kv reset` clears the data but
preserves the memory pointer.

### Per-entry memory links {#per-entry-memory}

Individual history and list entries can carry their own memory link.
This allows different entries within the same key to reference different
knowledge nodes.

**Set at creation time:**

``` bash
# Link a new entry to a knowledge node on push
mx kv push decisions "adopted per-entry memory links" --memory kn-abc123
```

**Set on an existing entry:**

``` bash
# Link by numeric index
mx kv set decisions --id 17 --memory kn-abc123

# Link by entry ID
mx kv set decisions --id kv-A3fB --memory kn-def456

# Clear a per-entry link
mx kv set decisions --id 17 --memory ""
```

The `--id` flag on `set` requires `--memory` -- it cannot be used alone.
ID matching is prefix-based: `kv-A3f` matches if the prefix uniquely
identifies one entry. If the prefix is ambiguous, an error is returned.
If the entry is not found, exit code 4 (invalid input) is returned.

### Resolution hierarchy

When `--memory` is passed on a read command, memory links are resolved
in priority order:

1.  **Per-entry `memory` field** -- if the entry has its own memory
    link, that link is resolved and displayed. This is the highest
    priority.

2.  **Legacy `kn-` value** -- if the entry's value string starts with
    `kn-`, it is treated as a memory reference and resolved. This
    provides backward compatibility with entries that stored knowledge
    node IDs as their value.

3.  **Key-level memory** -- after all entries are printed, the key-level
    memory pointer (if any) is resolved once at the end.

Per-entry memory wins. An entry with a `memory` field set will use that
link regardless of whether the key itself also has a memory pointer. The
key-level link serves as a fallback that applies to the key as a whole.

### Resolving memory links

``` bash
# Read a key and show its linked memory entry
mx kv get decisions --memory

# Show the last 5 entries plus linked memory
mx kv last decisions --count 5 --memory

# Look up a specific entry and resolve its memory link
mx kv get decisions --id 17 --memory

# Dump everything with all memory links resolved
mx kv dump --memory
```

Resolution connects to the memory store (SurrealDB). If the store is
unavailable or the linked entry has been deleted, a warning is printed
to stderr but the KV data is still shown. KV data is always primary --
memory links are supplementary context.

::: {.admonition .note}
**NOTE:** Memory links are only available on history, list, and state
types. String and counter keys do not support `--memory`.
:::

## JSON output

The `--json` flag outputs results as pretty-printed JSON instead of the
human-readable format. It is available on six commands: `get`, `last`,
`search`, `random`, `since`, and `count`.

JSON output is designed for scripting and piping to tools like `jq`.
When `--json` is active, human formatting is skipped and `--memory`
resolution is not performed -- the raw `kn-` ID is included in each
entry's `memory` field for the caller to resolve if needed.

### Entry format

Commands that return entries (`get`, `last`, `search`, `random`,
`since`) emit a JSON array of entry objects. Each object has this shape:

``` json
{
  "index": 42,
  "id": "A3fB",
  "value": "palmtop DSI fix",
  "ts": "2026-05-08T14:30:00+00:00",
  "data": {"status": "active", "tags": ["palmtop", "i915"]},
  "memory": "kn-e1f646aa"
}
```

`index`

:   Numeric sequence index (integer).

`id`

:   Stable entry ID (base58 string, without the `kv-` prefix).

`value`

:   The entry's text value.

`ts`

:   Timestamp in ISO-8601 format.

`data`

:   Structured data object, omitted when the entry has no data.

`memory`

:   Per-entry memory link (`kn-` ID), omitted when not set.

The `data` and `memory` fields are omitted entirely (not `null`) when
they have no value. This keeps the output clean and avoids forcing
callers to handle nulls.

### Special output shapes

`get --json` without `--id` adapts to the key type:

- **History and list keys:** JSON array of all entries (same format as
  above).

- **Scalar keys** (string, counter, state): `{"value": "..."}` -- the
  formatted value as a string.

`count --json` emits a count object:

``` json
{"count": 12}
```

When filtering is active (value substring, `--where`, or time range),
the object includes additional context:

``` json
{"count": 5, "total": 12, "latest_ts": "2026-05-08T14:30:00+00:00"}
```

`count`

:   Number of matched entries (always present).

`total`

:   Total entries before filtering (present when filtering).

`latest_ts`

:   Timestamp of the most recent matched entry (present when filtering).

### Piping to `jq`

The primary use case for `--json` is piping to `jq` for complex queries
that go beyond what `--where` provides:

``` bash
# Extract all status values
mx kv last projects --json | jq '.[].data.status'

# Filter by a nested condition
mx kv search projects --where status=active --json \
  | jq 'map(select(.data.tags | contains(["rust"])))'

# Get the count as a bare number
mx kv count shipped --json | jq '.count'

# Build a CSV of shipped items
mx kv last shipped --count 100 --json \
  | jq -r '.[] | [.index, .id, .value, .ts] | @csv'
```

::: {.admonition .note}
**NOTE:** `--json` is available on `get`, `last`, `search`, `random`,
`since`, and `count`. It is not available on `push`, `pop`, `set`,
`inc`, `dec`, `remove`, `reset`, `keys`, or `dump` (which already has
`--format json`).
:::

# State

Emotional state tensors for agent co-regulation.

------------------------------------------------------------------------

::: {.admonition .deprecated}
**DEPRECATED:** `mx state` is deprecated and will be removed in a future
release. Use `mx kv` with structured data (`--data`) instead. See the KV
documentation for the replacement workflow.

The state subsystem encodes multidimensional emotional state into
compact tensor strings. A tensor is a vector of float values (each
0.0--1.0) mapped to named dimensions defined by a schema. Schemas are
user-authored YAML files; the default `tensor` schema ships with six
dimensions and self-seeds on first use.

Tensors are designed to be cheap to produce, cheap to parse, and
self-identifying. The wire format embeds the schema ID so a decoder
always knows which schema to load:
:::

    @state:tensor|0.40|0.50|0.50|0.40|0.55|0.30

Each pipe-separated value corresponds to a dimension in the schema's
declared order. Schemas can also define *moods* -- named landmarks in
the state space with canonical tensor values, optional per-dimension
weights, and a tolerance radius. When encoding, the nearest mood (by
weighted Euclidean distance within tolerance) is derived automatically.

## Table of contents

- Encoding

- Decoding

- Listing schemas

- Moods

- Schema info

- Schema file format

## Encoding

## `mx state encode`

Encode dimensional values into a tensor string. Values can be provided
as a positional pipe-separated argument, as named dimension key=value
pairs, or read from a file. If no values are given, the schema's
defaults are used.

The `--guided` flag launches an interactive mode that walks through each
dimension, showing its name, anchors (low/mid/high descriptions), and
default value, then prompts for input.

### Flags

  **Flag**             **Type**   **Description**
  -------------------- ---------- --------------------------------------------------------------------------------------------------------------------------------------------------------
  `<values>`           `string`   Positional: pipe-separated values (e.g., `"0.3|0.2|0.7|0.8|0.4|0.5"`). Conflicts with `--dimensions` and `--file`.
  `-d, --dimensions`   `string`   Named dimension values (e.g., `"entropy=0.4 agency=0.7"`). Dimension names support prefix abbreviation. Conflicts with positional values and `--file`.
  `-f, --file`         `path`     Read values from a file. Accepts pipe-separated or one-value-per-line format. Conflicts with positional values.
  `-s, --schema`       `string`   Schema ID or path. Default: `tensor`.
  `-g, --guided`       `flag`     Interactive guided mode -- walks through each dimension with anchor descriptions.
  `-F, --format`       `string`   Output format: `tensor` (default), `json`, `human`, `bootstrap`.
  `--runes`            `flag`     Include rune prefixes in tensor output (e.g., decorative Unicode characters per dimension).

### Examples

``` bash
# Pipe-separated positional values (six dimensions for default tensor schema)
mx state encode "0.4|0.6|0.5|0.3|0.7|0.2"
```

``` bash
# Named dimensions with prefix abbreviation
mx state encode -d "entropy=0.4 agency=0.6 temp=0.5 verb=0.3 skep=0.7 humor=0.2"
```

``` bash
# Default values from schema
mx state encode
```

``` bash
# Human-readable output with nearest mood
mx state encode "0.4|0.6|0.5|0.3|0.7|0.2" -F human
```

``` bash
# Bootstrap format (self-documenting, with rune legend)
mx state encode "0.4|0.6|0.5|0.3|0.7|0.2" -F bootstrap
```

``` bash
# With rune decoration
mx state encode "0.4|0.6|0.5|0.3|0.7|0.2" --runes
```

``` bash
# Read from file
mx state encode -f state-values.txt
```

``` bash
# Interactive guided mode
mx state encode --guided
```

``` bash
# Use a custom schema
mx state encode -s crewu "0.3|0.2|0.7|0.8|0.4"
```

### Output formats

`tensor`

:   The default. Prints the encoded tensor string:
    `@state:tensor|0.40|0.60|...`. With `--runes`, each value is
    prefixed by its dimension's rune character.

`json`

:   Structured JSON with `schema_id` and `values` fields.

`human`

:   Each dimension printed as `Name: value (anchor description)`,
    followed by the nearest mood if one falls within tolerance.

`bootstrap`

:   Self-documenting multiline output designed for session bootstrap.
    Line 1 is the rune-encoded tensor, line 2 is a rune legend mapping
    runes to dimension IDs, then a blank line, then interpolated anchor
    descriptions with values.

### Named dimensions

The `-d` / `--dimensions` flag accepts space-separated `key=value`
pairs. Keys are matched against dimension IDs case-insensitively, with
prefix abbreviation: `temp=0.5` matches `temperature`, `ent=0.4` matches
`entropy`. Every dimension in the schema must be covered -- missing
dimensions produce an error listing the expected set.

### Value clamping

All values are clamped to the 0.0--1.0 range. Out-of-bounds values are
silently clamped, never rejected.

## Decoding

## `mx state decode`

Decode a tensor string back to human-readable values. The schema ID is
embedded in the tensor string (`@state:schema_id|...`) and used to load
the matching schema automatically. If `--schema` is provided, it
overrides the embedded ID.

Input can be provided as a positional argument or piped via stdin.

### Flags

  **Flag**         **Type**   **Description**
  ---------------- ---------- --------------------------------------------------------------------------------------------------------
  `<input>`        `string`   Positional: encoded tensor string (e.g., `"@state:tensor|0.3|0.2|..."`). If omitted, reads from stdin.
  `-s, --schema`   `string`   Schema ID or path. Overrides the schema ID embedded in the tensor string.
  `-F, --format`   `string`   Output format: `human` (default), `json`, `tensor`, `mood`.

### Examples

``` bash
mx state decode "@state:tensor|0.40|0.60|0.50|0.30|0.70|0.20"
```

``` bash
# Pipe from another command
echo "@state:tensor|0.40|0.60|0.50|0.30|0.70|0.20" | mx state decode
```

``` bash
# JSON output
mx state decode "@state:tensor|0.40|0.60|0.50|0.30|0.70|0.20" -F json
```

``` bash
# Show only the nearest mood
mx state decode "@state:tensor|0.40|0.60|0.50|0.30|0.70|0.20" -F mood
```

``` bash
# Re-encode (roundtrip)
mx state decode "@state:tensor|0.40|0.60|0.50|0.30|0.70|0.20" -F tensor
```

### Output formats

`human`

:   The default. Prints each dimension as
    `Name: value (anchor description)`, followed by the nearest mood if
    one falls within tolerance.

`json`

:   Structured JSON with `schema_id` and `values` fields.

`tensor`

:   Re-encodes the tensor. Useful for normalizing or roundtripping.

`mood`

:   Prints only the nearest mood name, its description, and distance. If
    no mood is within tolerance, prints `(unnamed region)`.

### Rune stripping

Tensor strings may contain rune prefixes on values (e.g.,
`@state:tensor|ᚣ0.30|ᚤ0.20|...`). The decoder strips any non-digit,
non-dot, non-minus prefix characters before parsing, so rune-encoded and
plain tensors decode identically.

## Listing schemas {#schemas}

## `mx state schemas`

List all available schemas. Scans `$MX_HOME/state/schemas/` for files
with `.yaml`, `.yml`, or `.json` extensions. Each schema is loaded to
display its name, dimension count, and mood count.

### Flags

  **Flag**   **Type**   **Description**
  ---------- ---------- ---------------------------------------------------------------------------
  `--json`   `flag`     Output as JSON array with `id`, `name`, `dimensions`, and `moods` fields.

### Examples

``` bash
mx state schemas
```

``` bash
mx state schemas --json
```

::: {.admonition .note}
**NOTE:** On first invocation of any `mx state` command, the default
`tensor` schema is self-seeded into `$MX_HOME/state/schemas/tensor.yaml`
if no file exists at that path. User-authored files are never
overwritten.
:::

## Moods

## `mx state moods`

List moods defined in a schema, or show details for a specific mood.

Without a mood argument, lists all moods with their canonical tensor
values and descriptions. With a mood name, shows the full definition:
description, tolerance, and per-dimension values with weights.

### Flags

  **Flag**         **Type**   **Description**
  ---------------- ---------- --------------------------------------------
  `<mood>`         `string`   Optional positional: mood name to inspect.
  `-s, --schema`   `string`   Schema ID or path. Default: `tensor`.
  `--json`         `flag`     Output as JSON.

### Examples

``` bash
# List all moods for the default schema
mx state moods
```

``` bash
# List moods for a specific schema
mx state moods -s crewu
```

``` bash
# Show details for a specific mood
mx state moods calm
```

``` bash
# JSON output
mx state moods --json
```

### Mood matching

When encoding or decoding, the nearest mood is found by weighted
Euclidean distance. Each mood defines a canonical tensor (the center
point), optional per-dimension weights (default 1.0), and a tolerance
radius (default 0.30). A mood matches when the distance is within
tolerance. If multiple moods match, the closest one wins.

The distance formula:

$$d = \sqrt{\sum_{i = 0}^{n - 1}w_{i}\left( v_{i} - c_{i} \right)^{2}}$$

where $v_{i}$ is the tensor value, $c_{i}$ is the mood's canonical
value, and $w_{i}$ is the per-dimension weight.

## Schema info {#info}

## `mx state info`

Show full details for a schema: name, version, all dimensions with their
anchors and defaults, and all moods with descriptions and tolerances.

### Flags

  **Flag**         **Type**   **Description**
  ---------------- ---------- -------------------------------------------------
  `-s, --schema`   `string`   Schema ID or path. Default: `tensor`.
  `--json`         `flag`     Output as JSON (the full parsed schema object).

### Examples

``` bash
mx state info
```

``` bash
mx state info -s crewu
```

``` bash
mx state info --json
```

## Schema file format {#schema-format}

Schemas are YAML files stored in `$MX_HOME/state/schemas/`. The file
stem is the schema ID (e.g., `tensor.yaml` has ID `tensor`). JSON is
also accepted as a fallback format.

### Top-level fields

`id`

:   Required. Schema identifier. Must match the file stem.

`name`

:   Required. Human-readable name.

`version`

:   Optional. Integer version number. Default: `1`.

`dimensions`

:   Required. Ordered list of dimension definitions.

`moods`

:   Optional. Map of mood name to mood definition. Default: empty.

### Dimension definition

Each dimension in the `dimensions` list has these fields:

`id`

:   Required. Unique identifier (e.g., `entropy`, `temperature`).

`name`

:   Required. Human-readable display name.

`rune`

:   Optional. Decorative Unicode character used when `--runes` is
    enabled.

`default`

:   Optional. Default value (0.0--1.0). Default: `0.5`.

`anchors`

:   Required. Object with anchor descriptions:

    - `low`: Required. Description for values near 0.0.

    - `mid`: Optional. Description for values near 0.5.

    - `high`: Required. Description for values near 1.0.

### Mood definition

Each entry in the `moods` map has these fields:

`description`

:   Required. Human-readable description of the mood.

`tensor`

:   Required. List of canonical float values, one per dimension, in the
    dimension order declared by the schema.

`weights`

:   Optional. List of per-dimension weights for distance calculation.
    Default: `1.0` for all dimensions.

`tolerance`

:   Optional. Maximum weighted Euclidean distance for a tensor to be
    considered "in" this mood. Default: `0.30`.

### Example schema

``` yaml
id: example
name: Example Schema
version: 1

dimensions:
  - id: entropy
    name: Entropy
    rune: "ᙣ"
    anchors:
      low: ordered / focused / coherent
      mid: structured but breathing
      high: chaotic / associative / wild
    default: 0.4

  - id: agency
    name: Agency
    anchors:
      low: receptive / yielding
      mid: collaborative
      high: active / driving / proactive
    default: 0.5

moods:
  calm:
    description: Settled, receptive, low entropy
    tensor: [0.2, 0.3]
    weights: [1.0, 0.8]
    tolerance: 0.3

  driven:
    description: High agency, moderate entropy
    tensor: [0.5, 0.9]
    tolerance: 0.25
```

### Default tensor schema

The built-in `tensor` schema ships with six dimensions in this order:

1.  **Entropy** -- ordered/focused (0.0) to chaotic/wild (1.0). Default:
    0.4.

2.  **Agency** -- receptive/yielding (0.0) to active/driving (1.0).
    Default: 0.5.

3.  **Temperature** -- cold/precise (0.0) to warm/casual (1.0). Default:
    0.5.

4.  **Verbosity** -- terse/minimal (0.0) to expansive/elaborate (1.0).
    Default: 0.4.

5.  **Skepticism** -- agreeable/affirming (0.0) to challenging/pushback
    (1.0). Default: 0.55.

6.  **Humor** -- serious/matter-of-fact (0.0) to playful/quippy (1.0).
    Default: 0.3.

The default schema has no moods defined. Add moods to your local copy at
`$MX_HOME/state/schemas/tensor.yaml` to enable mood matching.

### Schema resolution

The `--schema` flag on all commands accepts either a schema ID or a
direct file path. The heuristic: if the argument contains a slash or
ends with `.yaml`, `.yml`, or `.json`, it is treated as a path and
loaded directly. Otherwise it is treated as an ID and resolved from
`$MX_HOME/state/schemas/` with an extension fallback chain of `.yaml`,
`.yml`, `.json`.

::: {.admonition .tip}
**TIP:** To reference a schema file in the current directory by relative
path, use `./my-schema.yaml` rather than `my-schema.yaml` -- the latter
would be treated as an ID lookup.
:::

# Sync

GitHub sync for issues and discussions.

------------------------------------------------------------------------

## Overview

`mx sync` provides bidirectional synchronization between GitHub and
local YAML files. Issues and discussions are pulled from GitHub into a
local cache as YAML, edited locally, and pushed back.

The sync subsystem uses two API layers internally: the GitHub REST API
for issues and the GitHub GraphQL API for discussions. Authentication is
handled automatically through a token stored in `~/.claude.json`.

All YAML files live in a sync cache directory at
`$MX_HOME/cache/sync/<owner>-<repo>/` by default. Each file represents a
single issue or discussion.

## Subcommands

`mx sync` has three subcommands:

- `pull` -- download issues and discussions from GitHub to local YAML

- `push` -- upload local YAML changes back to GitHub

- `issues` -- run a full bidirectional sync (pull then push)

Every subcommand accepts a `--dry-run` flag that previews what would
happen without making any changes.

## Pull

## `mx sync pull <repo>`

Download open issues and discussions from a GitHub repository into local
YAML files. Issues are fetched via the REST API; discussions via
GraphQL. Each item becomes a separate YAML file in the output directory.

### Flags

  **Flag**           **Type**     **Description**
  ------------------ ------------ ----------------------------------------------------------------------
  `repo`             positional   Repository in `owner/repo` format.
  `-o`, `--output`   path         Output directory. Defaults to `$MX_HOME/cache/sync/<owner>-<repo>/`.
  `--dry-run`        flag         Preview what would be pulled without writing files.

### Examples

``` bash
mx sync pull coryzibell/mx
```

``` bash
mx sync pull coryzibell/mx --output ./local-issues
```

``` bash
mx sync pull coryzibell/mx --dry-run
```

### What pull does

1.  Fetches all open issues via the REST API, including comments.

2.  Fetches all discussions via the GraphQL API, including comments.

3.  For each item, checks whether a local YAML file already exists
    (matched by issue number or discussion ID).

4.  **New items** get a fresh YAML file. The filename is derived from
    the number and a slugified title: `42-fix-crash-on-empty-input.yaml`
    for issues, `d7-feature-request-dark-mode.yaml` for discussions.

5.  **Existing items** are updated with the latest remote state -- but
    only if the local copy has not been modified since the last sync. If
    local changes are detected, the item is skipped to avoid overwriting
    your edits.

### Local change detection

Pull uses a `last_synced` snapshot stored in each YAML file's metadata
to detect local modifications. When a file is synced, the snapshot
records the title, body, labels, and timestamp at that moment. On the
next pull, the current local values are compared against the snapshot:

- If they match, the file is safe to overwrite with the remote state.

- If they differ, pull skips the file and prints a message indicating
  local changes were preserved.

This is a safety mechanism. If you have edited a YAML file locally and
want to pull the remote state anyway, you must push your changes first
(or discard them by deleting the file and re-pulling).

### Pull output

Pull prints a summary showing counts of created, updated, and unchanged
items for both issues and discussions:

    Issues: 3 created, 5 updated, 2 unchanged
    Discussions: 1 created, 0 updated, 4 unchanged

## Push

## `mx sync push <repo>`

Upload local YAML changes to GitHub. Creates new issues or discussions
for items without a GitHub ID, and updates existing ones using three-way
merge to handle concurrent remote edits.

### Flags

  **Flag**          **Type**     **Description**
  ----------------- ------------ -------------------------------------------------------------------------------------------
  `repo`            positional   Repository in `owner/repo` format.
  `-i`, `--input`   path         Input directory containing YAML files. Defaults to `$MX_HOME/cache/sync/<owner>-<repo>/`.
  `--dry-run`       flag         Preview what would be pushed without modifying GitHub.

### Examples

``` bash
mx sync push coryzibell/mx
```

``` bash
mx sync push coryzibell/mx --input ./local-issues
```

``` bash
mx sync push coryzibell/mx --dry-run
```

### Item types

Push routes items based on their `type` field:

- **issue** (default) -- created and updated via the REST API. Supports
  title, body, labels, and assignees.

- **idea** or **discussion** -- created and updated via the GraphQL API.
  Supports title, body, labels, and discussion category.

### Creating new items

A YAML file without a `github_issue_number` or `github_discussion_id` is
treated as a new item. Push creates it on GitHub and then updates the
local file with the assigned number/ID, timestamp, and a `last_synced`
snapshot. The file is also renamed to include the newly assigned number.

For new discussions, push looks up the repository's discussion
categories by slug. If the category specified in the YAML does not exist
on the repository, the item is skipped.

### Updating existing items

For items that already have a GitHub ID, push uses a three-way merge to
reconcile local edits, remote edits, and the `last_synced` base:

1.  The local state is read from the YAML file.

2.  The current remote state is fetched from GitHub.

3.  The `last_synced` snapshot provides the common base.

4.  Each field (title, body, labels, assignees) is compared across all
    three states to determine what changed and where.

The merge engine handles five cases per field:

- **Unchanged** -- all three match. Nothing to do.

- **Local only** -- local differs from base, remote matches base. Local
  wins.

- **Remote only** -- remote differs from base, local matches base.
  Remote wins.

- **Both same** -- both changed to the same value. Either wins (they
  agree).

- **Conflict** -- both changed to different values. Resolved
  automatically by preferring the local value.

### Label merge semantics

Labels use union merge rather than the field-level conflict model. The
formula is:

    merged = base + local_additions + remote_additions - local_deletions - remote_deletions

This means labels added on either side are preserved, and labels deleted
on either side are removed. There are no label conflicts -- both sides"
intentions are honored. Assignees follow the same union merge logic.

### Push output

Push prints a summary matching the pull format:

    Issues: 1 created, 3 updated, 8 unchanged
    Discussions: 0 created, 1 updated, 2 unchanged

## Issues

## `mx sync issues <repo>`

Run a full bidirectional sync: pull from GitHub, then push local changes
back. This is a convenience wrapper that calls `pull` followed by `push`
with default directories.

### Flags

  **Flag**      **Type**     **Description**
  ------------- ------------ --------------------------------------------------------
  `repo`        positional   Repository in `owner/repo` format.
  `--dry-run`   flag         Preview both pull and push without making any changes.

### Examples

``` bash
mx sync issues coryzibell/mx
```

``` bash
mx sync issues coryzibell/mx --dry-run
```

The output separates the two phases with headers:

    === Bidirectional Issue Sync ===

    --- Pull (GitHub -> Local) ---
    ...pull output...

    --- Push (Local -> GitHub) ---
    ...push output...

    === Sync Complete ===

## YAML file format

Each synced item is stored as a YAML file with this structure:

``` yaml
metadata:
  title: "Issue title"
  type: issue        # or "idea" for discussions
  labels:
    - bug
    - enhancement
  assignees:
    - username
  state: open
  category: ideas    # discussions only
  github_issue_number: 42
  github_updated_at: "2025-01-15T10:30:00Z"
  last_synced:
    title: "Issue title"
    body: "Body at last sync"
    labels:
      - bug
      - enhancement
    updated_at: "2025-01-15T10:30:00Z"
    assignees:
      - username
body_markdown: |
  The full issue body in markdown.
comments:
  - id: "123456"
    author: username
    created_at: "2025-01-15T10:30:00Z"
    body: "Comment text"
```

Fields can also be placed at the root level (`title`, `body`, `type`,
`labels`, `assignees`, `category`) for convenience when authoring new
items by hand. Root-level fields take precedence over their `metadata.*`
counterparts during push.

::: {.admonition .tip}
**TIP:** To create a new issue from scratch, write a minimal YAML file
with just `title`, `body`, and optionally `labels`, then run
`mx sync push`. The file will be updated with the GitHub issue number
and renamed automatically.
:::

## Authentication

Sync reads the GitHub token from `~/.claude.json`, looking for
`projects.<project>.mcpServers.github.env.GITHUB_PERSONAL_ACCESS_TOKEN`
across all configured projects.

::: {.admonition .note}
**NOTE:** The token needs `repo` scope for issues, and `read:discussion`
plus `write:discussion` for discussions.
:::

## Related commands

- `mx convert md2yaml` -- convert markdown files to the YAML format used
  by sync. Useful for bulk-importing issues from markdown notes.

# PR

Pull request merge with encoded commits.

------------------------------------------------------------------------

## Overview

`mx pr merge` merges a GitHub pull request through the `gh` CLI and
encodes the resulting commit message with base-d, keeping the merge
commit consistent with the encoding applied by `mx commit`. Without this
command, PR merges would produce plain-text commit messages that break
the encoded history visible through `mx log`.

The command fetches the PR diff and metadata from GitHub, encodes the
title and body, and passes the encoded values to `gh pr merge`. After a
successful merge it performs automatic post-merge cleanup unless told
otherwise.

## Basic usage

``` bash
mx pr merge 42
```

This squash-merges PR #42 with an encoded commit message, then switches
your local checkout to the target branch and deletes the local source
branch.

## Merge strategies

Three merge strategies are available. They are mutually exclusive -- at
most one flag may be passed.

## `mx pr merge <number>`

Squash merge (default). All commits on the PR are collapsed into a
single encoded commit on the target branch. This is the most common
workflow and keeps the target branch history linear.

### Flags

  **Flag**           **Type**   **Description**
  ------------------ ---------- ----------------------------------------------------------------------------------------------------------------------------------------------
  `--rebase`         flag       Use rebase merge instead of squash. Replays the PR's commits onto the target branch individually. The final commit message is still encoded.
  `--merge-commit`   flag       Use a standard merge commit instead of squash. Preserves the full branch topology in the target branch history.

### Examples

``` bash
mx pr merge 42
```

``` bash
mx pr merge 42 --rebase
```

``` bash
mx pr merge 42 --merge-commit
```

When deciding which strategy to use:

- **Squash** (default) is best for feature branches where individual
  commits are implementation noise. One clean encoded commit on `main`.

- **Rebase** preserves each commit as a separate entry but linearizes
  the history. Useful when each commit is meaningful on its own.

- **Merge commit** preserves full branch topology. Useful for long-lived
  branches where the merge point itself is significant.

## Post-merge cleanup

After a successful merge, `mx pr merge` performs automatic cleanup to
keep your local repository in sync with the remote. This prevents the
common footgun where you are left on a dead branch whose remote ref was
deleted by GitHub, causing the next `git pull --rebase` to fail.

The cleanup sequence:

1.  `git fetch origin --prune` -- sync remote state and remove stale
    tracking refs.

2.  `git checkout <target-branch>` -- switch to the branch the PR was
    merged into (usually `main`).

3.  `git pull --ff-only` -- fast-forward the target branch to include
    the merge commit.

4.  `git branch -d <source-branch>` -- delete the local source branch
    using a safe delete. The `-d` flag (not `-D`) refuses to delete the
    branch if it contains commits that have not been merged, preventing
    accidental data loss.

Each cleanup step is best-effort. The merge has already succeeded on
GitHub at this point, so a cleanup failure emits a warning but does not
cause the command to exit non-zero.

### Safety guards

Cleanup is skipped entirely when any of these conditions are detected:

- **Uncommitted changes.** If your working tree has staged or unstaged
  modifications to tracked files, cleanup is skipped with a warning to
  stash or commit first. Untracked files do not block cleanup.

- **Unpushed commits.** If the local source branch has commits that are
  not on `origin/<source-branch>`, cleanup is skipped to avoid deleting
  a branch with unreplicated work.

- **Missing branch metadata.** If the PR metadata does not include
  source or target branch names, cleanup is skipped because the command
  cannot determine where to switch.

- **Same source and target.** If the source and target branches are the
  same (unusual but possible), cleanup is a no-op.

- **Source branch does not exist locally.** If the source branch has no
  local ref (e.g., you merged someone else's PR), the unpushed-commits
  check and branch deletion are skipped, but fetch, checkout, and pull
  still run.

## Opting out

``` bash
mx pr merge 42 --no-cleanup
```

The `--no-cleanup` flag skips the entire post-merge cleanup sequence.
The PR is merged on GitHub but your local checkout stays on whatever
branch you were on, and no local branches are deleted. Useful when you
want to continue working on the source branch or handle cleanup
manually.

## Encoding

The merge commit is encoded the same way as a regular `mx commit`:

1.  The **PR diff** (fetched via `gh pr diff`) is hashed with a randomly
    selected dictionary to produce the encoded commit title.

2.  The **PR title and body** (fetched via `gh pr view`) are
    concatenated, compressed, and encoded with a second randomly
    selected dictionary to produce the commit body.

3.  A **footer** tag in the format
    `[hash_algo:title_dict|compress_algo:body_dict]` is appended so
    `mx log` can decode the message later.

The encoded title and body are passed to `gh pr merge` via the
`--subject` and `--body` flags, so the merge commit on GitHub contains
the full encoded message. Use `mx log` to read the decoded history.

::: {.admonition .note}
**NOTE:** The encoding uses the same `base-d` pipeline as `mx commit`.
See base-d for details on dictionaries, hash algorithms, and compression
codecs.
:::

# GitHub

GitHub operations: cleanup and commenting.

------------------------------------------------------------------------

## Overview

`mx github` groups operations that interact with GitHub repositories
beyond the commit-and-merge workflow covered by commit and PR. Currently
this means two things: bulk cleanup of issues and discussions, and
posting comments to either.

## Cleanup

## `mx github cleanup <repo>`

Close issues and delete discussions in a GitHub repository. Useful for
sweeping stale tracking items after a batch of work lands. Both flags
are optional, but at least one must be provided -- the command does
nothing if neither `--issues` nor `--discussions` is set.

### Flags

  **Flag**          **Type**     **Description**
  ----------------- ------------ -----------------------------------------------------
  `repo`            positional   Repository in `owner/repo` format.
  `--issues`        string       Comma-separated issue numbers to close.
  `--discussions`   string       Comma-separated discussion numbers to delete.
  `--dry-run`       flag         Show what would be done without making any changes.

### Examples

``` bash
mx github cleanup coryzibell/mx --issues 10,11,12
```

``` bash
mx github cleanup coryzibell/mx --discussions 5,8
```

``` bash
mx github cleanup coryzibell/mx --issues 10 --discussions 5 --dry-run
```

::: {.admonition .tip}
**TIP:** Run with `--dry-run` first to verify the target list before
closing or deleting anything. Deleted discussions cannot be recovered.
:::

## Commenting

`mx github comment` posts a comment to an issue or discussion. Both
subcommands accept an optional `--identity` flag that appends a
signature line to the comment, useful when multiple agents or personas
share a GitHub account.

### Issues

## `mx github comment issue <repo> <number> <message>`

Post a comment on a GitHub issue.

### Flags

  **Flag**       **Type**     **Description**
  -------------- ------------ -----------------------------------------------------------------------
  `repo`         positional   Repository in `owner/repo` format.
  `number`       positional   Issue number.
  `message`      positional   Comment body text.
  `--identity`   string       Identity signature appended to the comment (e.g. `"smith"`, `"neo"`).

### Examples

``` bash
mx github comment issue coryzibell/mx 42 "Fixed in abc123."
```

``` bash
mx github comment issue coryzibell/mx 42 "Resolved." --identity smith
```

### Discussions

## `mx github comment discussion <repo> <number> <message>`

Post a comment on a GitHub discussion.

### Flags

  **Flag**       **Type**     **Description**
  -------------- ------------ -----------------------------------------------------------------------
  `repo`         positional   Repository in `owner/repo` format.
  `number`       positional   Discussion number.
  `message`      positional   Comment body text.
  `--identity`   string       Identity signature appended to the comment (e.g. `"smith"`, `"neo"`).

### Examples

``` bash
mx github comment discussion coryzibell/mx 7 "Sounds good, let's proceed."
```

``` bash
mx github comment discussion coryzibell/mx 7 "Acknowledged." --identity neo
```

# Convert

Format conversion utilities.

------------------------------------------------------------------------

## Overview

`mx convert` provides bidirectional conversion between markdown and
YAML. These commands are the format bridge for the sync workflow:
markdown is the human-friendly authoring format, YAML is the machine
format used by `mx sync` to round-trip issues and discussions with
GitHub.

Two subcommands, one for each direction:

- `md2yaml` -- markdown to YAML (for feeding into `mx sync push`)

- `yaml2md` -- YAML to markdown (for reading sync output as prose)

Both commands accept a single file or an entire directory. When given a
directory, every file with the matching extension (`.md` for md2yaml,
`.yaml` or `.yml` for yaml2md) is converted.

## md2yaml

## `mx convert md2yaml <input>`

Convert markdown files to the YAML format used by `mx sync`. The input
can be a single `.md` file or a directory of markdown files. Output YAML
files use the same base filename with a `.yaml` extension.

### Flags

  **Flag**           **Type**     **Description**
  ------------------ ------------ -------------------------------------------------------------------------------------------------------------------------
  `input`            positional   Path to a markdown file or directory of markdown files.
  `-o`, `--output`   path         Output directory for generated YAML files. Defaults to the current working directory.
  `--dry-run`        flag         Preview what would be created without writing any files. Prints the output path, title, type, and labels for each file.

### Examples

``` bash
mx convert md2yaml notes/backlog.md
```

``` bash
mx convert md2yaml notes/ --output ./yaml-issues
```

``` bash
mx convert md2yaml notes/backlog.md --dry-run
```

### Markdown input formats

md2yaml understands two styles of markdown input.

**Frontmatter style** uses a YAML frontmatter block delimited by `---`.
This is the preferred format for clean round-trips:

``` markdown
---
title: "Add dark mode support"
type: issue
labels:
  - enhancement
  - ui
priority: P2
---

## Context

Users have requested a dark mode option...
```

Supported frontmatter fields: `title`, `type` (defaults to `issue`),
`labels` (list), and `priority` (converted to a `priority:<value>` label
automatically).

**Inline style** uses a heading and bold metadata lines. This is
convenient for quick authoring:

``` markdown
# Add dark mode support

**Type:** `issue`
**Labels:** `enhancement`, `ui`

## Context

Users have requested a dark mode option...
```

In both formats, everything after the metadata (frontmatter or inline
fields) becomes the `body_markdown` field in the output YAML.

### Output format

The generated YAML matches the schema used by `mx sync`. The output can
be pushed directly to GitHub with `mx sync push`:

``` bash
mx convert md2yaml notes/dark-mode.md --output ./sync-cache
mx sync push coryzibell/mx --input ./sync-cache
```

See Sync for the full YAML file format specification.

## yaml2md

## `mx convert yaml2md <input>`

Convert YAML files (from `mx sync pull` or hand-authored) back to
readable markdown. The input can be a single `.yaml`/`.yml` file or a
directory. Output filenames are derived from the issue number and title
slug (e.g., `42-fix-crash-on-empty-input.md`) when a GitHub issue number
is present, or from the original filename otherwise.

### Flags

  **Flag**           **Type**     **Description**
  ------------------ ------------ ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------
  `input`            positional   Path to a YAML file or directory of YAML files.
  `-o`, `--output`   path         Output directory for generated markdown files. Defaults to the current working directory.
  `-r`, `--repo`     string       Repository in `owner/repo` format. Used for GitHub URL references in the output. If omitted, the repo is inferred from the parent directory name (e.g., a directory named `coryzibell-mx` becomes `coryzibell/mx`).
  `--dry-run`        flag         Preview what would be created without writing any files. Prints the output path, title, and issue/discussion number for each file.

### Examples

``` bash
mx convert yaml2md cache/sync/coryzibell-mx/42-dark-mode.yaml
```

``` bash
mx convert yaml2md cache/sync/coryzibell-mx/ --output ./readable
```

``` bash
mx convert yaml2md issue.yaml --repo coryzibell/mx
```

``` bash
mx convert yaml2md cache/sync/coryzibell-mx/ --dry-run
```

### Output structure

The generated markdown uses YAML frontmatter for metadata, followed by
the issue body and any comments:

``` markdown
---
title: "Add dark mode support"
type: issue
labels:
  - enhancement
  - ui
state: open
github_issue: 42
github_repo: coryzibell/mx
updated_at: 2025-01-15T10:30:00Z
---

The full issue body in markdown...

---

## Comments

### username (Jan 15, 2025)
Comment text here...
```

The frontmatter preserves enough metadata for a clean round-trip back
through `md2yaml` if needed.

### Repo inference

When `--repo` is not provided, yaml2md infers the repository from the
parent directory name by splitting on the first hyphen. A file at
`cache/sync/coryzibell-mx/42-dark-mode.yaml` infers `coryzibell/mx`. If
the directory name has no hyphen, the repo defaults to
`unknown/<dirname>`.

For reliable results, pass `--repo` explicitly.

## Typical workflows

### Bulk-importing issues from markdown notes

Convert a directory of markdown notes to YAML and push them as new
GitHub issues:

``` bash
mx convert md2yaml notes/ --output ./import-batch
mx sync push coryzibell/mx --input ./import-batch
```

Each markdown file becomes a new issue. After push, the YAML files are
updated with assigned issue numbers.

### Reading sync output as prose

Pull issues from GitHub, then convert to readable markdown for review:

``` bash
mx sync pull coryzibell/mx
mx convert yaml2md ~/.wonka/cache/sync/coryzibell-mx/ --output ./issues-readable
```

### Dry-run preview

Both commands support `--dry-run` to preview the conversion without
writing files:

``` bash
mx convert md2yaml notes/ --dry-run
mx convert yaml2md cache/ --dry-run
```

Dry-run output shows the file path that would be created, along with key
metadata (title, type, labels, issue number) for each item.

## Related commands

- `mx sync` -- the sync workflow that consumes and produces the YAML
  format these commands convert to and from.

# Session

Deprecated session export.

------------------------------------------------------------------------

::: {.admonition .deprecated}
**DEPRECATED:** `mx session` is deprecated. Use `mx codex export`
instead.
:::

## What it was

`mx session export` exported the most recent Claude session as markdown.
It walked `~/.claude/projects/`, found the newest non-agent session
JSONL by mtime, and rendered it to stdout or a file.

This functionality now lives in `mx codex export`, which reads from the
codex archive, supports filtering by `--session`, `--project`, and
`--date`, offers multiple output formats, and inlines sub-agent
transcripts by default.

## Current behavior

The command still works. Running it will:

1.  Print a deprecation notice to stderr.

2.  Run `mx codex archive --all` to ensure live sessions are ingested.

3.  Forward to `mx codex export` with markdown output and default-clean
    includes.

The old flags are accepted:

## `mx session export [path] [-o output]`

Export a session as markdown. Thin alias for `mx codex export`.

### Flags

  **Flag**           **Type**     **Description**
  ------------------ ------------ ---------------------------------------------------------------------------------
  `path`             positional   Path to a session JSONL file, or a bare UUID. Omit for the most recent session.
  `-o`, `--output`   path         Output file. Defaults to stdout.

## Replacement

See Codex for the full replacement command surface.

# Wiki

GitHub wiki page sync.

------------------------------------------------------------------------

## Overview

`mx wiki sync` pushes local markdown files to a GitHub repository's
wiki. It clones the wiki repo into a temporary directory, copies your
files in with sanitized page names, commits, and pushes -- all in one
step.

This is a one-way sync: local files are copied to the wiki. Changes made
directly on the wiki through the GitHub UI are overwritten on the next
sync.

## Sync

## `mx wiki sync <repo> <source>`

Sync a local markdown file or directory to a GitHub wiki. The source can
be a single `.md` file or a directory containing markdown files. Page
names are derived from filenames: lowercased, spaces replaced with
hyphens, non-alphanumeric characters stripped.

### Flags

  **Flag**        **Type**     **Description**
  --------------- ------------ --------------------------------------------------------------------------------------------------------------------------------------------------------------------------
  `repo`          positional   Repository in `owner/repo` format.
  `source`        positional   Path to a markdown file or directory of markdown files.
  `--page-name`   string       Custom page name for the wiki page. Only valid when syncing a single file. Ignored characters are stripped and the name is sanitized the same way as auto-derived names.
  `--dry-run`     flag         Preview what would be synced without cloning, committing, or pushing.

### Examples

``` bash
mx wiki sync coryzibell/mx docs/wiki/architecture.md
```

``` bash
mx wiki sync coryzibell/mx docs/wiki/architecture.md --page-name "API Reference"
```

``` bash
mx wiki sync coryzibell/mx docs/wiki/
```

``` bash
mx wiki sync coryzibell/mx docs/wiki/ --dry-run
```

### What sync does

1.  Clones the wiki repository (`<repo>.wiki.git`) into a temporary
    directory using your GitHub token for authentication.

2.  Copies each source file into the cloned wiki with a sanitized
    filename. If `--page-name` is provided, that name is used instead of
    the source filename.

3.  Commits the changes with the message `"Sync from mx CLI"`.

4.  Pushes to the wiki's `master` branch.

5.  Prints the wiki URL and a list of synced pages.

The temporary clone is discarded after the push completes.

### Page name sanitization

Filenames and custom page names go through the same sanitization
pipeline:

1.  Lowercased.

2.  Spaces replaced with hyphens.

3.  Non-alphanumeric characters (except hyphens) removed.

4.  A `.md` extension is appended if not already present.

For example, `"API Reference (v2)"` becomes `api-reference-v2.md`.

### Directory sync

When the source is a directory, every `.md` file in it is synced. Two
exceptions apply:

- Non-markdown files are silently skipped.

- Files whose names start with a number followed by a hyphen (e.g.,
  `42-fix-crash.md`) are skipped. These are assumed to be issue files
  from `mx sync` and are not intended for the wiki.

Skipped files are printed in the output for visibility.

::: {.admonition .note}
**NOTE:** `--page-name` cannot be used with a directory source. Each
file's wiki page name is derived from its filename automatically.
:::

### Dry run

With `--dry-run`, the command prints which files would be synced and
their target page names, but does not clone, commit, or push anything.
Useful for verifying file selection and page naming before writing to
the wiki.

``` bash
mx wiki sync coryzibell/mx docs/wiki/ --dry-run
```

## Authentication

Wiki sync reads the GitHub token from `~/.claude.json`, the same token
used by `mx sync`. The token needs `repo` scope to clone and push to
wiki repositories.

## Related commands

- `mx sync` -- bidirectional GitHub issue and discussion sync (distinct
  from wiki sync).

# Base-d Encoding

Dictionary-based commit message encoding.

------------------------------------------------------------------------

## Overview

Base-d is a universal, multi-dictionary encoding library published as
the base-d crate. mx uses it to encode every commit message made with
`mx commit`, producing output that is intentionally unreadable in raw
`git log` but decodes cleanly with `mx log`.

The purpose is **obfuscation through encoding**. Commit messages are
transformed into sequences of glyphs -- hieroglyphs, chess pieces,
alchemical symbols, emoji, or any of 50+ dictionaries -- that carry no
human-readable meaning on their own. The original message is fully
recoverable because each commit carries a footer tag identifying the
exact algorithms and dictionaries used.

This is not encryption. The footer is plaintext and the dictionaries are
public. Anyone with `mx log` (or the base-d crate) can decode the
message. The goal is not secrecy but **noise reduction**: encoded
commits are visually distinct from human-authored text, making the
commit log resistant to casual reading while remaining fully reversible
by tooling.

## How it works

Every encoded commit has three parts:

1.  **Title** -- a hash of the staged diff, encoded through a randomly
    selected dictionary.

2.  **Body** -- the human-readable commit message, compressed and then
    encoded through a second randomly selected dictionary.

3.  **Footer** -- a bracket-delimited tag recording the algorithms and
    dictionary names used:
    `[hash_algo:title_dict|compress_algo:body_dict]`.

The title is a fingerprint of what changed. The body is the author's
description of why it changed. The footer is the decoder ring.

When you run:

``` bash
mx commit "fix session export crash on empty JSONL" -a
```

mx internally:

1.  Runs `git diff --staged` to capture the diff.

2.  Hashes the diff with a randomly chosen hash algorithm and encodes
    the hash through a random dictionary. This becomes the commit title.

3.  Compresses your message with a randomly chosen compression algorithm
    and encodes the compressed bytes through another random dictionary.
    This becomes the commit body.

4.  Assembles the footer tag from the algorithm and dictionary names.

5.  Commits with the three-part message: title, body, footer.

The result in raw `git log` looks something like:

    commit abc1234...
        U+1F711 U+1F754 U+1F72E U+1F716...

        8NO48P3FCDPIGSJ5C5I6QP9978G76R39...

        [sha384:base32hex|snappy:base32hex]

But `mx log` shows:

    abc1234 fix session export crash on empty JSONL

## Dictionaries

A dictionary is a mapping from binary data to a character set (or word
list). Base-d ships with over 50 built-in dictionaries spanning several
categories:

- **RFC standards** -- base2, base4, base8, base16, base32, base32hex,
  base32_crockford, base32_zbase, base32_geohash, base36, base45,
  base58, base58flickr, base58ripple, base62, base64, base64url,
  base64_imap, base64_radix, base85, base91, base100, base1024.

- **Legacy formats** -- ascii85, z85, uuencode, xxencode, binhex.

- **Ancient scripts** -- hieroglyphs, cuneiform, runic.

- **Symbols** -- alchemy, arrows, blocks, blocks_full, boxdraw, chess,
  domino, mahjong, music, zodiac, barcode, gradient, volume.

- **Emoji** -- emoji_faces, emoji_animals.

- **Specialized** -- cards (playing cards), dna (nucleotide encoding),
  weather, binary.

Each dictionary has a `common` flag (default: `true`). Only `common`
dictionaries are eligible for random selection during encoding.
Dictionaries marked `common = false` (such as `music`, which does not
render consistently across platforms) are available for explicit use but
excluded from the random pool.

Dictionaries are loaded from the built-in registry via
`DictionaryRegistry::load_default()`. Users can also define custom
dictionaries in `~/.config/base-d/dictionaries.toml`, which are merged
into the registry at load time.

### Encoding modes

Each dictionary operates in one of three modes:

- **Radix** -- true base conversion treating data as a large number.
  Works with any dictionary size.

- **Chunked** -- fixed-size bit groups, compatible with RFC 4648
  standards (base64, base32, etc.). Supports padding characters.

- **ByteRange** -- direct 1:1 byte-to-codepoint mapping using a
  contiguous Unicode range. Zero encoding overhead.

The mode is determined by the dictionary configuration, not by the
caller.

## Title encoding

The commit title is produced by hashing the staged diff:

1.  The staged diff (output of `git diff --staged`) is captured as raw
    bytes.

2.  A hash algorithm is chosen at random from the full set: MD5,
    SHA-224, SHA-256, SHA-384, SHA-512, SHA3-224, SHA3-256, SHA3-384,
    SHA3-512, Keccak-224, Keccak-256, Keccak-384, Keccak-512, Blake2b,
    Blake2s, Blake3, CRC-16, CRC-32, CRC-32C, CRC-64, xxHash32,
    xxHash64, XXH3-64, XXH3-128, Ascon, or K12.

3.  The hash is computed over the diff bytes.

4.  A dictionary is chosen at random from the common pool.

5.  The hash bytes are encoded through the dictionary.

The result is a fingerprint of the diff -- not human text. Two identical
diffs will produce different titles because the hash algorithm and
dictionary are re-rolled each time. The title exists so that `mx log`
can identify which commit produced which diff, not for human
consumption.

::: {.admonition .note}
**NOTE:** The title is a hash of the *diff*, not of the commit message.
It fingerprints what changed, not what the author said about it.
:::

## Body encoding

The commit body is produced by compressing and encoding the author's
message:

1.  The human-readable commit message is captured as UTF-8 bytes.

2.  A compression algorithm is chosen at random: LZMA, Zstd, Brotli,
    Gzip, LZ4, or Snappy.

3.  The message bytes are compressed.

4.  A second dictionary is chosen at random from the common pool
    (independently of the title dictionary).

5.  The compressed bytes are encoded through the dictionary.

The result is a compressed, encoded representation of the original
message. Decoding reverses the process: look up the dictionary from the
footer, decode back to compressed bytes, then decompress to recover the
original UTF-8 text.

## Footer format

The footer is a single line at the end of the commit message, formatted
as:

    [hash_algo:title_dict|compress_algo:body_dict]

For example:

    [sha384:base62|lzma:uuencode]

This tells the decoder:

- The title was produced by hashing with SHA-384 and encoding through
  the `base62` dictionary.

- The body was produced by compressing with LZMA and encoding through
  the `uuencode` dictionary.

The decoder (`mx log`) reads this footer, loads the named dictionaries
from the registry, and reverses the encoding. If the footer is missing
or malformed, the commit is treated as a plain (un-encoded) message and
displayed as-is.

### Footer validation

Not every line that matches the `[a:b|c:d]` shape is a real footer. The
decoder validates that the compression algorithm slot names a known
algorithm (LZMA, Zstd, Brotli, Gzip, LZ4, or Snappy) before treating the
line as a footer. This prevents user-authored text like `[link|here]` or
markdown bracket notation from being mistaken for encoding metadata.

## Dejavu markers

When both the title dictionary and the body dictionary happen to be the
same (by pure chance -- both are selected independently at random), the
footer includes a **dejavu marker**: the word `whoa.` appended on the
line after the footer tag.

    [sha384:base62|lzma:base62]
    whoa.

This is an easter egg. It has no functional significance. The encoding
and decoding work identically whether dejavu occurs or not. It simply
marks the coincidence that two independent random draws landed on the
same dictionary.

When `mx commit --show-encoded` is used, dejavu commits display an extra
line:

    Dejavu: true (both used base62)

## Encoding safety

Some dictionary and algorithm combinations produce encoded output
containing NUL bytes or control characters that would break git's
command-line argument handling. The encoder validates all output and
retries with a freshly rolled dictionary if unsafe characters are
detected, up to 5 attempts. Failed attempts are logged to stderr with
the dictionary that produced the problem.

If all 5 attempts produce unsafe output (statistically unlikely given
the dictionary pool size), the commit fails with an error listing every
dictionary combination that was tried.

## Decoding

`mx log` reverses the encoding:

1.  It runs `git log` and parses each commit into title, body, and
    lines.

2.  It scans the body for the last footer-shaped line -- a line matching
    `[hash:dict|compress:dict]` where the compression slot names a known
    algorithm.

3.  It splits the body into the encoded payload (everything above the
    footer) and trailing content (everything below the footer, including
    any dejavu marker).

4.  It looks up the body dictionary from the footer, decodes the payload
    back to compressed bytes, then decompresses to recover the original
    message.

5.  Non-encoded commits (those without a recognizable footer) pass
    through unchanged.

The footer-scan uses a "last wins" heuristic: if multiple footer-shaped
lines appear in the message (e.g., a user amended extra text that quotes
a prior footer), the last one is used. This covers the common case where
the real footer is near the bottom and any trailing content (dejavu
marker, user-appended notes) appears after it.

For full usage of the decoded log, see log.

## The base-d crate

Base-d is an independent crate published on crates.io. mx depends on
`base-d` version 3 and uses its `prelude` module for the core encoding
API:

- `DictionaryRegistry::load_default()` -- loads all built-in
  dictionaries.

- `hash_encode(data, registry)` -- hashes data with a random algorithm
  and encodes through a random dictionary. Returns the encoded string,
  hash algorithm name, and dictionary name.

- `compress_encode(data, registry)` -- compresses data with a random
  algorithm and encodes through a random dictionary. Returns the encoded
  string, compression algorithm name, and dictionary name.

- `decode(encoded, dictionary)` -- reverses the encoding for a known
  dictionary.

- `decompress(data, algorithm)` -- reverses the compression.

- `detect_dictionary(encoded)` -- auto-detects which dictionary was used
  (used as a fallback for old commits that lack dictionary names in
  their footer).

The crate supports SIMD acceleration (AVX2/SSSE3 on x86_64, NEON on
aarch64), streaming encoding/decoding for large files, custom user
dictionaries, and word-based encoding modes. mx uses only the
character-based encoding path.

## Dry-run and encode-only

Two modes let you inspect encoding without creating a commit:

``` bash
# Preview what a real commit would produce
mx commit "your message" --dry-run

# Encode arbitrary title/body text (no git state required)
mx commit --encode-only --title "refactor store" --body "split backends"
```

Dry-run runs the full encoding pipeline and validates the output, but
skips all git mutations. Encode-only takes explicit title and body text,
encodes them, and prints the result. Both are useful for testing
dictionary behavior or debugging encoding issues.

For the full commit flag reference, see commit.

# Filesystem Layout

Paths, environment variables, and configuration.

------------------------------------------------------------------------

This page is the canonical reference for where `mx` reads and writes
files, which environment variables control those locations, and what to
expect when upgrading from an earlier release.

## Table of contents

- The principle

- Layout

- Environment variables

- Migration & legacy fallbacks

- Renamed and removed CLI commands

- Examples

- Notes for contributors

## The principle

Every path `mx` touches derives from a single base directory:
`$MX_HOME`, which defaults to `~/.mx/`. Each subsystem owns a
subdirectory beneath that base.

Overrides are layered. From most specific to least specific:

1.  **Per-file** -- e.g. `MX_KV_SCHEMA`, `MX_KV_DATA`. These point at
    exact files (and may include a `{agent}` placeholder).

2.  **Per-subsystem** -- e.g. `MX_SURREAL_ROOT`, `MX_CODEX_PATH`,
    `MX_ISOLATE_MODELS`. These move one subsystem's root.

3.  **Base** -- `MX_HOME`. Moves the entire tree at once.

4.  **Default** -- `~/.mx/`.

A more specific override always wins. Setting `MX_KV_DATA=/etc/foo.json`
keeps that one file at `/etc/foo.json` regardless of `MX_HOME` or any
subsystem override.

The base path is hardcoded in exactly one place: `src/paths.rs`.
Everything else asks `paths.rs` for its location.

## Layout

    ~/.mx/                          # base; override $MX_HOME
    ├── kv/
    │   ├── schema/{agent}.toml     # override MX_KV_SCHEMA
    │   └── data/{agent}.json       # override MX_KV_DATA
    ├── state/
    │   └── schemas/{id}.yaml       # default ID: "tensor"; CLI --schema flag
    ├── memory/
    │   ├── surreal/                # override MX_SURREAL_ROOT
    │   ├── embed/                  # only when MX_ISOLATE_MODELS is set
    │   └── seed/
    │       ├── agents/             # *.md (frontmatter)
    │       └── knowledge/          # *.jsonl (markdown ingest tracked in #257)
    ├── codex/                      # override MX_CODEX_PATH
    ├── cache/sync/{owner-repo}/
    ├── artifacts/
    └── swap/

### `kv/` {#layout-kv}

The KV store: per-agent schema (TOML) and data (JSON). Used by `mx kv`
and any agent that needs fast local state. Each agent gets one schema
file and one data file, keyed off the `MX_CURRENT_AGENT` environment
variable.

- `kv/schema/{agent}.toml` -- TOML schema declaring keys, types, and
  defaults.

- `kv/data/{agent}.json` -- JSON-serialized current values, written
  atomically.

Override either file with `MX_KV_SCHEMA` / `MX_KV_DATA`. Both env vars
accept the literal string `{agent}`, which is substituted with the
active agent name at resolution time.

### `state/schemas/` {#layout-state}

YAML (or JSON) schemas for the emotional-state tensor system used by
`mx state`. The default schema ID is `tensor`, resolving to
`state/schemas/tensor.yaml`.

Pick a different schema with `mx state ... --schema {id|path}`. The flag
argument is classified as a path or an ID by a simple heuristic, then
looked up:

- **Bare ID** (e.g. `--schema tensor`): the loader tries
  `state/schemas/{id}.yaml` first, then `state/schemas/{id}.yml`, then
  `state/schemas/{id}.json`. The first file that exists wins; if none
  exist, the lookup fails with a "schema not found" error.

- **Direct path** (e.g. `--schema /tmp/foo.json`): the file is loaded
  directly. Any extension is accepted; the parser tries YAML first and
  falls back to JSON based on file contents, not the extension.

The argument is classified as a path if it contains a `/` **or** ends
with `.yaml`, `.yml`, or `.json`; otherwise it is classified as a bare
ID. A dot elsewhere in the name is irrelevant -- only those three
suffixes flip the classification. So `--schema my.schema` is treated as
an **ID** (no slash, no recognized extension) and, if no matching file
exists under `state/schemas/`, fails with a "schema not found" error. To
force a path lookup of a dotted name without one of the recognized
extensions, prefix it with `./` (e.g. `--schema ./my.schema`) -- the
slash flips it to path mode.

There is no env-var override for the schema choice anymore -- the old
`MX_STATE_SCHEMA` was replaced by the CLI flag. (See Renamed and removed
CLI commands.)

### `memory/` {#layout-memory}

The knowledge graph backend and its inputs.

- `memory/surreal/` -- SurrealKV embedded database files. Override the
  whole directory with `MX_SURREAL_ROOT`. For network-mode SurrealDB,
  the directory is unused -- see the SurrealDB connection vars.

- `memory/embed/` -- only created when `MX_ISOLATE_MODELS` (or legacy
  `MX_ISOLATE_FASTEMBED`) is set. Holds downloaded ONNX model and
  tokenizer files that would otherwise live in the shared XDG cache
  (`$XDG_CACHE_HOME/huggingface/`). Use this when you want mx's model
  cache isolated from other tools.

- `memory/seed/agents/` -- markdown files with YAML frontmatter, one per
  agent. Loaded by `mx memory seed agents`.

- `memory/seed/knowledge/` -- one or more `*.jsonl` files. Loaded by
  `mx memory seed knowledge`, which scans the directory for every
  `.jsonl` it finds.

### `codex/` {#layout-codex}

Session archives written by `mx codex archive` -- transcripts, extracted
images, and per-archive manifests. Override with `MX_CODEX_PATH`. This
is typically the largest directory in `~/.mx/`; point it at a roomier
disk if you archive a lot of sessions.

### `cache/sync/{owner-repo}/` {#layout-cache}

Per-repo cache directory used by `mx sync` to track GitHub issues and
discussions across runs. The repo slug replaces the `/` in `owner/repo`
with `-` (so `coryzibell/mx` becomes `coryzibell-mx`). Safe to delete --
it will be rebuilt on the next `mx sync pull`.

### `artifacts/` {#layout-artifacts}

Generic output directory for handlers that need to drop a file somewhere
predictable but don't have a more specific home. Treat as ephemeral.

### `swap/` {#layout-swap}

Scratch space for in-flight operations. Cleared opportunistically; do
not store anything you want to keep.

## Environment variables {#env-vars}

### Path overrides {#env-paths}

+---------------------+-----------------+-----------------------------------+---------------------------+
| **Variable**        | **Type**        | **Default**                       | **Overrides**             |
+=====================+=================+===================================+===========================+
| `MX_HOME`           | path            | `~/.mx/`                          | The base directory; moves |
|                     |                 |                                   | the entire tree           |
+---------------------+-----------------+-----------------------------------+---------------------------+
| `MX_SURREAL_ROOT`   | path            | `$MX_HOME/memory/surreal/`        | SurrealKV                 |
|                     |                 |                                   | embedded-database root    |
+---------------------+-----------------+-----------------------------------+---------------------------+
| `MX_CODEX_PATH`     | path            | `$MX_HOME/codex/`                 | Codex archive directory   |
+---------------------+-----------------+-----------------------------------+---------------------------+
| `MX_KV_SCHEMA`      | path template   | `$MX_HOME/kv/schema/{agent}.toml` | KV schema file; `{agent}` |
|                     |                 |                                   | placeholder substituted   |
+---------------------+-----------------+-----------------------------------+---------------------------+
| `MX_KV_DATA`        | path template   | `$MX_HOME/kv/data/{agent}.json`   | KV data file; `{agent}`   |
|                     |                 |                                   | placeholder substituted   |
+---------------------+-----------------+-----------------------------------+---------------------------+
| `MX_ISOLATE_MODELS` | boolean flag    | unset                             | When non-empty, redirects |
|                     |                 |                                   | the model cache from XDG  |
|                     |                 |                                   | to                        |
|                     |                 |                                   | `$MX_HOME/memory/embed/`. |
|                     |                 |                                   | Legacy                    |
|                     |                 |                                   | `MX_ISOLATE_FASTEMBED` is |
|                     |                 |                                   | also honored              |
+---------------------+-----------------+-----------------------------------+---------------------------+

Empty-string values for the path overrides in this table (`MX_HOME`,
`MX_SURREAL_ROOT`, `MX_CODEX_PATH`, `MX_KV_SCHEMA`, `MX_KV_DATA`,
`MX_ISOLATE_MODELS`) are treated as unset and fall back to the default.

The same is **not** uniformly true of other `MX_*` env vars. In
particular, the SurrealDB connection vars below do not all filter empty
strings: setting `MX_SURREAL_USER=""` produces an empty username, not
the default `root`. Among the connection vars only `MX_SURREAL_PASS` /
`MX_SURREAL_PASS_FILE` are empty-filtered. To restore a default, **leave
the variable unset entirely** rather than setting it to an empty string.

The boolean flag (`MX_ISOLATE_MODELS`) is "on" for any non-empty value
(`1`, `true`, `yes` -- it doesn't parse, it just checks for
non-emptiness).

### SurrealDB connection {#env-surreal}

These configure the SurrealDB driver. They affect which database mx
talks to, not where files live on disk (with the exception of
`MX_SURREAL_ROOT` above, which is the embedded-mode storage location).

+-------------------------+-----------------+-----------------------+-------------------+
| **Variable**            | **Type**        | **Default**           | **Purpose**       |
+=========================+=================+=======================+===================+
| `MX_SURREAL_MODE`       | enum            | `embedded`            | `embedded` for    |
|                         |                 |                       | local SurrealKV,  |
|                         |                 |                       | `network` for     |
|                         |                 |                       | WebSocket         |
+-------------------------+-----------------+-----------------------+-------------------+
| `MX_SURREAL_URL`        | URL             | `ws://localhost:8000` | WebSocket URL     |
|                         |                 |                       | (network mode     |
|                         |                 |                       | only)             |
+-------------------------+-----------------+-----------------------+-------------------+
| `MX_SURREAL_USER`       | string          | `root`                | Username          |
+-------------------------+-----------------+-----------------------+-------------------+
| `MX_SURREAL_PASS`       | secret          | unset                 | Password (literal |
|                         |                 |                       | value)            |
+-------------------------+-----------------+-----------------------+-------------------+
| `MX_SURREAL_PASS_FILE`  | path            | unset                 | Path to a file    |
|                         |                 |                       | containing the    |
|                         |                 |                       | password (e.g. an |
|                         |                 |                       | agenix secret);   |
|                         |                 |                       | read when         |
|                         |                 |                       | `MX_SURREAL_PASS` |
|                         |                 |                       | is unset          |
+-------------------------+-----------------+-----------------------+-------------------+
| `MX_SURREAL_NS`         | string          | `memory`              | Namespace         |
+-------------------------+-----------------+-----------------------+-------------------+
| `MX_SURREAL_DB`         | string          | `knowledge`           | Database name     |
+-------------------------+-----------------+-----------------------+-------------------+
| `MX_SURREAL_AUTH_LEVEL` | enum            | `root`                | One of `root`,    |
|                         |                 |                       | `namespace` (or   |
|                         |                 |                       | `ns`), `database` |
|                         |                 |                       | (or `db`)         |
+-------------------------+-----------------+-----------------------+-------------------+

### GitHub App auth (sync) {#env-github}

Optional. Only needed when `mx sync` runs against a private repo via a
GitHub App rather than via your personal `gh` token.

+-----------------------------+-----------------+-----------------+-----------------+
| **Variable**                | **Type**        | **Default**     | **Purpose**     |
+=============================+=================+=================+=================+
| `MX_GITHUB_APP_ID`          | string          | unset           | GitHub App ID   |
+-----------------------------+-----------------+-----------------+-----------------+
| `MX_GITHUB_INSTALLATION_ID` | string          | unset           | App             |
|                             |                 |                 | installation ID |
|                             |                 |                 | for the target  |
|                             |                 |                 | org/user        |
+-----------------------------+-----------------+-----------------+-----------------+
| `MX_GITHUB_PRIVATE_KEY`     | secret (PEM)    | unset           | App private     |
|                             |                 |                 | key,            |
|                             |                 |                 | PEM-encoded     |
+-----------------------------+-----------------+-----------------+-----------------+

### Identity & display {#env-other}

+---------------------+-----------------+-------------------------+------------------+
| **Variable**        | **Type**        | **Default**             | **Purpose**      |
+=====================+=================+=========================+==================+
| `MX_CURRENT_AGENT`  | string          | unset                   | Active agent     |
|                     |                 |                         | identity.        |
|                     |                 |                         | Required for     |
|                     |                 |                         | `mx memory wake` |
|                     |                 |                         | and any command  |
|                     |                 |                         | that             |
|                     |                 |                         | reads/writes     |
|                     |                 |                         | per-agent KV.    |
|                     |                 |                         | Also the default |
|                     |                 |                         | for              |
|                     |                 |                         | `--source-agent` |
|                     |                 |                         | on               |
|                     |                 |                         | `mx memory add`  |
+---------------------+-----------------+-------------------------+------------------+
| `MX_USER_NAME`      | string          | `git config user.name`, | Display name for |
|                     |                 | else `"User"`           | "user" turns in  |
|                     |                 |                         | codex            |
|                     |                 |                         | transcripts.     |
|                     |                 |                         | Resolution       |
|                     |                 |                         | order: env var   |
|                     |                 |                         | \> git config \> |
|                     |                 |                         | literal `"User"` |
+---------------------+-----------------+-------------------------+------------------+
| `MX_ASSISTANT_NAME` | string          | `"Orchestrator"`        | Display name for |
|                     |                 |                         | "assistant"      |
|                     |                 |                         | turns in codex   |
|                     |                 |                         | transcripts. No  |
|                     |                 |                         | git fallback --  |
|                     |                 |                         | the default is   |
|                     |                 |                         | the literal      |
|                     |                 |                         | string           |
|                     |                 |                         | `Orchestrator`   |
+---------------------+-----------------+-------------------------+------------------+

### Tuning {#env-tuning}

+-----------------------+-----------------+-----------------+-----------------+
| **Variable**          | **Type**        | **Default**     | **Purpose**     |
+=======================+=================+=================+=================+
| `MX_WAKE_CHUNK_BYTES` | integer         | `28000`         | Maximum bytes   |
|                       |                 |                 | per chunk       |
|                       |                 |                 | during the      |
|                       |                 |                 | wake-ritual     |
|                       |                 |                 | presentation    |
|                       |                 |                 | step. Values    |
|                       |                 |                 | that fail to    |
|                       |                 |                 | parse or are    |
|                       |                 |                 | zero fall back  |
|                       |                 |                 | silently to the |
|                       |                 |                 | default         |
+-----------------------+-----------------+-----------------+-----------------+

### Removed {#env-removed}

+-----------------------------------+-----------------------------------+
| **Variable**                      | **Replacement**                   |
+===================================+===================================+
| `MX_MEMORY_PATH`                  | Use `MX_SURREAL_ROOT` for just    |
|                                   | the database, or `MX_HOME` to     |
|                                   | move everything together. Setting |
|                                   | the old name now emits a one-line |
|                                   | stderr note and is otherwise      |
|                                   | ignored                           |
+-----------------------------------+-----------------------------------+
| `MX_STATE_SCHEMA`                 | Use the                           |
|                                   | `mx state ... --schema {id|path}` |
|                                   | CLI flag. The default schema ID   |
|                                   | also changed: it is now `tensor`  |
|                                   | (was `crewu`)                     |
+-----------------------------------+-----------------------------------+

## Migration & legacy fallbacks {#migration}

The path-alignment refactor (#255, merged via PR #259) moved several
files without breaking older installs. For one release cycle, mx will
read from the old locations as a soft fallback and emit a one-line
`note:` to stderr telling you what moved. **No data is lost. The
warnings are informative, not errors.**

+------------------------------------+------------------------------------------+-----------------------+
| **Legacy location**                | **New location**                         | **Behavior**          |
+====================================+==========================================+=======================+
| `~/.crewu/kv/{agent}.schema.toml`, | `$MX_HOME/kv/schema/{agent}.toml`,       | Read-only fallback;   |
| `~/.crewu/kv/{agent}.data.json`    | `$MX_HOME/kv/data/{agent}.json`          | consolidated stderr   |
|                                    |                                          | note fires once per   |
|                                    |                                          | process               |
+------------------------------------+------------------------------------------+-----------------------+
| `$MX_HOME/agents/` (agent seed     | `$MX_HOME/memory/seed/agents/`           | Read-only fallback;   |
| `*.md`)                            |                                          | stderr note when used |
+------------------------------------+------------------------------------------+-----------------------+
| `$MX_HOME/memory/index.jsonl`      | `$MX_HOME/memory/seed/knowledge/*.jsonl` | Read-only fallback.   |
| (knowledge seed)                   |                                          | This is a *shape*     |
|                                    |                                          | change, not a rename: |
|                                    |                                          | the old location was  |
|                                    |                                          | a single hardcoded    |
|                                    |                                          | file (`index.jsonl`); |
|                                    |                                          | the new location is a |
|                                    |                                          | directory scanned for |
|                                    |                                          | every `*.jsonl` it    |
|                                    |                                          | finds. Stderr note    |
|                                    |                                          | when the legacy file  |
|                                    |                                          | is read               |
+------------------------------------+------------------------------------------+-----------------------+
| `MX_MEMORY_PATH` env var           | `MX_SURREAL_ROOT` env var                | Old var **not         |
|                                    |                                          | honored**; setting it |
|                                    |                                          | just triggers a       |
|                                    |                                          | rename note           |
+------------------------------------+------------------------------------------+-----------------------+

To silence the warnings, move the files (or rename the env var). The
fallbacks will be removed in a future release. To track the removal in
source, grep for `TODO(*-migration)` and `TODO(memory-path-rename-note)`
in the codebase.

## Renamed and removed CLI commands {#renamed-commands}

+--------------------------------------+-----------------------------------+----------------------------+
| **Old**                              | **New**                           | **Notes**                  |
+======================================+===================================+============================+
| `mx agents seed`                     | `mx memory seed agents`           | Old form still parses but  |
|                                      |                                   | bails with a one-line      |
|                                      |                                   | pointer to the new command |
+--------------------------------------+-----------------------------------+----------------------------+
| `mx memory import`                   | `mx memory seed knowledge`        | Now scans a directory;     |
|                                      |                                   | loads every `*.jsonl` it   |
|                                      |                                   | finds rather than a single |
|                                      |                                   | hardcoded file             |
+--------------------------------------+-----------------------------------+----------------------------+
| `mx memory rebuild`                  | (removed)                         | Reindexing moved out of    |
|                                      |                                   | the user-facing surface;   |
|                                      |                                   | see issue #258 for         |
|                                      |                                   | `mx doctor memory rebuild` |
+--------------------------------------+-----------------------------------+----------------------------+
| `mx state ... --env-MX_STATE_SCHEMA` | `mx state ... --schema {id|path}` | CLI flag replaces the env  |
|                                      |                                   | var; accepts a bare schema |
|                                      |                                   | ID or a direct path        |
+--------------------------------------+-----------------------------------+----------------------------+
| `codex save`                         | `codex archive`                   | Renamed for clarity        |
+--------------------------------------+-----------------------------------+----------------------------+
| `session export`                     | `codex export`                    | Moved under the codex      |
|                                      |                                   | subcommand                 |
+--------------------------------------+-----------------------------------+----------------------------+

::: {.admonition .deprecated}
**DEPRECATED:** The old command names in this table still parse in
current builds but emit a pointer to their replacement. They will be
removed in a future release.
:::

## Examples {#examples}

Move the entire mx tree to a different disk:

``` bash
export MX_HOME=/data/mx
```

Keep mx's defaults but put the SurrealDB store on a fast SSD:

``` bash
export MX_SURREAL_ROOT=/mnt/ssd/mx-surreal
```

Use a custom KV schema for the `inkwell` agent (without overriding the
data file):

``` bash
export MX_KV_SCHEMA=/etc/mx/inkwell-schema.toml
```

Use a path template that resolves per-agent (one variable, many agents):

``` bash
export MX_KV_SCHEMA='/etc/mx/schemas/{agent}.toml'
export MX_KV_DATA='/var/lib/mx/{agent}.json'
```

Isolate the model cache so it doesn't share with other tools:

``` bash
export MX_ISOLATE_MODELS=1
# Models will now download into $MX_HOME/memory/embed/
```

Encode a tensor. The first form uses the default `tensor` schema; the
second points at an explicit schema file:

``` bash
mx state encode --dimensions "temp=0.8 entropy=0.75 agency=0.4"
mx state encode --schema /tmp/myschema.yaml -d "temp=0.5"
```

To target a non-default schema by ID, drop a YAML file at
`$MX_HOME/state/schemas/{id}.yaml` and pass `--schema {id}`. The bare-ID
form is what the lookup helper handles; for an absolute or relative file
path, just pass the path directly (see `state/schemas/` for the
path-vs-ID heuristic).

Point SurrealDB at a remote network instance:

``` bash
export MX_SURREAL_MODE=network
export MX_SURREAL_URL=ws://surreal.internal:8000
export MX_SURREAL_USER=mx
export MX_SURREAL_PASS_FILE=/run/agenix/mx-surreal-pass
```

## Notes for contributors {#contributors}

Every path in mx routes through `src/paths.rs`. New helpers follow the
`_with(env_val: Option<&str>, home: &Path)` test-seam pattern -- see
`paths::codex_dir_with` for the canonical example. The pattern keeps
resolution logic pure: tests call the `_with` variant directly with
explicit arguments and never mutate process env state, so the suite runs
safely in parallel.

Two rules:

1.  Do not call `dirs::home_dir()` outside `src/paths.rs`. If you need a
    home-relative path, add a helper to `paths.rs` and call it from your
    module. `paths.rs` itself is the *only* legitimate caller of
    `dirs::home_dir()` in the tree. It uses it for: `mx_home()` (the
    `~/.mx/` default), `legacy_crewu_kv_schema_path` and
    `legacy_crewu_kv_data_path` (the legacy fallbacks for the kv
    migration), and `claude_projects_dir` / `claude_config_path`
    (read-only locations owned by another tool, Claude). Anything new
    that needs `home_dir()` -- including helpers for paths owned by
    other tools -- belongs in `paths.rs` too, so this rule stays
    absolute everywhere else. Do not "fix" the existing calls for
    consistency; they are the carve-out.

2.  Do not read `MX_*` env vars in handlers if a path helper already
    encapsulates that override. Add the env-var read inside the helper
    instead, behind the `_with` seam.

# Architecture

System internals for contributors.

------------------------------------------------------------------------

This page describes how mx is built. It covers the module structure,
dispatch model, storage backends, and encoding pipeline. The audience is
contributors reading the source code, not users running commands.

## Table of contents

- Overview

- Module structure

- Command dispatch

- Path management

- SurrealDB integration

- Knowledge graph data model

- Codex archive format

- KV store format

- Base-d integration

- Testing patterns

## Overview {#overview}

mx is a single-binary Rust CLI built on three pillars:

1.  **clap derive** for the command tree -- every subcommand, flag, and
    validation rule is expressed as Rust types in `src/cli.rs`.

2.  **SurrealDB** for the knowledge graph -- an embedded SurrealKV
    database (or optional network WebSocket connection) stores entries,
    relationships, tags, embeddings, and metadata.

3.  **base-d** for commit encoding -- a separate crate that hashes,
    compresses, and encodes commit messages through randomly selected
    dictionaries.

The binary is `mx`. There is no library crate; `main.rs` declares
modules and calls into handlers. The Rust edition is 2024.

Key dependencies:

+-----------------------+-----------------------+-----------------------+
| **Crate**             | **Version**           | **Role**              |
+=======================+=======================+=======================+
| `clap`                | 4                     | CLI parsing with      |
|                       |                       | derive macros         |
+-----------------------+-----------------------+-----------------------+
| `surrealdb`           | 2                     | Embedded + WebSocket  |
|                       |                       | knowledge store       |
+-----------------------+-----------------------+-----------------------+
| `base-d`              | 3                     | Dictionary-based      |
|                       |                       | hash/compress         |
|                       |                       | encoding              |
+-----------------------+-----------------------+-----------------------+
| `tokio`               | 1                     | Async runtime for     |
|                       |                       | SurrealDB             |
|                       |                       | (multi-thread)        |
+-----------------------+-----------------------+-----------------------+
| `tract-onnx`          | 0.22                  | Local vector          |
|                       |                       | embeddings via ONNX   |
|                       |                       | inference             |
|                       |                       | (BGE-Base-EN-v1.5,    |
|                       |                       | 768-dim)              |
+-----------------------+-----------------------+-----------------------+
| `serde` /             | 1 / 1 / 0.8 / 0.9     | Serialization across  |
| `serde_json` / `toml` |                       | JSON, TOML, YAML      |
| / `serde_yaml`        |                       |                       |
+-----------------------+-----------------------+-----------------------+
| `chrono`              | 0.4                   | Timestamps with serde |
|                       |                       | support               |
+-----------------------+-----------------------+-----------------------+
| `anyhow` /            | 1 / 2                 | Error handling        |
| `thiserror`           |                       | (anyhow for handlers, |
|                       |                       | thiserror for typed   |
|                       |                       | errors)               |
+-----------------------+-----------------------+-----------------------+
| `reqwest`             | 0.12                  | HTTP client for       |
|                       |                       | GitHub API calls      |
+-----------------------+-----------------------+-----------------------+
| `jsonwebtoken`        | 10                    | JWT signing for       |
|                       |                       | GitHub App auth       |
+-----------------------+-----------------------+-----------------------+
| `pulldown-cmark`      | 0.13                  | Fence-aware heading   |
|                       |                       | extraction            |
+-----------------------+-----------------------+-----------------------+
| `colored`             | 2                     | Terminal colors       |
+-----------------------+-----------------------+-----------------------+

## Module structure

All source lives under `src/`. The top-level modules declared in
`main.rs` are:

    src/
     main.rs            # entry point, Cli::parse(), match on Commands
     cli.rs             # the full command tree (clap derive enums)
     paths.rs           # single source of path truth
     handlers/          # command handler routing
       mod.rs           # top-level dispatchers (pr, github, codex, log, show, etc.)
       memory.rs        # mx memory subcommand handler
       kv.rs            # mx kv subcommand handler
       metadata.rs      # metadata subcommand handler (categories, tags, etc.)
       state.rs         # mx state subcommand handler (deprecated)
     commit.rs          # encoding pipeline (hash + compress + encode)
     knowledge.rs       # KnowledgeEntry struct (the core data model)
     store.rs           # KnowledgeStore trait (abstract storage interface)
     surreal_db/        # SurrealDB implementation of KnowledgeStore
       mod.rs           # SurrealDatabase struct, with_db! macro, RecordId
       connection.rs    # SurrealMode, SurrealConfig, SurrealConnection enum
       knowledge.rs     # SurrealKnowledgeRecord DTO, query hydration
       queries.rs       # backup operations, query helpers
       lookups.rs       # lookup table CRUD (categories, agents, projects, etc.)
       relationships.rs # graph edge operations (relates_to)
       trait_impl.rs    # KnowledgeStore impl for SurrealDatabase
       tests.rs         # integration tests
     codex/             # session conversation archival
       mod.rs           # manifest types, re-exports
       archive/         # the archive pipeline
         mod.rs         # ArchiveRequest, ArchiveOptions, entry points
         include.rs     # IncludeSet (--include flag parser)
         write.rs       # per-session writer, --all driver loop
         sources.rs     # source walkers (subagent discovery, etc.)
         paths.rs       # archive-folder naming, short-ID extraction
         backfill.rs    # vault backfill (--backfill flag)
       export/          # mx codex export pipeline
       index/           # codex indexing
       images.rs        # base64 image extraction from JSONL
       transcript.rs    # conversation.md rendering
       read.rs          # list, read, search operations
       migrate.rs       # v1->v2 archive migration
       notices.rs       # vault-present warnings
     chunking.rs        # token-aware text chunking for embeddings
     embeddings.rs      # EmbeddingProvider trait, TractProvider
     kv.rs              # KV store engine (schema TOML + data JSON)
     types.rs           # shared domain types (Agent, Category, Project, etc.)
     display.rs         # safe_truncate, formatting helpers
     tensor.rs          # emotional state tensor encode/decode (deprecated, serves mx state)
     github.rs          # GitHub API operations (cleanup, comments)
     sync/              # GitHub sync (issues, wiki)
     convert.rs         # md2yaml / yaml2md conversion
     session.rs         # deprecated session export (forwards to codex)
     index.rs           # legacy index operations
     helpers.rs         # shared utilities
     wake_chunk.rs      # wake ritual chunking
     wake_ritual.rs     # wake ritual flow
     wake_token.rs      # HMAC-signed wake session tokens
     engage.rs          # interactive wake engage mode
     content_ops.rs     # content editing operations (find/replace, append, etc.)

### Module boundaries

The codebase follows a layered pattern:

1.  **CLI layer** (`cli.rs`) -- pure data. No logic, no imports beyond
    clap. Every command variant, flag, and validation constraint is a
    type.

2.  **Handler layer** (`handlers/`) -- orchestration. Reads CLI args,
    calls into domain modules, formats output. Handlers own `println!`
    and `eprintln!`. They do not own business logic.

3.  **Domain layer** (`commit.rs`, `knowledge.rs`, `store.rs`, `kv.rs`,
    `codex/`, `embeddings.rs`, `tensor.rs`) -- the actual work. Pure
    functions where possible, side effects isolated to well-defined
    boundaries (git subprocesses, database calls, filesystem writes).

4.  **Infrastructure layer** (`surreal_db/`, `paths.rs`, `github.rs`) --
    external integrations. SurrealDB, filesystem, GitHub API.

## Command dispatch

The dispatch path is:

    main() -> Cli::parse() -> match cli.command { ... }

`main.rs` is small by design. It does three things:

1.  Emits a legacy-path deprecation note if `MX_MEMORY_PATH` is set.

2.  Parses the CLI with `clap::Parser::parse()`.

3.  Pattern-matches on the top-level `Commands` enum and calls the
    appropriate handler.

Some commands dispatch directly to domain functions from `main.rs`:

``` rust
Commands::Commit { .. } => commit::upload_commit(..),
Commands::Log { .. } => handle_log(..),
Commands::Show { .. } => handle_show(..),
```

Others dispatch through `handlers/mod.rs`:

``` rust
Commands::Memory { command } => handle_memory(command, cli.verbose),
Commands::Kv { command } => handle_kv(command, cli.verbose),
Commands::Codex { command } => handle_codex(command),
```

The handler functions in `handlers/mod.rs` then match on the subcommand
enum and call into domain modules. For example, `handle_codex` matches
on `CodexCommands::Archive`, `CodexCommands::Export`, etc., and routes
each to the appropriate function in `codex::archive`, `codex::export`,
or `codex::read`.

### The `Commit` command

The `Commit` variant is handled inline in `main.rs` rather than through
a handler, because it has two distinct modes selected by the
`--encode-only` flag:

1.  **Normal mode**: calls `commit::upload_commit()` with the message,
    stage/push flags, and display preferences.

2.  **Encode-only mode**: calls `commit::encode_commit_message()` with
    explicit title and body text, prints the result, and exits. No git
    state is touched.

### Exit codes

Most commands exit 0 on success or propagate an `anyhow::Error` (which
prints the error chain to stderr and exits non-zero). The `kv`
subcommand is the exception: it uses typed exit codes (0 = OK, 1 = key
not found, 2 = type mismatch, 3 = schema missing, 4 = invalid input) so
callers can distinguish failure modes programmatically. The `KvError`
enum covers five typed variants: `KeyNotFound`, `TypeMismatch`,
`SchemaMissing`, `EntryNotFound` (a specific entry ID was not found
within a key), and `AmbiguousId` (an ID prefix matched multiple
entries). Both `EntryNotFound` and `AmbiguousId` map to exit code 4.

## Path management

`src/paths.rs` is the single source of truth for every filesystem path
mx touches. The module is deliberately the *only* file in the codebase
that calls `dirs::home_dir()`. Every other module that needs a path
calls a function from `paths.rs`.

### The base directory

All paths derive from `mx_home()`, which resolves once per process via
`OnceLock`:

1.  If `MX_HOME` is set and non-empty, use it.

2.  Otherwise, use `~/.mx/`.

The result is cached for the lifetime of the process.

### Derived paths

Each subsystem has its own function in `paths.rs`:

  **Function**                    **Returns**
  ------------------------------- -----------------------------------------------------
  `mx_home()`                     `$MX_HOME` or `~/.mx/`
  `kv_schema_path(agent)`         `$MX_HOME/kv/schema/{agent}.toml`
  `kv_data_path(agent)`           `$MX_HOME/kv/data/{agent}.json`
  `surreal_root()`                `$MX_SURREAL_ROOT` or `$MX_HOME/memory/surreal/`
  `codex_dir()`                   `$MX_CODEX_PATH` or `$MX_HOME/codex/`
  `model_cache_dir()`             XDG cache or `$MX_HOME/memory/embed/` when isolated
  `memory_seed_agents_dir()`      `$MX_HOME/memory/seed/agents/`
  `memory_seed_knowledge_dir()`   `$MX_HOME/memory/seed/knowledge/`
  `state_schemas_dir()`           `$MX_HOME/state/schemas/`
  `swap_dir()`                    `$MX_HOME/swap/`
  `sync_cache_dir(repo)`          `$MX_HOME/cache/sync/{repo-slug}/`

### The `_with()` test-seam pattern {#with-pattern}

Pure resolution logic is factored into `_with` variants that take
env-var values as explicit parameters instead of reading `std::env`:

``` rust
fn codex_dir_with(env_val: Option<&str>, home: &Path) -> PathBuf {
    if let Some(path) = env_val && !path.is_empty() {
        return PathBuf::from(path);
    }
    home.join("codex")
}

pub fn codex_dir() -> PathBuf {
    codex_dir_with(
        std::env::var("MX_CODEX_PATH").ok().as_deref(),
        mx_home(),
    )
}
```

Tests call the `_with` variant directly with controlled inputs. The
public function is a thin wrapper that reads the env var and passes it
in. This keeps tests parallel-safe (no env-var mutation) and the
resolution logic unit-testable in isolation.

The same pattern is used by `surreal_root_with`, `model_cache_dir_with`,
`resolve_mx_home_with`, and `resolve_kv_path_with`.

### External paths (read-only)

`paths.rs` also provides helpers for locations owned by other tools that
mx reads but never writes:

- `claude_dir()` -- `~/.claude/`

- `claude_projects_dir()` -- `~/.claude/projects/` (override:
  `MX_CLAUDE_PROJECTS_DIR` for tests)

- `claude_subagents_dir(slug, session)` -- subagent JSONL location

- `claude_sessions_dir()` -- per-PID liveness JSONs

- `claude_history_jsonl()` -- slash-command history

- `claude_mcp_logs_dir(slug)` -- MCP server log parent directory

- `wonka_vault_archives_dir()` -- legacy vault snapshots
  (`~/.wonka/vault/archives/`)

These are centralized in `paths.rs` so the codex archive source walkers
have a single source of truth for Claude's on-disk layout.

## SurrealDB integration

The knowledge graph is backed by SurrealDB. The integration supports two
connection modes:

### Embedded mode (default)

Uses the `SurrealKV` engine -- a local, file-based key-value store
compiled into the mx binary. No external server process is required. The
database files live at `$MX_HOME/memory/surreal/` (override with
`MX_SURREAL_ROOT`).

On first connection, the schema file (`schema/surrealdb-schema.surql`)
is applied via `include_str!`. This is compiled into the binary -- there
is no runtime file read. The schema uses `DEFINE ... IF NOT EXISTS` and
`UPSERT` throughout, making it safe to re-apply on every startup.

### Network mode

When `MX_SURREAL_MODE=network`, mx connects to an external SurrealDB
instance over WebSocket (`ws://` or `wss://`). The local `surreal_root`
path is unused. Authentication supports three levels (root, namespace,
database), configured via `MX_SURREAL_AUTH_LEVEL`. Password can be
provided directly (`MX_SURREAL_PASS`) or read from a file
(`MX_SURREAL_PASS_FILE`, useful for agenix-managed secrets on NixOS).

### Connection architecture

The connection is represented as an enum:

``` rust
pub enum SurrealConnection {
    Embedded(Surreal<surrealdb::engine::local::Db>),
    Network(Surreal<WsClient>),
}
```

A `with_db!` macro dispatches across both variants:

``` rust
macro_rules! with_db {
    ($self:expr, $db:ident, $body:expr) => {
        match &$self.conn {
            SurrealConnection::Embedded($db) => $body,
            SurrealConnection::Network($db) => $body,
        }
    };
}
```

This allows every query function to be written once and work against
both backends. The `SurrealDatabase` struct wraps the connection and
exposes synchronous methods that internally use a `block_on` bridge over
a global `OnceLock<Runtime>` tokio runtime.

### The `KnowledgeStore` trait

`src/store.rs` defines the `KnowledgeStore` trait -- the abstract
interface for knowledge storage. `SurrealDatabase` implements this trait
in `surreal_db/trait_impl.rs`. The trait surface includes:

- CRUD: `upsert_knowledge`, `get`, `delete`

- Search: `search` (full-text BM25), `semantic_search` (vector cosine
  similarity)

- Listing: `list_by_category`, `count_by_category`, `list_all`, `count`

- Wake cascade: `wake_cascade` (layered identity retrieval)

- Lookups: categories, agents, projects, sessions, relationships, tags

- Reinforcement: `reinforce` (increment resonance, update activation
  metadata), `update_activations` (batch-reset decay clocks for search
  activation)

- Backups: pre-mutation content snapshots

The trait exists to decouple handler logic from the storage backend. In
practice, `SurrealDatabase` is the only implementation.

## Knowledge graph data model {#knowledge-graph}

The schema lives in `schema/surrealdb-schema.surql` and is compiled into
the binary. It defines a SCHEMAFULL relational-graph model.

### Core entity: `knowledge`

The central table is `knowledge`. Each row represents one knowledge
entry with the following field groups:

**Identity and content:**

- `title` (string), `body` (optional string), `summary` (optional
  string)

- `content_hash` (string) -- for change detection during seed/import

- `format` -- `markdown`, `json`, or `stele:*` variants

**Classification (record links):**

- `category` (record\<category\>) -- pattern, technique, insight,
  gotcha, reference, decision, bloom, session

- `source_type` (record\<source_type\>) -- manual, ram, cache,
  agent_session

- `entry_type` (record\<entry_type\>) -- primary, summary, synthesis

- `content_type` (record\<content_type\>) -- text, code, config, data,
  binary

- `source_project`, `source_agent`, `session` -- optional record links

**Visibility:**

- `visibility` -- `public` or `private` (ASSERT constraint)

- `owner` -- agent ID for private entries

**Resonance (wake-up cascade):**

- `resonance` (int) -- importance level, 1--10 with overflow for
  transcendent

- `resonance_type` -- foundational, transformative, relational,
  operational, ephemeral, session

- `last_activated` (datetime), `activation_count` (int)

- `decay_rate` (float, 0.0--1.0) -- some memories fade, some do not

- `anchors` (array\<string\>) -- IDs of related blooms this entry
  connects to

- `wake_phrases` (array\<string\>) -- verification phrases for the wake
  ritual

- `wake_order` (optional int) -- custom sequence position

**Embeddings:**

- `embedding` (optional array\<float\>) -- 768-dim vector
  (BGE-Base-EN-v1.5). For chunked entries, this holds a normalized mean
  vector of all chunk embeddings (used by `auto-anchor`).

- `embedding_model` (optional string), `embedded_at` (optional datetime)

- `chunk_count` (int, default 0) -- number of embedding chunks. Zero
  means the entry is unchunked (single embedding). A positive value
  means the entry was split into overlapping chunks stored in the
  `embedding_chunk` table.

### Graph relations

SurrealDB's graph relations replace traditional junction tables:

  **Relation table**      **Direction**                      **Purpose**
  ----------------------- ---------------------------------- ------------------------------------------------
  `tagged_with`           knowledge -\> tag                  Freeform labels
  `applies_to`            knowledge -\> applicability_type   Scope constraints (language, platform, domain)
  `relates_to`            knowledge -\> knowledge            Inter-entry graph edges
  `project_tagged_with`   project -\> tag                    Project-level tags
  `project_applies_to`    project -\> applicability_type     Project scope

The `relates_to` relation carries a `relationship_type` field
(record\<relationship_type\>) and is uniquely indexed on the triple
(from, to, type). Relationship types are: related, supersedes, extends,
implements, contradicts, example_of.

### Lookup tables

Eight lookup tables provide controlled vocabularies: `category`,
`project`, `agent`, `applicability_type`, `source_type`, `entry_type`,
`content_type`, `relationship_type`, `session_type`, `tag`. Default seed
data is applied via `UPSERT` in the schema file. Users can extend them
through `mx memory categories add`, `mx memory agents add`, etc.

### Full-text search

A `simple` analyzer (blank + class tokenizers, lowercase filter) powers
BM25 search indexes on `title`, `body`, and `summary`. Searches via
`mx memory search` query all three indexes.

### Vector search

Embeddings are 768-dimensional float arrays generated by tract-onnx
(BGE-Base-EN-v1.5, local inference). The search strategy is brute-force
cosine similarity -- no HNSW index. This is deliberate at the current
scale; the schema comment notes to reconsider when the store exceeds 50K
vectors or 100ms query latency.

The `EmbeddingProvider` trait in `embeddings.rs` abstracts the embedding
backend. `TractProvider` is the sole implementation. The model cache
location is controlled by `paths::model_cache_dir()`.

#### Two-phase semantic search {#two-phase-search}

Semantic search uses a two-phase strategy to cover both unchunked
entries and chunked entries:

1.  **Phase 1a**: Query unchunked entries (those with `chunk_count <= 0`
    or absent) by cosine similarity against their `embedding` field.
    Returns up to `limit` results.

2.  **Phase 1b**: Query the `embedding_chunk` table by cosine
    similarity. Returns up to `limit * 3` results (over-fetching for
    deduplication).

Both queries run in a single SurrealDB request (chained statements).

1.  **Phase 2 (merge)**: Chunk results are deduplicated by `entry_id`,
    keeping the maximum similarity score per entry. For each unique
    chunk entry, the full `knowledge` record is fetched (with
    visibility, category, and resonance filters applied). The unchunked
    and chunk results are merged into a single scored map: if an entry
    appears in both result sets, the higher score wins. The final list
    is sorted by score descending and truncated to `limit`.

This design means a long entry surfaces in search results if *any*
400-token section is semantically relevant, rather than only when the
mean vector (which averages over all sections) happens to score well.

### Embedding chunks

The `embedding_chunk` table stores per-chunk embeddings for long entries
(those exceeding 400 tokens). Each row represents one chunk of a chunked
entry:

- `entry_id` (string) -- the `kn-` prefixed ID of the parent knowledge
  entry

- `chunk_index` (int) -- zero-based position within the entry's chunk
  sequence

- `chunk_text` (string) -- the decoded text of this chunk

- `token_offset` (int) -- token offset from the start of the original
  text

- `token_count` (int) -- number of tokens in this chunk

- `embedding` (array\<float\>) -- 768-dim vector for this chunk

- `embedding_model` (string) -- model ID that generated the embedding

- `created_at` (datetime)

The table is indexed on `entry_id` (for bulk deletion) and uniquely
indexed on `(entry_id, chunk_index)` (for upsert). Chunks are deleted
and re-created on every re-embed of the parent entry. When a knowledge
entry is deleted, its chunks are cleaned up on a best-effort basis.

Chunking parameters: 400 tokens per chunk, 100-token overlap (stride
300). These are defined in `ChunkConfig::default()` in
`src/chunking.rs`.

### Backups

The `memory_backup` table stores pre-mutation content snapshots. Before
any update, edit, append, prepend, or delete operation, the current
content is written to a backup row. Backups reference entries by plain
string ID (not a record link) so they survive entry deletion.

## Codex archive format {#codex-archive}

The codex is the session conversation archive. `mx codex archive`
captures Claude Code sessions from `~/.claude/projects/` into permanent
storage at `$MX_HOME/codex/`.

### Archive directory layout

Each archive is a directory named with the pattern:

    {date}_{short-session-id}[_{counter}]

For example: `2026-04-30_abc12345` or `2026-04-30_abc12345_2` for
incremental saves.

Inside each archive directory:

    {archive}/
      manifest.json       # metadata (version, timestamps, counts, checksums)
      session.jsonl        # raw session JSONL (unless --clean)
      conversation.md      # clean markdown transcript (when --clean or migrated)
      images/              # extracted base64 images (v2+)
        image_001.png
        image_002.png
      agents/              # subagent session JSONLs (when --include subagents)
        agent-{uuid}.jsonl

### Manifest

The manifest is a JSON file tracking archive metadata. The current write
version is 5. All fields added since v2 are `Option` so older archives
deserialize cleanly.

Key fields:

- `version` -- manifest format version (2--5)

- `session_id` -- the Claude session UUID

- `archived_at`, `session_start`, `session_end` -- timestamps

- `project_path` -- the working directory of the session

- `message_count`, `agent_count` -- summary statistics

- `agents` -- array of `AgentInfo` (id, file, message count)

- `size_bytes`, `checksum` -- integrity data

- `image_count`, `images` -- v2: extracted image metadata

- `has_clean_transcript` -- v3: whether `conversation.md` exists

- `user_name`, `assistant_name` -- v4: configurable speaker names

- `source_breakdown` -- v5: per-sidecar byte counts

### The `IncludeSet`

The `--include` flag on `mx codex archive` controls which optional
source artifacts are captured. It parses a comma-separated string into a
struct with boolean fields:

- `subagents` (default: true) -- capture subagent session JSONLs

- `mcp` -- capture MCP server logs

- `tool_output` -- capture `/tmp` tool outputs

- `history` -- capture `history.jsonl` slice

- `all` / `none` -- shortcuts

### Source walkers

The archive pipeline uses source walkers to discover files for capture.
Currently `sources.rs` implements subagent discovery
(`find_agent_sessions`). The other source types (MCP, tool-output,
history) are declared in the `IncludeSet` but their walkers are pending
implementation in future PRs.

## KV store format {#kv-store}

The KV store (`src/kv.rs`) is a lightweight local state engine for
agents. No networking, no database -- just a TOML schema file and a JSON
data file per agent.

### Schema (TOML)

Each agent's schema lives at `$MX_HOME/kv/schema/{agent}.toml` and
declares the keys, types, constraints, and defaults:

``` toml
[keys.commit_count]
type = "counter"
min = 0

[keys.recent_files]
type = "history"
max_entries = 50

[keys.current_task]
type = "string"
default = ""

[keys.focus_areas]
type = "list"
description = "Areas of active focus"

[keys.session_state]
type = "state"
fields = ["mode", "context", "priority"]
```

Supported types:

+-----------------------------------+-----------------------------------+
| **Type**                          | **Behavior**                      |
+===================================+===================================+
| `counter`                         | Integer with optional `min`/`max` |
|                                   | bounds. Supports `inc`, `dec`,    |
|                                   | `set`, `get`                      |
+-----------------------------------+-----------------------------------+
| `string`                          | Simple string value. Supports     |
|                                   | `set`, `get`                      |
+-----------------------------------+-----------------------------------+
| `history`                         | Timestamped append-only log with  |
|                                   | optional `max_entries` cap.       |
|                                   | Supports `push`, `last`, `since`, |
|                                   | `search`, `count`, `random`,      |
|                                   | `update`, `migrate`. Each entry   |
|                                   | gets a numeric index and a stable |
|                                   | base58 entry ID (`kv-` prefix).   |
|                                   | Entries can carry optional        |
|                                   | structured JSON data (`--data` on |
|                                   | push/update, `--where` on         |
|                                   | queries). The `last`, `search`,   |
|                                   | `count`, and `random` commands    |
|                                   | accept time-range flags (`--day`, |
|                                   | `--month`, `--week`, `--since`,   |
|                                   | `--from`/`--to`) for date         |
|                                   | filtering.                        |
+-----------------------------------+-----------------------------------+
| `list`                            | Ordered list with timestamps.     |
|                                   | Supports `push`, `pop`, `remove`, |
|                                   | `search`, `count`, `random`,      |
|                                   | `update`, `migrate`. Each entry   |
|                                   | gets a numeric index and a stable |
|                                   | base58 entry ID. Entries can      |
|                                   | carry optional structured JSON    |
|                                   | data. The `last`, `search`,       |
|                                   | `count`, and `random` commands    |
|                                   | accept the same time-range flags  |
|                                   | as history.                       |
+-----------------------------------+-----------------------------------+
| `state`                           | Named fields (like a struct).     |
|                                   | Supports single-field set         |
|                                   | (`set <key> <field> <value>`),    |
|                                   | batch set                         |
|                                   | (`set <key> field=value ...` or   |
|                                   | `set <key> --json '{...}'`),      |
|                                   | tensor positional set             |
|                                   | (`set <key> --json '[...]'`), and |
|                                   | `get`. Batch operations validate  |
|                                   | all fields against the schema     |
|                                   | before writing.                   |
+-----------------------------------+-----------------------------------+

### Data (JSON)

The data file at `$MX_HOME/kv/data/{agent}.json` holds current values.
All writes are atomic: serialize to a temp file, fsync, rename. The
format is a flat JSON object keyed by the key names from the schema.

History and list entries are stored as objects with `id` (stable entry
ID, serialized from the `id` field), `hash` (legacy on-disk name for the
entry ID, read via `serde(rename)`), `value`, `ts`, an optional `data`
field (arbitrary JSON object for structured metadata), and an optional
`memory` field (a `kn-` ID linking the entry to a knowledge node in the
memory graph). In the Rust structs, the numeric sequence number is the
`index` field (serialized as `id` on disk) and the stable base58
identifier is the `id` field (serialized as `hash` on disk). The on-disk
names are preserved via `serde(rename)` for backward compatibility -- no
data migration is needed. The entry ID is a short base58 string
generated from `blake3(key + timestamp + index)` via base-d, providing a
stable identifier independent of numeric ordering. The `id` (entry ID),
`data`, and `memory` fields all use `#[serde(default)]` for backward
compatibility -- files written before these fields existed are
back-filled on first load (IDs are generated, data and memory default to
`None`) and saved automatically.

### Schema mutation

The `KvStore` struct holds a `schema_path` field alongside the existing
`data_path`. The `add_key_to_schema()` method validates the key name
(alphanumeric, underscores, hyphens; max 128 chars; no dots), appends a
`[keys.<name>]` block to the TOML file without reformatting existing
content, and re-parses the file to update the in-memory `Schema`. This
is exposed through `push --create <type>` at the CLI layer, where the
handler calls `add_key_to_schema` before the normal push path. If the
key already exists, the method is a no-op.

The `rename_key()` method moves a key from one name to another in both
the schema and data files. It validates the new name, checks that the
old key exists and the new key does not, then atomically swaps the
in-memory entries before persisting. Data is written first (higher-value
file), then schema. If the data write fails, in-memory mutations are
rolled back. Entry IDs are stable across renames -- they were hashed
from the original key name at creation time and are never regenerated.
This is exposed through `mx kv rename <old> <new>` at the CLI layer.

### Per-agent keying

The active agent is determined by the `MX_CURRENT_AGENT` environment
variable. Schema and data files are resolved via
`paths::kv_schema_path(agent)` and `paths::kv_data_path(agent)`. The
path resolution includes a legacy fallback to `~/.crewu/kv/` for
migration purposes.

### Memory pointers

KV keys can optionally link to a knowledge entry in the SurrealDB store
via a `kn-` ID reference. This allows an agent to associate fast local
state with richer knowledge graph entries. The `--memory` flag on `get`,
`last`, `since`, `search`, `random`, and `dump` resolves these
references and displays the linked entry.

Memory links exist at two levels: key-level (one pointer per key) and
per-entry (one pointer per history or list entry). Per-entry links are
set via `push --memory` at creation time or `set --id --memory` on
existing entries. When resolving, per-entry memory wins over a legacy
`kn-` value prefix, which wins over the key-level fallback. The
`SearchHit` struct (returned by `last`, `random`, `search`, `since`, and
`get --id`) carries the per-entry `memory` field for the handler to
resolve.

`SearchHit` derives `serde::Serialize` to support the `--json` output
flag. The serialized field names are the Rust struct names (`index`,
`id`, `value`, `ts`, `data`, `memory`) -- deliberately different from
the on-disk `serde(rename)` aliases used by `HistoryEntry` and
`ListEntry`. The `data` and `memory` fields use
`#[serde(skip_serializing_if = "Option::is_none")]` so they are omitted
from JSON output when not set.

## Base-d integration

The `base-d` crate (version 3) provides the encoding layer. It is used
in three places:

### `commit.rs` -- the encoding pipeline

When `mx commit` runs:

1.  `get_staged_diff()` captures the output of `git diff --staged`.

2.  `encode_hash_with_registry()` hashes the diff bytes with a random
    hash algorithm and encodes the hash through a random dictionary.
    This produces the commit title.

3.  `encode_compress_with_registry()` compresses the commit message with
    a random compression algorithm and encodes the compressed bytes
    through a second random dictionary. This produces the commit body.

4.  A footer tag is assembled:
    `[hash_algo:title_dict|compress_algo:body_dict]`.

5.  If both dictionaries are the same (dejavu), the marker `whoa.` is
    appended.

6.  All parts are validated for unsafe characters (NUL, C0/C1 controls).
    If validation fails, the entire encode is retried with freshly
    rolled dictionaries, up to 5 attempts.

7.  `git_commit()` writes the three-part message (title, body, footer)
    as the commit message.

The `EncodedCommit` struct captures all parts:

``` rust
pub struct EncodedCommit {
    pub title: String,
    pub body: String,
    pub footer: String,
    pub dejavu: bool,
    pub title_dict: String,
    pub body_dict: String,
}
```

### `handlers/mod.rs` -- the decoding pipeline

`mx log` uses a four-phase architecture:

1.  **Parse** -- raw CLI arguments (received as trailing varargs) are
    parsed into a structured `LogOptions` with separate fields for
    count, display mode (`Compact`, `Full`, `Oneline`, format presets,
    or custom format string), diff mode (`None`, `Stat`, `ShortStat`,
    `Patch`), decorate preference, and filter arguments. Custom
    `--format` strings and `--graph` are detected here and trigger a
    passthrough to raw `git log` with a stderr note.

2.  **Harvest** -- a single `git log` call with a structured format
    string retrieves commit metadata (full hash, short hash,
    decorations, parents, author, date, committer, commit date, subject,
    body). Each commit body is decoded via `try_decode_commit_body()`.

3.  **Attach diffs** -- if a diff mode was requested, a second `git log`
    call retrieves the diff output. Each diff block is matched to its
    corresponding commit by hash and attached as a string field.

4.  **Render** -- the display mode selects a renderer. Each renderer
    prints the decoded message with the appropriate header format,
    followed by any attached diff output.

The `-n`/`--count` and `--full` flags are not clap-managed -- they are
parsed internally from the trailing varargs, following the same pattern
as `mx show`.

`try_decode_commit_body()` scans for the last footer-shaped line
(validated against the known compression algorithm vocabulary).
Everything above the footer is the encoded payload; everything below is
trailing content (dejavu markers, user-appended notes).
`commit::decode_body()` looks up the dictionary from the footer,
decodes, and decompresses. The scan uses a "last wins" heuristic: if
multiple footer-shaped lines appear (e.g., from a user-amended commit
that quotes a prior footer), the last one is used.

`handle_show()` uses a two-pass approach: Pass 1 retrieves commit
metadata and the encoded message (with `--no-patch`), decodes it, and
prints the header. Pass 2 retrieves the diff output (with `--format=""`)
and streams it as-is. Passthrough detection skips decoding entirely for
`ref:path` syntax (file content viewing) and `--format`/`--pretty`
(user-controlled output).

### `commit.rs` -- PR merge encoding

`mx pr merge` follows the same pipeline but sources the diff from
`gh pr diff` and the message from the PR title and body. The encoded
message is passed to `gh pr merge --subject ... --body ...`.

### `knowledge.rs` -- content hashing

`KnowledgeEntry` uses base-d's hash encoding for content hashing (via
`base_d::hash` and `base_d::encode`), producing the `content_hash` field
used for change detection during seed/import operations.

## Testing patterns

### The `_with()` seam

The primary testing pattern in the codebase is the `_with()` test seam
described in Path management. Any function that reads from the
environment or calls `dirs::home_dir()` is split into:

- A `_with(...)` variant that takes all external inputs as parameters
  (pure function).

- A public wrapper that reads the environment and delegates.

Tests call the `_with` variant directly, avoiding all process-global
state. This means the test suite runs safely in parallel without
`#[serial]` except for the handful of tests that must observe the public
wrapper's env-var behavior.

### `serial_test`

Tests that mutate process environment (e.g., clearing
`MX_CLAUDE_PROJECTS_DIR` to observe the default fallback) are marked
with `#[serial]` from the `serial_test` crate. These are a small
minority -- the `_with()` pattern eliminates the need for serialization
in most cases.

### `proptest`

The `proptest` crate is available in dev-dependencies for property-based
testing. It is used selectively where input domains are large (e.g.,
Unicode boundary testing for `safe_truncate`).

### Round-trip encoder tests

The `try_decode_commit_body_tests` module in `handlers/mod.rs` tests the
encode-decode round trip by calling `encode_commit()` with known inputs
and verifying that `try_decode_commit_body()` recovers the original
message. An `encode_until` helper retries encoding with different random
dictionaries until a predicate is satisfied (e.g., dejavu vs.
non-dejavu), filtering out dictionary/codec pairings that produce unsafe
output or fail round-trip.

### KV store tests

The KV engine uses the same `_with()` approach for path resolution
(`resolve_kv_path_with`). Store tests operate on temp directories and
never touch the user's real `~/.mx/kv/` state.

### SurrealDB integration tests

The `surreal_db/tests.rs` module contains integration tests that open a
temporary embedded SurrealKV database, apply the schema, and exercise
the full `KnowledgeStore` trait surface. Each test gets an isolated
database directory.
