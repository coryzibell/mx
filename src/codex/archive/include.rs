//! Opt-in source selection for `mx codex archive`.
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
    /// The set that reproduces today's `mx codex archive` defaults
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
    /// `tool-output` (or `tool_output`), `history`, `all`, `none`.
    /// Unknown tokens print a warning to stderr and are skipped — we
    /// don't fail-hard so a future rename can land alongside an
    /// older user script gracefully.
    ///
    /// `all` and `none` are exclusive overrides: if either appears
    /// anywhere in the list, it wins for the whole field it touches
    /// (`all` sets every flag on, `none` sets every flag off). When
    /// they appear together, later tokens win — left-to-right reading
    /// order, the same way a shell would interpret a chain of toggles.
    pub fn parse(s: &str) -> Result<Self> {
        let mut set = Self::default();
        for raw in s.split(',') {
            let token = raw.trim().to_ascii_lowercase();
            if token.is_empty() {
                continue;
            }
            match token.as_str() {
                "all" => set = Self::all(),
                "none" => set = Self::none(),
                "subagents" => set.subagents = true,
                "mcp" => set.mcp = true,
                "tool-output" | "tool_output" => set.tool_output = true,
                "history" => set.history = true,
                other => {
                    eprintln!(
                        "warning: ignoring unknown --include token '{}' \
                         (recognized: subagents, mcp, tool-output, history, all, none)",
                        other
                    );
                }
            }
        }
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
    fn parse_dash_and_underscore_synonyms() {
        let a = IncludeSet::parse("tool-output").unwrap();
        let b = IncludeSet::parse("tool_output").unwrap();
        assert_eq!(a, b);
        assert!(a.tool_output);
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
    fn parse_all_then_none_left_to_right_none_wins() {
        // 'none' appears later -> resets the field.
        let s = IncludeSet::parse("all,none").unwrap();
        assert_eq!(s, IncludeSet::none());
    }

    #[test]
    fn parse_none_then_subagents_re_enables() {
        let s = IncludeSet::parse("none,subagents").unwrap();
        assert!(s.subagents);
        assert!(!s.mcp);
    }
}
