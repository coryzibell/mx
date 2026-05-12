#import "lib.typ": *

#page-header("State", "Emotional state tensors for agent co-regulation.")

#deprecated[
  `mx state` is deprecated and will be removed in a future release. Use `mx kv`
  with structured data (`--data`) instead. See the #link("kv.html")[KV documentation]
  for the replacement workflow.
]

The state subsystem encodes multidimensional emotional state into compact tensor
strings. A tensor is a vector of float values (each 0.0--1.0) mapped to named
dimensions defined by a schema. Schemas are user-authored YAML files; the
default `tensor` schema ships with six dimensions and self-seeds on first use.

Tensors are designed to be cheap to produce, cheap to parse, and
self-identifying. The wire format embeds the schema ID so a decoder always knows
which schema to load:

```
@state:tensor|0.40|0.50|0.50|0.40|0.55|0.30
```

Each pipe-separated value corresponds to a dimension in the schema's declared
order. Schemas can also define _moods_ -- named landmarks in the state space
with canonical tensor values, optional per-dimension weights, and a tolerance
radius. When encoding, the nearest mood (by weighted Euclidean distance within
tolerance) is derived automatically.

== Table of contents

- #link(<encoding>)[Encoding]
- #link(<decoding>)[Decoding]
- #link(<schemas>)[Listing schemas]
- #link(<moods>)[Moods]
- #link(<info>)[Schema info]
- #link(<schema-format>)[Schema file format]

// ═══════════════════════════════════════════════════════════════════════
// ENCODING
// ═══════════════════════════════════════════════════════════════════════

== Encoding <encoding>

#command(
  "mx state encode",
  [Encode dimensional values into a tensor string. Values can be provided as
  a positional pipe-separated argument, as named dimension key=value pairs, or
  read from a file. If no values are given, the schema's defaults are used.

  The `--guided` flag launches an interactive mode that walks through each
  dimension, showing its name, anchors (low/mid/high descriptions), and default
  value, then prompts for input.],
  flags: (
    ([`<values>`],       [`string`], [Positional: pipe-separated values (e.g., `"0.3|0.2|0.7|0.8|0.4|0.5"`). Conflicts with `--dimensions` and `--file`.]),
    ([`-d, --dimensions`], [`string`], [Named dimension values (e.g., `"entropy=0.4 agency=0.7"`). Dimension names support prefix abbreviation. Conflicts with positional values and `--file`.]),
    ([`-f, --file`],     [`path`],   [Read values from a file. Accepts pipe-separated or one-value-per-line format. Conflicts with positional values.]),
    ([`-s, --schema`],   [`string`], [Schema ID or path. Default: `tensor`.]),
    ([`-g, --guided`],   [`flag`],   [Interactive guided mode -- walks through each dimension with anchor descriptions.]),
    ([`-F, --format`],   [`string`], [Output format: `tensor` (default), `json`, `human`, `bootstrap`.]),
    ([`--runes`],        [`flag`],   [Include rune prefixes in tensor output (e.g., decorative Unicode characters per dimension).]),
  ),
  examples: (
    "# Pipe-separated positional values (six dimensions for default tensor schema)\nmx state encode \"0.4|0.6|0.5|0.3|0.7|0.2\"",
    "# Named dimensions with prefix abbreviation\nmx state encode -d \"entropy=0.4 agency=0.6 temp=0.5 verb=0.3 skep=0.7 humor=0.2\"",
    "# Default values from schema\nmx state encode",
    "# Human-readable output with nearest mood\nmx state encode \"0.4|0.6|0.5|0.3|0.7|0.2\" -F human",
    "# Bootstrap format (self-documenting, with rune legend)\nmx state encode \"0.4|0.6|0.5|0.3|0.7|0.2\" -F bootstrap",
    "# With rune decoration\nmx state encode \"0.4|0.6|0.5|0.3|0.7|0.2\" --runes",
    "# Read from file\nmx state encode -f state-values.txt",
    "# Interactive guided mode\nmx state encode --guided",
    "# Use a custom schema\nmx state encode -s crewu \"0.3|0.2|0.7|0.8|0.4\"",
  ),
)

=== Output formats

/ `tensor`: The default. Prints the encoded tensor string: `@state:tensor|0.40|0.60|...`. With `--runes`, each value is prefixed by its dimension's rune character.
/ `json`: Structured JSON with `schema_id` and `values` fields.
/ `human`: Each dimension printed as `Name: value (anchor description)`, followed by the nearest mood if one falls within tolerance.
/ `bootstrap`: Self-documenting multiline output designed for session bootstrap. Line 1 is the rune-encoded tensor, line 2 is a rune legend mapping runes to dimension IDs, then a blank line, then interpolated anchor descriptions with values.

=== Named dimensions

The `-d` / `--dimensions` flag accepts space-separated `key=value` pairs. Keys
are matched against dimension IDs case-insensitively, with prefix abbreviation:
`temp=0.5` matches `temperature`, `ent=0.4` matches `entropy`. Every dimension
in the schema must be covered -- missing dimensions produce an error listing the
expected set.

=== Value clamping

All values are clamped to the 0.0--1.0 range. Out-of-bounds values are silently
clamped, never rejected.


// ═══════════════════════════════════════════════════════════════════════
// DECODING
// ═══════════════════════════════════════════════════════════════════════

== Decoding <decoding>

#command(
  "mx state decode",
  [Decode a tensor string back to human-readable values. The schema ID is
  embedded in the tensor string (`@state:schema_id|...`) and used to load the
  matching schema automatically. If `--schema` is provided, it overrides the
  embedded ID.

  Input can be provided as a positional argument or piped via stdin.],
  flags: (
    ([`<input>`],      [`string`], [Positional: encoded tensor string (e.g., `"@state:tensor|0.3|0.2|..."`). If omitted, reads from stdin.]),
    ([`-s, --schema`], [`string`], [Schema ID or path. Overrides the schema ID embedded in the tensor string.]),
    ([`-F, --format`], [`string`], [Output format: `human` (default), `json`, `tensor`, `mood`.]),
  ),
  examples: (
    "mx state decode \"@state:tensor|0.40|0.60|0.50|0.30|0.70|0.20\"",
    "# Pipe from another command\necho \"@state:tensor|0.40|0.60|0.50|0.30|0.70|0.20\" | mx state decode",
    "# JSON output\nmx state decode \"@state:tensor|0.40|0.60|0.50|0.30|0.70|0.20\" -F json",
    "# Show only the nearest mood\nmx state decode \"@state:tensor|0.40|0.60|0.50|0.30|0.70|0.20\" -F mood",
    "# Re-encode (roundtrip)\nmx state decode \"@state:tensor|0.40|0.60|0.50|0.30|0.70|0.20\" -F tensor",
  ),
)

=== Output formats

/ `human`: The default. Prints each dimension as `Name: value (anchor description)`, followed by the nearest mood if one falls within tolerance.
/ `json`: Structured JSON with `schema_id` and `values` fields.
/ `tensor`: Re-encodes the tensor. Useful for normalizing or roundtripping.
/ `mood`: Prints only the nearest mood name, its description, and distance. If no mood is within tolerance, prints `(unnamed region)`.

=== Rune stripping

Tensor strings may contain rune prefixes on values (e.g.,
`@state:tensor|ᚣ0.30|ᚤ0.20|...`). The decoder strips any non-digit,
non-dot, non-minus prefix characters before parsing, so rune-encoded and plain
tensors decode identically.


// ═══════════════════════════════════════════════════════════════════════
// SCHEMAS
// ═══════════════════════════════════════════════════════════════════════

== Listing schemas <schemas>

#command(
  "mx state schemas",
  [List all available schemas. Scans `$MX_HOME/state/schemas/` for files with
  `.yaml`, `.yml`, or `.json` extensions. Each schema is loaded to display its
  name, dimension count, and mood count.],
  flags: (
    ([`--json`], [`flag`], [Output as JSON array with `id`, `name`, `dimensions`, and `moods` fields.]),
  ),
  examples: (
    "mx state schemas",
    "mx state schemas --json",
  ),
)

#note[On first invocation of any `mx state` command, the default `tensor` schema
is self-seeded into `$MX_HOME/state/schemas/tensor.yaml` if no file exists at
that path. User-authored files are never overwritten.]


// ═══════════════════════════════════════════════════════════════════════
// MOODS
// ═══════════════════════════════════════════════════════════════════════

== Moods <moods>

#command(
  "mx state moods",
  [List moods defined in a schema, or show details for a specific mood.

  Without a mood argument, lists all moods with their canonical tensor values
  and descriptions. With a mood name, shows the full definition: description,
  tolerance, and per-dimension values with weights.],
  flags: (
    ([`<mood>`],         [`string`], [Optional positional: mood name to inspect.]),
    ([`-s, --schema`],   [`string`], [Schema ID or path. Default: `tensor`.]),
    ([`--json`],         [`flag`],   [Output as JSON.]),
  ),
  examples: (
    "# List all moods for the default schema\nmx state moods",
    "# List moods for a specific schema\nmx state moods -s crewu",
    "# Show details for a specific mood\nmx state moods calm",
    "# JSON output\nmx state moods --json",
  ),
)

=== Mood matching

When encoding or decoding, the nearest mood is found by weighted Euclidean
distance. Each mood defines a canonical tensor (the center point), optional
per-dimension weights (default 1.0), and a tolerance radius (default 0.30).
A mood matches when the distance is within tolerance. If multiple moods match,
the closest one wins.

The distance formula:

$ d = sqrt(sum_(i=0)^(n-1) w_i (v_i - c_i)^2) $

where $v_i$ is the tensor value, $c_i$ is the mood's canonical value, and $w_i$
is the per-dimension weight.


// ═══════════════════════════════════════════════════════════════════════
// INFO
// ═══════════════════════════════════════════════════════════════════════

== Schema info <info>

#command(
  "mx state info",
  [Show full details for a schema: name, version, all dimensions with their
  anchors and defaults, and all moods with descriptions and tolerances.],
  flags: (
    ([`-s, --schema`], [`string`], [Schema ID or path. Default: `tensor`.]),
    ([`--json`],       [`flag`],   [Output as JSON (the full parsed schema object).]),
  ),
  examples: (
    "mx state info",
    "mx state info -s crewu",
    "mx state info --json",
  ),
)


// ═══════════════════════════════════════════════════════════════════════
// SCHEMA FILE FORMAT
// ═══════════════════════════════════════════════════════════════════════

== Schema file format <schema-format>

Schemas are YAML files stored in `$MX_HOME/state/schemas/`. The file stem is
the schema ID (e.g., `tensor.yaml` has ID `tensor`). JSON is also accepted as a
fallback format.

=== Top-level fields

/ `id`: Required. Schema identifier. Must match the file stem.
/ `name`: Required. Human-readable name.
/ `version`: Optional. Integer version number. Default: `1`.
/ `dimensions`: Required. Ordered list of dimension definitions.
/ `moods`: Optional. Map of mood name to mood definition. Default: empty.

=== Dimension definition

Each dimension in the `dimensions` list has these fields:

/ `id`: Required. Unique identifier (e.g., `entropy`, `temperature`).
/ `name`: Required. Human-readable display name.
/ `rune`: Optional. Decorative Unicode character used when `--runes` is enabled.
/ `default`: Optional. Default value (0.0--1.0). Default: `0.5`.
/ `anchors`: Required. Object with anchor descriptions:
  - `low`: Required. Description for values near 0.0.
  - `mid`: Optional. Description for values near 0.5.
  - `high`: Required. Description for values near 1.0.

=== Mood definition

Each entry in the `moods` map has these fields:

/ `description`: Required. Human-readable description of the mood.
/ `tensor`: Required. List of canonical float values, one per dimension, in the dimension order declared by the schema.
/ `weights`: Optional. List of per-dimension weights for distance calculation. Default: `1.0` for all dimensions.
/ `tolerance`: Optional. Maximum weighted Euclidean distance for a tensor to be considered "in" this mood. Default: `0.30`.

=== Example schema

```yaml
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

=== Default tensor schema

The built-in `tensor` schema ships with six dimensions in this order:

+ *Entropy* -- ordered/focused (0.0) to chaotic/wild (1.0). Default: 0.4.
+ *Agency* -- receptive/yielding (0.0) to active/driving (1.0). Default: 0.5.
+ *Temperature* -- cold/precise (0.0) to warm/casual (1.0). Default: 0.5.
+ *Verbosity* -- terse/minimal (0.0) to expansive/elaborate (1.0). Default: 0.4.
+ *Skepticism* -- agreeable/affirming (0.0) to challenging/pushback (1.0). Default: 0.55.
+ *Humor* -- serious/matter-of-fact (0.0) to playful/quippy (1.0). Default: 0.3.

The default schema has no moods defined. Add moods to your local copy at
`$MX_HOME/state/schemas/tensor.yaml` to enable mood matching.

=== Schema resolution

The `--schema` flag on all commands accepts either a schema ID or a direct file
path. The heuristic: if the argument contains a slash or ends with `.yaml`,
`.yml`, or `.json`, it is treated as a path and loaded directly. Otherwise it is
treated as an ID and resolved from `$MX_HOME/state/schemas/` with an extension
fallback chain of `.yaml`, `.yml`, `.json`.

#tip[To reference a schema file in the current directory by relative path, use
`./my-schema.yaml` rather than `my-schema.yaml` -- the latter would be treated
as an ID lookup.]
