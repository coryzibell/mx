//! Modes, scope values and places: where a segment may open doors, and which
//! doors a place admits.

use std::fmt;

/// `<slug>` or `<slug>@<n>`, the part after `book:` in a scope value or mode.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Book {
    pub slug: String,
    /// Reading position. `None` is an unknown position, read as page one.
    pub at: Option<u32>,
}

impl Book {
    fn parse(rest: &str) -> Result<Book, String> {
        let (slug, at) = match rest.split_once('@') {
            Some((s, n)) => (s, Some(n)),
            None => (rest, None),
        };
        let slug_ok = slug
            .bytes()
            .next()
            .is_some_and(|b| b.is_ascii_lowercase() || b.is_ascii_digit())
            && slug
                .bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-');
        if !slug_ok {
            return Err(format!(
                "has book slug {slug:?}; a slug is lowercase letters, digits and '-', starting with a letter or digit"
            ));
        }
        let at = match at {
            None => None,
            Some(n) => {
                let digits = !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit());
                if !digits || n.bytes().all(|b| b == b'0') {
                    return Err(format!(
                        "has position {n:?}; the part after '@' is a whole number of 1 or more"
                    ));
                }
                if n.starts_with('0') {
                    return Err(format!(
                        "has position {n:?}; write the position without leading zeros"
                    ));
                }
                match n.parse::<u32>() {
                    Ok(v) => Some(v),
                    Err(_) => {
                        return Err(format!(
                            "has position {n:?}, which is too large; the most is {}",
                            u32::MAX
                        ));
                    }
                }
            }
        };
        Ok(Book {
            slug: slug.to_string(),
            at,
        })
    }
}

impl fmt::Display for Book {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.at {
            Some(n) => write!(f, "book:{}@{}", self.slug, n),
            None => write!(f, "book:{}", self.slug),
        }
    }
}

/// One value of an entry's `scope` list.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ScopeValue {
    House,
    /// Fires in every place and every mode.
    Pinned,
    Book(Book),
}

impl ScopeValue {
    /// The error reads after the value, e.g. `"person:x" arrives in P3`.
    pub fn parse(s: &str) -> Result<ScopeValue, String> {
        match s {
            "house" => Ok(ScopeValue::House),
            "pinned" => Ok(ScopeValue::Pinned),
            _ => {
                if let Some(rest) = s.strip_prefix("book:") {
                    return Book::parse(rest).map(ScopeValue::Book);
                }
                if s.starts_with("person:") || s.starts_with("agent:") {
                    return Err(
                        "is not accepted yet: person: and agent: scopes arrive in a later phase (P3)"
                            .to_string(),
                    );
                }
                Err(
                    "is not a scope; accepted: house, pinned, book:<slug>, book:<slug>@<n>"
                        .to_string(),
                )
            }
        }
    }
}

/// How the session is reading pastes right now.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum Mode {
    #[default]
    House,
    /// Pastes are ours: never quarantined, no hints.
    Ours,
    Book(Book),
}

impl Mode {
    pub fn parse(s: &str) -> Result<Mode, String> {
        match s {
            "house" => Ok(Mode::House),
            "ours" => Ok(Mode::Ours),
            _ => match s.strip_prefix("book:") {
                Some(rest) => Book::parse(rest).map(Mode::Book),
                None => Err("is not a mode; use house, ours or book:<slug>[@<n>]".to_string()),
            },
        }
    }
}

impl fmt::Display for Mode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Mode::House => f.write_str("house"),
            Mode::Ours => f.write_str("ours"),
            Mode::Book(b) => b.fmt(f),
        }
    }
}

/// A place a segment can open doors in.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Place {
    House,
    Book(String),
}

impl fmt::Display for Place {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Place::House => f.write_str("house"),
            Place::Book(slug) => write!(f, "book:{slug}"),
        }
    }
}

/// The places a segment opens doors in. Empty is `∅`: quarantined, only
/// pinned doors match.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Places(pub Vec<Place>);

impl Places {
    pub fn house() -> Places {
        Places(vec![Place::House])
    }

    pub fn book(slug: &str) -> Places {
        Places(vec![Place::Book(slug.to_string())])
    }

    pub fn none() -> Places {
        Places(Vec::new())
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// The budget pool a hit here spends. `∅` spends the house pool.
    pub fn pool(&self) -> Pool {
        if self.0.iter().any(|p| matches!(p, Place::Book(_))) {
            Pool::Book
        } else {
            Pool::House
        }
    }

    pub fn labels(&self) -> Vec<String> {
        self.0.iter().map(Place::to_string).collect()
    }
}

impl fmt::Display for Places {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.0.is_empty() {
            f.write_str("\u{2205} (quarantined)")
        } else {
            write!(f, "{{{}}}", self.labels().join(", "))
        }
    }
}

/// One of the two per-prompt budgets.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Pool {
    House,
    Book,
}

impl Pool {
    pub fn label(self) -> &'static str {
        match self {
            Pool::House => "house",
            Pool::Book => "book",
        }
    }
}

/// The values a door's scope holds; absent is `[house]`. A value that does not
/// parse (a hand-edited store) admits nothing.
fn scope_values(scope: Option<&[String]>) -> Vec<ScopeValue> {
    match scope {
        None => vec![ScopeValue::House],
        Some(list) => list
            .iter()
            .filter_map(|v| ScopeValue::parse(v).ok())
            .collect(),
    }
}

pub fn is_pinned(scope: Option<&[String]>) -> bool {
    scope.is_some_and(|s| s.iter().any(|v| v == "pinned"))
}

/// The spoiler gate. A door at position `k` opens only once the mode has read
/// to `k` in the same book; an ordinal-less door opens at any position, and an
/// ordinal-less mode is page one.
fn gate_open(door: &Book, mode: &Mode) -> bool {
    match door.at {
        None => true,
        Some(k) => {
            matches!(mode, Mode::Book(m) if m.slug == door.slug && m.at.is_some_and(|n| k <= n))
        }
    }
}

/// The place in `places` a door with `scope` opens through under `mode`, if
/// any. Pinned is not consulted here: it bypasses places altogether.
pub fn admitting_place<'p>(
    scope: Option<&[String]>,
    places: &'p Places,
    mode: &Mode,
) -> Option<&'p Place> {
    let values = scope_values(scope);
    places.0.iter().find(|place| {
        values.iter().any(|v| match (v, place) {
            (ScopeValue::House, Place::House) => true,
            (ScopeValue::Book(door), Place::Book(slug)) => {
                door.slug == *slug && gate_open(door, mode)
            }
            _ => false,
        })
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(v: &[&str]) -> Vec<String> {
        v.iter().map(|x| x.to_string()).collect()
    }

    #[test]
    fn scope_values_accept_the_documented_forms() {
        for ok in [
            "house",
            "pinned",
            "book:walden",
            "book:walden@3",
            "book:9-lives",
            "book:a@1",
        ] {
            assert!(ScopeValue::parse(ok).is_ok(), "{ok}");
        }
    }

    #[test]
    fn scope_values_reject_everything_else() {
        for bad in [
            "",
            "House",
            "book:",
            "book:Walden",
            "book:-x",
            "book:x@0",
            "book:x@",
            "book:x@abc",
            "book:x@+3",
            "book:x@-1",
            "book:x@1@2",
            "everyone",
        ] {
            assert!(ScopeValue::parse(bad).is_err(), "{bad:?}");
        }
        for later in ["person:someone", "agent:helper"] {
            let e = ScopeValue::parse(later).unwrap_err();
            assert!(e.contains("later phase"), "{e}");
        }
    }

    #[test]
    fn ordinals_refuse_leading_zeros_and_name_overflow() {
        for bad in ["book:x@01", "book:x@007", "book:x@00"] {
            assert!(ScopeValue::parse(bad).is_err(), "{bad:?}");
            assert!(Mode::parse(bad).is_err(), "{bad:?}");
        }
        let e = ScopeValue::parse("book:x@01").unwrap_err();
        assert!(e.contains("leading zeros"), "{e}");
        let e = ScopeValue::parse("book:x@99999999999999999999").unwrap_err();
        assert!(e.contains("too large") && !e.contains("1 or more"), "{e}");
        assert!(ScopeValue::parse("book:x@4294967295").is_ok());
        assert!(ScopeValue::parse("book:x@10").is_ok());
    }

    #[test]
    fn modes_parse_and_print() {
        for m in ["house", "ours", "book:walden", "book:walden@12"] {
            assert_eq!(Mode::parse(m).unwrap().to_string(), m);
        }
        for bad in ["", "book", "pinned", "book:x@0", "Book:x"] {
            assert!(Mode::parse(bad).is_err(), "{bad:?}");
        }
    }

    fn admits(scope: &[&str], places: &Places, mode: &str) -> bool {
        let scope = s(scope);
        admitting_place(Some(&scope), places, &Mode::parse(mode).unwrap()).is_some()
    }

    #[test]
    fn spoiler_gate() {
        let walden = Places::book("walden");
        assert!(!admits(&["book:walden@12"], &walden, "book:walden@11"));
        assert!(admits(&["book:walden@12"], &walden, "book:walden@12"));
        assert!(admits(&["book:walden@12"], &walden, "book:walden@40"));
        assert!(
            !admits(&["book:walden@1"], &walden, "book:walden"),
            "no ordinal is page one: only ordinal-less doors"
        );
        assert!(admits(&["book:walden"], &walden, "book:walden"));
        assert!(admits(&["book:walden"], &walden, "book:walden@3"));
        assert!(!admits(&["book:prince"], &walden, "book:walden"));
    }

    #[test]
    fn absent_scope_is_house_and_or_across_values() {
        assert!(admitting_place(None, &Places::house(), &Mode::House).is_some());
        assert!(admitting_place(None, &Places::book("walden"), &Mode::House).is_none());
        assert!(admits(
            &["book:walden", "house"],
            &Places::house(),
            "book:walden"
        ));
        assert!(!admits(&["pinned"], &Places::house(), "house"));
        assert!(!admits(&["house"], &Places::none(), "house"));
    }

    #[test]
    fn places_print_and_pool() {
        assert_eq!(Places::house().to_string(), "{house}");
        assert_eq!(Places::book("walden").to_string(), "{book:walden}");
        assert_eq!(Places::none().to_string(), "\u{2205} (quarantined)");
        assert_eq!(Places::none().pool(), Pool::House);
        assert_eq!(Places::book("walden").pool(), Pool::Book);
    }
}
