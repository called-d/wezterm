//! Records reported through OSC 7501, the Program Status Protocol.
//! <https://www.superlogical.com/rex/docs/build/program-status>
//!
//! The records are published to the GUI (and through the mux to its
//! clients) as JSON in the `WEZTERM_PROGRAM_STATUS` user var, so that the
//! existing `user-var-changed` event and `pane:get_user_vars()` can see them
//! without a new mux protocol message.
use serde_json::{json, Map, Value};
use wezterm_escape_parser::osc::{ProgramStatusReport, ProgramStatusState};

pub const PROGRAM_STATUS_USER_VAR: &str = "WEZTERM_PROGRAM_STATUS";

/// The spec asks for at least 64 and suggests 256
const MAX_RECORDS: usize = 256;

#[derive(Debug, Default)]
pub(crate) struct ProgramStatusTable {
    /// Least recently reported first; the root record has an empty id
    records: Vec<(String, ProgramStatusReport)>,
}

impl ProgramStatusTable {
    pub fn apply(&mut self, report: ProgramStatusReport) {
        let id = report.id.clone().unwrap_or_default();
        if report.state == ProgramStatusState::Clear {
            self.clear(&id);
            return;
        }
        // A report replaces its record completely
        self.records.retain(|(rid, _)| *rid != id);
        self.records.push((id, report));
        if self.records.len() > MAX_RECORDS {
            self.records.remove(0);
        }
    }

    /// Removes `id` and its descendants; every record is a descendant
    /// of the root
    fn clear(&mut self, id: &str) {
        if id.is_empty() {
            self.records.clear();
            return;
        }
        let prefix = format!("{id}/");
        self.records
            .retain(|(rid, _)| rid != id && !rid.starts_with(&prefix));
    }

    /// A new shell prompt began (OSC 133 A): whatever was running in the
    /// foreground is gone.  `done` and `error` must survive; dropping
    /// `idle` is optional, but nothing is left to be idle.
    pub fn prompt_started(&mut self) {
        self.records.retain(|(_, r)| {
            !matches!(
                r.state,
                ProgramStatusState::Working
                    | ProgramStatusState::Blocked
                    | ProgramStatusState::Idle
            )
        });
    }

    pub fn reset(&mut self) {
        self.records.clear();
    }

    /// `app` comes from the nearest ancestor that has one
    fn effective_app(&self, id: &str) -> Option<&str> {
        let mut id = id;
        loop {
            if let Some(app) = self
                .records
                .iter()
                .find(|(rid, _)| rid == id)
                .and_then(|(_, r)| r.app.as_deref())
            {
                return Some(app);
            }
            if id.is_empty() {
                return None;
            }
            id = id.rsplit_once('/').map(|(parent, _)| parent).unwrap_or("");
        }
    }

    /// The records sorted by id, so that parents precede their children
    pub fn to_json(&self) -> String {
        let mut records: Vec<_> = self.records.iter().collect();
        records.sort_by(|(a, _), (b, _)| a.cmp(b));
        let records: Vec<Value> = records
            .into_iter()
            .map(|(id, r)| {
                let mut obj = Map::new();
                obj.insert("id".to_string(), json!(id));
                obj.insert("state".to_string(), json!(r.state.as_str()));
                if let Some(app) = self.effective_app(id) {
                    obj.insert("app".to_string(), json!(app));
                }
                if let Some(kind) = r.kind {
                    obj.insert("kind".to_string(), json!(kind.as_str()));
                }
                if let Some(progress) = r.progress {
                    obj.insert("progress".to_string(), json!(progress));
                }
                if let Some(title) = &r.title {
                    obj.insert("title".to_string(), json!(title));
                }
                if let Some(msg) = &r.msg {
                    obj.insert("msg".to_string(), json!(msg));
                }
                Value::Object(obj)
            })
            .collect();
        Value::Array(records).to_string()
    }
}

#[cfg(test)]
mod test {
    use super::*;

    fn report(
        state: ProgramStatusState,
        id: Option<&str>,
        app: Option<&str>,
    ) -> ProgramStatusReport {
        let mut r = ProgramStatusReport::new(state);
        r.id = id.map(str::to_string);
        r.app = app.map(str::to_string);
        r
    }

    fn ids(table: &ProgramStatusTable) -> Vec<&str> {
        let mut ids: Vec<_> = table.records.iter().map(|(id, _)| id.as_str()).collect();
        ids.sort();
        ids
    }

    #[test]
    fn replace_and_clear() {
        use ProgramStatusState::*;
        let mut t = ProgramStatusTable::default();
        t.apply(report(Working, None, Some("rex")));
        t.apply(report(Working, Some("deploy"), None));
        t.apply(report(Blocked, Some("deploy/eu-west"), None));
        t.apply(report(Working, Some("deployment"), None));
        t.apply(report(Done, Some("deploy"), None));
        assert_eq!(ids(&t), ["", "deploy", "deploy/eu-west", "deployment"]);
        assert_eq!(t.records.iter().filter(|(id, _)| id == "deploy").count(), 1);

        t.apply(report(Clear, Some("deploy"), None));
        assert_eq!(ids(&t), ["", "deployment"]);
        t.apply(report(Clear, None, None));
        assert!(ids(&t).is_empty());
    }

    #[test]
    fn app_inheritance() {
        use ProgramStatusState::*;
        let mut t = ProgramStatusTable::default();
        t.apply(report(Working, None, Some("rex")));
        t.apply(report(Working, Some("a/b"), None));
        t.apply(report(Working, Some("a/b/c"), Some("cargo")));
        assert_eq!(t.effective_app("a/b"), Some("rex"));
        assert_eq!(t.effective_app("a/b/c"), Some("cargo"));
        assert_eq!(t.effective_app("a/b/c/d"), Some("cargo"));
    }

    #[test]
    fn prompt_keeps_finished_records() {
        use ProgramStatusState::*;
        let mut t = ProgramStatusTable::default();
        t.apply(report(Working, Some("w"), None));
        t.apply(report(Blocked, Some("b"), None));
        t.apply(report(Idle, Some("i"), None));
        t.apply(report(Done, Some("d"), None));
        t.apply(report(Error, Some("e"), None));
        t.prompt_started();
        assert_eq!(ids(&t), ["d", "e"]);
    }

    #[test]
    fn evicts_least_recent() {
        let mut t = ProgramStatusTable::default();
        for i in 0..=MAX_RECORDS {
            t.apply(report(
                ProgramStatusState::Working,
                Some(&format!("r{i}")),
                None,
            ));
        }
        assert_eq!(t.records.len(), MAX_RECORDS);
        assert!(!t.records.iter().any(|(id, _)| id == "r0"));
    }

    #[test]
    fn json() {
        let mut t = ProgramStatusTable::default();
        let mut r = report(ProgramStatusState::Blocked, Some("x"), None);
        r.kind = Some(wezterm_escape_parser::osc::ProgramStatusKind::Question);
        r.msg = Some("ok?".to_string());
        t.apply(r);
        t.apply(report(ProgramStatusState::Idle, None, Some("claude-code")));
        assert_eq!(
            serde_json::from_str::<Value>(&t.to_json()).unwrap(),
            json!([
                {"app": "claude-code", "id": "", "state": "idle"},
                {"app": "claude-code", "id": "x", "kind": "question", "msg": "ok?", "state": "blocked"},
            ])
        );
    }
}
