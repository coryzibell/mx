//! Shared harness helpers for the CLI-level integration tests.
//!
//! Every test under `tests/` that spawns `CARGO_BIN_EXE_mx` inherits the
//! invoking shell's environment. On a live workstation that shell exports
//! `MX_SURREAL_MODE=network` plus the production URL/credentials (`activate`
//! and the Claude PreToolUse hook both inject them), so a test that only sets
//! `MX_HOME=<tempdir>` still resolves its store from the ambient env and runs
//! against the **production graph** — reads and writes alike.
//!
//! That is not hypothetical: `tests/triggers_cli.rs` calls `mx memory add`, and
//! a `Trig Test` fixture from it was found sitting in the live graph. It is the
//! same failure mode the unit layer already guards at
//! `SurrealDatabase::open_at` (see `src/surreal_db/connection.rs`), where a
//! dim-4 fixture once leaked into production and poisoned every cosine scan.
//! This module is that guardrail for the integration layer.

use std::path::Path;
use std::process::Command;

/// Pin a spawned `mx` to a hermetic, embedded, tempdir-backed store.
///
/// Forces embedded mode the same way `tests/hidden_private_hint_cli.rs` does
/// (`MX_SURREAL_MODE=embedded` + an explicit `MX_SURREAL_ROOT` under `home`) and
/// strips every ambient `MX_SURREAL_*` var that could otherwise redirect the
/// child at a shared database. `MX_MEMORY_PATH` is removed rather than set: it
/// is deprecated and merely prints a stderr deprecation note (`src/paths.rs`),
/// which would contaminate stderr assertions.
///
/// `home` must be a `tempfile::TempDir` path so each test gets a fresh store.
pub fn isolate(cmd: &mut Command, home: &Path) {
    cmd.env("MX_HOME", home)
        .env("MX_SURREAL_MODE", "embedded")
        .env("MX_SURREAL_ROOT", home.join("surreal"))
        .env_remove("MX_SURREAL_URL")
        .env_remove("MX_SURREAL_USER")
        .env_remove("MX_SURREAL_PASS")
        .env_remove("MX_SURREAL_PASS_FILE")
        .env_remove("MX_SURREAL_NS")
        .env_remove("MX_SURREAL_DB")
        .env_remove("MX_SURREAL_AUTH_LEVEL")
        .env_remove("MX_MEMORY_PATH")
        .env_remove("MX_MEMORY_BACKEND");
}
