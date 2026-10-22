//! Session state for `mx doors`: the mode a session reads in, and the hints
//! that nudge it. Pure; the `doors_session` reads and writes live in the
//! handler.

use chrono::{DateTime, Local, Utc};

use super::classify::Side;
use super::route::Mode;
use super::{Origin, Segment, Verdict};

/// The kv history holding mode, hint and foreign-paste rows per session.
pub const SESSION_KEY: &str = "doors_session";

/// How long a book mode goes without a mode or foreign-paste row before the
/// stale hint asks whether the reading is over.
const STALE_AFTER_HOURS: i64 = 6;

/// The least time between two `foreign_paste` writes of one session.
const FOREIGN_PASTE_EVERY_HOURS: i64 = 1;

/// One `doors_session` row of the session being evaluated.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionRow {
    pub event: String,
    pub mode: Option<String>,
    pub kind: Option<String>,
    pub by: Option<String>,
    pub ts: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "lowercase")]
pub enum HintKind {
    Foreign,
    Ours,
    Stale,
}

impl HintKind {
    pub fn label(self) -> &'static str {
        match self {
            HintKind::Foreign => "foreign",
            HintKind::Ours => "ours",
            HintKind::Stale => "stale",
        }
    }

    fn parse(s: &str) -> Option<HintKind> {
        match s {
            "foreign" => Some(HintKind::Foreign),
            "ours" => Some(HintKind::Ours),
            "stale" => Some(HintKind::Stale),
            _ => None,
        }
    }
}

/// What a session's rows say right now.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SessionState {
    /// The latest mode row's mode, or house when there is none.
    pub mode: Mode,
    /// `(ts, by)` of the latest mode row.
    pub since: Option<(String, String)>,
    /// Hint kinds already shown in the current mode epoch.
    pub hinted: Vec<HintKind>,
    /// `ts` of the latest mode or foreign-paste row.
    pub last_activity: Option<String>,
}

/// Fold one session's rows, NEWEST FIRST (kv history order), into its state.
/// Position decides "latest", never the ts strings.
pub fn session_state(rows: &[SessionRow]) -> SessionState {
    let mut st = SessionState::default();
    for r in rows {
        if st.last_activity.is_none() && matches!(r.event.as_str(), "mode" | "foreign_paste") {
            st.last_activity = Some(r.ts.clone());
        }
        match r.event.as_str() {
            "mode" => {
                st.mode = r
                    .mode
                    .as_deref()
                    .and_then(|m| Mode::parse(m).ok())
                    .unwrap_or_default();
                st.since = Some((r.ts.clone(), r.by.clone().unwrap_or_default()));
                break;
            }
            "hint" => {
                if let Some(k) = r.kind.as_deref().and_then(HintKind::parse)
                    && !st.hinted.contains(&k)
                {
                    st.hinted.push(k);
                }
            }
            _ => {}
        }
    }
    st
}

/// One hint line to print after the doors.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Hint {
    pub kind: HintKind,
    pub text: String,
}

fn pastes(segments: &[Segment]) -> impl Iterator<Item = &Segment> {
    segments.iter().filter(|s| s.origin == Origin::Paste)
}

fn verdict_of(s: &Segment) -> Option<Verdict> {
    s.classified.as_ref().map(|c| c.verdict)
}

/// Whether this prompt earns a `foreign_paste` row: book mode, at least one
/// paste reading as someone else's prose, and no mode or foreign-paste row in
/// the last hour. The row only feeds the stale timer, which stays accurate to
/// the hour, so most paste prompts skip the write.
pub fn foreign_paste(segments: &[Segment], state: &SessionState, now: DateTime<Utc>) -> bool {
    matches!(state.mode, Mode::Book(_))
        && !state.last_activity.as_deref().is_some_and(|ts| {
            younger_than(ts, now, chrono::Duration::hours(FOREIGN_PASTE_EVERY_HOURS))
        })
        && pastes(segments).any(|s| verdict_of(s) == Some(Verdict::ForeignProse))
}

fn younger_than(ts: &str, now: DateTime<Utc>, age: chrono::Duration) -> bool {
    DateTime::parse_from_rfc3339(ts).is_ok_and(|t| now - t.with_timezone(&Utc) < age)
}

fn older_than_stale(ts: &str, now: DateTime<Utc>) -> bool {
    DateTime::parse_from_rfc3339(ts)
        .is_ok_and(|t| now - t.with_timezone(&Utc) > chrono::Duration::hours(STALE_AFTER_HOURS))
}

/// The hints this prompt would print under `mode`, each kind at most once per
/// mode epoch. `ours` mode never hints.
pub fn hints(
    segments: &[Segment],
    mode: &Mode,
    state: &SessionState,
    now: DateTime<Utc>,
) -> Vec<Hint> {
    let fresh = |k: HintKind| !state.hinted.contains(&k);
    let mut out = Vec::new();
    match mode {
        Mode::House => {
            let foreign = pastes(segments)
                .filter_map(|s| s.classified.as_ref())
                .find(|c| c.verdict == Verdict::ForeignProse);
            if let Some(c) = foreign
                && fresh(HintKind::Foreign)
            {
                let mut signals: Vec<_> = c
                    .signals
                    .iter()
                    .filter(|s| s.side == Side::Foreign)
                    .collect();
                signals.sort_by_key(|s| std::cmp::Reverse(s.weight));
                let names: Vec<&str> = signals.iter().take(2).map(|s| s.name).collect();
                out.push(Hint {
                    kind: HintKind::Foreign,
                    text: format!(
                        "[doors] paste reads as someone else's prose ({}): house doors held on it. Reading? mx doors mode book:<name>",
                        names.join(", ")
                    ),
                });
            }
        }
        Mode::Book(b) => {
            if fresh(HintKind::Ours)
                && pastes(segments).any(|s| verdict_of(s) == Some(Verdict::Ours))
            {
                out.push(Hint {
                    kind: HintKind::Ours,
                    text: format!(
                        "[doors] mode is book:{} but this paste reads as ours; routed to house. Done reading? mx doors mode house",
                        b.slug
                    ),
                });
            }
            let stale = state
                .last_activity
                .as_deref()
                .is_some_and(|ts| older_than_stale(ts, now));
            let since = state
                .since
                .as_ref()
                .and_then(|(ts, _)| DateTime::parse_from_rfc3339(ts).ok());
            if fresh(HintKind::Stale)
                && stale
                && pastes(segments).next().is_some()
                && let Some(since) = since
            {
                out.push(Hint {
                    kind: HintKind::Stale,
                    text: format!(
                        "[doors] book:{} since {}; still reading? mx doors mode house",
                        b.slug,
                        since.with_timezone(&Local).format("%H:%M")
                    ),
                });
            }
        }
        Mode::Ours => {}
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(event: &str, extra: &[(&str, &str)], ts: &str) -> SessionRow {
        let get = |k: &str| {
            extra
                .iter()
                .find(|(n, _)| *n == k)
                .map(|(_, v)| v.to_string())
        };
        SessionRow {
            event: event.to_string(),
            mode: get("mode"),
            kind: get("kind"),
            by: get("by"),
            ts: ts.to_string(),
        }
    }

    #[test]
    fn no_rows_is_house_never_set() {
        let st = session_state(&[]);
        assert_eq!(st.mode, Mode::House);
        assert!(st.since.is_none() && st.hinted.is_empty() && st.last_activity.is_none());
    }

    #[test]
    fn latest_mode_row_by_position_bounds_the_epoch() {
        // Newest first. The ts strings disagree with position on purpose:
        // position decides.
        let rows = [
            row("hint", &[("kind", "ours")], "2026-01-01T00:00:00Z"),
            row("foreign_paste", &[], "2026-01-01T00:00:00Z"),
            row(
                "mode",
                &[("mode", "book:walden@3"), ("by", "q")],
                "2027-01-01T00:00:00Z",
            ),
            row("hint", &[("kind", "stale")], "2026-01-01T00:00:00Z"),
            row("mode", &[("mode", "house")], "2028-01-01T00:00:00Z"),
        ];
        let st = session_state(&rows);
        assert_eq!(st.mode.to_string(), "book:walden@3");
        assert_eq!(
            st.since,
            Some(("2027-01-01T00:00:00Z".to_string(), "q".to_string()))
        );
        assert_eq!(st.hinted, vec![HintKind::Ours], "stale was a past epoch");
        assert_eq!(st.last_activity.as_deref(), Some("2026-01-01T00:00:00Z"));
    }

    #[test]
    fn foreign_paste_row_is_written_at_most_hourly() {
        use crate::doors::classify::Classified;
        use crate::doors::route::Places;
        let seg = Segment {
            origin: Origin::Paste,
            text: String::new(),
            classified: Some(Classified {
                verdict: Verdict::ForeignProse,
                f: 0,
                o: 0,
                signals: Vec::new(),
                gated: false,
            }),
            places: Places::none(),
        };
        let now = DateTime::parse_from_rfc3339("2026-01-01T12:00:00Z")
            .unwrap()
            .with_timezone(&Utc);
        let book = |last: Option<&str>| SessionState {
            mode: Mode::parse("book:walden").unwrap(),
            last_activity: last.map(str::to_string),
            ..Default::default()
        };
        let segs = std::slice::from_ref(&seg);
        assert!(!foreign_paste(
            segs,
            &book(Some("2026-01-01T11:01:00Z")),
            now
        ));
        assert!(foreign_paste(
            segs,
            &book(Some("2026-01-01T11:00:00Z")),
            now
        ));
        assert!(foreign_paste(
            segs,
            &book(Some("2026-01-01T10:59:00Z")),
            now
        ));
        assert!(foreign_paste(segs, &book(None), now));
        assert!(!foreign_paste(segs, &SessionState::default(), now), "house");
    }
}
