# mx

mx — Rust CLI for memory graph and KV store operations.

Used by hearth and companions to persist memory, traits, and session state to
a SurrealDB backend. Companions invoke it via the `hearth mx` proxy, which
injects credentials at runtime. No runbook yet — the codebase is the reference.

Key commands: `memory add`, `memory list`, `memory get`, `kv set`, `kv get`.
The `--mine` flag scopes queries to the calling agent (`MX_CURRENT_AGENT`).

## Maintenance Rule

**Before working in this directory, read the runbook.** It has architecture decisions, known gotchas, and context you need.

If you change code here, update the runbook to match. The runbook is only useful if it matches reality.
