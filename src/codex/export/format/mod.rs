//! Output format emitters for `mx codex export`.

pub mod json;
pub mod markdown;

/// Which renderer(s) to drive.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Format {
    /// Markdown — what humans read. Default.
    Markdown,
    /// Structured JSON for tool consumers.
    Json,
    /// Both — JSON to the output file (or stdout), markdown to stderr
    /// commentary. The CLI handler is responsible for routing.
    Both,
}

impl Format {
    /// Parse the `--format` CLI value.
    pub fn parse(s: &str) -> anyhow::Result<Self> {
        match s.trim().to_ascii_lowercase().as_str() {
            "markdown" | "md" => Ok(Self::Markdown),
            "json" => Ok(Self::Json),
            "both" => Ok(Self::Both),
            other => anyhow::bail!(
                "unknown --format '{}' (expected markdown, json, or both)",
                other
            ),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_recognized() {
        assert_eq!(Format::parse("markdown").unwrap(), Format::Markdown);
        assert_eq!(Format::parse("MD").unwrap(), Format::Markdown);
        assert_eq!(Format::parse("json").unwrap(), Format::Json);
        assert_eq!(Format::parse("Both").unwrap(), Format::Both);
    }

    #[test]
    fn parse_rejects_unknown() {
        assert!(Format::parse("xml").is_err());
    }
}
