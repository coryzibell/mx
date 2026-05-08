#import "lib.typ": *

#page-header("log", "Decoded git log with full git-log parity.")

== Overview

`mx log` decodes the commit history that `mx commit` encodes. Because
#link("commit.html")[`mx commit`] compresses and encodes every commit message
through a randomly selected #link("base-d.html")[base-d] dictionary, raw
`git log` output is unreadable glyphs. `mx log` reverses the encoding and
displays your original messages.

The command has full parity with `git log`. Every display flag, format preset,
and filter option you know from git works here, with transparent decoding
applied to every commit message. If you know `git log`, you know `mx log`.

The round-trip works because each encoded commit carries a footer tag that
identifies the dictionary and compression algorithm used. `mx log` reads the
footer, looks up the dictionary, decompresses the body, and prints the
human-readable message.

#note[Always use `mx log` to read commit history. Raw `git log` will show
encoded noise.]

== Basic usage

Show the last 10 commits (the default):

```bash
mx log
```

Show the last 3 commits using the `-N` shorthand:

```bash
mx log -3
```

Show the last 20 commits:

```bash
mx log -n 20
```

Show full commit details (hash, author, date, decoded message):

```bash
mx log --full
```

== Output formats

`mx log` supports several display modes. All of them decode encoded messages
transparently.

=== Compact (default)

One line per commit: short hash and decoded subject, truncated to 72
characters. This is the default when no display flag is given.

```
a1b2c3d fix session export crash on empty JSONL
e4f5g6h add retry logic to sync pull
```

=== Full (`--full`)

Full hash, author, date, and decoded message, styled like `git log`. If the
commit has trailing post-footer content (e.g. a dejavu marker), it is rendered
in dim text beneath the decoded message. This is an mx-specific display mode
preserved for backward compatibility.

=== Oneline (`--oneline`)

One line per commit with short hash, ref decorations (branch/tag names), and
decoded subject. Matches git's `--oneline` output but with decoded messages.

```
a1b2c3d (HEAD -> main, origin/main) fix session export crash on empty JSONL
e4f5g6h add retry logic to sync pull
```

Use `--no-decorate` to suppress the ref decorations:

```bash
mx log --oneline --no-decorate
```

=== Format presets

The standard git format presets all work with decoded messages:

- `--format=short` -- commit hash, author, decoded subject.
- `--format=medium` -- commit hash, author, date, decoded subject and body.
  This matches git's default format.
- `--format=full` -- commit hash, author, committer, decoded subject and body.
- `--format=fuller` -- commit hash, author with date, committer with date,
  decoded subject and body.

These can also be specified with `--pretty`:

```bash
mx log --pretty=fuller -3
```

== Diff output

Attach diff information below each decoded commit header:

```bash
# Diffstat (files changed, insertions, deletions)
mx log --stat

# One-line summary of changes
mx log --shortstat

# Full patch output
mx log -p
mx log --patch
```

Diff flags compose with any display mode:

```bash
mx log --oneline --stat -5
mx log --format=short -p -3
```

== Filtering

All git log filter flags pass through to the underlying `git log` call. This
lets you filter by path, author, date range, or any other git-log option:

```bash
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

== Ref decorations

By default, ref decorations (branch names, tags, `HEAD`) are shown in
`--oneline` mode. You can control this explicitly:

```bash
mx log --oneline --decorate       # show decorations (default)
mx log --oneline --no-decorate    # hide decorations
```

== Count

Several syntaxes are accepted for limiting the number of commits:

```bash
mx log -3              # -N shorthand (like git log -3)
mx log -n 5            # -n with space
mx log -n5             # -n without space
mx log --max-count=7   # git's long form
```

When no count is specified, `mx log` defaults to 10 commits. This differs from
`git log` (which defaults to unlimited) and is intentional -- it keeps the
default output concise.

== Passthrough modes

In two cases, `mx log` skips decoding entirely and falls through to raw
`git log` with a stderr note:

=== `--graph`

Graph rendering requires line-level control that the four-phase architecture
cannot replicate without reimplementing git's graph layout. When `--graph` is
present, the command passes through to raw `git log`. A note is printed to
stderr:

```
note: --graph bypasses message decoding
```

=== Custom `--format` strings

When `--format` or `--pretty` is set to a custom format string (anything other
than the named presets `oneline`, `short`, `medium`, `full`, `fuller`), the
command passes through to raw `git log`. A note is printed to stderr:

```
note: custom --format bypasses message decoding
```

In both passthrough modes, the count, diff flags, and filter args are still
forwarded.

== Flags reference

#command("mx log",
  [Display decoded git log. Commits encoded by `mx commit` are decoded back to their original messages. Non-encoded commits pass through unchanged.],
  flags: (
    ([`-N`], [shorthand], [Number of commits to show, as a bare number after the dash. Example: `-3`, `-10`. Equivalent to git's `-N` shorthand.]),
    ([`-n`], [integer], [Number of commits to show. Accepts `-n 5` or `-n5`. Defaults to `10`.]),
    ([`--max-count`], [integer], [Number of commits to show (git's long form). Example: `--max-count=7`.]),
    ([`--full`], [flag], [Show full commit details: full hash, author, date, and complete decoded message. An mx-specific display mode.]),
    ([`--oneline`], [flag], [One line per commit: short hash, ref decorations, decoded subject.]),
    ([`--stat`], [flag], [Show diffstat below each commit.]),
    ([`--shortstat`], [flag], [Show a one-line summary of changes below each commit.]),
    ([`-p`, `--patch`], [flag], [Show the full patch below each commit.]),
    ([`--decorate`], [flag], [Show ref decorations (branch, tag, HEAD). On by default in `--oneline` mode.]),
    ([`--no-decorate`], [flag], [Suppress ref decorations.]),
    ([`--format`], [preset], [Format preset: `short`, `medium`, `full`, `fuller`. Named presets decode messages; custom format strings pass through to raw git.]),
    ([`--pretty`], [preset], [Alias for `--format`.]),
    ([`--graph`], [flag], [Passthrough to raw `git log` with graph rendering. Decoding is skipped.]),
  ),
)

=== Filter passthrough

Any additional arguments not listed above are passed through to the underlying
`git log` call. Common examples:

#table(
  columns: (auto, 1fr),
  table.header([*Argument*], [*Description*]),
  [`--author=<pattern>`], [Filter by author name or email.],
  [`--since=<date>`], [Show commits after a date.],
  [`--until=<date>`], [Show commits before a date.],
  [`--all`], [Show commits from all refs, not just the current branch.],
  [`--reverse`], [Show commits in reverse chronological order.],
  [`-- <path>`], [Filter to commits touching the given path(s).],
)

== Architecture

`mx log` uses a four-phase architecture:

+ *Parse* -- raw CLI arguments are parsed into a structured `LogOptions` with
  separate fields for count, display mode, diff mode, decorate preference, and
  filter arguments. Custom `--format` strings and `--graph` are detected here
  and trigger passthrough.
+ *Harvest* -- a single `git log` call with a structured format string
  retrieves commit metadata (hashes, author, dates, decorations) and the
  encoded message body. Each commit is parsed and decoded.
+ *Attach diffs* -- if `--stat`, `--shortstat`, or `-p` was requested, a
  second `git log` call retrieves the diff output. Each diff block is attached
  to its corresponding commit.
+ *Render* -- the display mode selects a renderer (compact, full, oneline,
  or a format preset). Each renderer prints the decoded message with the
  appropriate header format, followed by any attached diff output.

This architecture ensures that decoding is always applied before rendering, and
that diff output appears in the correct position regardless of the display
format.

== Relationship to mx commit and mx show

`mx commit`, `mx log`, and #link("show.html")[`mx show`] form the encoding
round-trip:

+ `mx commit` compresses your message, encodes it through a random dictionary,
  and writes the encoded result as the git commit body with a footer tag.
+ `mx log` reads the footer tag, reverses the encoding, decompresses, and
  displays your original message across the commit history.
+ `mx show` does the same decoding for individual commits, replacing
  `git show`.

Both `mx log` and `mx show` have full parity with their git counterparts.
Every flag that `git log` or `git show` accepts works with the mx versions,
with transparent decoding applied to encoded messages. Non-encoded commits
(e.g. commits made with raw `git commit`) pass through unchanged.

For the full encoding specification, see #link("commit.html")[commit].
