//! Opt-in source selection for `mx codex save`.
//!
//! `IncludeSet` controls which optional sidecars the writer captures.
//! Today only `subagents` is on by default — the same artifact the
//! pre-unification archive flow always copied. The other three flags
//! (`mcp`, `tool_output`, `history`) opt in to the new walkers added
//! in PR 2 and are off by default until export (PR 3) and the broader
//! UX have stabilized.
//!
//! The set is parsed from a comma-separated CLI string (`--include
//! subagents,mcp,history`). Two special tokens short-circuit:
//! `all` enables every flag, `none` disables every flag.

use anyhow::Result;

/// Which optional source artifacts to capture during archive.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct IncludeSet {
    /// Subagent JSONLs (`agents/`). Default: ON — matches the pre-PR-2
    /// behavior, where the archive always copied them.
    pub subagents: bool,
    /// MCP server logs (`mcp/`). Default: OFF.
    pub mcp: bool,
    /// `/tmp/.../tasks/*.output` snapshots (`tool-output/`). Default: OFF.
    pub tool_output: bool,
    /// Sliced `~/.claude/history.jsonl` lines (`history/`). Default: OFF.
    pub history: bool,
}

impl IncludeSet {
    /// The set that reproduces today's `mx codex save` defaults
    /// byte-for-byte: subagents on, everything else off.
    pub fn status_quo() -> Self {
        Self {
            subagents: true,
            mcp: false,
            tool_output: false,
            history: false,
        }
    }

    /// All four sources on.
    pub fn all() -> Self {
        Self {
            subagents: true,
            mcp: true,
            tool_output: true,
            history: true,
        }
    }

    /// Nothing on. Useful for `--include none` and for tests that want a
    /// minimum-noise archive (just session.jsonl + manifest).
    pub fn none() -> Self {
        Self::default()
    }

    /// Parse a comma-separated CLI value.
    ///
    /// Tokens are case-insensitive. Recognized: `subagents`, `mcp`,
    /// `tool-output`, `history`, `all`, `none`.
    /// Unknown tokens print a warning to stderr and are skipped — we
    /// don't fail-hard so a future rename can land alongside an
    /// older user script gracefully.
    ///
    /// N4: the hyphenated `tool-output` is the canonical form (matches
    /// the help text and the directory name on disk). The previous
    /// `tool_output` underscore alias drifted from the help docs and
    /// has been removed; users typing `tool_output` get the standard
    /// "unknown token" warning.
    ///
    /// `all` and `none` are exclusive overrides: passing BOTH in the
    /// same input is rejected as user error (S3). Empty tokens (e.g.
    /// `,,` or trailing/leading commas) are also rejected so typos in
    /// the CLI value surface immediately rather than silently no-op.
    pub fn parse(s: &str) -> Result<Self> {
        let mut set = Self::default();
        let mut saw_all = false;
        let mut saw_none = false;
        let mut saw_any_meaningful = false;

        for (idx, raw) in s.split(',').enumerate() {
            let token = raw.trim().to_ascii_lowercase();
            if token.is_empty() {
                // Allow a fully-empty input string ("" -> none-set) for
                // backward compat with the existing tests, but reject
                // empty tokens *between* commas — those are clear typos.
                if s.trim().is_empty() && idx == 0 {
                    continue;
                }
                anyhow::bail!(
                    "empty token in --include list (got '{}'); \
                     remove the stray comma or use --include none",
                    s
                );
            }
            match token.as_str() {
                "all" => {
                    saw_all = true;
                    set = Self::all();
                    saw_any_meaningful = true;
                }
                "none" => {
                    saw_none = true;
                    set = Self::none();
                    saw_any_meaningful = true;
                }
                "subagents" => {
                    set.subagents = true;
                    saw_any_meaningful = true;
                }
                "mcp" => {
                    set.mcp = true;
                    saw_any_meaningful = true;
                }
                "tool-output" => {
                    set.tool_output = true;
                    saw_any_meaningful = true;
                }
                "history" => {
                    set.history = true;
                    saw_any_meaningful = true;
                }
                other => {
                    eprintln!(
                        "warning: ignoring unknown --include token '{}' \
                         (recognized: subagents, mcp, tool-output, history, all, none)",
                        other
                    );
                }
            }
        }

        if saw_all && saw_none {
            anyhow::bail!(
                "--include cannot contain both 'all' and 'none' (got '{}'); \
                 pick one — they are mutually exclusive",
                s
            );
        }
        let _ = saw_any_meaningful;
        Ok(set)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn status_quo_matches_pre_pr2_default() {
        let s = IncludeSet::status_quo();
        assert!(s.subagents);
        assert!(!s.mcp);
        assert!(!s.tool_output);
        assert!(!s.history);
    }

    #[test]
    fn parse_single_token() {
        let s = IncludeSet::parse("subagents").unwrap();
        assert_eq!(s, IncludeSet::status_quo());
    }

    #[test]
    fn parse_multiple_tokens() {
        let s = IncludeSet::parse("subagents,mcp,history").unwrap();
        assert!(s.subagents);
        assert!(s.mcp);
        assert!(!s.tool_output);
        assert!(s.history);
    }

    #[test]
    fn parse_canonical_tool_output_token() {
        // N4: `tool-output` is the canonical form. The previous
        // `tool_output` underscore alias has been removed; passing it
        // now lands in the unknown-token branch (warning, no flag set).
        let a = IncludeSet::parse("tool-output").unwrap();
        assert!(a.tool_output);

        let b = IncludeSet::parse("tool_output").unwrap();
        assert!(!b.tool_output, "tool_output is no longer recognized");
    }

    #[test]
    fn parse_case_insensitive() {
        let s = IncludeSet::parse("SubAgents,MCP,Tool-Output,HISTORY").unwrap();
        assert_eq!(s, IncludeSet::all());
    }

    #[test]
    fn parse_all_token() {
        let s = IncludeSet::parse("all").unwrap();
        assert_eq!(s, IncludeSet::all());
    }

    #[test]
    fn parse_none_token() {
        let s = IncludeSet::parse("none").unwrap();
        assert_eq!(s, IncludeSet::none());
    }

    #[test]
    fn parse_empty_string_is_none() {
        let s = IncludeSet::parse("").unwrap();
        assert_eq!(s, IncludeSet::none());
    }

    #[test]
    fn parse_unknown_token_warns_and_skips() {
        // Should NOT error; should emit a stderr warning we don't capture in
        // this test, and the resulting set should be otherwise valid.
        let s = IncludeSet::parse("subagents,bogus,mcp").unwrap();
        assert!(s.subagents);
        assert!(s.mcp);
        assert!(!s.tool_output);
    }

    #[test]
    fn parse_trims_whitespace() {
        let s = IncludeSet::parse("  subagents , mcp  ").unwrap();
        assert!(s.subagents);
        assert!(s.mcp);
    }

    #[test]
    fn parse_all_and_none_together_is_rejected() {
        // S3: combining `all` and `none` in one --include is a user
        // error. The previous behavior silently let 'whichever came
        // last' win, which made typos invisible.
        let err = IncludeSet::parse("all,none").unwrap_err();
        let msg = format!("{err:#}");
        assert!(msg.contains("'all'") || msg.contains("all"));
        assert!(msg.contains("'none'") || msg.contains("none"));
    }

    #[test]
    fn parse_none_then_subagents_re_enables() {
        let s = IncludeSet::parse("none,subagents").unwrap();
        assert!(s.subagents);
        assert!(!s.mcp);
    }

    #[test]
    fn parse_empty_token_between_commas_is_rejected() {
        // S3: `subagents,,mcp` is a typo, not a degenerate-but-valid
        // request — the previous parser would silently drop the empty
        // segment, hiding the typo.
        let err = IncludeSet::parse("subagents,,mcp").unwrap_err();
        let msg = format!("{err:#}");
        assert!(msg.contains("empty token"), "got: {msg}");
    }
}
