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

Both types support `push`, `last`, `search`, `count`, and `remove`. Only
lists support `pop`. Only history supports `since` (time-based queries).

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
  first). For list keys, "last" means the tail of the list.],
  flags: (
    ([`--count <n>`], [integer], [Number of entries to return (default: 1)]),
    ([`--memory`], [flag], [Resolve and display any linked memory entry]),
  ),
  examples: (
    "mx kv last decisions",
    "mx kv last decisions --count 5",
    "mx kv last todos --count 3 --memory",
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
  Prints matching entries with their ID, value, and timestamp.],
  flags: (
    ([`--memory`], [flag], [Resolve and display any linked memory entry]),
  ),
  examples: (
    "mx kv search decisions \"typst\"",
    "mx kv search todos \"test\"",
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
  example, what fraction of your decisions mentioned a particular topic.],
  examples: (
    "mx kv count decisions",
    "mx kv count decisions \"typst\"",
    "mx kv count todos \"blocked\"",
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
`search`, `dump`) can resolve the link with `--memory`, which fetches the
linked entry from SurrealDB and prints its title, category, and body.

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
