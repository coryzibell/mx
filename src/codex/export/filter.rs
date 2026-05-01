//! Selection and resolution for `mx codex export`.
//!
//! Three things live here:
//!
//! 1. `Selector` — what the caller asked for (single session, project,
//!    date range, or "latest").
//! 2. `SessionRef` / `DateRange` — the parameter shapes selectors carry.
//! 3. The resolution functions that turn a selector into the concrete
//!    list of codex archive directories to render.
//!
//! Resolution is read-only: it walks `<codex_dir>/<archive_dir>/manifest.json`,
//! never `~/.claude/projects/`. (The architecture is explicit: export must
//! NOT read live Claude data — only the codex.)

use anyhow::{Context, Result};
use chrono::{DateTime, NaiveDate, TimeZone, Utc};
use std::path::{Path, PathBuf};

use crate::codex::Manifest;

/// What to export.
#[derive(Debug, Clone)]
pub enum Selector {
    /// A specific session by full UUID or short prefix.
    Session(SessionRef),
    /// Every session for a project (path / slug / basename).
    Project(String),
    /// Every session whose `session_start` falls in the range.
    Date(DateRange),
    /// The most recent codex session — default when no other selector
    /// is supplied.
    Latest,
}

/// Reference to a session by ID. Either the full UUID or a unique prefix.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionRef(pub String);

impl SessionRef {
    /// True if `id` matches this ref. Prefix match: `c3744b8d` matches
    /// `c3744b8d-5719-4df2-924f-707945438494`.
    pub fn matches(&self, id: &str) -> bool {
        id.starts_with(&self.0)
    }
}

/// Inclusive date range for `Selector::Date`. Both ends are UTC midnights.
///
/// Parser accepts:
/// - `YYYY-MM-DD` → that single day, midnight to next-midnight
/// - `YYYY-MM-DD..YYYY-MM-DD` → inclusive day range
/// - `YYYY-MM` → the whole month
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DateRange {
    pub start: DateTime<Utc>,
    /// Exclusive end so we can use `< end` semantics. The CLI form is
    /// inclusive at the day level — for `2026-04-29..2026-04-30` end
    /// is `2026-05-01T00:00:00Z`.
    pub end: DateTime<Utc>,
}

impl DateRange {
    /// True iff `ts` is in `[start, end)`.
    pub fn contains(&self, ts: DateTime<Utc>) -> bool {
        ts >= self.start && ts < self.end
    }

    /// Parse a CLI date expression.
    pub fn parse(s: &str) -> Result<Self> {
        let s = s.trim();
        if s.is_empty() {
            anyhow::bail!("empty --date value");
        }

        // Range form: `YYYY-MM-DD..YYYY-MM-DD`.
        if let Some((lhs, rhs)) = s.split_once("..") {
            let start_day = parse_ymd(lhs.trim())
                .with_context(|| format!("--date range start '{}' is not YYYY-MM-DD", lhs))?;
            let end_day = parse_ymd(rhs.trim())
                .with_context(|| format!("--date range end '{}' is not YYYY-MM-DD", rhs))?;
            if end_day < start_day {
                anyhow::bail!(
                    "--date range '{}' is inverted (start {} is after end {})",
                    s,
                    start_day,
                    end_day
                );
            }
            return Ok(Self {
                start: midnight_utc(start_day),
                end: midnight_utc(end_day + chrono::Duration::days(1)),
            });
        }

        // Single day: `YYYY-MM-DD`.
        if s.len() == 10
            && let Ok(day) = parse_ymd(s)
        {
            return Ok(Self {
                start: midnight_utc(day),
                end: midnight_utc(day + chrono::Duration::days(1)),
            });
        }

        // Month: `YYYY-MM`.
        if s.len() == 7
            && let Some((y, m)) = s.split_once('-')
        {
            let year: i32 = y
                .parse()
                .with_context(|| format!("--date '{}' has non-numeric year", s))?;
            let month: u32 = m
                .parse()
                .with_context(|| format!("--date '{}' has non-numeric month", s))?;
            if !(1..=12).contains(&month) {
                anyhow::bail!("--date '{}' month out of range (1..=12)", s);
            }
            let start_day = NaiveDate::from_ymd_opt(year, month, 1)
                .with_context(|| format!("--date '{}' is not a valid year-month", s))?;
            let end_day = if month == 12 {
                NaiveDate::from_ymd_opt(year + 1, 1, 1)
            } else {
                NaiveDate::from_ymd_opt(year, month + 1, 1)
            }
            .context("internal date overflow")?;
            return Ok(Self {
                start: midnight_utc(start_day),
                end: midnight_utc(end_day),
            });
        }

        anyhow::bail!(
            "--date '{}' is not a recognized form (expected YYYY-MM-DD, \
             YYYY-MM-DD..YYYY-MM-DD, or YYYY-MM)",
            s
        );
    }
}

fn parse_ymd(s: &str) -> Result<NaiveDate> {
    NaiveDate::parse_from_str(s, "%Y-%m-%d").with_context(|| format!("'{}' is not YYYY-MM-DD", s))
}

fn midnight_utc(date: NaiveDate) -> DateTime<Utc> {
    Utc.from_utc_datetime(&date.and_hms_opt(0, 0, 0).expect("00:00:00 is valid"))
}

/// One archive directory the export will render. Pairs the directory
/// path with the deserialized manifest so emitters don't re-read it.
#[derive(Debug, Clone)]
pub struct ResolvedArchive {
    pub archive_dir: PathBuf,
    pub manifest: Manifest,
}

/// Walk `<codex_dir>/` and return every archive whose manifest parses.
/// Skips the by-project / staging / .old subdirs.
pub fn collect_codex_archives(codex_dir: &Path) -> Result<Vec<ResolvedArchive>> {
    let mut out = Vec::new();
    if !codex_dir.exists() {
        return Ok(out);
    }
    for entry in std::fs::read_dir(codex_dir)? {
        let entry = entry?;
        let archive_dir = entry.path();
        if !archive_dir.is_dir() {
            continue;
        }
        let name = match archive_dir.file_name().and_then(|n| n.to_str()) {
            Some(n) => n,
            None => continue,
        };
        if matches!(name, "by-project" | "by-project.staging" | "by-project.old") {
            continue;
        }
        let manifest_path = archive_dir.join("manifest.json");
        if !manifest_path.exists() {
            continue;
        }
        let raw = match std::fs::read_to_string(&manifest_path) {
            Ok(r) => r,
            Err(_) => continue,
        };
        let manifest: Manifest = match serde_json::from_str(&raw) {
            Ok(m) => m,
            Err(_) => continue,
        };
        out.push(ResolvedArchive {
            archive_dir,
            manifest,
        });
    }
    Ok(out)
}

/// Pick a single archive for a `Selector::Session` query.
///
/// - Exact match wins.
/// - Otherwise prefix match; multiple prefix matches → error listing IDs.
pub fn resolve_session(
    archives: Vec<ResolvedArchive>,
    sref: &SessionRef,
) -> Result<ResolvedArchive> {
    let exact: Vec<ResolvedArchive> = archives
        .iter()
        .filter(|a| a.manifest.session_id == sref.0)
        .cloned()
        .collect();
    if exact.len() == 1 {
        return Ok(exact.into_iter().next().unwrap());
    }
    if exact.len() > 1 {
        let ids: Vec<String> = exact
            .iter()
            .map(|a| a.manifest.session_id.clone())
            .collect();
        anyhow::bail!(
            "session ID '{}' is duplicated in the codex (matched {}). \
             This usually means a manifest has been edited; re-archive to disambiguate.",
            sref.0,
            ids.join(", ")
        );
    }
    let prefix: Vec<ResolvedArchive> = archives
        .into_iter()
        .filter(|a| sref.matches(&a.manifest.session_id))
        .collect();
    match prefix.len() {
        0 => anyhow::bail!(
            "no archived session matches '{}' (full UUID or unique prefix)",
            sref.0
        ),
        1 => Ok(prefix.into_iter().next().unwrap()),
        _ => {
            let ids: Vec<String> = prefix
                .iter()
                .map(|a| a.manifest.session_id.clone())
                .collect();
            anyhow::bail!(
                "session prefix '{}' is ambiguous; matches {}. \
                 Use a longer prefix or the full UUID.",
                sref.0,
                ids.join(", ")
            );
        }
    }
}

/// Filter archives down to those belonging to the specified project.
///
/// `query` accepts the same three forms as `ProjectIndex::lookup`. Hits
/// the index when possible and falls back to a manifest walk on its
/// own filtered scan.
pub fn resolve_project(
    archives: Vec<ResolvedArchive>,
    query: &str,
) -> Result<Vec<ResolvedArchive>> {
    // For projects we always re-filter the manifests directly — the
    // index gives us the canonical project_path(s) but we still need
    // the loaded manifests here so the caller doesn't double-read.
    let idx = crate::codex::index::ProjectIndex::open()?;
    let entry = idx.lookup(query)?;
    let project_paths: Vec<String> = entry
        .absolute_paths
        .iter()
        .map(|p| p.to_string_lossy().to_string())
        .collect();
    let mut matched: Vec<ResolvedArchive> = archives
        .into_iter()
        .filter(|a| {
            a.manifest
                .project_path
                .as_ref()
                .map(|p| project_paths.iter().any(|pp| pp == p))
                .unwrap_or(false)
        })
        .collect();
    matched.sort_by_key(|a| a.manifest.session_start);
    if matched.is_empty() {
        anyhow::bail!(
            "project '{}' resolved to {:?} but no archived sessions reference it",
            query,
            project_paths
        );
    }
    Ok(matched)
}

/// Filter archives down to those whose `session_start` lies in `range`.
pub fn resolve_date(archives: Vec<ResolvedArchive>, range: &DateRange) -> Vec<ResolvedArchive> {
    let mut matched: Vec<ResolvedArchive> = archives
        .into_iter()
        .filter(|a| range.contains(a.manifest.session_start))
        .collect();
    matched.sort_by_key(|a| a.manifest.session_start);
    matched
}

/// The most recent archive (greatest `session_start`).
pub fn resolve_latest(archives: Vec<ResolvedArchive>) -> Result<ResolvedArchive> {
    archives
        .into_iter()
        .max_by_key(|a| a.manifest.session_start)
        .context("no archived sessions in codex (run `mx codex archive --all`)")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn date(s: &str) -> DateTime<Utc> {
        s.parse().unwrap()
    }

    // ------------------------------------------------------------------
    // DateRange::parse
    // ------------------------------------------------------------------

    #[test]
    fn date_range_parses_single_day() {
        let r = DateRange::parse("2026-04-29").unwrap();
        assert_eq!(r.start, date("2026-04-29T00:00:00Z"));
        assert_eq!(r.end, date("2026-04-30T00:00:00Z"));
        assert!(r.contains(date("2026-04-29T12:00:00Z")));
        assert!(!r.contains(date("2026-04-30T00:00:00Z")));
    }

    #[test]
    fn date_range_parses_inclusive_day_range() {
        let r = DateRange::parse("2026-04-01..2026-04-30").unwrap();
        assert_eq!(r.start, date("2026-04-01T00:00:00Z"));
        // End is exclusive at the next day's midnight, so 2026-04-30 is in range.
        assert!(r.contains(date("2026-04-30T23:59:59Z")));
        assert!(!r.contains(date("2026-05-01T00:00:00Z")));
    }

    #[test]
    fn date_range_parses_whole_month() {
        let r = DateRange::parse("2026-04").unwrap();
        assert_eq!(r.start, date("2026-04-01T00:00:00Z"));
        assert_eq!(r.end, date("2026-05-01T00:00:00Z"));
    }

    #[test]
    fn date_range_parses_december_month_rolls_year() {
        let r = DateRange::parse("2026-12").unwrap();
        assert_eq!(r.start, date("2026-12-01T00:00:00Z"));
        assert_eq!(r.end, date("2027-01-01T00:00:00Z"));
    }

    #[test]
    fn date_range_rejects_inverted_range() {
        let err = DateRange::parse("2026-04-30..2026-04-01").unwrap_err();
        let msg = format!("{err:#}");
        assert!(msg.contains("inverted"), "got: {msg}");
    }

    #[test]
    fn date_range_rejects_garbage() {
        assert!(DateRange::parse("yesterday").is_err());
        assert!(DateRange::parse("").is_err());
        assert!(DateRange::parse("2026-13").is_err());
    }

    #[test]
    fn date_range_rejects_invalid_calendar_day() {
        // Feb 30 isn't a calendar day — must error rather than silently
        // sliding to March 2.
        assert!(DateRange::parse("2026-02-30").is_err());
    }

    // ------------------------------------------------------------------
    // SessionRef::matches
    // ------------------------------------------------------------------

    #[test]
    fn session_ref_prefix_match() {
        let r = SessionRef("c3744b8d".to_string());
        assert!(r.matches("c3744b8d-5719-4df2-924f-707945438494"));
        assert!(!r.matches("d3744b8d-5719-4df2-924f-707945438494"));
    }

    #[test]
    fn session_ref_exact_match() {
        let full = "c3744b8d-5719-4df2-924f-707945438494";
        let r = SessionRef(full.to_string());
        assert!(r.matches(full));
    }
}
