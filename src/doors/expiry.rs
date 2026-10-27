//! `expires`: when a door stops opening. Stored as an absolute RFC 3339 UTC
//! time; a door is expired iff `now >= expires`. Everything takes `now` so the
//! boundary can be pinned in tests.

use chrono::{DateTime, Duration, SecondsFormat, Utc};

const FORMS: &str = "use <n>h, <n>d or <n>w (n of 1 or more) or an RFC 3339 time such as \
                     2026-11-01T00:00:00Z; \"\" clears it on update";

/// Parse an authored `--expires` into the stored form. `""` comes back empty
/// (the clear signal). `<n>h`, `<n>d` and `<n>w` are `now` plus that long; an
/// RFC 3339 time is normalized to UTC. A time already past is accepted; the
/// caller warns.
pub fn parse_expires(input: &str, now: DateTime<Utc>) -> Result<String, String> {
    if input.is_empty() {
        return Ok(String::new());
    }
    let bad = || format!("{input:?} is not an expiry; {FORMS}");
    if let Some((n, unit)) = input
        .char_indices()
        .last()
        .map(|(i, u)| (&input[..i], u))
        .filter(|(n, _)| !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit()))
    {
        let n: i64 = n.parse().map_err(|_| bad())?;
        if n < 1 {
            return Err(bad());
        }
        let span = match unit {
            'h' => Duration::try_hours(n),
            'd' => Duration::try_days(n),
            'w' => Duration::try_weeks(n),
            _ => return Err(bad()),
        };
        let at = span
            .and_then(|d| now.checked_add_signed(d))
            .ok_or_else(|| format!("{input:?} is too far in the future"))?;
        return Ok(at.to_rfc3339_opts(SecondsFormat::Secs, true));
    }
    let at = DateTime::parse_from_rfc3339(input).map_err(|_| bad())?;
    Ok(at
        .with_timezone(&Utc)
        .to_rfc3339_opts(SecondsFormat::AutoSi, true))
}

/// The stored expiry as a time, or `None` when it does not parse.
fn expiry(stored: &str) -> Option<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(stored)
        .ok()
        .map(|t| t.with_timezone(&Utc))
}

/// Expired iff `now >= expires`. A stored value that does not parse (a hand
/// edit) counts as expired, the way an unparseable scope value admits nothing.
pub fn is_expired(stored: &str, now: DateTime<Utc>) -> bool {
    expiry(stored).is_none_or(|t| now >= t)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn t(s: &str) -> DateTime<Utc> {
        DateTime::parse_from_rfc3339(s).unwrap().with_timezone(&Utc)
    }

    const NOW: &str = "2026-10-27T12:00:00Z";

    #[test]
    fn relative_forms_add_to_now() {
        let now = t(NOW);
        assert_eq!(parse_expires("1h", now).unwrap(), "2026-10-27T13:00:00Z");
        assert_eq!(parse_expires("3d", now).unwrap(), "2026-10-30T12:00:00Z");
        assert_eq!(parse_expires("2w", now).unwrap(), "2026-11-10T12:00:00Z");
        assert_eq!(parse_expires("48h", now).unwrap(), "2026-10-29T12:00:00Z");
    }

    #[test]
    fn absolute_forms_normalize_to_utc() {
        let now = t(NOW);
        assert_eq!(
            parse_expires("2026-11-01T02:00:00+02:00", now).unwrap(),
            "2026-11-01T00:00:00Z"
        );
        assert_eq!(
            parse_expires("2026-11-01T00:00:00.5Z", now).unwrap(),
            "2026-11-01T00:00:00.500Z"
        );
        assert_eq!(
            parse_expires("2020-01-01T00:00:00Z", now).unwrap(),
            "2020-01-01T00:00:00Z",
            "a past time is accepted; the caller warns"
        );
    }

    #[test]
    fn empty_is_the_clear_signal() {
        assert_eq!(parse_expires("", t(NOW)).unwrap(), "");
    }

    #[test]
    fn bad_forms_name_the_accepted_ones() {
        for bad in [
            "0h",
            "0d",
            "00w",
            "-1d",
            "+1d",
            "1",
            "h",
            "1m",
            "1s",
            "1y",
            "1.5d",
            " 1d",
            "1d ",
            "1 d",
            "1D",
            "tomorrow",
            "2026-11-01",
            "2026-11-01 00:00:00",
            "2026-13-01T00:00:00Z",
        ] {
            let e = parse_expires(bad, t(NOW)).unwrap_err();
            assert!(
                e.contains("<n>h, <n>d or <n>w") && e.contains("RFC 3339"),
                "{bad:?}: {e}"
            );
        }
        let e = parse_expires("99999999999999w", t(NOW)).unwrap_err();
        assert!(e.contains("too far"), "{e}");
        assert!(parse_expires("99999999999999999999d", t(NOW)).is_err());
    }

    #[test]
    fn expired_iff_now_is_at_or_past_the_boundary() {
        let at = "2026-11-01T00:00:00Z";
        assert!(!is_expired(at, t("2026-10-31T23:59:59Z")));
        assert!(!is_expired(at, t("2026-10-31T23:59:59.999999999Z")));
        assert!(is_expired(at, t(at)), "at the boundary is expired");
        assert!(is_expired(at, t("2026-11-01T00:00:00.000000001Z")));
        assert!(is_expired(at, t("2027-01-01T00:00:00Z")));
    }

    #[test]
    fn an_unparseable_stored_value_is_expired() {
        for junk in ["", "soon", "2026-11-01", "1d"] {
            assert!(is_expired(junk, t(NOW)), "{junk:?}");
            assert!(expiry(junk).is_none());
        }
    }
}
