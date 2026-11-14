//! Codex Desktop's cards sound exactly as on `origin/fork` (ticket 41, after
//! its fifth verifier). Its rollouts show a request it waits on as well as
//! its card (`Channel::transcript_shows_pending`), so its cards get no early
//! row and wait for the rollout, as before ticket 41. Each scenario is one of
//! the verifier's Codex probes, told as a trace; `tools/compare-fork.sh` in
//! the ticket's evidence runs this same file on an `origin/fork` archive and
//! compares the traces byte for byte. The cards are built as
//! `platform/src/clients/codex/desktop/pending.rs` makes them (no `callId`)
//! unless a scenario names one through the native API.
use super::*;

struct Clock;
impl Clock {
    fn at(now: i64) -> Self {
        crate::snapshot::PINNED_CLOCK.set(Some(now));
        Self
    }
}
impl Drop for Clock {
    fn drop(&mut self) {
        crate::snapshot::PINNED_CLOCK.set(None);
    }
}

const THREAD: &str = "00000000-0000-4000-8000-000000000041";

struct Lab {
    core: AgentHudCore,
    folder: PathBuf,
    now: i64,
    trace: Vec<String>,
    _temp: tempfile::TempDir,
    _clock: Clock,
}

impl Lab {
    fn new() -> Self {
        let temp = tempfile::tempdir().unwrap();
        let now = Utc::now().timestamp_millis();
        let clock = Clock::at(now);
        let folder = temp.path().join(".codex/sessions");
        fs::create_dir_all(&folder).unwrap();
        let mut core = AgentHudCore::new(vec![], temp.path().join("app"));
        core.provider_roots = vec![(folder.clone(), "codex")];
        core.apply_setting("sound", &json!(true)).unwrap();
        core.request("snapshot", json!({})).unwrap();
        Self {
            core,
            folder,
            now,
            trace: Vec::new(),
            _temp: temp,
            _clock: clock,
        }
    }
    /// A card of `kind` as the Desktop parser makes it, or naming `call`.
    fn card(&self, id: &str, kind: &str, call: Option<&str>) {
        let mut view = json!({"source":"codex","vendor":"Codex","sessionId":THREAD,"title":"Synthetic","summary":"Synthetic","toolName":"request_user_input_async","kind":kind,"input":"","detail":null,"cwd":"/synthetic","createdAt":crate::iso(self.now),"allowsAlways":false,"answersQuestion":kind=="question","answersPlan":false,"plan":null,"suggestions":[],"questions":[]});
        if let Some(call) = call {
            view["callId"] = json!(call);
        }
        let ticket = self
            .core
            .requests()
            .register_native(id.into(), view, self.now)
            .unwrap();
        std::mem::forget(ticket);
    }
    /// The rollout with `calls` open, each a `request_user_input_async`.
    fn rollout(&self, calls: &[&str]) {
        let mut lines = vec![
            json!({"timestamp":crate::iso(self.now-3000),"type":"session_meta","payload":{"id":THREAD,"timestamp":crate::iso(self.now-3000),"source":"vscode","originator":"Codex Desktop","cwd":"/synthetic"}}),
            json!({"timestamp":crate::iso(self.now-2000),"type":"event_msg","payload":{"type":"task_started","turn_id":"turn-41"}}),
        ];
        for call in calls {
            lines.push(json!({"timestamp":crate::iso(self.now-1000),"type":"response_item","payload":{"type":"function_call","call_id":call,"name":"request_user_input_async","arguments":"{}"}}));
        }
        let text: String = lines.iter().map(|line| format!("{line}\n")).collect();
        fs::write(self.folder.join(format!("rollout-{THREAD}.jsonl")), text).unwrap();
    }
    /// One look, noted as `name`: whether the session has a row, and how
    /// many `needs-you` alerts the snapshot carries.
    fn look(&mut self, name: &str, refresh: bool) {
        let method = if refresh {
            "refresh"
        } else {
            "request_snapshot"
        };
        let snapshot = self.core.request(method, json!({})).unwrap();
        let rows = snapshot["sessions"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|r| r["id"] == THREAD)
            .count();
        let needs = snapshot["soundAlerts"]["items"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|a| a["kind"] == "needs-you")
            .count();
        self.trace.push(format!("{name}:rows={rows},needs={needs}"));
    }
    fn later(&mut self, ms: i64) {
        crate::snapshot::PINNED_CLOCK.set(Some(self.now + ms));
    }
}

/// Every scenario's trace, one line each.
fn traces() -> Vec<String> {
    let mut out = Vec::new();
    // Round 1: one and two question cards, the rollout after them, at once
    // or five minutes later, or both in one look.
    for (name, count, together, delay) in [
        ("one-question", 1, false, 0),
        ("two-questions", 2, false, 0),
        ("two-questions-5min", 2, false, 300_000),
        ("two-questions-together", 2, true, 0),
    ] {
        let mut lab = Lab::new();
        for i in 0..count {
            lab.card(&format!("codex-desktop-question:{i}"), "question", None);
        }
        if !together {
            lab.look("early", false);
        }
        lab.later(delay);
        lab.rollout(&["call-41"]);
        lab.look("rollout", true);
        lab.look("again", false);
        out.push(format!("{name} {}", lab.trace.join(" ")));
    }
    // Round 2: an older permission card, then a rollout with a question of
    // its own, then that question's card; the permission settles.
    {
        let mut lab = Lab::new();
        lab.card("codex-desktop-permission:old", "permission", None);
        lab.look("early", false);
        lab.rollout(&["separate-new-question"]);
        lab.look("rollout", true);
        lab.card("codex-desktop-question:7", "question", None);
        lab.look("card", false);
        lab.core
            .requests()
            .settle_native("codex-desktop-permission:old");
        lab.look("settled", false);
        out.push(format!("permission-then-question {}", lab.trace.join(" ")));
    }
    // Round 3: a card naming its call, and a rollout with that call and a
    // new one; the new one's card after.
    {
        let mut lab = Lab::new();
        lab.card("named-native-card", "question", Some("known-question"));
        lab.look("early", false);
        lab.rollout(&["known-question", "brand-new-question"]);
        lab.look("rollout", true);
        lab.card("codex-desktop-question:9", "question", None);
        lab.look("card", false);
        out.push(format!("named-and-new {}", lab.trace.join(" ")));
    }
    // Round 5: a card naming its call, refreshed without it, and a rollout
    // with only a new call (`round5_compare_new_question_after_an_early_named_card_and_metadata_refresh`).
    {
        let mut lab = Lab::new();
        lab.card("same-live-card", "question", Some("a"));
        lab.look("early", false);
        let view =
            json!({"source":"codex","sessionId":THREAD,"kind":"question","title":"A refreshed"});
        lab.core.requests().refresh_native("same-live-card", view);
        lab.rollout(&["brand-new"]);
        lab.look("rollout", true);
        lab.look("again", false);
        out.push(format!("refreshed-named {}", lab.trace.join(" ")));
    }
    // Round 4: a named and an unnamed card; the rollout with both calls,
    // gone, then back with a new one.
    {
        let mut lab = Lab::new();
        lab.card("r4-named", "question", Some("a"));
        lab.card("r4-unnamed", "question", None);
        lab.look("early", false);
        lab.rollout(&["a", "b"]);
        lab.look("rollout", true);
        fs::remove_file(lab.folder.join(format!("rollout-{THREAD}.jsonl"))).unwrap();
        lab.look("gone", true);
        lab.rollout(&["a", "b", "brand-new"]);
        lab.look("back", true);
        out.push(format!("mixed-two-takeovers {}", lab.trace.join(" ")));
    }
    out
}

#[test]
fn codex_cards_sound_as_on_fork() {
    let got = traces();
    for line in &got {
        eprintln!("TRACE {line}");
    }
    let expected = [
        "one-question early:rows=0,needs=0 rollout:rows=1,needs=1 again:rows=1,needs=1",
        "two-questions early:rows=0,needs=0 rollout:rows=1,needs=1 again:rows=1,needs=1",
        "two-questions-5min early:rows=0,needs=0 rollout:rows=1,needs=1 again:rows=1,needs=1",
        "two-questions-together rollout:rows=1,needs=1 again:rows=1,needs=1",
        "permission-then-question early:rows=0,needs=0 rollout:rows=1,needs=1 card:rows=1,needs=1 settled:rows=1,needs=1",
        "named-and-new early:rows=0,needs=0 rollout:rows=1,needs=1 card:rows=1,needs=1",
        "refreshed-named early:rows=0,needs=0 rollout:rows=1,needs=1 again:rows=1,needs=1",
        "mixed-two-takeovers early:rows=0,needs=0 rollout:rows=1,needs=1 gone:rows=0,needs=1 back:rows=1,needs=2",
    ];
    assert_eq!(got, expected);
}
