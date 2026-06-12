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
