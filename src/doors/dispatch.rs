//! Subagent delivery: the `PreToolUse(Agent)` payload, and the `updatedInput`
//! that carries a door into the dispatched agent's prompt. Pure; the store and
//! stdio live in the handler.
//!
//! `tool_input` stays a raw `Value` from parse to print. `updatedInput` replaces
//! the whole input and a rewrite that fails validation kills the tool call, so
//! a field dropped by a typed struct would cost the dispatch, not just the door.

use serde_json::{Value, json};

use super::{FiredDoor, Reader};

const DISPATCH_PREFIX: &str = "dispatch:";

/// The `origin` of a dispatch fire row. It carries both the path and the
/// dispatch id, which has no field of its own on the row.
pub fn dispatch_origin(tool_use_id: &str) -> String {
    format!("{DISPATCH_PREFIX}{tool_use_id}")
}

/// Whether a fire row's `origin` came from the dispatch path.
pub fn is_dispatch(origin: &str) -> bool {
    origin.starts_with(DISPATCH_PREFIX)
}

/// The `stats` grouping of an `origin`: everything before the first `:`, or
/// `legacy` for a row written before origins existed.
pub fn origin_label(origin: &str) -> &str {
    match origin.split(':').next() {
        Some(label) if !label.is_empty() => label,
        _ => "legacy",
    }
}

/// A non-blank string, or a number as its text.
fn id_text(v: Option<&Value>) -> Option<String> {
    match v? {
        Value::String(s) if !s.trim().is_empty() => Some(s.clone()),
        Value::Number(n) => Some(n.to_string()),
        _ => None,
    }
}

fn non_blank(v: Option<&Value>) -> Option<&str> {
    v?.as_str().filter(|s| !s.trim().is_empty())
}

/// One `Agent` or `Task` tool call about to run.
#[derive(Debug, Clone, PartialEq)]
pub struct Dispatch {
    /// The root session, even when a subagent is the one dispatching.
    pub session: String,
    pub tool_use_id: String,
    pub prompt: String,
    subagent_type: String,
    tool_input: Value,
}

impl Dispatch {
    /// `None` for anything but a `PreToolUse` on `Agent` or `Task` with a
    /// prompt, a subagent type and both ids.
    pub fn parse(mut payload: Value) -> Option<Dispatch> {
        if payload.get("hook_event_name")?.as_str()? != "PreToolUse" {
            return None;
        }
        if !matches!(payload.get("tool_name")?.as_str()?, "Agent" | "Task") {
            return None;
        }
        let session = id_text(payload.get("session_id"))?;
        let tool_use_id = id_text(payload.get("tool_use_id"))?;
        let tool_input = payload.get_mut("tool_input")?.take();
        tool_input.as_object()?;
        Some(Dispatch {
            session,
            tool_use_id,
            prompt: non_blank(tool_input.get("prompt"))?.to_string(),
            subagent_type: non_blank(tool_input.get("subagent_type"))?.to_string(),
            tool_input,
        })
    }

    /// The agent about to be dispatched. Never the payload's `agent_type` or
    /// `agent_id`, which name the dispatcher on a nested dispatch, and never
    /// `MX_CURRENT_AGENT`, which belongs to the hook command. Not on the main
    /// thread, so no door without an `agent:` value reaches it.
    pub fn reader(&self) -> Reader {
        Reader::resolve(&self.subagent_type, None, false)
    }

    /// The hook's one line of stdout: `tool_input` with the door lines after
    /// the prompt. No `permissionDecision`: a door does not approve a dispatch.
    pub fn output(&self, fired: &[FiredDoor]) -> String {
        let lines: Vec<String> = fired.iter().map(FiredDoor::render).collect();
        let mut updated = self.tool_input.clone();
        updated["prompt"] = format!("{}\n\n{}", self.prompt, lines.join("\n")).into();
        json!({
            "hookSpecificOutput": {
                "hookEventName": "PreToolUse",
                "updatedInput": updated,
            }
        })
        .to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn payload() -> Value {
        json!({
            "session_id": "root",
            "hook_event_name": "PreToolUse",
            "tool_name": "Agent",
            "tool_use_id": "toolu_1",
            "tool_input": {
                "description": "a task",
                "prompt": "do the thing",
                "subagent_type": "Helper",
                "run_in_background": true,
                "future_field": {"nested": [1, {"deep": null}]},
            },
        })
    }

    fn with(path: &[&str], v: Value) -> Value {
        let mut p = payload();
        let (last, parents) = path.split_last().unwrap();
        let mut at = &mut p;
        for k in parents {
            at = &mut at[*k];
        }
        at[*last] = v;
        p
    }

    fn door(trigger: &str) -> FiredDoor {
        FiredDoor {
            key: "facts".to_string(),
            id: "abc123".to_string(),
            trigger: trigger.to_string(),
            fragment: "a fact".to_string(),
            dig: "facts/kv-abc123".to_string(),
            origin: "typed",
            place: "house".to_string(),
        }
    }

    #[test]
    fn origin_round_trips_through_its_helpers() {
        let o = dispatch_origin("toolu_1");
        assert_eq!(o, "dispatch:toolu_1");
        assert!(is_dispatch(&o));
        for other in ["", "typed", "paste", "channel", "authored", "dispatched"] {
            assert!(!is_dispatch(other), "{other}");
        }
    }

    #[test]
    fn origin_label_is_the_text_before_the_first_colon() {
        assert_eq!(origin_label("dispatch:toolu:1"), "dispatch");
        assert_eq!(origin_label("typed"), "typed");
        assert_eq!(origin_label(""), "legacy");
    }

    #[test]
    fn parses_a_captured_payload() {
        let d = Dispatch::parse(payload()).unwrap();
        assert_eq!(d.session, "root");
        assert_eq!(d.tool_use_id, "toolu_1");
        assert_eq!(d.prompt, "do the thing");
        assert_eq!(
            d.reader(),
            Reader {
                name: "helper".to_string(),
                main_thread: false
            }
        );
    }

    #[test]
    fn task_is_the_same_tool() {
        assert!(Dispatch::parse(with(&["tool_name"], json!("Task"))).is_some());
    }

    #[test]
    fn a_numeric_id_reads_as_its_text() {
        let d = Dispatch::parse(with(&["session_id"], json!(42))).unwrap();
        assert_eq!(d.session, "42");
    }

    #[test]
    fn the_reader_ignores_the_dispatcher() {
        let mut p = payload();
        p["agent_type"] = json!("lead");
        p["agent_id"] = json!("a1");
        assert_eq!(Dispatch::parse(p).unwrap().reader().name, "helper");
    }

    #[test]
    fn anything_else_is_not_a_dispatch() {
        for bad in [
            with(&["hook_event_name"], json!("PostToolUse")),
            with(&["tool_name"], json!("Bash")),
            with(&["session_id"], json!("")),
            with(&["session_id"], json!(null)),
            with(&["session_id"], json!({"id": 1})),
            with(&["tool_use_id"], json!("")),
            with(&["tool_input"], json!("do the thing")),
            with(&["tool_input", "prompt"], json!(7)),
            with(&["tool_input", "prompt"], json!("  \n")),
            with(&["tool_input", "subagent_type"], json!("")),
            with(&["tool_input", "subagent_type"], json!(null)),
            json!({}),
            json!([]),
        ] {
            assert_eq!(Dispatch::parse(bad.clone()), None, "{bad}");
        }
        for field in ["hook_event_name", "tool_name", "session_id", "tool_use_id"] {
            let mut p = payload();
            p.as_object_mut().unwrap().remove(field);
            assert_eq!(Dispatch::parse(p), None, "{field}");
        }
    }

    #[test]
    fn output_changes_only_the_prompt() {
        let d = Dispatch::parse(payload()).unwrap();
        let out: Value =
            serde_json::from_str(&d.output(&[door("widget"), door("sprocket")])).unwrap();
        let spec = &out["hookSpecificOutput"];
        assert_eq!(spec["hookEventName"], "PreToolUse");
        assert_eq!(spec.as_object().unwrap().len(), 2, "no permissionDecision");

        let updated = &spec["updatedInput"];
        assert_eq!(
            updated["prompt"],
            format!(
                "do the thing\n\n{}\n{}",
                door("widget").render(),
                door("sprocket").render()
            )
        );
        let mut same = updated.clone();
        same["prompt"] = payload()["tool_input"]["prompt"].clone();
        assert_eq!(same, payload()["tool_input"]);
    }

    #[test]
    fn output_is_one_line() {
        let d = Dispatch::parse(with(&["tool_input", "prompt"], json!("a\nb"))).unwrap();
        assert_eq!(d.output(&[door("widget")]).lines().count(), 1);
    }
}
