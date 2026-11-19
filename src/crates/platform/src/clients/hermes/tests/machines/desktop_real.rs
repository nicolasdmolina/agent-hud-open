//! Ticket 39 with real Hermes: ticket 19's lab of a desktop-style
//! backend. One throwaway Hermes home runs a real gateway and, beside it,
//! a backend started as the Hermes desktop app starts one over SSH
//! (`HERMES_DESKTOP=1 hermes serve --isolated ...`). Each has a session
//! waiting on a question. The far login of the lab's `sshd` is told that
//! home as its `HERMES_HOME`, so the HUD reaches it as a far machine.
//! Run by hand, once the lab's driver says it is up:
//!
//! ```sh
//! AGENT_HUD_DESKTOP_LAB=/path/to/lab \
//!   cargo test -p agent-hud-platform --lib \
//!   clients::hermes::tests_unix::machines::desktop_real -- --ignored --nocapture
//! ```
//!
//! The lab folder holds `lab39.json`, the driver's ids and its home,
//! never the user's own. What the HUD showed is written to `out/` there.
use super::{
    desktop::{NOTE, NOTE_ZH},
    fleet_with, here,
    mirrors::far,
    pace, ssh, until, Fleet,
};
use crate::clients::hermes::{history::Plan, tap, Pace};
use serde_json::{json, Value};
use std::{
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

const LAB: &str = "AGENT_HUD_DESKTOP_LAB";

/// The row core lists as `id`, once it does.
fn listed(fleet: &mut Fleet, id: &str) -> Value {
    let mut found = Value::Null;
    until(&format!("{id} is listed"), || {
        fleet.turn();
        let page = fleet.core.request("sessions", json!({"scope": "all"}));
        let page = page.unwrap();
        let items = page["items"].as_array().unwrap();
        found = (items.iter().find(|item| item["id"] == id))
            .cloned()
            .unwrap_or_default();
        !found.is_null()
    });
    found
}

fn note(fleet: &mut Fleet) -> Option<String> {
    let row = fleet.row("Mini").unwrap_or_default();
    row["note"].as_str().map(str::to_owned)
}

/// Every frame written to a gateway that names one of `ids`, by method.
fn naming(frames: &[Value], ids: &[&str]) -> Vec<String> {
    let names = |frame: &&Value| ids.iter().any(|id| frame.to_string().contains(id));
    let method = |frame: &Value| frame["method"].as_str().unwrap_or("a result").to_owned();
    frames.iter().filter(names).map(method).collect()
}

/// What the HUD shows now, in `language`, written to `out/<name>`, once
/// the machines list speaks it: its words are made as its status is told.
fn keep(fleet: &mut Fleet, out: &Path, name: &str, language: &str) {
    let setting = json!({"key": "language", "value": language});
    fleet.core.request("set_setting", setting).unwrap();
    let said = if language == "en" { NOTE } else { NOTE_ZH };
    until(&format!("the note in {language}"), || {
        fleet.turn();
        note(fleet).as_deref() == Some(said)
    });
    let snapshot = fleet.snapshot();
    let text = serde_json::to_string_pretty(&snapshot).unwrap();
    std::fs::write(out.join(format!("{name}-{language}.json")), text).unwrap();
}

#[test]
#[ignore = "by hand: needs ticket 39's lab of real Hermes, named in AGENT_HUD_DESKTOP_LAB"]
fn a_real_desktop_backends_question_is_a_row_and_the_gateways_a_card() {
    let Ok(lab) = std::env::var(LAB) else {
        println!("skipped: {LAB} names no lab");
        return;
    };
    let lab = PathBuf::from(lab);
    let ids: Value =
        serde_json::from_slice(&std::fs::read(lab.join("lab39.json")).unwrap()).unwrap();
    let root = ids["root"].as_str().unwrap();
    let own = std::env::var("HOME").unwrap_or_default();
    assert!(
        !Path::new(root).starts_with(Path::new(&own).join(".hermes")),
        "never the user's own Hermes home"
    );
    let id = |who: &str, which: &str| ids[who][which].as_str().unwrap().to_owned();
    let (desk, desk_live) = (id("desktop", "stored"), id("desktop", "live"));
    let (gate, gate_live) = (id("gateway", "stored"), id("gateway", "live"));
    let out = lab.join("out");
    std::fs::create_dir_all(&out).unwrap();

    tap::keep_frames();
    let Some(mut fleet) = fleet_with(1, &[("HERMES_HOME", root)]) else {
        return;
    };
    fleet.reads_mirrors();
    fleet.write(&[here(), ssh("a", "Mini", "m0")]);
    let history = Plan {
        clock: false,
        ..Plan::default()
    };
    fleet.start_at(Pace { history, ..pace() });
    let key = fleet.connected("Mini");
    let refresh = fleet.book().history_of("a").expect("its refresh");

    // The gateway's session waits with its card.
    let card = fleet.card();
    println!(
        "card: {}",
        json!({"sessionId": card["sessionId"], "machine": card["machine"]})
    );
    assert_eq!(card["machine"], "Mini");
    let carded = card["sessionId"].as_str().unwrap();
    assert!(
        carded == far(&key, &gate) || carded == far(&key, &gate_live),
        "{carded}"
    );

    // The desktop backend's: a row with its machine and where it was
    // started, from the far database and the held file.
    until("a tick that read", || refresh.tick().reads > 0);
    let row = listed(&mut fleet, &far(&key, &desk));
    println!(
        "desktop row: {}",
        json!({"id": row["id"], "machine": row["machine"],
        "origins": row["origins"], "state": row["state"]})
    );
    assert_eq!(row["machine"], "Mini", "{row}");
    assert_eq!(row["origins"]["init"], "desktop:hermes", "{row}");
    assert_ne!(row["state"], "waiting", "{row}");
    let gateway_row = listed(&mut fleet, &far(&key, &gate));
    println!(
        "gateway row: {}",
        json!({"id": gateway_row["id"], "machine": gateway_row["machine"],
        "origins": gateway_row["origins"], "state": gateway_row["state"]})
    );
    until("the note", || note(&mut fleet).as_deref() == Some(NOTE));

    // Twenty seconds of polls: still one card, and no frame the HUD wrote
    // to the gateway names the backend's session.
    let start = Instant::now();
    while start.elapsed() < Duration::from_secs(20) {
        fleet.turn();
        std::thread::sleep(Duration::from_millis(200));
    }
    refresh.tick();
    assert_eq!(note(&mut fleet).as_deref(), Some(NOTE));
    let cards = fleet.cards();
    assert_eq!(cards.len(), 1, "{cards:?}");
    let frames = tap::frames();
    let methods = |frames: &[Value]| {
        let mut all: Vec<String> = frames
            .iter()
            .map(|f| f["method"].as_str().unwrap_or("a result").to_owned())
            .collect();
        all.sort();
        all.dedup();
        all
    };
    let named = naming(&frames, &[&desk, &desk_live]);
    let gateway_named = naming(&frames, &[&gate, &gate_live]);
    let log = json!({"frames": frames.len(), "methods": methods(&frames),
        "naming_desktop": named, "naming_gateway": gateway_named});
    std::fs::write(
        out.join("frames-held.json"),
        serde_json::to_string_pretty(&log).unwrap(),
    )
    .unwrap();
    println!("frames while held: {log}");
    assert_eq!(
        named,
        Vec::<String>::new(),
        "a call named the desktop session"
    );
    assert!(
        !gateway_named.is_empty(),
        "the gateway's session was polled"
    );
    keep(&mut fleet, &out, "held", "zh-CN");
    keep(&mut fleet, &out, "held", "en");

    // A gateway client asks the gateway to take the session up (ticket
    // 19's resume, which 0.22.5 took). Where the gateway then lists it, it
    // is the gateway's: the note goes and calls name it. Where it refuses
    // (0.23.1 says `runtime_coordination_required`), nothing changes here.
    let from = tap::frames().len();
    std::fs::write(lab.join("resume"), "").unwrap();
    until("the driver resumed it", || lab.join("resumed").exists());
    let said: Value =
        serde_json::from_slice(&std::fs::read(lab.join("logs/resume.json")).unwrap()).unwrap();
    println!("the gateway's answer to the resume: {said}");
    let taken = said["error"].is_null();
    if taken {
        until("the note goes", || {
            fleet.turn();
            note(&mut fleet).is_none()
        });
    } else {
        let start = Instant::now();
        while start.elapsed() < Duration::from_secs(10) {
            fleet.turn();
            std::thread::sleep(Duration::from_millis(200));
        }
        assert_eq!(note(&mut fleet).as_deref(), Some(NOTE));
        assert_eq!(fleet.cards().len(), 1);
    }
    let after = tap::frames()[from..].to_vec();
    let named = naming(&after, &[&desk, &desk_live]);
    let log = json!({"taken": taken, "frames": after.len(), "methods": methods(&after),
        "naming_desktop": named});
    let text = serde_json::to_string_pretty(&log).unwrap();
    std::fs::write(out.join("frames-after-resume.json"), text).unwrap();
    println!("frames after the resume: {log}");
    assert_eq!(
        !named.is_empty(),
        taken,
        "calls name it only once it is listed"
    );
    if !taken {
        keep(&mut fleet, &out, "after-resume", "en");
    }
}
