# mx

mx — Rust CLI for memory graph and KV store operations.

Used by hearth and companions to persist memory, traits, and session state to
a SurrealDB backend. Companions invoke it via the `hearth mx` proxy, which
injects credentials at runtime. No runbook yet — the codebase is the reference.

Key commands: `memory add`, `memory list`, `memory get`, `kv set`, `kv get`.
The `--mine` flag scopes queries to the calling agent (`MX_CURRENT_AGENT`).

## Maintenance Rule

If you change behavior of existing commands, note it in the CHANGELOG and
verify companion-facing usage in `~/.soren/config/agents/` still holds.

Write-boundary dedup (W447, `mx memory add` / `add-batch`) is documented as
in-process, best-effort duplicate prevention: read-then-write within a
single invocation, no DB-level unique constraint backing it, and not wired
to the legacy `content_hash` field. See `docs/src/memory.typ`'s
"Write-boundary deduplication" section for the full behavior contract.
