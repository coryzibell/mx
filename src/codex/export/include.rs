//! Render-time include set for `mx codex export`.
//!
//! Distinct from `archive::IncludeSet`: archive's set gates which source
//! files are *captured* into the codex archive, while this set gates
//! which captured artifacts are *rendered* into the export output. The
//! tokens overlap (`subagents`, `mcp`, `history`) but the semantics
//! differ — `tools`, `system-reminders`, and `tool-output` are
//! export-only knobs that govern what the markdown/JSON emitters
//! include from the session.jsonl that's already on disk.
//!
//! Tokens are case-insensitive. Recognized:
//!
//! - `subagents` — render sub-agent transcripts (DEFAULT ON)
//! - `tools` — render `tool_use` blocks (default OFF)
//! - `system-reminders` — render `<system-reminder>` blocks (default OFF)
//! - `mcp` — render captured MCP log sidecars (default OFF)
//! - `tool-output` — render captured /tmp tool outputs (default OFF)
//! - `history` — render captured history slice (default OFF)
//! - `all` / `none` — exclusive overrides
//!
//! Validation matches `archive::IncludeSet`:
//!
//! - `all,none` together is a hard error
//! - empty tokens between commas (`subagents,,mcp`) are rejected as typos
//! - unknown tokens emit a stderr warning and are ignored

use anyhow::Result;

/// Which captured/in-session content to render in the export output.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ExportIncludeSet {
    /// Sub-agent transcripts inlined at the parent's Task tool boundary.
    pub subagents: bool,
    /// `tool_use` blocks (the assistant's tool invocations).
    pub tools: bool,
    /// `<system-reminder>` blocks.
    pub system_reminders: bool,
    /// MCP log sidecars captured at archive time.
    pub mcp: bool,
    /// /tmp tool-output sidecars captured at archive time.
    pub tool_output: bool,
    /// `history.jsonl` slice captured at archive time.
    pub history: bool,
}

impl ExportIncludeSet {
    /// "Clean human conversation": sub-agents in, every other extra off.
    /// This is the export default — what someone running
    /// `mx codex export` with no `--include` sees.
    pub fn default_clean() -> Self {
        Self {
            subagents: true,
            tools: false,
            system_reminders: false,
            mcp: false,
            tool_output: false,
            history: false,
        }
    }

    /// Everything on. Useful for `--include all`.
    pub fn all() -> Self {
        Self {
            subagents: true,
            tools: true,
            system_reminders: true,
            mcp: true,
            tool_output: true,
            history: true,
        }
    }

    /// Nothing on. Yields a transcript with only human/assistant prose.
    pub fn none() -> Self {
        Self::default()
    }

    /// Parse a comma-separated CLI value.
    ///
    /// Mirrors `archive::IncludeSet::parse` for consistency: tokens are
    /// trimmed and lowercased, unknown tokens warn-and-skip, `all+none`
    /// in the same input is rejected, and empty tokens between commas
    /// are rejected.
    pub fn parse(s: &str) -> Result<Self> {
        let mut set = Self::default();
        let mut saw_all = false;
        let mut saw_none = false;

        for (idx, raw) in s.split(',').enumerate() {
            let token = raw.trim().to_ascii_lowercase();
            if token.is_empty() {
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
                }
                "none" => {
                    saw_none = true;
                    set = Self::none();
                }
                "subagents" => set.subagents = true,
                "tools" => set.tools = true,
                "system-reminders" => set.system_reminders = true,
                "mcp" => set.mcp = true,
                "tool-output" => set.tool_output = true,
                "history" => set.history = true,
                other => {
                    eprintln!(
                        "warning: ignoring unknown --include token '{}' \
                         (recognized: subagents, tools, system-reminders, mcp, \
                         tool-output, history, all, none)",
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
        Ok(set)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_clean_matches_brief() {
        let s = ExportIncludeSet::default_clean();
        assert!(s.subagents);
        assert!(!s.tools);
        assert!(!s.system_reminders);
        assert!(!s.mcp);
        assert!(!s.tool_output);
        assert!(!s.history);
    }

    #[test]
    fn parse_subagents_only_is_default_clean() {
        let s = ExportIncludeSet::parse("subagents").unwrap();
        assert_eq!(s, ExportIncludeSet::default_clean());
    }

    #[test]
    fn parse_all_token() {
        let s = ExportIncludeSet::parse("all").unwrap();
        assert_eq!(s, ExportIncludeSet::all());
    }

    #[test]
    fn parse_none_token() {
        let s = ExportIncludeSet::parse("none").unwrap();
        assert_eq!(s, ExportIncludeSet::none());
    }

    #[test]
    fn parse_export_only_tokens() {
        // tools and system-reminders only exist on the export side.
        let s = ExportIncludeSet::parse("tools,system-reminders").unwrap();
        assert!(s.tools);
        assert!(s.system_reminders);
        assert!(!s.subagents);
    }

    #[test]
    fn parse_case_insensitive() {
        let s = ExportIncludeSet::parse("SubAgents,TOOLS,System-Reminders,MCP").unwrap();
        assert!(s.subagents);
        assert!(s.tools);
        assert!(s.system_reminders);
        assert!(s.mcp);
    }

    #[test]
    fn parse_all_and_none_together_is_rejected() {
        let err = ExportIncludeSet::parse("all,none").unwrap_err();
        let msg = format!("{err:#}");
        assert!(msg.to_lowercase().contains("all"));
        assert!(msg.to_lowercase().contains("none"));
    }

    #[test]
    fn parse_empty_token_between_commas_is_rejected() {
        let err = ExportIncludeSet::parse("subagents,,mcp").unwrap_err();
        let msg = format!("{err:#}");
        assert!(msg.contains("empty token"), "got: {msg}");
    }

    #[test]
    fn parse_empty_string_yields_none() {
        let s = ExportIncludeSet::parse("").unwrap();
        assert_eq!(s, ExportIncludeSet::none());
    }

    #[test]
    fn parse_trims_whitespace() {
        let s = ExportIncludeSet::parse(" subagents , history ").unwrap();
        assert!(s.subagents);
        assert!(s.history);
    }

    #[test]
    fn parse_unknown_token_warns_and_skips() {
        let s = ExportIncludeSet::parse("subagents,bogus,history").unwrap();
        assert!(s.subagents);
        assert!(s.history);
        assert!(!s.tools);
    }
}
