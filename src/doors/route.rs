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
    /// Opens where this person is speaking (a channel sender) or is the owner
    /// of typed text.
    Person(String),
    /// The reader axis: the door fires only for this reader.
    Agent(String),
}

pub(super) const AGENT_PREFIX: &str = "agent:";

/// `[a-z0-9][a-z0-9._-]*`: a person name in a scope value or `doors_people`.
pub fn valid_person(s: &str) -> bool {
    name_ok(s, b"._-")
}

/// `[a-z0-9][a-z0-9._:-]*`: agent types carry `-` and `plugin:` prefixes.
pub fn valid_agent(s: &str) -> bool {
    name_ok(s, b"._:-")
}

fn name_ok(s: &str, extra: &[u8]) -> bool {
    let word = |b: u8| b.is_ascii_lowercase() || b.is_ascii_digit();
    s.bytes().next().is_some_and(word) && s.bytes().all(|b| word(b) || extra.contains(&b))
}

impl ScopeValue {
    /// The error reads after the value, e.g. `"book:" has book slug ""`.
    pub fn parse(s: &str) -> Result<ScopeValue, String> {
        match s {
            "house" => Ok(ScopeValue::House),
            "pinned" => Ok(ScopeValue::Pinned),
            _ => {
                if let Some(rest) = s.strip_prefix("book:") {
                    return Book::parse(rest).map(ScopeValue::Book);
                }
                if let Some(name) = s.strip_prefix("person:") {
                    return if valid_person(name) {
                        Ok(ScopeValue::Person(name.to_string()))
                    } else {
                        Err(format!(
                            "has person name {name:?}; a person name is lowercase letters, digits, '.', '_' and '-', starting with a letter or digit"
                        ))
                    };
                }
                if let Some(name) = s.strip_prefix(AGENT_PREFIX) {
                    return if valid_agent(name) {
                        Ok(ScopeValue::Agent(name.to_string()))
                    } else {
                        Err(format!(
                            "has agent name {name:?}; an agent name is lowercase letters, digits, '.', '_', ':' and '-', starting with a letter or digit"
                        ))
                    };
                }
                Err(
                    "is not a scope; accepted: house, pinned, book:<slug>, book:<slug>@<n>, person:<name>, agent:<name>"
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
    Person(String),
}

impl Place {
    /// The budget pool a hit admitted through this place spends: a book place
    /// spends the book pool, `house` and `person:*` the house pool.
    pub fn pool(&self) -> Pool {
        match self {
            Place::Book(_) => Pool::Book,
            Place::House | Place::Person(_) => Pool::House,
        }
    }
}

impl fmt::Display for Place {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Place::House => f.write_str("house"),
            Place::Book(slug) => write!(f, "book:{slug}"),
            Place::Person(name) => write!(f, "person:{name}"),
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

    /// The budget pool a pinned hit here spends. `∅` spends the house pool.
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

/// The place-axis values a door's scope holds; absent is `[house]`, and so is
/// a scope holding only `agent:` values. A value that does not parse (a
/// hand-edited store) admits nothing.
fn place_values(scope: Option<&[String]>) -> Vec<ScopeValue> {
    let Some(list) = scope else {
        return vec![ScopeValue::House];
    };
    let places: Vec<&String> = list
        .iter()
        .filter(|v| !v.starts_with(AGENT_PREFIX))
        .collect();
    if places.is_empty() && !list.is_empty() {
        return vec![ScopeValue::House];
    }
    places
        .into_iter()
        .filter_map(|v| ScopeValue::parse(v).ok())
        .collect()
}

pub fn is_pinned(scope: Option<&[String]>) -> bool {
    scope.is_some_and(|s| s.iter().any(|v| v == "pinned"))
}

/// Whether two doors' scopes can open on the same segment for the same
/// reader. Place axis: either is pinned; or they share a place value (books by
/// slug, ordinals ignored); or one has `house` or `book:*` and the other
/// `person:*`. Reader axis: they miss only when both name agents and no name
/// is shared. Absent scope is `[house]`.
pub fn scopes_overlap(a: Option<&[String]>, b: Option<&[String]>) -> bool {
    places_overlap(a, b) && readers_overlap(a, b)
}

fn places_overlap(a: Option<&[String]>, b: Option<&[String]>) -> bool {
    if is_pinned(a) || is_pinned(b) {
        return true;
    }
    let (va, vb) = (place_values(a), place_values(b));
    let person = |v: &[ScopeValue]| v.iter().any(|x| matches!(x, ScopeValue::Person(_)));
    let wide = |v: &[ScopeValue]| {
        v.iter()
            .any(|x| matches!(x, ScopeValue::House | ScopeValue::Book(_)))
    };
    let shared = va.iter().any(|x| {
        vb.iter().any(|y| match (x, y) {
            (ScopeValue::House, ScopeValue::House) => true,
            (ScopeValue::Book(p), ScopeValue::Book(q)) => p.slug == q.slug,
            (ScopeValue::Person(p), ScopeValue::Person(q)) => p == q,
            _ => false,
        })
    });
    shared || (wide(&va) && person(&vb)) || (person(&va) && wide(&vb))
}

fn readers_overlap(a: Option<&[String]>, b: Option<&[String]>) -> bool {
    match (
        super::reader::reader_values(a),
        super::reader::reader_values(b),
    ) {
        (Some(x), Some(y)) => x.iter().any(|n| y.contains(n)),
        _ => true,
    }
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
    let values = place_values(scope);
    places.0.iter().find(|place| {
        values.iter().any(|v| match (v, place) {
            (ScopeValue::House, Place::House) => true,
            (ScopeValue::Book(door), Place::Book(slug)) => {
                door.slug == *slug && gate_open(door, mode)
            }
            (ScopeValue::Person(door), Place::Person(name)) => door == name,
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
            "person:alice",
            "person:a.b_c-1",
            "person:9",
            "agent:helper",
            "agent:general-purpose",
            "agent:plugin:reviewer",
            "agent:a.b_c",
        ] {
            assert!(ScopeValue::parse(ok).is_ok(), "{ok}");
        }
        assert_eq!(
            ScopeValue::parse("person:alice"),
            Ok(ScopeValue::Person("alice".to_string()))
        );
        assert_eq!(
            ScopeValue::parse("agent:plugin:reviewer"),
            Ok(ScopeValue::Agent("plugin:reviewer".to_string()))
        );
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
        for bad in [
            "person:",
            "person:Alice",
            "person:-alice",
            "person:.alice",
            "person:al ice",
            "person:al:ice",
            "person:al=ice",
            "person:al/ice",
        ] {
            let e = ScopeValue::parse(bad).unwrap_err();
            assert!(e.contains("person name"), "{bad:?}: {e}");
        }
        for bad in [
            "agent:",
            "agent:Helper",
            "agent::x",
            "agent:-x",
            "agent:a b",
        ] {
            let e = ScopeValue::parse(bad).unwrap_err();
            assert!(e.contains("agent name"), "{bad:?}: {e}");
        }
        let e = ScopeValue::parse("everyone").unwrap_err();
        assert!(
            e.contains("person:<name>") && e.contains("agent:<name>"),
            "{e}"
        );
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
    fn person_values_admit_through_their_person_place() {
        let alice = Places(vec![Place::Person("alice".to_string())]);
        assert!(admits(&["person:alice"], &alice, "house"));
        assert!(!admits(&["person:bob"], &alice, "house"));
        assert!(
            !admits(&["house"], &alice, "house"),
            "a person place is not house"
        );
        assert!(!admits(&["person:alice"], &Places::house(), "house"));
        let both = Places(vec![Place::House, Place::Person("alice".to_string())]);
        assert_eq!(
            admitting_place(Some(&s(&["person:alice"])), &both, &Mode::House),
            Some(&Place::Person("alice".to_string()))
        );
    }

    #[test]
    fn agent_values_are_not_places() {
        assert!(
            admits(&["agent:helper"], &Places::house(), "house"),
            "agent-only is house"
        );
        assert!(!admits(
            &["agent:helper"],
            &Places::book("walden"),
            "book:walden"
        ));
        assert!(admits(
            &["agent:helper", "book:walden"],
            &Places::book("walden"),
            "book:walden"
        ));
        assert!(!admits(
            &["agent:helper", "book:walden"],
            &Places::house(),
            "house"
        ));
        assert!(
            !admits(&["agent:helper", "bogus"], &Places::house(), "house"),
            "an unparseable place value still admits nothing"
        );
        let empty: Vec<String> = Vec::new();
        assert!(
            admitting_place(Some(&empty), &Places::house(), &Mode::House).is_none(),
            "a hand-edited empty scope admits nothing, as before"
        );
    }

    #[test]
    fn a_place_spends_its_own_pool() {
        assert_eq!(Place::House.pool(), Pool::House);
        assert_eq!(Place::Person("a".to_string()).pool(), Pool::House);
        assert_eq!(Place::Book("w".to_string()).pool(), Pool::Book);
        assert_eq!(Place::Person("a".to_string()).to_string(), "person:a");
    }

    type Pair<'a> = (Option<&'a [&'a str]>, Option<&'a [&'a str]>);

    fn overlap(a: Option<&[&str]>, b: Option<&[&str]>) -> bool {
        let (a, b) = (a.map(s), b.map(s));
        let ab = scopes_overlap(a.as_deref(), b.as_deref());
        assert_eq!(ab, scopes_overlap(b.as_deref(), a.as_deref()), "symmetric");
        ab
    }

    #[test]
    fn scopes_overlap_on_the_place_axis() {
        let yes: &[Pair<'_>] = &[
            (None, None),
            (None, Some(&["house"])),
            (Some(&["pinned"]), Some(&["book:walden"])),
            (Some(&["pinned"]), Some(&["person:alice"])),
            (Some(&["book:walden@3"]), Some(&["book:walden@9"])),
            (Some(&["book:walden"]), Some(&["book:walden@2", "house"])),
            (Some(&["person:alice"]), Some(&["person:alice"])),
            (None, Some(&["person:alice"])),
            (Some(&["book:walden"]), Some(&["person:alice"])),
            (Some(&["agent:helper"]), None),
        ];
        for (a, b) in yes {
            assert!(overlap(*a, *b), "{a:?} {b:?}");
        }
        let no: &[Pair<'_>] = &[
            (None, Some(&["book:walden"])),
            (Some(&["book:walden"]), Some(&["book:prince"])),
            (Some(&["person:alice"]), Some(&["person:bob"])),
            (Some(&["bogus"]), None),
        ];
        for (a, b) in no {
            assert!(!overlap(*a, *b), "{a:?} {b:?}");
        }
    }

    #[test]
    fn scopes_overlap_on_the_reader_axis() {
        assert!(overlap(Some(&["agent:helper"]), Some(&["agent:helper"])));
        assert!(overlap(
            Some(&["agent:helper", "agent:x"]),
            Some(&["agent:x"])
        ));
        assert!(
            overlap(Some(&["agent:helper"]), Some(&["house"])),
            "main thread of --agent helper"
        );
        assert!(!overlap(Some(&["agent:helper"]), Some(&["agent:reviewer"])));
        assert!(
            !overlap(
                Some(&["pinned", "agent:helper"]),
                Some(&["pinned", "agent:reviewer"])
            ),
            "both axes must overlap"
        );
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
