//! Sessions the Hermes desktop app hosts on a far machine (ticket 39), in
//! the lab of `machines.rs`. Ticket 19's lab ran the desktop app's own
//! backend (`hermes serve --isolated` with `HERMES_DESKTOP=1`) beside a
//! gateway in one home: its session waited on a question, the gateway
//! listed nothing (`session.active_list` empty), the stored row said
//! `source: desktop`, and `session.resume` of it was refused
//! (`spike-19/raw/isolated-vs-gateway.json`). Hermes's lease registry
//! keeps such a session in `runtime/active_sessions.json` by its stored
//! id with `surface: desktop` (`tui_gateway/session_lifecycle.py`,
//! `hermes_cli/active_sessions.py`). That backend is not run here: what
//! it leaves is written in that shape, a far database row and the held
//! file, and the far gateway is the pretend one, which never hears of
//! the session, as the real one did not.
//!
//! What is listed decides, not the held mark: a session in a gateway's
//! live list is the gateway's, with its cards, whatever the held file
//! says.
use super::{
    super::fake::Fake,
    fleet, here,
    history::{far_db, now, plan},
    mirrors::far,
    pace, ssh, until, Fleet, INSTALL,
};
use crate::clients::hermes::{history::History, Pace};
use rusqlite::Connection;
use serde_json::{json, Value};
use std::sync::Arc;

/// What the machines list says of a machine where the desktop app hosts
/// such a session.
pub(in crate::clients::hermes) const NOTE: &str =
    "Questions asked there are answered in Hermes desktop.";
pub(in crate::clients::hermes) const NOTE_ZH: &str =
    "在这台机器上提出的问题，请在 Hermes 桌面版中回答。";

/// The entries of the list, by id, label and host name.
const ENTRIES: [(&str, &str, &str); 2] = [("a", "Mini", "m0"), ("b", "Studio", "m1")];

/// Far machines, each followed with its refresh ticked by hand, its
/// gateway and its database, which its Hermes holds open.
pub(in crate::clients::hermes) struct Desk {
    pub fleet: Fleet,
    pub gateways: Vec<Fake>,
    pub dbs: Vec<Connection>,
    pub keys: Vec<String>,
    pub history: Vec<Arc<History>>,
}

/// `hosts` far machines, `Mini` and `Studio`; None where this machine
/// cannot run an `sshd`.
pub(in crate::clients::hermes) fn desk(hosts: usize) -> Option<Desk> {
    let mut fleet = fleet(hosts)?;
    fleet.reads_mirrors();
    let dbs = (fleet.hosts.iter())
        .map(|host| far_db(&host.root))
        .collect();
    let gateways = (fleet.hosts.iter().enumerate())
        .map(|(n, host)| host.gateway(&format!("instance-{n}"), INSTALL[n]))
        .collect();
    let mut entries = vec![here()];
    entries.extend((ENTRIES[..hosts].iter()).map(|(id, label, host)| ssh(id, label, host)));
    fleet.write(&entries);
    fleet.start_at(Pace {
        history: plan(),
        ..pace()
    });
    let keys = (ENTRIES[..hosts].iter())
        .map(|(_, label, _)| fleet.connected(label))
        .collect();
    let history = (ENTRIES[..hosts].iter())
        .map(|(id, _, _)| fleet.book().history_of(id).expect("its refresh"))
        .collect();
    Some(Desk {
        fleet,
        gateways,
        dbs,
        keys,
        history,
    })
}

/// A far session started from `source` a minute ago, in a turn, and
/// stored as Hermes stores one.
pub(in crate::clients::hermes) fn started(db: &Connection, id: &str, source: &str) {
    db.execute(
        "INSERT INTO sessions (id, source, model, started_at, input_tokens, output_tokens,
            cache_read_tokens, cache_write_tokens, cwd, title, last_activity_at)
         VALUES (?1, ?2, 'claude-fable-5', ?3, 0, 0, 0, 0, '/Users/far/work', ?4, ?3)",
        (id, source, now() - 60.0, format!("Made-up {id}")),
    )
    .expect("a session");
}

impl Desk {
    /// The far `runtime/active_sessions.json` of host `n` holds `ids` for
    /// the desktop app, as Hermes's lease registry writes it.
    pub(in crate::clients::hermes) fn holds(&self, n: usize, ids: &[&str]) {
        let runtime = self.fleet.hosts[n].root.join("runtime");
        std::fs::create_dir_all(&runtime).unwrap(); // plain directory
        let entries: Vec<Value> = (ids.iter().enumerate())
            .map(|(pid, id)| {
                json!({"session_id": id, "surface": "desktop", "pid": pid + 1,
                "metadata": {"live_session_id": "9c884584"}})
            })
            .collect();
        let text = json!({"entries": entries}).to_string();
        std::fs::write(runtime.join("active_sessions.json"), text).unwrap();
    }

    /// One tick of the refresh of host `n` that read the far side.
    pub(in crate::clients::hermes) fn tick(&self, n: usize) {
        until("a tick that read", || self.history[n].tick().reads > 0);
    }

    /// The far session `id` of host `n` as core lists it, once it does.
    pub(in crate::clients::hermes) fn listed(&mut self, n: usize, id: &str) -> Value {
        let id = far(&self.keys[n], id);
        let mut found = Value::Null;
        until(&format!("{id} is listed"), || {
            self.fleet.turn();
            let query = json!({"scope": "all", "search": ""});
            let page = self.fleet.core.request("sessions", query).unwrap();
            let items = page["items"].as_array().unwrap();
            found = (items.iter().find(|item| item["id"] == id))
                .cloned()
                .unwrap_or_default();
            !found.is_null()
        });
        found
    }

    /// The note on the machine `label`, once it is `note`.
    pub(in crate::clients::hermes) fn notes(&mut self, label: &str, note: Option<&str>) {
        until(&format!("{label}'s note is {note:?}"), || {
            let row = self.fleet.row(label).unwrap_or_default();
            row["note"].as_str() == note
        });
    }

    /// Waits until the gateway of host `n` was polled `more` times more.
    fn polled(&self, n: usize, more: usize) {
        let from = self.gateways[n].world().listed.len();
        until("more polls", || {
            self.gateways[n].world().listed.len() >= from + more
        });
    }

    /// Every frame the gateway of host `n` received from `from` on that
    /// names `id` anywhere in it.
    fn naming(&self, n: usize, from: usize, id: &str) -> Vec<Value> {
        let world = self.gateways[n].world();
        let named = |frame: &&Value| frame.to_string().contains(id);
        let received = world.received.iter().skip(from).map(|(_, frame)| frame);
        received.filter(named).cloned().collect()
    }
}

#[test]
#[ignore = "starts processes: run by the_lab_tests_pass"]
fn a_desktop_hosted_session_is_listed_with_its_machine_and_origin_and_no_card() {
    let Some(mut desk) = desk(2) else {
        return;
    };
    // On the Mini: the desktop app's session, held and in no live list,
    // and a gateway's session waiting on a question.
    started(&desk.dbs[0], "d-desk", "desktop");
    started(&desk.dbs[0], "g-tele", "telegram");
    desk.holds(0, &["d-desk"]);
    desk.gateways[0].asks("default", "g-tele", "p-tele");
    // On the Studio: a stored session, and the desktop app holds none.
    started(&desk.dbs[1], "s-tele", "telegram");
    desk.tick(0);
    desk.tick(1);

    // The gateway's session waits with its card.
    let card = desk.fleet.card();
    assert_eq!(card["sessionId"], far(&desk.keys[0], "g-tele"));
    assert_eq!(card["machine"], "Mini");
    // The desktop app's: a row with its machine and where it was started,
    // its state from its stored activity, never waiting.
    let row = desk.listed(0, "d-desk");
    assert_eq!(row["machine"], "Mini", "{row}");
    assert_eq!(
        row["origins"],
        json!({"init": "desktop:hermes", "latest": "desktop:hermes"})
    );
    assert_ne!(row["state"], "waiting", "{row}");
    let gateway = desk.listed(0, "g-tele");
    assert_eq!(gateway["origins"]["init"], "others:telegram");
    // The Studio's stored session is listed too, under its own machine.
    assert_eq!(desk.listed(1, "s-tele")["machine"], "Studio");

    // The note is on the machine that has such a session, and only there.
    desk.notes("Mini", Some(NOTE));
    desk.notes("Studio", None);
    assert_eq!(desk.fleet.row("Studio").unwrap()["state"], "connected");

    // Polls go on, and none of them names the desktop app's session:
    // no `session.resume`, no look, no read of its events.
    desk.polled(0, 4);
    let cards = desk.fleet.cards();
    assert_eq!(cards.len(), 1, "{cards:?}");
    let resumed = desk.gateways[0].world().calls("session.resume");
    assert!(!format!("{resumed:?}").contains("d-desk"), "{resumed:?}");
    for n in 0..2 {
        assert_eq!(desk.naming(n, 0, "d-desk"), Vec::<Value>::new());
    }
    assert!(!desk.naming(0, 0, "g-tele").is_empty());
    // Still listed, still noted, still no card, after all those polls.
    desk.tick(0);
    desk.notes("Mini", Some(NOTE));
    assert_eq!(
        desk.listed(0, "d-desk")["origins"]["init"],
        "desktop:hermes"
    );
    let cards = desk.fleet.cards();
    assert!(cards
        .iter()
        .all(|card| card["sessionId"] != far(&desk.keys[0], "d-desk")));
}

#[test]
#[ignore = "starts processes: run by the_lab_tests_pass"]
fn a_held_session_a_gateway_lists_is_the_gateways_and_one_no_longer_held_is_ordinary() {
    let Some(mut desk) = desk(1) else {
        return;
    };
    let session = far(&desk.keys[0], "d-desk");
    started(&desk.dbs[0], "d-desk", "desktop");
    desk.holds(0, &["d-desk"]);
    desk.tick(0);
    desk.listed(0, "d-desk");
    desk.notes("Mini", Some(NOTE));
    desk.polled(0, 2);
    assert!(desk.fleet.cards().is_empty());
    assert_eq!(desk.naming(0, 0, "d-desk"), Vec::<Value>::new());

    // The gateway now lists it, waiting on a question: held and listed,
    // it is the gateway's. Its card shows at the next poll, the gateway
    // is asked about it, and the machine has no such session any more.
    desk.gateways[0].asks_quietly("default", "d-desk", "p-desk");
    let card = desk.fleet.card();
    assert_eq!(card["sessionId"], session);
    assert!(!desk.naming(0, 0, "d-desk").is_empty());
    desk.notes("Mini", None);

    // The gateway lets go of it, its question settled: held and in no
    // live list, it is the desktop app's again.
    let settled = json!({"prompt_id": "p-desk", "outcome": "answered"});
    desk.gateways[0].tells_quietly("default", "d-desk", "clarify.settled", settled);
    until("the card goes", || desk.fleet.cards().is_empty());
    {
        let profile = desk.gateways[0].named("default");
        let mut world = desk.gateways[0].world();
        world.streams.remove(&(profile, "d-desk".to_owned()));
    }
    desk.notes("Mini", Some(NOTE));
    let from = desk.gateways[0].world().received.len();

    // The desktop app lets go of it: an ordinary stored session. Listed,
    // started where it was, no card, no note, and nobody asks about it.
    desk.holds(0, &[]);
    desk.tick(0);
    desk.notes("Mini", None);
    let row = desk.listed(0, "d-desk");
    assert_eq!(row["origins"]["init"], "desktop:hermes");
    assert_eq!(row["machine"], "Mini");
    desk.polled(0, 3);
    assert!(desk.fleet.cards().is_empty());
    assert_eq!(desk.naming(0, from, "d-desk"), Vec::<Value>::new());
}
