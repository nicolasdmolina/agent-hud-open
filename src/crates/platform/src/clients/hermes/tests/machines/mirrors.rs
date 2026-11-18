//! Far sessions as rows (ticket 34), in the lab of `machines.rs`: the
//! machines the app runs, a far gateway behind a throwaway `sshd`, and a
//! real core that reads the mirrors' folder as one of its roots.
//!
//! Core has no file watcher here. Before every pass it hears that no
//! file changed, as from a watcher with nothing to report, so a row is
//! listed only because the mirrors' writer said which file it wrote. A
//! turn is what the runtime does when a channel's wake asks for one
//! (`agent_hud_runtime`, `request`): the paths told go to
//! `AgentHudCore::written_files`, and a pass reads them.
use super::{
    super::fake::{Fake, Hold},
    fleet, here, ssh, until, Fleet, INSTALL,
};
use crate::clients::hermes::mirror::Writer;
use agent_hud_agents::hermes::{
    gateway::{Listed, Listing, Profile},
    mirror,
};
use agent_hud_core::{AgentHudCore, SourceRoot};
use serde_json::{json, Value};
use std::{
    collections::BTreeMap,
    os::unix::fs::{MetadataExt, PermissionsExt},
    path::{Path, PathBuf},
    sync::Arc,
    time::SystemTime,
};

/// One far session as core lists it.
#[derive(Clone, Debug, PartialEq)]
pub(in crate::clients::hermes) struct Row {
    pub id: String,
    pub machine: Value,
    pub title: Value,
    pub tokens: Value,
}

impl Fleet {
    /// From here on core reads the far machines' mirrors, as the app's
    /// core does: their folder in the data folder is one of its roots.
    /// The first pass walks it, as at launch. Called before `start`, and
    /// again for a relaunch.
    pub(in crate::clients::hermes) fn reads_mirrors(&mut self) {
        let root = SourceRoot {
            path: self.data.join(mirror::ROOT),
            provider: "hermes",
        };
        self.core = AgentHudCore::new(vec![root], self.data.join("core"));
        self.requests = self.core.requests();
        self.silent = true;
        self.core.request("refresh", json!({})).unwrap();
    }

    /// One turn of the collector at a channel's wake: how many files the
    /// writers had told of.
    pub(in crate::clients::hermes) fn turn(&mut self) -> usize {
        self.hear();
        let told = std::mem::take(&mut *self.told.lock().unwrap());
        let count = told.len();
        if count == 0 {
            self.core.request("request_snapshot", json!({})).unwrap();
        } else {
            self.core.written_files(told);
            self.core.request("snapshot", json!({})).unwrap();
        }
        count
    }

    /// Every far session core lists, by id.
    pub(in crate::clients::hermes) fn far_rows(&mut self) -> Vec<Row> {
        let query = json!({"scope": "all", "search": ""});
        let found = self.core.request("sessions", query).unwrap();
        let far = |item: &&Value| item["id"].as_str().is_some_and(|id| id.contains('/'));
        let mut rows: Vec<Row> = (found["items"].as_array().unwrap().iter())
            .filter(far)
            .map(|item| Row {
                id: item["id"].as_str().unwrap().to_owned(),
                machine: item["machine"].clone(),
                title: item["title"].clone(),
                tokens: item["tokens"].clone(),
            })
            .collect();
        rows.sort_by(|a, b| a.id.cmp(&b.id));
        rows
    }

    /// Waits until the far session `id` is listed, taking turns as the
    /// writers ask for them.
    pub(in crate::clients::hermes) fn listed(&mut self, id: &str) -> Row {
        let mut found = None;
        until(&format!("{id} is listed"), || {
            self.turn();
            found = self.far_rows().into_iter().find(|row| row.id == id);
            found.is_some()
        });
        found.unwrap()
    }

    /// Whether a writer has told of a file since the last turn: the
    /// collector is asked for one.
    pub(in crate::clients::hermes) fn asked(&self) -> bool {
        !self.told.lock().unwrap().is_empty()
    }

    /// Whether core lists the far session `id`, which is titled `title`
    /// and found by that: a page of the list holds 200 sessions at most.
    pub(in crate::clients::hermes) fn lists(&mut self, id: &str, title: &str) -> bool {
        let query = json!({"scope": "all", "search": title});
        let found = self.core.request("sessions", query).unwrap();
        let items = found["items"].as_array().unwrap();
        items.iter().any(|item| item["id"] == id)
    }

    /// How many sessions core lists.
    pub(in crate::clients::hermes) fn count(&mut self) -> usize {
        let query = json!({"scope": "all", "search": ""});
        let found = self.core.request("sessions", query).unwrap();
        found["total"].as_u64().unwrap() as usize
    }

    /// The mirror writer of the machine the entry `entry` owns.
    pub(in crate::clients::hermes) fn writer(&self, entry: &str) -> Arc<Writer> {
        let followers = self.book().followers_of(entry).expect("followed");
        followers.mirror.clone().expect("a writer")
    }

    /// Every mirror there is, by its folder's name.
    fn mirrors(&self) -> Vec<String> {
        let folders = std::fs::read_dir(self.data.join(mirror::ROOT).join("profiles"));
        let mut names: Vec<String> = (folders.into_iter().flatten().flatten())
            .map(|folder| folder.file_name().to_string_lossy().into_owned())
            .collect();
        names.sort();
        names
    }

    pub(in crate::clients::hermes) fn mirror(&self, key: &str, profile: &str) -> PathBuf {
        let folder = format!("{key}-{profile}");
        (self.data.join(mirror::ROOT).join("profiles").join(folder)).join("state.db")
    }
}

/// The far session id of `session` of the default profile of `key`.
pub(in crate::clients::hermes) fn far(key: &str, session: &str) -> String {
    format!("hermes:{key}/default:{session}")
}

/// A session of the default profile is in a turn, and the live list says
/// this of it. No hint is given.
pub(in crate::clients::hermes) fn works(gateway: &Fake, session: &str, title: &str) {
    let mut world = gateway.world();
    let stream = world.stream_in("default", session);
    stream.epoch = "epoch-1".into();
    stream.running = true;
    stream.row = json!({"title": title, "model": "claude-test", "source": "telegram",
        "started_at": 1_760_000_000.0, "last_active": 1_760_000_100.0, "message_count": 3});
}

/// Every row of a mirror's tables, read without changing the file.
fn dump(path: &Path) -> String {
    let flags = rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY;
    let db = rusqlite::Connection::open_with_flags(path, flags).unwrap();
    let mut all = String::new();
    for table in [
        "sessions",
        "session_model_usage",
        "hud_mirror",
        "hud_rows",
        "hud_live",
    ] {
        let sql = format!("SELECT * FROM {table} ORDER BY 1, 2");
        let mut statement = db.prepare(&sql).unwrap();
        let width = statement.column_count();
        let mut rows = statement.query([]).unwrap();
        while let Some(row) = rows.next().unwrap() {
            let cells: Vec<rusqlite::types::Value> =
                (0..width).map(|n| row.get(n).unwrap()).collect();
            all += &format!("{table} {cells:?}\n");
        }
    }
    all
}

/// The mirror's own file: its size and when it last changed.
fn stamp(path: &Path) -> (u64, SystemTime) {
    let meta = std::fs::metadata(path).unwrap();
    (meta.len(), meta.modified().unwrap())
}

/// A listing of the default profile with one session, for a write put
/// on a writer's queue by hand.
fn listing(session: &str, read_ms: i64) -> Listing {
    let said = Listed {
        title: Some("Put there by hand".into()),
        ..Listed::default()
    };
    let rows = vec![(session.to_owned(), said)];
    Listing {
        read_ms,
        profiles: BTreeMap::from([(Profile::Default, rows)]),
    }
}

/// One far machine, `Mini`, followed, with a session in a turn that has
/// its row; the machine's key.
fn one_machine() -> Option<(Fleet, Fake, String)> {
    let mut fleet = fleet(2)?;
    fleet.reads_mirrors();
    let gateway = fleet.hosts[0].gateway("instance-1", INSTALL[0]);
    works(&gateway, "s-one", "Plan the week");
    fleet.write(&[here(), ssh("a", "Mini", "m0")]);
    fleet.start();
    let key = fleet.connected("Mini");
    fleet.listed(&far(&key, "s-one"));
    Some((fleet, gateway, key))
}

#[test]
#[ignore = "starts processes: run by the_lab_tests_pass"]
fn a_far_session_gets_its_row_from_the_live_list() {
    let Some(mut fleet) = fleet(1) else {
        return;
    };
    fleet.reads_mirrors();
    let before = fleet.snapshot();
    let gateway = fleet.hosts[0].gateway("instance-1", INSTALL[0]);
    works(&gateway, "s-one", "Plan the week");
    fleet.write(&[here(), ssh("a", "Mini", "m0")]);
    fleet.start();
    let key = fleet.connected("Mini");
    let row = fleet.listed(&far(&key, "s-one"));
    println!("a far row: {row:?}");
    println!("{}", fleet.log());
    assert_eq!(row.machine, "Mini");
    assert_eq!(row.title, "Plan the week");
    // Nothing is counted for it: no tokens, and the totals as they were.
    assert_eq!(row.tokens, 0);
    let after = fleet.snapshot();
    assert_eq!(after["totals"], before["totals"]);

    // Its mirror is the machine's one file, this user's alone, with a
    // row the live list made: no counts, and not durable.
    assert_eq!(fleet.mirrors(), [format!("{key}-default")]);
    let path = fleet.mirror(&key, "default");
    let mode = |path: &Path| std::fs::metadata(path).unwrap().permissions().mode() & 0o777;
    assert_eq!(mode(&path), 0o600);
    assert_eq!(mode(path.parent().unwrap()), 0o700);
    assert_eq!(mode(&fleet.data.join(mirror::ROOT)), 0o700);
    let rows = dump(&path);
    assert!(rows.contains("sessions [Text(\"s-one\"), Real(1760000000.0), Text(\"Plan the week\"), Null, Text(\"telegram\"), Text(\"claude-test\"), Null, Null, Null, Null, Null, Null, Null, Null, Null, Null]"), "{rows}");
    assert!(
        rows.contains(&format!(
            "hud_mirror [Integer(1), Text(\"{key}\"), Text(\"Mini\"), Text(\"default\")]"
        )),
        "{rows}"
    );
    assert!(
        rows.contains("hud_rows [Text(\"s-one\"), Integer(0), "),
        "{rows}"
    );
    assert!(
        !rows.contains("session_model_usage") && !rows.contains("hud_live"),
        "{rows}"
    );

    // A second session, a few polls on: one more row, by one more write.
    works(&gateway, "s-two", "Fix the build");
    let second = fleet.listed(&far(&key, "s-two"));
    assert_eq!((second.machine, second.tokens), (json!("Mini"), json!(0)));
    assert_eq!(fleet.far_rows().len(), 2);

    // Nothing new ran on the far machine for any of it: the gateway was
    // asked what it was asked before there were rows, on the one link.
    let asked: std::collections::BTreeSet<String> = (gateway.world().received.iter())
        .filter_map(|(_, frame)| frame["method"].as_str().map(str::to_owned))
        .collect();
    let known = [
        "profiles.list",
        "session.active_list",
        "session.events.since",
    ];
    assert!(
        asked.iter().all(|method| known.contains(&method.as_str())),
        "{asked:?}"
    );
    assert_eq!(fleet.masters().len(), 1);
}

/// Ticket 54: the desktop app's list can hold an `ssh` entry with no
/// label. Its machine was followed and had cards, and no mirror could be
/// claimed for it, so its sessions had no rows. It goes by its host.
#[test]
#[ignore = "starts processes: run by the_lab_tests_pass"]
fn an_entry_with_no_label_gets_its_rows_under_its_host() {
    let Some(mut fleet) = fleet(1) else {
        return;
    };
    fleet.reads_mirrors();
    let gateway = fleet.hosts[0].gateway("instance-1", INSTALL[0]);
    works(&gateway, "s-one", "Plan the week");
    let mut unnamed = ssh("a", "", "m0");
    unnamed.as_object_mut().unwrap().remove("label");
    fleet.write(&[here(), unnamed]);
    fleet.start();
    let key = fleet.connected("m0");
    let row = fleet.listed(&far(&key, "s-one"));
    assert_eq!(row.machine, "m0");
    let rows = dump(&fleet.mirror(&key, "default"));
    let named =
        format!("hud_mirror [Integer(1), Text(\"{key}\"), Text(\"m0\"), Text(\"default\")]");
    assert!(rows.contains(&named), "{rows}");
    assert!(!fleet.log().contains("mirror failed"), "{}", fleet.log());
    // Given a label, it is the same machine under it.
    fleet.write(&[here(), ssh("a", "Mini", "m0")]);
    assert_eq!(fleet.connected("Mini"), key);
    until("its rows say the label", || {
        fleet.turn();
        fleet.far_rows().iter().all(|row| row.machine == "Mini")
    });
}

#[test]
#[ignore = "starts processes: run by the_lab_tests_pass"]
fn a_second_machines_mirror_made_after_startup_is_found_without_a_relaunch() {
    let Some((mut fleet, _gateway, key)) = one_machine() else {
        return;
    };
    // An entry for another machine is added while everything runs.
    let other = fleet.hosts[1].gateway("instance-2", INSTALL[1]);
    works(&other, "o-one", "Water the plants");
    fleet.write(&[here(), ssh("a", "Mini", "m0"), ssh("b", "Studio", "m1")]);
    let second = fleet.connected("Studio");
    assert_ne!(second, key);
    let row = fleet.listed(&far(&second, "o-one"));
    assert_eq!(row.machine, "Studio");
    assert_eq!(fleet.mirrors(), {
        let mut both = [format!("{key}-default"), format!("{second}-default")];
        both.sort();
        both
    });
    let labels: Vec<Value> = fleet
        .far_rows()
        .into_iter()
        .map(|row| row.machine)
        .collect();
    assert_eq!(labels.len(), 2);
    assert!(labels.contains(&json!("Mini")) && labels.contains(&json!("Studio")));
}

#[test]
#[ignore = "starts processes: run by the_lab_tests_pass"]
fn a_renamed_entry_reaches_the_rows() {
    let Some((mut fleet, gateway, key)) = one_machine() else {
        return;
    };
    works(&gateway, "s-two", "Fix the build");
    fleet.listed(&far(&key, "s-two"));
    let ids: Vec<String> = fleet.far_rows().into_iter().map(|row| row.id).collect();
    let path = fleet.mirror(&key, "default");
    assert!(dump(&path).contains("Text(\"Mini\")"));

    fleet.write(&[here(), ssh("a", "Mac mini", "m0")]);
    // The label in `hud_mirror` changes, by the writer.
    until("the mirror says the new label", || {
        dump(&path).contains(&format!(
            "hud_mirror [Integer(1), Text(\"{key}\"), Text(\"Mac mini\")"
        ))
    });
    // And every session of that machine shows it after the next pass.
    until("the writer told core", || {
        !fleet.told.lock().unwrap().is_empty()
    });
    assert_eq!(fleet.turn(), 1);
    let rows = fleet.far_rows();
    assert_eq!(
        rows.iter().map(|row| row.id.clone()).collect::<Vec<_>>(),
        ids
    );
    assert!(rows.iter().all(|row| row.machine == "Mac mini"), "{rows:?}");
    // The same machine, by the same route: nothing was dialled anew.
    assert_eq!(fleet.book().far(), [(key, Some("a".into()), 1)]);
}

#[test]
#[ignore = "starts processes: run by the_lab_tests_pass"]
fn retiring_the_last_owner_stops_writes() {
    let Some((mut fleet, gateway, key)) = one_machine() else {
        return;
    };
    let path = fleet.mirror(&key, "default");
    let writer = fleet.writer("a");

    // The only entry of the machine is removed.
    fleet.write(&[here()]);
    until("the machine is retired", || fleet.book().far().is_empty());
    // Its writer has stopped and closed the file by now: this is the
    // mirror as it stays.
    let (rows, stamped) = (dump(&path), stamp(&path));
    fleet.turn();

    // The far machine goes on: a new session there, and writes put on
    // the old writer's queue by hand, one under each route number the
    // machine can have had by then, so that it is the retirement that
    // stops them and not a number that moved on.
    works(&gateway, "s-after", "After it was retired");
    for number in 1..=3 {
        let session = format!("s-by-hand-{number}");
        writer.listed(listing(&session, i64::MAX - 3 + number as i64), number);
    }
    writer.settle();
    fleet.looked(8);
    std::thread::sleep(std::time::Duration::from_millis(300));
    assert_eq!(dump(&path), rows);
    assert_eq!(stamp(&path), stamped);
    assert!(fleet.told.lock().unwrap().is_empty(), "nothing was written");

    // Its sessions are still listed, with its label, pass after pass.
    for _ in 0..3 {
        fleet.snapshot();
        let rows = fleet.far_rows();
        assert_eq!(rows.len(), 1, "{rows:?}");
        assert_eq!(
            (rows[0].id.clone(), rows[0].machine.clone()),
            (far(&key, "s-one"), json!("Mini"))
        );
    }
    assert!(fleet.log().contains(&format!("retired machine={key}")));
}

#[test]
#[ignore = "starts processes: run by the_lab_tests_pass"]
fn same_machine_new_address_keeps_the_mirror_writable() {
    let Some((mut fleet, gateway, key)) = one_machine() else {
        return;
    };
    let path = fleet.mirror(&key, "default");
    let (file, writer) = (std::fs::metadata(&path).unwrap().ino(), fleet.writer("a"));
    assert_eq!(fleet.book().far(), [(key.clone(), Some("a".into()), 1)]);

    // The entry's address is another name for the same install.
    fleet.write(&[here(), ssh("a", "Mini", "m0b")]);
    until("the route changed", || {
        fleet.book().far() == [(key.clone(), Some("a".into()), 2)]
    });
    // A session started after the change gets its row, in the same file
    // and by the same writer.
    works(&gateway, "s-two", "Started after the change");
    let row = fleet.listed(&far(&key, "s-two"));
    assert_eq!(row.machine, "Mini");
    assert_eq!(fleet.mirrors(), [format!("{key}-default")]);
    assert_eq!(std::fs::metadata(&path).unwrap().ino(), file);
    assert!(Arc::ptr_eq(&fleet.writer("a"), &writer));
    assert_eq!(fleet.far_rows().len(), 2);
    assert!(!fleet.log().contains("retired"), "{}", fleet.log());
}

#[test]
#[ignore = "starts processes: run by the_lab_tests_pass"]
fn one_alias_retargeted_leaves_the_owners_mirror_writable() {
    let Some(mut fleet) = fleet(2) else {
        return;
    };
    fleet.reads_mirrors();
    let gateway = fleet.hosts[0].gateway("instance-1", INSTALL[0]);
    works(&gateway, "s-one", "Plan the week");
    // One install behind two entries, the first of which owns it.
    fleet.write(&[
        here(),
        ssh("a", "Mini", "m0"),
        ssh("b", "Mini over VPN", "m0b"),
    ]);
    fleet.start();
    let key = fleet.connected("Mini");
    fleet.stands("Mini over VPN", "same-machine");
    until("the first entry owns the machine", || {
        fleet.book().far() == [(key.clone(), Some("a".into()), fleet.number())]
    });
    fleet.listed(&far(&key, "s-one"));
    let (number, writer) = (fleet.number(), fleet.writer("a"));

    // The second entry is pointed at another install.
    let other = fleet.hosts[1].gateway("instance-2", INSTALL[1]);
    works(&other, "o-one", "Water the plants");
    fleet.write(&[here(), ssh("a", "Mini", "m0"), ssh("b", "Studio", "m1")]);
    let second = fleet.connected("Studio");
    assert_ne!(second, key);

    // The first machine's mirror goes on taking writes, by its writer,
    // over the route it had.
    works(&gateway, "s-two", "Started after the retarget");
    let row = fleet.listed(&far(&key, "s-two"));
    assert_eq!(row.machine, "Mini");
    assert!(Arc::ptr_eq(&fleet.writer("a"), &writer));
    let first = fleet
        .book()
        .far()
        .into_iter()
        .find(|far| far.0 == key)
        .unwrap();
    assert_eq!(first, (key.clone(), Some("a".into()), number));
    // And the new machine has a mirror of its own.
    let row = fleet.listed(&far(&second, "o-one"));
    assert_eq!(row.machine, "Studio");
    let mut both = [format!("{key}-default"), format!("{second}-default")];
    both.sort();
    assert_eq!(fleet.mirrors(), both);
    assert!(!dump(&fleet.mirror(&second, "default")).contains("s-one"));
    assert!(!dump(&fleet.mirror(&key, "default")).contains("o-one"));
}

/// A poll's list is held back at the gateway with a session in it that
/// no later list has; the route changes; the list comes back. The
/// session gets no row. Nor does a write put on the writer's queue under
/// the old route number.
#[test]
#[ignore = "starts processes: run by the_lab_tests_pass"]
fn an_old_routes_late_write_is_rejected() {
    use std::sync::atomic::Ordering;
    for retarget in [false, true] {
        let Some((mut fleet, gateway, key)) = one_machine() else {
            return;
        };
        let path = fleet.mirror(&key, "default");
        let writer = fleet.writer("a");
        let other = fleet.hosts[1].gateway("instance-2", INSTALL[1]);
        works(&other, "o-one", "Water the plants");

        // The next list is answered with `s-late` in it, and held.
        let hold = Arc::new(Hold::default());
        works(&gateway, "s-late", "Read under the old route");
        gateway.world().hold_next = Some(("session.active_list", hold.clone()));
        until("a list is held", || hold.entered.load(Ordering::Acquire));
        gateway
            .world()
            .streams
            .remove(&("default".to_owned(), "s-late".to_owned()));

        // The route changes while it is held: to another name for the
        // same install, or to another install.
        let to = if retarget { "m1" } else { "m0b" };
        fleet.write(&[here(), ssh("a", "Mini", to)]);
        match retarget {
            false => until("the route changed", || {
                fleet.book().far() == [(key.clone(), Some("a".into()), 2)]
            }),
            true => until("the old machine is retired", || {
                !fleet.book().far().iter().any(|far| far.0 == key)
            }),
        }
        hold.released.store(true, Ordering::Release);
        // And a write read under the old number arrives by hand.
        writer.listed(listing("s-by-hand", i64::MAX - 1), 1);
        writer.settle();

        // The machine as it is now gets its rows.
        match retarget {
            false => {
                works(&gateway, "s-new", "Read under the new route");
                fleet.listed(&far(&key, "s-new"));
                // A write under the number that is current is taken.
                writer.listed(listing("s-current", i64::MAX), 2);
                writer.settle();
                fleet.listed(&far(&key, "s-current"));
            }
            true => {
                let second = fleet.connected("Mini");
                assert_ne!(second, key);
                fleet.listed(&far(&second, "o-one"));
                assert!(!dump(&fleet.mirror(&second, "default")).contains("s-"));
            }
        }
        fleet.looked(5);
        fleet.turn();
        let rows = dump(&path);
        assert!(
            !rows.contains("s-late") && !rows.contains("s-by-hand"),
            "{rows}"
        );
        let listed = fleet.far_rows();
        let late = |row: &&Row| row.id.contains("s-late") || row.id.contains("s-by-hand");
        assert_eq!(listed.iter().filter(late).count(), 0, "{listed:?}");
        let log = fleet.log();
        match retarget {
            false => assert!(
                log.contains("mirror dropped profile=default why=route"),
                "{log}"
            ),
            true => assert!(log.contains(&format!("retired machine={key}")), "{log}"),
        }
    }
}

#[test]
#[ignore = "starts processes: run by the_lab_tests_pass"]
fn stored_rows_show_at_launch_with_no_list() {
    let Some((mut fleet, _gateway, key)) = one_machine() else {
        return;
    };
    let path = fleet.mirror(&key, "default");
    // The earlier run ends, as quitting the app ends it: the writer
    // closes the file and says so, which is that run's last word.
    fleet.stop();
    fleet.told.lock().unwrap().clear();
    let (rows, stamped, knocks) = (dump(&path), stamp(&path), fleet.knocks(0));
    assert!(rows.contains("s-one"));

    // No list at all, then one that does not parse.
    for broken in [false, true] {
        match broken {
            false => std::fs::remove_file(&fleet.list).unwrap(),
            true => {
                fleet.write_text("{\"version\": 2, \"connections\": [{\"id\": \"a\", \"kind\":")
            }
        }
        // A launch: core starts from what it saved, and walks its roots.
        fleet.reads_mirrors();
        fleet.start();
        fleet.looked(6);
        let listed = fleet.far_rows();
        assert_eq!(listed.len(), 1, "{listed:?}");
        assert_eq!(listed[0].id, far(&key, "s-one"));
        assert_eq!(listed[0].machine, "Mini");
        // The machine shows from its binding, and nothing is dialled.
        let machines = fleet.rows();
        let bound = machines.iter().find(|row| row["id"] == key.as_str());
        assert_eq!(bound.expect("its status")["label"], "Mini");
        assert!(fleet.note().is_some(), "the list could not be read");
        assert!(fleet.masters().is_empty(), "{:?}", fleet.masters());
        assert_eq!(fleet.knocks(0), knocks, "nothing reached the sshd");
        assert!(fleet.book().far().is_empty());
        // Nothing wrote the mirror.
        assert_eq!(dump(&path), rows);
        assert_eq!(stamp(&path), stamped);
        assert!(fleet.told.lock().unwrap().is_empty());
        println!("stored rows at launch, broken list={broken}: {listed:?}");
        fleet.stop();
    }
}
