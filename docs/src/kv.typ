#import "lib.typ": *

#page-header("KV Store", "Fast local key-value state per agent.")

The KV subsystem gives each agent a lightweight, schema-driven key-value store
for operational state that needs to be fast and local. Counters, strings, lists,
timestamped history, and structured state fields -- all backed by a TOML schema
file and a JSON data file. No networking, no database. Reads and writes are
direct file operations with atomic saves (serialize to tmp, fsync, rename).

Use KV for state that lives within a single agent session or across sessions:
build counters, track decisions as a history log, maintain a todo list, or
store the current goal as a string. For cross-agent knowledge that needs search,
tagging, and relationships, use #link("memory.html")[Memory] instead.

== Concepts

=== Data types

Every key has a type declared in the schema. Five types are supported:

/ string: A single text value. Has an optional `default`.
/ counter: An integer with optional `min`, `max`, and `default`. Clamped on every write.
/ history: A timestamped append-only log. Newest entries first. Has an optional `max_entries` cap that drops the oldest entries on overflow.
/ list: An ordered collection with timestamps. Supports push and pop. Also has an optional `max_entries` cap.
/ state: A structured record with named fields. Fields are declared in the schema and validated on write.

=== Schema files

Each agent has a TOML schema file that declares every valid key, its type,
and any constraints. The schema lives at:

```
$MX_HOME/kv/schema/{agent}.toml
```

The data file (JSON, auto-created on first write) lives at:

```
$MX_HOME/kv/data/{agent}.json
```

The active agent is determined by the `MX_CURRENT_AGENT` environment variable.

You can override the paths with `MX_KV_SCHEMA` and `MX_KV_DATA` environment
variables. Both support an `{agent}` placeholder that expands to the current
agent name.

=== Schema format

A schema file is TOML with a `[keys.<name>]` section per key:

```toml
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

[keys.todos]
type = "list"
max_entries = 20

[keys.context]
type = "state"
fields = ["goal", "phase", "blocker"]
```

Schema fields:

/ `type`: Required. One of `string`, `counter`, `history`, `list`, `state`.
/ `default`: Optional. Initial value for string and counter types.
/ `min`: Optional. Minimum value for counters (clamped, never errors).
/ `max`: Optional. Maximum value for counters (clamped, never errors).
/ `max_entries`: Optional. Maximum entries for history and list types. Oldest entries are dropped when exceeded.
/ `fields`: Optional. List of valid field names for state types. Writes to unlisted fields are rejected.

=== Agent keying

All KV operations require `MX_CURRENT_AGENT` to be set. Each agent gets its
own schema and data file -- there is no cross-agent state leakage. Two agents
can define entirely different schemas with different keys.

=== Exit codes

KV commands use structured exit codes for scripting:

/ `0`: Success.
/ `1`: Key not found (or no data yet for that key).
/ `2`: Type mismatch (e.g., `inc` on a string key).
/ `3`: Schema file not found.

== Basic operations

#command(
  "mx kv get <key>",
  [Get the current value for a key. Prints the raw value for strings and
  counters. For history and list types, prints all entries with IDs and
  timestamps. For state types, prints fields as JSON.],
  flags: (
    ([`--memory`], [flag], [Resolve and display any linked memory entry]),
  ),
  examples: (
    "mx kv get session_goal",
    "mx kv get builds",
    "mx kv get decisions",
    "mx kv get context --memory",
  ),
)

#command(
  "mx kv set <key> <value> [field_value]",
  [Set a value for a string, counter, or state key.

  For *string* keys: `mx kv set <key> <value>` sets the value directly.

  For *counter* keys: `mx kv set <key> <value>` parses the value as an integer
  and clamps to min/max.

  For *state* keys: `mx kv set <key> <field> <value>` sets a single field.
  The field name must be declared in the schema.],
  flags: (
    ([`--memory <kn-id>`], [string], [Link a memory entry (kn- ID) to this key, or `""` to clear]),
  ),
  examples: (
    "mx kv set session_goal \"ship the docs\"",
    "mx kv set builds 0",
    "mx kv set context goal \"finish KV docs\"",
    "mx kv set context phase \"writing\"",
    "mx kv set decisions --memory kn-abc123",
    "mx kv set decisions --memory \"\"",
  ),
)

#command(
  "mx kv keys",
  [List all keys defined in the schema with their types. Output is
  two columns: key name (left-aligned, 30 chars) and type.],
  examples: (
    "mx kv keys",
  ),
)

== Counters

#command(
  "mx kv inc <key>",
  [Increment a counter key. Returns the new value after incrementing.
  The result is clamped to the schema's min/max bounds -- it never errors
  on overflow, it just stops at the limit.],
  flags: (
    ([`--by <n>`], [integer], [Amount to increment by (default: 1)]),
  ),
  examples: (
    "mx kv inc builds",
    "mx kv inc builds --by 5",
  ),
)

#command(
  "mx kv dec <key>",
  [Decrement a counter key. Returns the new value after decrementing.
  Like `inc`, the result is clamped to schema bounds.],
  flags: (
    ([`--by <n>`], [integer], [Amount to decrement by (default: 1)]),
  ),
  examples: (
    "mx kv dec retries",
    "mx kv dec retries --by 3",
  ),
)

== Lists & History

History and list types both store timestamped entries with auto-assigned IDs.
The difference is semantic: history is append-only (newest first, no pop),
while lists support push/pop and maintain insertion order.

Both types support `push`, `last`, `search`, `count`, `random`, and `remove`.
Only lists support `pop`. Only history supports `since` (time-based queries).

=== push

#command(
  "mx kv push <key> <value>",
  [Push a value onto a history or list key. The entry is automatically
  timestamped and assigned a unique ID.

  For *history* keys, new entries are inserted at the front (newest first).
  If the key has a `max_entries` schema constraint, the oldest entries are
  truncated after the push.

  For *list* keys, new entries are appended to the end. The same
  `max_entries` truncation applies, dropping from the front.],
  examples: (
    "mx kv push decisions \"chose Typst for docs\"",
    "mx kv push todos \"write tests for kv handler\"",
  ),
)

=== pop

#command(
  "mx kv pop <key>",
  [Pop the last item from a list key. Prints the removed entry with its ID,
  value, and timestamp. Returns silently if the list is empty.

  Only works on list types. History keys are append-only and do not support
  pop.],
  examples: (
    "mx kv pop todos",
  ),
)

=== last

#command(
  "mx kv last <key>",
  [Get the last N entries from a history or list key. Entries are printed
  with their ID, value, and timestamp.

  For history keys, "last" means the most recent (entries are stored newest
  first). For list keys, "last" means the tail of the list.

  Time-range flags narrow the result set before `--count` is applied. See
  #link(<time-range-queries>)[Time-range queries] for details and examples.],
  flags: (
    ([`--count <n>`], [integer], [Number of entries to return (default: 1)]),
    ([`--memory`], [flag], [Resolve and display any linked memory entry]),
    ([`--day <YYYY-MM-DD>`], [string], [Entries from a specific day (UTC)]),
    ([`--month <YYYY-MM>`], [string], [Entries from a specific month (UTC)]),
    ([`--week <YYYY-Www>`], [string], [Entries from an ISO week, Monday to Sunday]),
    ([`--from <YYYY-MM-DD>`], [string], [Start of date range, inclusive (UTC)]),
    ([`--to <YYYY-MM-DD>`], [string], [End of date range, inclusive (UTC)]),
    ([`--since <relative-or-iso>`], [string], [Filter entries since a relative time (`30d`, `1w`, `2h`, `30m`) or ISO-8601 timestamp]),
  ),
  examples: (
    "mx kv last decisions",
    "mx kv last decisions --count 5",
    "mx kv last todos --count 3 --memory",
    "mx kv last shipped --day 2026-04-25",
    "mx kv last shipped --month 2026-04",
    "mx kv last shipped --month 2026-04 --count 5",
    "mx kv last shipped --since 1w",
  ),
)

=== since

#command(
  "mx kv since <key> <timeref>",
  [Get history entries since a time reference. Only works on history keys.

  The time reference can be relative or absolute:
  - Relative: `30m` (minutes), `1h` (hours), `7d` (days), `2w` (weeks)
  - Absolute: ISO-8601 format (e.g., `2025-01-15T10:00:00Z`)

  Entries are printed with their ID, value, and timestamp.],
  flags: (
    ([`--memory`], [flag], [Resolve and display any linked memory entry]),
  ),
  examples: (
    "mx kv since decisions 1h",
    "mx kv since decisions 7d",
    "mx kv since decisions 2w --memory",
    "mx kv since decisions 2025-01-15T10:00:00Z",
  ),
)

=== search

#command(
  "mx kv search <key> <query>",
  [Search entries in a list or history by case-insensitive substring match.
  Prints matching entries with their ID, value, and timestamp.

  Time-range flags narrow the search to entries within the specified period.
  See #link(<time-range-queries>)[Time-range queries] for details.],
  flags: (
    ([`--memory`], [flag], [Resolve and display any linked memory entry]),
    ([`--day <YYYY-MM-DD>`], [string], [Search within a specific day (UTC)]),
    ([`--month <YYYY-MM>`], [string], [Search within a specific month (UTC)]),
    ([`--week <YYYY-Www>`], [string], [Search within an ISO week, Monday to Sunday]),
    ([`--from <YYYY-MM-DD>`], [string], [Start of date range, inclusive (UTC)]),
    ([`--to <YYYY-MM-DD>`], [string], [End of date range, inclusive (UTC)]),
    ([`--since <relative-or-iso>`], [string], [Search since a relative time (`30d`, `1w`, `2h`, `30m`) or ISO-8601 timestamp]),
  ),
  examples: (
    "mx kv search decisions \"typst\"",
    "mx kv search todos \"test\"",
    "mx kv search shipped \"feature\" --month 2026-04",
    "mx kv search shipped \"feature\" --since 30d",
  ),
)

=== count

#command(
  "mx kv count <key> [value]",
  [Count entries in a list or history. Without a value filter, prints the
  total count. With a value filter, prints the matched count, total, and
  percentage.

  Unfiltered output: `<count>` or `<count> (latest: <timestamp>)`.

  Filtered output: `<matched>/<total> (<pct>%) --- latest: <timestamp>`.

  The percentage display makes it easy to gauge ratios at a glance -- for
  example, what fraction of your decisions mentioned a particular topic.

  Time-range flags restrict the count to entries within the specified period.
  See #link(<time-range-queries>)[Time-range queries] for details.],
  flags: (
    ([`--day <YYYY-MM-DD>`], [string], [Count within a specific day (UTC)]),
    ([`--month <YYYY-MM>`], [string], [Count within a specific month (UTC)]),
    ([`--week <YYYY-Www>`], [string], [Count within an ISO week, Monday to Sunday]),
    ([`--from <YYYY-MM-DD>`], [string], [Start of date range, inclusive (UTC)]),
    ([`--to <YYYY-MM-DD>`], [string], [End of date range, inclusive (UTC)]),
    ([`--since <relative-or-iso>`], [string], [Count since a relative time (`30d`, `1w`, `2h`, `30m`) or ISO-8601 timestamp]),
  ),
  examples: (
    "mx kv count decisions",
    "mx kv count decisions \"typst\"",
    "mx kv count todos \"blocked\"",
    "mx kv count shipped --day 2026-05-07",
    "mx kv count shipped --from 2026-04-01 --to 2026-04-15",
    "mx kv count shipped --since 1w",
  ),
)

=== random

#command(
  "mx kv random <key>",
  [Get N random entries from a history or list key. Entries are printed
  with their ID, value, and timestamp.

  Useful for inspiration (pick a random idea), spot-checking (sample from
  a large history), or building variety into automated workflows.

  When fewer entries are available than requested, all matching entries are
  returned and a note is printed to stderr. If a time range is specified,
  entries are filtered first, then random sampling is applied to the
  filtered set.],
  flags: (
    ([`--count <n>`], [integer], [Number of random entries to return (default: 1, must be >= 1)]),
    ([`--memory`], [flag], [Resolve and display any linked memory entry]),
    ([`--day <YYYY-MM-DD>`], [string], [Sample from entries on a specific day (UTC)]),
    ([`--month <YYYY-MM>`], [string], [Sample from entries in a specific month (UTC)]),
    ([`--week <YYYY-Www>`], [string], [Sample from entries in an ISO week, Monday to Sunday]),
    ([`--from <YYYY-MM-DD>`], [string], [Start of date range, inclusive (UTC)]),
    ([`--to <YYYY-MM-DD>`], [string], [End of date range, inclusive (UTC)]),
    ([`--since <relative-or-iso>`], [string], [Sample from entries since a relative time (`30d`, `1w`, `2h`, `30m`) or ISO-8601 timestamp]),
  ),
  examples: (
    "mx kv random shipped",
    "mx kv random shipped --count 5",
    "mx kv random ideas --count 1",
    "mx kv random shipped --count 3 --since 30d",
    "mx kv random decisions --month 2026-04 --count 3",
  ),
)

=== remove

#command(
  "mx kv remove <key> [value]",
  [Remove entries from a list or history by value substring or by numeric ID.
  You must provide either a value substring or `--id`.

  By default, only the first match is removed. Use `--all` to remove every
  matching entry.],
  flags: (
    ([`--id <n>`], [integer], [Remove the entry with this specific ID]),
    ([`--all`], [flag], [Remove all matching entries (default: first match only)]),
  ),
  examples: (
    "mx kv remove todos \"write tests\"",
    "mx kv remove todos --id 7",
    "mx kv remove decisions \"typo\" --all",
  ),
)

== Time-range queries <time-range-queries>

The `last`, `search`, `count`, and `random` subcommands accept time-range
flags that filter entries by their timestamp before any other processing. This
lets you answer questions like "what did I ship last Tuesday?" or "how many
decisions were recorded in April?" without scanning the full history.

=== Available flags

All time-range flags are mutually exclusive -- you can use one shorthand
(`--day`, `--month`, `--week`, `--since`) or one explicit range
(`--from`/`--to`), but not both.

#table(
  columns: (auto, auto, 1fr),
  table.header([*Flag*], [*Format*], [*Selects*]),
  [`--day`], [`YYYY-MM-DD`], [All entries from that calendar day (00:00 to 23:59 UTC)],
  [`--month`], [`YYYY-MM`], [All entries from that calendar month (first day to last day, UTC)],
  [`--week`], [`YYYY-Www`], [All entries from that ISO week (Monday 00:00 to Sunday 23:59 UTC)],
  [`--since`], [relative or ISO-8601], [All entries from the given point in time until now. Relative formats: `30d` (days), `1w` (weeks), `2h` (hours), `30m` (minutes). Also accepts full ISO-8601 timestamps.],
  [`--from`], [`YYYY-MM-DD`], [Start of range, inclusive (midnight UTC). Can be used alone (implies "to now")],
  [`--to`], [`YYYY-MM-DD`], [End of range, inclusive (end of day UTC). Can be used alone (implies "from the beginning")],
)

All dates are interpreted as UTC. The `--to` date is inclusive -- entries from
any time on that day are included.

=== Interaction with `--count`

When both a time range and `--count` are specified, the time range is applied
first, then `--count` limits the result. This applies to both `last` (which
takes the N most recent from the filtered set) and `random` (which samples N
entries from the filtered set).

```bash
# The 5 most recent entries from April 2026
mx kv last shipped --month 2026-04 --count 5

# 3 random entries from the last 30 days
mx kv random shipped --since 30d --count 3
```

=== Examples

```bash
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

=== Relationship to `since` subcommand

The `since` subcommand (`mx kv since <key> <timeref>`) is a standalone command
that returns all history entries since a time reference. It only works on
history keys and predates the time-range flag system.

The `--since` flag brings relative time filtering to all time-range-aware
subcommands (`last`, `search`, `count`, `random`) and works on both history
and list types. It accepts the same relative formats (`30d`, `1w`, `2h`, `30m`)
and ISO-8601 timestamps.

Use the `since` subcommand when you want a quick "everything since X" dump
from a history key. Use the `--since` flag when you want to combine relative
time filtering with other operations like counting, searching, or random
sampling, or when you need it on a list key.

#note[Time-range flags (`--day`, `--month`, `--week`, `--since`,
`--from`/`--to`) are available on `last`, `search`, `count`, and `random`.
The `since` subcommand is unchanged and continues to work for history keys.]

== Management

#command(
  "mx kv dump",
  [Dump all KV state. Defaults to JSON output (the full data file, pretty-
  printed). Compact format shows one line per key in `key=value` notation,
  designed for embedding in wake prompts or status lines.

  Compact format examples:
  - Counters: `builds=42`
  - Strings: `session_goal=ship the docs`
  - History: `decisions=[chose Typst\@14:30,fixed bug\@13:15]`
  - Lists: `todos=[write tests\@14:30,review PR\@13:15]`
  - State: `context={finish KV docs,writing,}`
  - Memory links appended: `decisions=[...](kn-abc123)`],
  flags: (
    ([`--format <fmt>`], [enum], [Output format: `json` (default) or `compact`]),
    ([`--memory`], [flag], [Resolve and display all linked memory entries]),
  ),
  examples: (
    "mx kv dump",
    "mx kv dump --format compact",
    "mx kv dump --memory",
  ),
)

#command(
  "mx kv reset <key>",
  [Reset a key to its schema default value. Counters return to their default
  (or 0). Strings return to their default (or empty). History and list keys
  are cleared to empty. State keys reset all fields to empty strings.],
  examples: (
    "mx kv reset builds",
    "mx kv reset decisions",
    "mx kv reset context",
  ),
)

== Memory linking

History, list, and state keys can be linked to a memory graph entry via the
`--memory` flag. This creates a pointer from the KV key to a knowledge entry
(a `kn-` ID), bridging fast local state with the persistent knowledge graph.

When a memory link is set, commands that read the key (`get`, `last`, `since`,
`search`, `random`, `dump`) can resolve the link with `--memory`, which fetches
the linked entry from SurrealDB and prints its title, category, and body.

=== Setting a memory link

```bash
# Link a key to a memory entry
mx kv set decisions --memory kn-abc123

# Clear a memory link (pass empty string)
mx kv set decisions --memory ""
```

Memory links are stored in the JSON data file alongside the key's entries.
They survive resets -- `mx kv reset` clears the data but preserves the memory
pointer.

=== Resolving memory links

```bash
# Read a key and show its linked memory entry
mx kv get decisions --memory

# Show the last 5 entries plus linked memory
mx kv last decisions --count 5 --memory

# Dump everything with all memory links resolved
mx kv dump --memory
```

Resolution connects to the memory store (SurrealDB). If the store is
unavailable or the linked entry has been deleted, a warning is printed to
stderr but the KV data is still shown. KV data is always primary -- memory
links are supplementary context.

#note[Memory links are only available on history, list, and state types.
String and counter keys do not support `--memory`.]
