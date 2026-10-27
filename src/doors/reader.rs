//! The reader axis: who reads the prompt, and which doors that reader may see.
//! A candidate filter that runs before matching; the matcher never sees it.

use super::route::{AGENT_PREFIX, valid_agent};

/// The reader-axis values a door's scope holds, or `None` when it has none.
/// An `agent:` value that does not parse matches no reader.
pub(super) fn reader_values(scope: Option<&[String]>) -> Option<Vec<&str>> {
    let list = scope?;
    let raw: Vec<&str> = list
        .iter()
        .filter_map(|v| v.strip_prefix(AGENT_PREFIX))
        .collect();
    (!raw.is_empty()).then(|| raw.into_iter().filter(|n| valid_agent(n)).collect())
}

/// Who is reading the prompt. `name` is lowercase; `main_thread` is false
/// inside a subagent.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Reader {
    pub name: String,
    pub main_thread: bool,
}

impl Reader {
    /// The first non-empty of `agent_type` and `MX_CURRENT_AGENT`, else
    /// `none`, lowercased.
    pub fn resolve(agent_type: &str, current_agent: Option<&str>, main_thread: bool) -> Reader {
        let name = [Some(agent_type), current_agent]
            .into_iter()
            .flatten()
            .map(str::trim)
            .find(|n| !n.is_empty())
            .unwrap_or("none")
            .to_lowercase();
        Reader { name, main_thread }
    }
}

/// The reader axis. A door with `agent:` values fires only for a reader it
/// names, main thread or not; a door without them fires only on the main
/// thread. Pinned does not change this.
pub fn reader_admits(scope: Option<&[String]>, reader: &Reader) -> bool {
    match reader_values(scope) {
        None => reader.main_thread,
        Some(names) => names.contains(&reader.name.as_str()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(v: &[&str]) -> Vec<String> {
        v.iter().map(|x| x.to_string()).collect()
    }

    fn main(name: &str) -> Reader {
        Reader {
            name: name.to_string(),
            main_thread: true,
        }
    }

    fn sub(name: &str) -> Reader {
        Reader {
            name: name.to_string(),
            main_thread: false,
        }
    }

    #[test]
    fn reader_is_agent_type_then_current_agent_then_none() {
        assert_eq!(
            Reader::resolve("Helper", Some("queue"), true),
            main("helper")
        );
        assert_eq!(Reader::resolve("", Some("Queue"), true), main("queue"));
        assert_eq!(Reader::resolve("", Some(""), false), sub("none"));
        assert_eq!(Reader::resolve("", None, true), main("none"));
    }

    #[test]
    fn a_door_without_agent_values_is_main_thread_only() {
        for scope in [None, Some(s(&["house"])), Some(s(&["pinned"]))] {
            assert!(reader_admits(scope.as_deref(), &main("queue")));
            assert!(reader_admits(scope.as_deref(), &main("none")));
            assert!(
                !reader_admits(scope.as_deref(), &sub("helper")),
                "{scope:?}"
            );
        }
    }

    #[test]
    fn a_door_with_agent_values_is_for_those_readers_only() {
        let scope = s(&["agent:helper", "agent:plugin:reviewer", "house"]);
        for r in [main("helper"), sub("helper"), sub("plugin:reviewer")] {
            assert!(reader_admits(Some(&scope), &r), "{r:?}");
        }
        for r in [main("queue"), sub("queue"), main("none")] {
            assert!(!reader_admits(Some(&scope), &r), "{r:?}");
        }
        let pinned = s(&["pinned", "agent:helper"]);
        assert!(
            !reader_admits(Some(&pinned), &main("queue")),
            "pinned is place-axis only"
        );
        assert!(reader_admits(Some(&pinned), &sub("helper")));
    }

    #[test]
    fn an_unparseable_agent_value_matches_no_reader() {
        let scope = s(&["agent:Helper"]);
        for r in [main("helper"), sub("helper"), main("queue")] {
            assert!(!reader_admits(Some(&scope), &r), "{r:?}");
        }
    }
}
