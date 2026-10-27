//! `doors_people`: who a Matrix sender is, and whose typed text this is.
//!
//! A state key whose field names are full Matrix user ids and whose values are
//! JSON objects: `{"person":"<name>","house":true|false,"owner":true}`. A
//! sender resolves on its `user_id` alone, by exact string match. Display
//! names never participate: `segment` already drops them.

use std::collections::HashMap;

use serde::Deserialize;

use super::route::{Place, Places, valid_person};

/// The kv state key holding the map.
pub const PEOPLE_KEY: &str = "doors_people";

/// One mapped user id.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct Person {
    pub person: String,
    #[serde(default)]
    pub house: bool,
    #[serde(default)]
    pub owner: bool,
}

/// The parsed map. Entries that do not parse, or whose `person` fails the name
/// syntax, are left out, so those senders are unmapped.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct People {
    by_id: HashMap<String, Person>,
    owners: Vec<String>,
}

impl People {
    /// Build from the state key's `(user_id, json)` fields, in field order.
    /// Owners keep that order, deduplicated by person name.
    pub fn from_fields<'a>(fields: impl IntoIterator<Item = (&'a str, &'a str)>) -> People {
        let mut out = People::default();
        for (user_id, raw) in fields {
            let Ok(p) = serde_json::from_str::<Person>(raw) else {
                continue;
            };
            if !valid_person(&p.person) {
                continue;
            }
            if p.owner && !out.owners.contains(&p.person) {
                out.owners.push(p.person.clone());
            }
            out.by_id.insert(user_id.to_string(), p);
        }
        out
    }

    /// The person names whose typed text this is.
    pub fn owners(&self) -> &[String] {
        &self.owners
    }

    /// The mapped entry for `user_id`, exact match.
    pub fn get(&self, user_id: &str) -> Option<&Person> {
        self.by_id.get(user_id)
    }

    /// Where a channel body from `user_id` opens doors, in every mode.
    /// Mapped: `{house, person:<p>}` when `house`, else `{person:<p>}`.
    /// Unmapped: `{person:<localpart>}`, never house (D3), or `∅` when the
    /// id has no usable localpart.
    pub fn channel_places(&self, user_id: &str) -> Places {
        match self.get(user_id) {
            Some(p) if p.house => Places(vec![Place::House, Place::Person(p.person.clone())]),
            Some(p) => Places(vec![Place::Person(p.person.clone())]),
            None => match localpart(user_id) {
                Some(l) => Places(vec![Place::Person(l)]),
                None => Places::none(),
            },
        }
    }
}

/// The text between a leading `@` and the first `:`, lowercased, when it is a
/// valid person name. Localparts carrying `=` or `/` fall out here, on purpose.
pub fn localpart(user_id: &str) -> Option<String> {
    let (l, _) = user_id.strip_prefix('@')?.split_once(':')?;
    let l = l.to_lowercase();
    valid_person(&l).then_some(l)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn people(fields: &[(&str, &str)]) -> People {
        People::from_fields(fields.iter().copied())
    }

    fn person(n: &str) -> Place {
        Place::Person(n.to_string())
    }

    #[test]
    fn mapped_house_sender_opens_house_and_person() {
        let p = people(&[("@alice:example.org", r#"{"person":"alice","house":true}"#)]);
        assert_eq!(
            p.channel_places("@alice:example.org"),
            Places(vec![Place::House, person("alice")])
        );
    }

    #[test]
    fn mapped_non_house_sender_opens_only_person() {
        for v in [r#"{"person":"bob","house":false}"#, r#"{"person":"bob"}"#] {
            let p = people(&[("@bob:example.org", v)]);
            assert_eq!(
                p.channel_places("@bob:example.org"),
                Places(vec![person("bob")]),
                "{v}"
            );
        }
    }

    #[test]
    fn unmapped_sender_is_its_localpart_and_never_house() {
        let p = People::default();
        assert_eq!(
            p.channel_places("@Mallory:example.org"),
            Places(vec![person("mallory")])
        );
        assert_eq!(
            p.channel_places("@m.a_l-1:x:y"),
            Places(vec![person("m.a_l-1")])
        );
    }

    #[test]
    fn malformed_user_ids_place_nothing() {
        let p = People::default();
        for bad in [
            "",
            "mallory:example.org",
            "@mallory",
            "@:example.org",
            "@mal=lory:example.org",
            "@mal/lory:example.org",
            "@-mallory:example.org",
            "@ mallory:example.org",
        ] {
            assert_eq!(p.channel_places(bad), Places::none(), "{bad:?}");
        }
    }

    #[test]
    fn resolution_is_exact_on_the_user_id() {
        let p = people(&[("@alice:example.org", r#"{"person":"alice","house":true}"#)]);
        for other in [
            "@Alice:example.org",
            "@alice:example.org ",
            "@alice:evil.org",
        ] {
            assert!(p.get(other).is_none(), "{other:?}");
            assert!(
                !p.channel_places(other).0.contains(&Place::House),
                "{other:?} must not reach house"
            );
        }
    }

    #[test]
    fn bad_entries_are_unmapped() {
        let p = people(&[
            ("@a:example.org", "not json"),
            ("@b:example.org", r#""bob""#),
            ("@c:example.org", r#"{"house":true}"#),
            ("@d:example.org", r#"{"person":"Bob","house":true}"#),
            ("@e:example.org", r#"{"person":"","house":true}"#),
            ("@f:example.org", r#"{"person":"bob","house":"yes"}"#),
            ("@g:example.org", r#"{"person":7,"house":true}"#),
        ]);
        for id in ["a", "b", "c", "d", "e", "f", "g"] {
            let uid = format!("@{id}:example.org");
            assert!(p.get(&uid).is_none(), "{uid}");
            assert_eq!(p.channel_places(&uid), Places(vec![person(id)]), "{uid}");
        }
        assert!(p.owners().is_empty());
    }

    #[test]
    fn owners_are_deduplicated_in_field_order() {
        let p = people(&[
            ("@a:example.org", r#"{"person":"bob","owner":true}"#),
            (
                "@b:example.org",
                r#"{"person":"alice","owner":true,"house":true}"#,
            ),
            ("@c:example.org", r#"{"person":"bob","owner":true}"#),
            ("@d:example.org", r#"{"person":"carol","owner":false}"#),
            ("@e:example.org", r#"{"person":"Dave","owner":true}"#),
        ]);
        assert_eq!(p.owners(), ["bob".to_string(), "alice".to_string()]);
    }
}
