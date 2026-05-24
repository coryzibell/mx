#import "lib.typ": *

#page-header(
  "Filesystem Layout",
  "Paths, environment variables, and configuration."
)

This page is the canonical reference for where `mx` reads and writes files,
which environment variables control those locations, and what to expect when
upgrading from an earlier release.

== Table of contents

- #link(<the-principle>)[The principle]
- #link(<layout>)[Layout]
- #link(<env-vars>)[Environment variables]
- #link(<migration>)[Migration & legacy fallbacks]
- #link(<renamed-commands>)[Renamed and removed CLI commands]
- #link(<examples>)[Examples]
- #link(<contributors>)[Notes for contributors]


// ═══════════════════════════════════════════════════════════════════════
// THE PRINCIPLE
// ═══════════════════════════════════════════════════════════════════════

== The principle <the-principle>

Every path `mx` touches derives from a single base directory: `$MX_HOME`, which
defaults to `~/.mx/`. Each subsystem owns a subdirectory beneath that base.

Overrides are layered. From most specific to least specific:

+ *Per-file* -- e.g. `MX_KV_SCHEMA`, `MX_KV_DATA`. These point at exact files
  (and may include a `{agent}` placeholder).
+ *Per-subsystem* -- e.g. `MX_SURREAL_ROOT`, `MX_CODEX_PATH`,
  `MX_ISOLATE_MODELS`. These move one subsystem's root.
+ *Base* -- `MX_HOME`. Moves the entire tree at once.
+ *Default* -- `~/.mx/`.

A more specific override always wins. Setting `MX_KV_DATA=/etc/foo.json` keeps
that one file at `/etc/foo.json` regardless of `MX_HOME` or any subsystem
override.

The base path is hardcoded in exactly one place:
#link("https://github.com/coryzibell/mx/blob/main/src/paths.rs")[`src/paths.rs`].
Everything else asks `paths.rs` for its location.


// ═══════════════════════════════════════════════════════════════════════
// LAYOUT
// ═══════════════════════════════════════════════════════════════════════

== Layout <layout>

```
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
```

=== `kv/` <layout-kv>

The KV store: per-agent schema (TOML) and data (JSON). Used by `mx kv` and any
agent that needs fast local state. Each agent gets one schema file and one data
file, keyed off the #link(<env-other>)[`MX_CURRENT_AGENT`] environment variable.

- `kv/schema/{agent}.toml` -- TOML schema declaring keys, types, and defaults.
- `kv/data/{agent}.json` -- JSON-serialized current values, written atomically.

Override either file with #link(<env-paths>)[`MX_KV_SCHEMA`] /
#link(<env-paths>)[`MX_KV_DATA`]. Both env vars accept the literal string
`{agent}`, which is substituted with the active agent name at resolution time.

=== `state/schemas/` <layout-state>

YAML (or JSON) schemas for the emotional-state tensor system used by
`mx state`. The default schema ID is `tensor`, resolving to
`state/schemas/tensor.yaml`.

Pick a different schema with `mx state ... --schema {id|path}`. The flag
argument is classified as a path or an ID by a simple heuristic, then looked
up:

- *Bare ID* (e.g. `--schema tensor`): the loader tries
  `state/schemas/{id}.yaml` first, then `state/schemas/{id}.yml`, then
  `state/schemas/{id}.json`. The first file that exists wins; if none exist,
  the lookup fails with a "schema not found" error.
- *Direct path* (e.g. `--schema /tmp/foo.json`): the file is loaded
  directly. Any extension is accepted; the parser tries YAML first and falls
  back to JSON based on file contents, not the extension.

The argument is classified as a path if it contains a `/` *or* ends with
`.yaml`, `.yml`, or `.json`; otherwise it is classified as a bare ID. A dot
elsewhere in the name is irrelevant -- only those three suffixes flip the
classification. So `--schema my.schema` is treated as an *ID* (no slash,
no recognized extension) and, if no matching file exists under
`state/schemas/`, fails with a "schema not found" error. To force a path
lookup of a dotted name without one of the recognized extensions, prefix it
with `./` (e.g. `--schema ./my.schema`) -- the slash flips it to path mode.

There is no env-var override for the schema choice anymore -- the old
`MX_STATE_SCHEMA` was replaced by the CLI flag. (See
#link(<renamed-commands>)[Renamed and removed CLI commands].)

=== `memory/` <layout-memory>

The knowledge graph backend and its inputs.

- `memory/surreal/` -- SurrealKV embedded database files. Override the whole
  directory with #link(<env-paths>)[`MX_SURREAL_ROOT`]. For network-mode
  SurrealDB, the directory is unused -- see the
  #link(<env-surreal>)[SurrealDB connection vars].
- `memory/embed/` -- only created when #link(<env-paths>)[`MX_ISOLATE_MODELS`]
  (or legacy `MX_ISOLATE_FASTEMBED`) is set. Holds downloaded ONNX model and
  tokenizer files that would otherwise live in the shared XDG cache
  (`$XDG_CACHE_HOME/huggingface/`). Use this when you want mx's model cache
  isolated from other tools.
- `memory/seed/agents/` -- markdown files with YAML frontmatter, one per agent.
  Loaded by `mx memory seed agents`.
- `memory/seed/knowledge/` -- one or more `*.jsonl` files. Loaded by
  `mx memory seed knowledge`, which scans the directory for every `.jsonl` it
  finds.

=== `codex/` <layout-codex>

Session archives written by `mx codex archive` -- transcripts, extracted images,
and per-archive manifests. Override with #link(<env-paths>)[`MX_CODEX_PATH`].
This is typically the largest directory in `~/.mx/`; point it at a roomier disk
if you archive a lot of sessions.

=== `cache/sync/{owner-repo}/` <layout-cache>

Per-repo cache directory used by `mx sync` to track GitHub issues and
discussions across runs. The repo slug replaces the `/` in `owner/repo` with
`-` (so `coryzibell/mx` becomes `coryzibell-mx`). Safe to delete -- it will be
rebuilt on the next `mx sync pull`.

=== `artifacts/` <layout-artifacts>

Generic output directory for handlers that need to drop a file somewhere
predictable but don't have a more specific home. Treat as ephemeral.

=== `swap/` <layout-swap>

Scratch space for in-flight operations. Cleared opportunistically; do not store
anything you want to keep.


// ═══════════════════════════════════════════════════════════════════════
// ENVIRONMENT VARIABLES
// ═══════════════════════════════════════════════════════════════════════

== Environment variables <env-vars>

=== Path overrides <env-paths>

#table(
  columns: (auto, auto, auto, 1fr),
  table.header([*Variable*], [*Type*], [*Default*], [*Overrides*]),
  [`MX_HOME`], [path], [`~/.mx/`], [The base directory; moves the entire tree],
  [`MX_SURREAL_ROOT`], [path], [`$MX_HOME/memory/surreal/`], [SurrealKV embedded-database root],
  [`MX_CODEX_PATH`], [path], [`$MX_HOME/codex/`], [Codex archive directory],
  [`MX_KV_SCHEMA`], [path template], [`$MX_HOME/kv/schema/{agent}.toml`], [KV schema file; `{agent}` placeholder substituted],
  [`MX_KV_DATA`], [path template], [`$MX_HOME/kv/data/{agent}.json`], [KV data file; `{agent}` placeholder substituted],
  [`MX_ISOLATE_MODELS`], [boolean flag], [unset], [When non-empty, redirects the model cache from XDG to `$MX_HOME/memory/embed/`. Legacy `MX_ISOLATE_FASTEMBED` is also honored],
)

Empty-string values for the path overrides in this table (`MX_HOME`,
`MX_SURREAL_ROOT`, `MX_CODEX_PATH`, `MX_KV_SCHEMA`, `MX_KV_DATA`,
`MX_ISOLATE_MODELS`) are treated as unset and fall back to the default.

The same is *not* uniformly true of other `MX_*` env vars. In particular,
the #link(<env-surreal>)[SurrealDB connection vars] below do not all filter
empty strings: setting `MX_SURREAL_USER=""` produces an empty username, not the
default `root`. Among the connection vars only `MX_SURREAL_PASS` /
`MX_SURREAL_PASS_FILE` are empty-filtered. To restore a default, *leave the
variable unset entirely* rather than setting it to an empty string.

The boolean flag (`MX_ISOLATE_MODELS`) is "on" for any non-empty value
(`1`, `true`, `yes` -- it doesn't parse, it just checks for non-emptiness).

=== SurrealDB connection <env-surreal>

These configure the SurrealDB driver. They affect which database mx talks to,
not where files live on disk (with the exception of `MX_SURREAL_ROOT` above,
which is the embedded-mode storage location).

#table(
  columns: (auto, auto, auto, 1fr),
  table.header([*Variable*], [*Type*], [*Default*], [*Purpose*]),
  [`MX_SURREAL_MODE`], [enum], [`embedded`], [`embedded` for local SurrealKV, `network` for WebSocket],
  [`MX_SURREAL_URL`], [URL], [`ws://localhost:8000`], [WebSocket URL (network mode only)],
  [`MX_SURREAL_USER`], [string], [`root`], [Username],
  [`MX_SURREAL_PASS`], [secret], [unset], [Password (literal value)],
  [`MX_SURREAL_PASS_FILE`], [path], [unset], [Path to a file containing the password (e.g. an agenix secret); read when `MX_SURREAL_PASS` is unset],
  [`MX_SURREAL_NS`], [string], [`memory`], [Namespace],
  [`MX_SURREAL_DB`], [string], [`knowledge`], [Database name],
  [`MX_SURREAL_AUTH_LEVEL`], [enum], [`root`], [One of `root`, `namespace` (or `ns`), `database` (or `db`)],
)

=== GitHub App auth (sync) <env-github>

Optional. Only needed when `mx sync` runs against a private repo via a GitHub
App rather than via your personal `gh` token.

#table(
  columns: (auto, auto, auto, 1fr),
  table.header([*Variable*], [*Type*], [*Default*], [*Purpose*]),
  [`MX_GITHUB_APP_ID`], [string], [unset], [GitHub App ID],
  [`MX_GITHUB_INSTALLATION_ID`], [string], [unset], [App installation ID for the target org/user],
  [`MX_GITHUB_PRIVATE_KEY`], [secret (PEM)], [unset], [App private key, PEM-encoded],
)

=== Identity & display <env-other>

#table(
  columns: (auto, auto, auto, 1fr),
  table.header([*Variable*], [*Type*], [*Default*], [*Purpose*]),
  [`MX_CURRENT_AGENT`], [string], [unset], [Active agent identity. Required for `mx memory wake` and any command that reads/writes per-agent KV. Also the default for `--source-agent` on `mx memory add`],
  [`MX_USER_NAME`], [string], [`git config user.name`, else `"User"`], [Display name for "user" turns in codex transcripts. Resolution order: env var > git config > literal `"User"`],
  [`MX_ASSISTANT_NAME`], [string], [`"Orchestrator"`], [Display name for "assistant" turns in codex transcripts. No git fallback -- the default is the literal string `Orchestrator`],
)

=== Tuning <env-tuning>

#table(
  columns: (auto, auto, auto, 1fr),
  table.header([*Variable*], [*Type*], [*Default*], [*Purpose*]),
  [`MX_WAKE_CHUNK_BYTES`], [integer], [`28000`], [Maximum bytes per chunk during the wake-ritual presentation step. Values that fail to parse or are zero fall back silently to the default],
)

=== Removed <env-removed>

#table(
  columns: (auto, 1fr),
  table.header([*Variable*], [*Replacement*]),
  [`MX_MEMORY_PATH`], [Use #link(<env-paths>)[`MX_SURREAL_ROOT`] for just the database, or #link(<env-paths>)[`MX_HOME`] to move everything together. Setting the old name now emits a one-line stderr note and is otherwise ignored],
  [`MX_STATE_SCHEMA`], [Use the `mx state ... --schema {id|path}` CLI flag. The default schema ID also changed: it is now `tensor` (was `crewu`)],
)


// ═══════════════════════════════════════════════════════════════════════
// MIGRATION & LEGACY FALLBACKS
// ═══════════════════════════════════════════════════════════════════════

== Migration & legacy fallbacks <migration>

The path-alignment refactor (\#255, merged via PR \#259) moved several files
without breaking older installs. For one release cycle, mx will read from the
old locations as a soft fallback and emit a one-line `note:` to stderr telling
you what moved. *No data is lost. The warnings are informative, not errors.*

#table(
  columns: (1fr, 1fr, 1fr),
  table.header([*Legacy location*], [*New location*], [*Behavior*]),
  [`~/.crewu/kv/{agent}.schema.toml`, `~/.crewu/kv/{agent}.data.json`],
    [`$MX_HOME/kv/schema/{agent}.toml`, `$MX_HOME/kv/data/{agent}.json`],
    [Read-only fallback; consolidated stderr note fires once per process],
  [`$MX_HOME/agents/` (agent seed `*.md`)],
    [`$MX_HOME/memory/seed/agents/`],
    [Read-only fallback; stderr note when used],
  [`$MX_HOME/memory/index.jsonl` (knowledge seed)],
    [`$MX_HOME/memory/seed/knowledge/*.jsonl`],
    [Read-only fallback. This is a _shape_ change, not a rename: the old location was a single hardcoded file (`index.jsonl`); the new location is a directory scanned for every `*.jsonl` it finds. Stderr note when the legacy file is read],
  [`MX_MEMORY_PATH` env var],
    [`MX_SURREAL_ROOT` env var],
    [Old var *not honored*; setting it just triggers a rename note],
)

To silence the warnings, move the files (or rename the env var). The fallbacks
will be removed in a future release. To track the removal in source, grep for
`TODO(*-migration)` and `TODO(memory-path-rename-note)` in the codebase.


// ═══════════════════════════════════════════════════════════════════════
// RENAMED AND REMOVED CLI COMMANDS
// ═══════════════════════════════════════════════════════════════════════

== Renamed and removed CLI commands <renamed-commands>

#table(
  columns: (auto, auto, 1fr),
  table.header([*Old*], [*New*], [*Notes*]),
  [`mx agents seed`], [`mx memory seed agents`], [Old form still parses but bails with a one-line pointer to the new command],
  [`mx memory import`], [`mx memory seed knowledge`], [Now scans a directory; loads every `*.jsonl` it finds rather than a single hardcoded file],
  [`mx memory rebuild`], [(removed)], [Reindexing moved out of the user-facing surface; see issue \#258 for `mx doctor memory rebuild`],
  [`mx state ... --env-MX_STATE_SCHEMA`], [`mx state ... --schema {id|path}`], [CLI flag replaces the env var; accepts a bare schema ID or a direct path],
  [`codex save`], [`codex archive`], [Renamed for clarity],
  [`session export`], [`codex export`], [Moved under the codex subcommand],
)

#deprecated[The old command names in this table still parse in current builds
but emit a pointer to their replacement. They will be removed in a future
release.]


// ═══════════════════════════════════════════════════════════════════════
// EXAMPLES
// ═══════════════════════════════════════════════════════════════════════

== Examples <examples>

Move the entire mx tree to a different disk:

```bash
export MX_HOME=/data/mx
```

Keep mx's defaults but put the SurrealDB store on a fast SSD:

```bash
export MX_SURREAL_ROOT=/mnt/ssd/mx-surreal
```

Use a custom KV schema for the `inkwell` agent (without overriding the data
file):

```bash
export MX_KV_SCHEMA=/etc/mx/inkwell-schema.toml
```

Use a path template that resolves per-agent (one variable, many agents):

```bash
export MX_KV_SCHEMA='/etc/mx/schemas/{agent}.toml'
export MX_KV_DATA='/var/lib/mx/{agent}.json'
```

Isolate the model cache so it doesn't share with other tools:

```bash
export MX_ISOLATE_MODELS=1
# Models will now download into $MX_HOME/memory/embed/
```

Encode a tensor. The first form uses the default `tensor` schema; the second
points at an explicit schema file:

```bash
mx state encode --dimensions "temp=0.8 entropy=0.75 agency=0.4"
mx state encode --schema /tmp/myschema.yaml -d "temp=0.5"
```

To target a non-default schema by ID, drop a YAML file at
`$MX_HOME/state/schemas/{id}.yaml` and pass `--schema {id}`. The bare-ID
form is what the lookup helper handles; for an absolute or relative file
path, just pass the path directly (see
#link(<layout-state>)[`state/schemas/`] for the path-vs-ID heuristic).

Point SurrealDB at a remote network instance:

```bash
export MX_SURREAL_MODE=network
export MX_SURREAL_URL=ws://surreal.internal:8000
export MX_SURREAL_USER=mx
export MX_SURREAL_PASS_FILE=/run/agenix/mx-surreal-pass
```


// ═══════════════════════════════════════════════════════════════════════
// NOTES FOR CONTRIBUTORS
// ═══════════════════════════════════════════════════════════════════════

== Notes for contributors <contributors>

Every path in mx routes through
#link("https://github.com/coryzibell/mx/blob/main/src/paths.rs")[`src/paths.rs`].
New helpers follow the `_with(env_val: Option<&str>, home: &Path)` test-seam
pattern -- see `paths::codex_dir_with` for the canonical example. The pattern
keeps resolution logic pure: tests call the `_with` variant directly with
explicit arguments and never mutate process env state, so the suite runs safely
in parallel.

Two rules:

+ Do not call `dirs::home_dir()` outside `src/paths.rs`. If you need a
  home-relative path, add a helper to `paths.rs` and call it from your module.
  `paths.rs` itself is the _only_ legitimate caller of `dirs::home_dir()` in
  the tree. It uses it for: `mx_home()` (the `~/.mx/` default),
  `legacy_crewu_kv_schema_path` and `legacy_crewu_kv_data_path` (the legacy
  fallbacks for the kv migration), and `claude_projects_dir` /
  `claude_config_path` (read-only locations owned by another tool, Claude).
  Anything new that needs `home_dir()` -- including helpers for paths owned
  by other tools -- belongs in `paths.rs` too, so this rule stays absolute
  everywhere else. Do not "fix" the existing calls for consistency; they are
  the carve-out.
+ Do not read `MX_*` env vars in handlers if a path helper already encapsulates
  that override. Add the env-var read inside the helper instead, behind the
  `_with` seam.
