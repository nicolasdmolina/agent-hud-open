//! Finding a gateway and deciding whether it is one to connect to: an old
//! build is left to the hooks, one that starts is waited for, the control
//! socket is asked only where it is the user's own, and no ticket is ever
//! written down.
use super::super::control::{self, Silent};
use super::{fake::fixture, fake::Fake, lab, until};
use serde_json::{json, Value};
use std::{
    io::{BufRead, BufReader, Write},
    os::unix::{fs::PermissionsExt, net::UnixListener},
    path::Path,
    time::Duration,
};

fn mode(path: &Path, bits: u32) {
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(bits)).unwrap();
}

#[test]
fn an_old_gateway_is_left_to_the_hooks_and_asked_again_now_and_then() {
    let old = fixture("gateway-self-description-old-build.SYNTHETIC");
    let cases = old["cases"].as_array().unwrap();
    let unsupported = |case: &&Value| {
        let reason = case["verdict"]["reason_code"].as_str().unwrap_or("");
        matches!(reason, "runtime_protocol" | "session_authority_unavailable")
    };
    let cases: Vec<&Value> = cases.iter().filter(unsupported).collect();
    assert!(cases.len() >= 2);
    for case in cases {
        let mut lab = lab();
        let gateway = Fake::start(&lab.home, "instance-1");
        let ready = gateway.world().identify.clone();
        // What a build without the live connection says of itself, in the
        // home this one runs in.
        let mut told = case["identify_result"].clone();
        let home = lab.home.to_string_lossy().into_owned();
        if told.get("served_profiles").is_some() {
            told["served_profiles"] = json!([{"profile_id": home, "home": home}]);
        }
        gateway.world().identify =
            Some(json!({"ok": true, "protocol": 1, "id": 1, "result": told}));
        lab.follow();
        let reason = case["verdict"]["reason_code"].as_str().unwrap();
        let said = format!("connection unsupported reason={reason}");
        until("it is found unsupported", || lab.log().contains(&said));
        std::thread::sleep(Duration::from_millis(300));
        // Nothing was asked of it beyond who it is: no ticket, no socket.
        assert!(gateway.world().granted.is_empty(), "{reason}");
        let dialled = |gateway: &Fake| {
            let world = gateway.world();
            world.sockets + world.turned_away
        };
        assert_eq!(dialled(&gateway), 0, "{reason}");
        assert!(lab.cards().is_empty());
        assert_eq!(
            lab.log().matches("connection unsupported").count(),
            1,
            "{}",
            lab.log()
        );

        // An update adds the live connection: it is found without a relaunch.
        gateway.world().identify = ready;
        until("the updated gateway is connected to", || {
            gateway.world().sockets == 1
        });
        lab.hears(&gateway, "question-answered-elsewhere", "clarify.request");
        lab.card();
    }
}

#[test]
fn a_control_socket_that_grants_no_ticket_is_an_old_gateway() {
    let mut lab = lab();
    let gateway = Fake::start(&lab.home, "instance-1");
    gateway.world().grants = false;
    lab.follow();
    until("it is found unsupported", || {
        lab.log()
            .contains("connection unsupported reason=unknown_verb")
    });
    let world = gateway.world();
    assert_eq!(world.sockets + world.turned_away, 0);
}

#[test]
fn a_gateway_that_is_starting_is_waited_for() {
    let mut lab = lab();
    let gateway = Fake::start(&lab.home, "instance-1");
    let ready = gateway.world().identify.clone();
    let starting = fixture("gateway-self-description")["control_socket"]
        ["identify_throwaway_starting"]
        .clone();
    gateway.world().identify =
        Some(json!({"ok": true, "protocol": 1, "id": 1, "result": starting}));
    lab.follow();
    until("it is being waited for", || {
        lab.log().contains("connection connecting")
    });
    std::thread::sleep(Duration::from_millis(200));
    // Asked again and again, never taken for a build without the live
    // connection, and never asked for a ticket before it is ready.
    assert!(!lab.log().contains("unsupported"), "{}", lab.log());
    assert!(gateway.world().granted.is_empty());
    gateway.world().identify = ready;
    until("it is connected to once ready", || {
        gateway.world().sockets == 1
    });
    assert!(
        lab.log().contains("connection ready instance=instance"),
        "{}",
        lab.log()
    );
}

#[test]
fn no_gateway_at_all_is_quiet_until_one_starts() {
    let mut lab = lab();
    lab.follow();
    let said = "connection failed reason=Unreachable";
    until("it is found unreachable", || lab.log().contains(said));
    std::thread::sleep(Duration::from_millis(400));
    // Said once, however often it is tried.
    assert_eq!(lab.log().matches(said).count(), 1);
    assert_eq!(lab.log().lines().count(), 3, "{}", lab.log());
    let gateway = Fake::start(&lab.home, "instance-1");
    until("the gateway that started is connected to", || {
        gateway.world().sockets == 1
    });
}

#[test]
fn no_ticket_is_written_to_disk_or_to_the_log() {
    let mut lab = lab();
    let gateway = Fake::start(&lab.home, "instance-1");
    lab.follow();
    lab.hears(&gateway, "question-answered-here", "clarify.request");
    let id = lab.card()["id"].as_str().unwrap().to_owned();
    let decision = json!({"kind": "answer", "answers": {"715a2db63d": {"selected": ["green"]}}});
    lab.requests
        .answer(&id, &decision, crate::hooks::now_ms())
        .unwrap();
    until("the card goes", || lab.cards().is_empty());
    // A socket whose ticket is refused, too: a second gateway instance
    // takes over, and the old instance's tickets are worth nothing.
    gateway.kill();
    let gateway = Fake::start(&lab.home, "instance-2");
    until("the new instance is polled", || {
        !gateway.world().calls("session.active_list").is_empty()
    });
    lab.observer.take();

    let tickets = gateway.world().granted.clone();
    assert!(!tickets.is_empty());
    let mut files = vec![];
    let mut folders = vec![lab.data.clone(), lab.home.clone()];
    while let Some(folder) = folders.pop() {
        for entry in std::fs::read_dir(folder).unwrap().flatten() {
            match entry.file_type().unwrap() {
                kind if kind.is_dir() => folders.push(entry.path()),
                kind if kind.is_file() => files.push(entry.path()),
                _ => {}
            }
        }
    }
    assert!(files
        .iter()
        .any(|file| file.ends_with("hermes-gateway.log")));
    for file in files {
        let text = String::from_utf8_lossy(&std::fs::read(&file).unwrap()).into_owned();
        assert!(!text.contains("ticket-instance"), "{file:?}");
        assert!(!text.contains("hermes-gateway-ticket"), "{file:?}");
    }
    // Nothing was written under the Hermes home but the gateway's own socket.
    let left: Vec<_> = std::fs::read_dir(&lab.home).unwrap().flatten().collect();
    assert!(
        left.iter().all(|entry| entry.file_name() == "gateway.sock"),
        "{left:?}"
    );
}

/// A listener that answers one line to each question, as the control
/// socket does.
fn answering(path: &Path) {
    let listener = UnixListener::bind(path).unwrap();
    mode(path, 0o600);
    std::thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            let mut line = String::new();
            let _ = BufReader::new(&stream).read_line(&mut line);
            let _ = (&stream).write_all(b"{\"ok\":true}\n");
        }
    });
}

#[test]
fn the_control_socket_is_asked_only_where_it_is_the_users_own() {
    let lab = lab();
    let wait = Duration::from_secs(10);
    let ask = |home: &Path| control::ask(home, &json!({"verb": "identify"}), wait);
    // No socket and no pointer: no gateway.
    assert_eq!(ask(&lab.home), Err(Silent::Absent));
    let socket = lab.home.join("gateway.sock");
    answering(&socket);
    assert_eq!(ask(&lab.home), Ok(json!({"ok": true})));
    // A socket others may connect to is not Hermes's.
    mode(&socket, 0o666);
    assert_eq!(ask(&lab.home), Err(Silent::Unsafe("socket")));
    mode(&socket, 0o600);
    // A home others may write could hold anyone's socket.
    for open in [0o770, 0o707] {
        mode(&lab.home, open);
        assert_eq!(ask(&lab.home), Err(Silent::Unsafe("home")));
    }
    mode(&lab.home, 0o700);
    assert_eq!(ask(&lab.home), Ok(json!({"ok": true})));
    // A socket nothing listens on, as a killed gateway leaves it.
    let stale = lab.data.join("stale");
    crate::os::private_directory(&stale).unwrap();
    drop(UnixListener::bind(stale.join("gateway.sock")).unwrap());
    mode(&stale.join("gateway.sock"), 0o600);
    assert_eq!(ask(&stale), Err(Silent::Absent));
}

#[test]
fn a_pointer_file_is_followed_only_to_the_folder_hermes_makes_for_that_home() {
    // A socket's path has to stay short (104 bytes on macOS), which the
    // usual temporary folder is not: Hermes uses a short one for this too.
    let short = tempfile::Builder::new()
        .prefix("ahud")
        .tempdir_in("/tmp")
        .unwrap();
    let beside = short.path().canonicalize().unwrap();
    let home = beside.join("home");
    crate::os::private_directory(&home).unwrap();
    let wait = Duration::from_secs(10);
    let ask = |home: &Path| control::ask(home, &json!({"verb": "identify"}), wait);
    // Where a home cannot hold a socket, Hermes listens in a private
    // folder named after the home and leaves a pointer to it.
    let folder = beside.join(control::folder_of(&home));
    crate::os::private_directory(&folder).unwrap();
    let socket = folder.join("control.sock");
    answering(&socket);
    let pointer = home.join("gateway.sock.path");
    std::fs::write(&pointer, socket.to_string_lossy().as_bytes()).unwrap();
    mode(&pointer, 0o600);
    assert_eq!(ask(&home), Ok(json!({"ok": true})));
    // A pointer others may write names nothing.
    mode(&pointer, 0o644);
    assert_eq!(ask(&home), Err(Silent::Unsafe("pointer")));
    mode(&pointer, 0o600);
    // The folder of another home, or a folder others may enter.
    let other = beside.join("hermes-gw-0123456789abcdef");
    crate::os::private_directory(&other).unwrap();
    answering(&other.join("control.sock"));
    let elsewhere = other.join("control.sock");
    std::fs::write(&pointer, elsewhere.to_string_lossy().as_bytes()).unwrap();
    assert_eq!(ask(&home), Err(Silent::Unsafe("pointer")));
    std::fs::write(&pointer, socket.to_string_lossy().as_bytes()).unwrap();
    mode(&folder, 0o755);
    assert_eq!(ask(&home), Err(Silent::Unsafe("socket")));
    // A link in place of the pointer is not read.
    mode(&folder, 0o700);
    std::fs::remove_file(&pointer).unwrap();
    std::os::unix::fs::symlink(&socket, &pointer).unwrap();
    assert_eq!(ask(&home), Err(Silent::Absent));
}
