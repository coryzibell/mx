#import "lib.typ": *

#page-header("KV Store", "Fast local key-value state per agent.")

The KV subsystem gives each agent a lightweight, schema-driven key-value store
for operational state that needs to be fast and local. Counters, strings, lists,
timestamped history, and structured state fields -- all backed by a TOML schema
file and a JSON data file. No networking, no database. Reads and writes are
direct file operations with atomic saves (serialize to tmp, fsync, rename).
History and list entries can carry structured JSON data for queryable metadata.

Use KV for state that lives within a single agent session or across sessions:
build counters, track decisions as a history log, maintain a todo list, or
store the current goal as a string. For cross-agent knowledge that needs search,
tagging, and relationships, use #link("memory.html")[Memory] instead.

== Concepts

=== Data types

Every key has a type declared in the schema. Five types are supported:

/ string: A single text value. Has an optional `default`.
/ counter: An integer with optional `min`, `max`, and `default`. Clamped on every write.
/ history: A timestamped append-only log. Newest entries first. Has an optional `max_entries` cap that drops the oldest entries on overflow. Entries can carry optional structured JSON data.
/ list: An ordered collection with timestamps. Supports push and pop. Also has an optional `max_entries` cap. Entries can carry optional structured JSON data.
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

[keys.ideas]
type = "list"

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
/ `max_entries`: Optional. Maximum entries for history and list types. Oldest entries are dropped when exceeded. Omit to allow unbounded growth.
/ `fields`: Optional. List of valid field names for state types. Writes to unlisted fields are rejected.

=== Agent keying

All KV operations require `MX_CURRENT_AGENT` to be set. Each agent gets its
own schema and data file -- there is no cross-agent state leakage. Two agents
can define entirely different schemas with different keys.

=== Exit codes

KV commands use structured exit codes for scripting:

/ `0`: Success.
/ `1`: Key not found (or no data yet for that key).
/ `2`: Type mismatch (e.g., `inc` on a string key, or `get --id` on a non-history/list key).
/ `3`: Schema file not found.
/ `4`: Invalid input (e.g., non-numeric ID in `--id` spec, reversed range, empty spec).

== Basic operations

#command(
  "mx kv get <key>",
  [Get the current value of a key, or look up specific entries by ID.

  Without `--id`, prints the full current value: raw text for strings and
  counters, all entries with IDs and timestamps for history and list types,
  and fields as JSON for state types.

  With `--id`, retrieves specific entries from a history or list by their
  numeric ID. Three ID formats are supported:

  / Single ID: `--id 35` -- returns exactly one entry.
  / Range: `--id 35-64` -- returns all entries with IDs 35 through 64 inclusive. Maximum range size is 10,000 entries.
  / Comma-separated: `--id 1,5,12` -- returns the listed entries. Duplicates are ignored.

  Formats cannot be combined (e.g., `--id 1,5-10` is not valid). If any
  requested IDs are not found, a note listing the missing IDs is printed to
  stderr. The found entries are still printed to stdout.

  The `--id` flag only works on history and list types. Using it on a
  string, counter, or state key returns exit code 2 (type mismatch).
  Parse failures (non-numeric IDs, reversed ranges, empty specs) return
  exit code 4 (invalid input).],
  flags: (
    ([`--id <spec>`], [string], [Entry ID (`35`), range (`35-64`), or comma-separated IDs (`1,5,12`)]),
    ([`--memory`], [flag], [Resolve and display any linked memory entry]),
  ),
  examples: (
    "mx kv get session_goal",
    "mx kv get builds",
    "mx kv get decisions",
    "mx kv get context --memory",
    "mx kv get shipped --id 35",
    "mx kv get shipped --id 35-64",
    "mx kv get shipped --id 1,5,12,35",
    "mx kv get shipped --id 35 --memory",
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

Both types support `push`, `last`, `search`, `count`, `random`, `remove`, and
entry lookup by ID via `get --id`. Both support structured data on entries
(`--data` on push) and structured data filtering (`--where` on queries).
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
  `max_entries` truncation applies, dropping from the front.

  Use `--data` to attach a JSON object to the entry. The data is stored
  alongside the value and timestamp, and is displayed inline in output.
  See #link(<structured-data>)[Structured data] for details and query
  examples.],
  flags: (
    ([`--data <json>`], [string], [Attach a JSON object to the entry. Must be a valid JSON object (not an array, string, or other type).]),
  ),
  examples: (
    "mx kv push decisions \"chose Typst for docs\"",
    "mx kv push todos \"write tests for kv handler\"",
    "mx kv push projects \"palmtop DSI fix\" --data '{\"tags\":[\"palmtop\",\"i915\"],\"status\":\"active\"}'",
    "mx kv push shipped \"v0.1.156\" --data '{\"pr\":305,\"scope\":\"kv\"}'",
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
  #link(<time-range-queries>)[Time-range queries] for details and examples.

  The `--where` flag filters entries by structured data fields. Multiple
  `--where` flags are ANDed. See #link(<structured-data>)[Structured data]
  for filtering semantics.],
  flags: (
    ([`--count <n>`], [integer], [Number of entries to return (default: 1)]),
    ([`--memory`], [flag], [Resolve and display any linked memory entry]),
    ([`--where <key=value>`], [string], [Filter by structured data field (repeatable, ANDed). Top-level fields only.]),
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
    "mx kv last projects --where status=active",
    "mx kv last projects --where status=active --count 3",
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
  "mx kv search <key> [query]",
  [Search entries in a list or history by case-insensitive substring match
  and/or structured data filters. Prints matching entries with their ID,
  value, timestamp, and any attached data.

  The text query is optional when `--where` filters are provided. You can
  search by text alone, by structured data alone, or by both. At least one
  of a text query or `--where` filter must be given.

  Multiple `--where` flags are ANDed. See
  #link(<structured-data>)[Structured data] for filtering semantics.

  Time-range flags narrow the search to entries within the specified period.
  See #link(<time-range-queries>)[Time-range queries] for details.],
  flags: (
    ([`--memory`], [flag], [Resolve and display any linked memory entry]),
    ([`--where <key=value>`], [string], [Filter by structured data field (repeatable, ANDed). Top-level fields only.]),
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
    "mx kv search projects --where status=active",
    "mx kv search projects \"DSI\" --where status=active",
    "mx kv search projects --where tags=palmtop --where status=active",
  ),
)

=== count

#command(
  "mx kv count <key> [value]",
  [Count entries in a list or history. Without a value filter or `--where`,
  prints the total count. With a value filter, `--where`, or both, prints
  the matched count, total, and percentage.

  Unfiltered output: `<count>` or `<count> (latest: <timestamp>)`.

  Filtered output: `<matched>/<total> (<pct>%) --- latest: <timestamp>`.

  The percentage display makes it easy to gauge ratios at a glance -- for
  example, what fraction of your decisions mentioned a particular topic,
  or how many entries have `status=active` in their structured data.

  Multiple `--where` flags are ANDed. See
  #link(<structured-data>)[Structured data] for filtering semantics.

  Time-range flags restrict the count to entries within the specified period.
  See #link(<time-range-queries>)[Time-range queries] for details.],
  flags: (
    ([`--where <key=value>`], [string], [Filter by structured data field (repeatable, ANDed). Top-level fields only.]),
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
    "mx kv count projects --where status=active",
    "mx kv count projects --where status=active --since 30d",
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
  returned and a note is printed to stderr. If a time range or `--where`
  filter is specified, entries are filtered first, then random sampling is
  applied to the filtered set.

  Multiple `--where` flags are ANDed. See
  #link(<structured-data>)[Structured data] for filtering semantics.],
  flags: (
    ([`--count <n>`], [integer], [Number of random entries to return (default: 1, must be >= 1)]),
    ([`--memory`], [flag], [Resolve and display any linked memory entry]),
    ([`--where <key=value>`], [string], [Filter by structured data field (repeatable, ANDed). Top-level fields only.]),
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
    "mx kv random projects --where status=active --count 3",
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

== Structured data <structured-data>

History and list entries can carry structured JSON data alongside their text
value. This turns each entry from a plain string into a string with queryable
metadata -- tags, status, priority, or any key-value pairs relevant to the
domain.

=== Pushing data

Use `--data` on `push` to attach a JSON object to the entry:

```bash
mx kv push projects "palmtop DSI fix" \
  --data '{"tags":["palmtop","i915"],"status":"active"}'

mx kv push shipped "v0.1.156" \
  --data '{"pr":305,"scope":"kv"}'
```

The data must be a valid JSON object. Arrays, strings, numbers, and other
non-object JSON types are rejected. If `--data` is omitted, the entry has no
structured data (backward compatible with all existing entries).

=== Output format

Entries with structured data display the JSON inline after the timestamp:

```
42: palmtop DSI fix (2026-05-08T14:30:00Z) {"tags":["palmtop","i915"],"status":"active"}
43: display rotation patch (2026-05-08T15:00:00Z)
```

Entries without data look exactly as they always have. The data suffix appears
on all commands that display entries: `get`, `last`, `search`, `since`, `pop`,
`random`, and `dump`.

=== Filtering with `--where`

The `--where` flag queries entries by their structured data fields. It is
available on `search`, `last`, `random`, and `count`.

```bash
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

=== Matching semantics

Each `--where` clause has the form `key=value` (split on the first `=`). The
match is evaluated against the top-level fields of the entry's JSON data:

/ String field: The field value must equal the clause value exactly.
/ Array field: The array must contain a string element equal to the clause value.
/ Number field: The field's string representation must equal the clause value (e.g., `--where pr=305`).
/ Boolean field: Matches against `true` or `false` as strings.
/ Missing field: Does not match. Entries without data never match any `--where` clause.

Only top-level fields are supported. Dot-path traversal (e.g.,
`--where nested.field=value`) is not available.

When multiple `--where` clauses are given, ALL must match (AND logic). There is
no OR operator -- use separate queries if you need union semantics.

=== Combining with other filters

The `--where` flag composes with both text queries and time-range flags. All
filters are applied together:

```bash
# Text + where + time range: all three must match
mx kv search projects "DSI" --where status=active --since 30d
```

Filter application order: time range first, then `--where`, then text query.
The `--count` limit is applied last.

=== Backward compatibility

Structured data is fully backward compatible. Existing data files written
before this feature was added continue to work without migration. Entries
without data are simply treated as having no structured fields -- they will
not match any `--where` clause, but they are otherwise unaffected.

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
