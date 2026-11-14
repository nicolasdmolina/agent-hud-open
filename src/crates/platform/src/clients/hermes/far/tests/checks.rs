//! What the relay checks before it sends anything: each refusal by its
//! name, and what the check on the listener does and does not prove.
use super::{
    super::{Far, Silent},
    fixture::{chmod, far_with, private_folder, text, write_pointer, FarLab, Manner, SHORT},
};
use agent_hud_agents::hermes::gateway::{
    far::{Quiet, Request, Unchecked},
    granted, identify, Identified, Scope,
};
use serde_json::json;
use std::{fs, os::unix::fs::symlink, path::Path};

fn refused(far: &Far, lab: &FarLab, home: &Path) -> Option<Unchecked> {
    match far.exchange(&lab.link, text(home), Request::Identify).said {
        Err(Silent::Relay(Quiet::Unchecked(why))) => Some(why),
        other => panic!("not refused by a check: {other:?}"),
    }
}

/// `identify`, then a ticket, through the relay, for the gateway of `home`.
fn identify_then_a_ticket(lab: &FarLab, home: &Path) {
    let far = Far::default();
    let told = far.exchange(&lab.link, text(home), Request::Identify);
    println!("identify took {:?}", told.took);
    let told = told.said.expect("an answer to identify");
    let Identified::Ready(instance) = identify(told.value(), text(home)) else {
        panic!("identify did not make an instance");
    };
    assert_eq!(instance.id, "one");
    let asked = Request::Ticket {
        profile_id: &instance.profile_id,
        instance_id: &instance.id,
        scope: Scope::Host,
    };
    let grant = far.exchange(&lab.link, text(home), asked);
    println!("session-ticket took {:?}", grant.took);
    let grant = grant.said.expect("an answer to session-ticket");
    let ticket = granted(grant.value(), &instance).expect("a ticket");
    assert!(ticket.subprotocols()[1].ends_with("a-ticket-of-the-lab"));
}

#[test]
#[ignore = "in the lab: tests::the_lab_tests_pass runs it"]
fn identify_and_a_ticket_come_back_for_a_home_with_a_socket() {
    let Some(lab) = FarLab::start() else { return };
    let home = lab.root();
    let gate = lab.gate(&home, Manner::gateway("one", &[&home]));
    identify_then_a_ticket(&lab, &home);
    let seen = gate.seen();
    assert_eq!(seen.len(), 2, "{seen:?}");
    assert_eq!(seen[0], Request::Identify.line());
    assert!(seen[1].contains("\"verb\":\"session-ticket\""), "{seen:?}");
}

#[test]
#[ignore = "in the lab: tests::the_lab_tests_pass runs it"]
fn identify_and_a_ticket_come_back_for_a_home_with_a_pointer_file() {
    let Some(lab) = FarLab::start() else { return };
    let home = lab.profile("ops");
    let gate = lab.gate_by_pointer(&home, Manner::gateway("one", &[&home]));
    assert!(!home.join("gateway.sock").exists());
    identify_then_a_ticket(&lab, &home);
    assert_eq!(gate.seen().len(), 2);
}

#[test]
#[ignore = "in the lab: tests::the_lab_tests_pass runs it"]
fn a_home_others_may_write_is_refused() {
    let Some(lab) = FarLab::start() else { return };
    let home = lab.root();
    let gate = lab.gate(&home, Manner::gateway("one", &[&home]));
    let far = Far::default();
    for mode in [0o720, 0o702, 0o777] {
        chmod(&home, mode);
        let why = refused(&far, &lab, &home);
        assert_eq!(why, Some(Unchecked::HomeOpenToOthers), "{mode:o}");
    }
    // Read by others is the user's own business, as it is for Hermes.
    chmod(&home, 0o755);
    assert!(far
        .exchange(&lab.link, text(&home), Request::Identify)
        .said
        .is_ok());
    // A home reached through a link is not the canonical one.
    let link = lab.dir.join("far").join("linked");
    symlink(&home, &link).unwrap();
    assert_eq!(
        refused(&far, &lab, &link),
        Some(Unchecked::HomeNotCanonical)
    );
    let dotted = home.join("..").join(".hermes");
    assert_eq!(
        refused(&far, &lab, &dotted),
        Some(Unchecked::HomeNotCanonical)
    );
    // Only the one exchange that was let through reached the gateway.
    assert_eq!(gate.seen().len(), 1);
}

#[test]
#[ignore = "in the lab: tests::the_lab_tests_pass runs it"]
fn a_socket_open_to_others_is_refused() {
    let Some(lab) = FarLab::start() else { return };
    let home = lab.root();
    let gate = lab.gate(&home, Manner::gateway("one", &[&home]));
    let far = Far::default();
    for mode in [0o660, 0o606, 0o640, 0o604] {
        chmod(&home.join("gateway.sock"), mode);
        let why = refused(&far, &lab, &home);
        assert_eq!(why, Some(Unchecked::SocketOpenToOthers), "{mode:o}");
    }
    assert_eq!(gate.connections(), 0);
    // A file where the socket should be is no socket of the user's.
    let other = lab.profile("plain");
    fs::write(other.join("gateway.sock"), b"").unwrap();
    chmod(&other.join("gateway.sock"), 0o600);
    assert_eq!(refused(&far, &lab, &other), Some(Unchecked::SocketNotYours));
}

#[test]
#[ignore = "in the lab: tests::the_lab_tests_pass runs it"]
fn a_pointer_that_is_a_link_is_refused() {
    let Some(lab) = FarLab::start() else { return };
    let home = lab.root();
    let gate = lab.gate_by_pointer(&home, Manner::gateway("one", &[&home]));
    let far = Far::default();
    assert!(far
        .exchange(&lab.link, text(&home), Request::Identify)
        .said
        .is_ok());
    // The same words, reached through a link.
    let pointer = home.join("gateway.sock.path");
    let real = lab.dir.join("real-pointer");
    fs::rename(&pointer, &real).unwrap();
    symlink(&real, &pointer).unwrap();
    assert_eq!(refused(&far, &lab, &home), Some(Unchecked::PointerIsALink));
    // And a pointer others may read or write.
    fs::remove_file(&pointer).unwrap();
    fs::rename(&real, &pointer).unwrap();
    chmod(&pointer, 0o644);
    assert_eq!(
        refused(&far, &lab, &home),
        Some(Unchecked::PointerOpenToOthers)
    );
    assert_eq!(gate.connections(), 1);
}

#[test]
#[ignore = "in the lab: tests::the_lab_tests_pass runs it"]
fn a_pointer_that_is_too_long_is_refused() {
    let Some(lab) = FarLab::start() else { return };
    let home = lab.root();
    let gate = lab.gate_by_pointer(&home, Manner::gateway("one", &[&home]));
    let far = Far::default();
    let named = lab.pointed(&home);
    // The right socket, written with more slashes in front. Any number of
    // them names the same place.
    let padded = |length: usize| {
        let mut long = "/".repeat(length + 1 - text(&named).len());
        long.push_str(&text(&named)[1..]);
        assert_eq!(long.len(), length);
        long
    };
    write_pointer(&home, padded(900).as_bytes());
    assert!(far
        .exchange(&lab.link, text(&home), Request::Identify)
        .said
        .is_ok());
    // At the bound it is read whole and is not too long. No system opens
    // a path of that length, so it is refused for what it names.
    write_pointer(&home, padded(4096).as_bytes());
    assert_eq!(refused(&far, &lab, &home), Some(Unchecked::FolderNotYours));
    // One byte more is too long, whatever it names.
    write_pointer(&home, padded(4097).as_bytes());
    assert_eq!(refused(&far, &lab, &home), Some(Unchecked::PointerTooLong));
    write_pointer(&home, &vec![b'x'; 64 * 1024]);
    assert_eq!(refused(&far, &lab, &home), Some(Unchecked::PointerTooLong));
    assert_eq!(gate.connections(), 1);
}

#[test]
#[ignore = "in the lab: tests::the_lab_tests_pass runs it"]
fn a_pointer_naming_a_path_outside_hermes_folder_is_refused() {
    let Some(lab) = FarLab::start() else { return };
    let home = lab.root();
    let other = lab.profile("ops");
    // A listener of the user's own, in a private folder, that is not the
    // one Hermes makes for this home.
    let elsewhere = lab.dir.join("t").join("hermes-gw-0000000000000000");
    private_folder(&elsewhere);
    let gate = super::fixture::Gate::serve(
        lab.listen(&elsewhere.join("control.sock")),
        Manner::Says(json!({"who": "elsewhere"})),
    );
    let mine = lab.gate_by_pointer(&other, Manner::gateway("one", &[&other]));
    let far = Far::default();
    let named = lab.pointed(&home);
    let folder = named.parent().unwrap();
    for names in [
        text(&elsewhere.join("control.sock")).to_owned(),
        // The folder of another home's socket.
        text(&lab.pointed(&other)).to_owned(),
        // The right folder, another file in it.
        text(&folder.join("other.sock")).to_owned(),
        // Not from the root.
        format!(
            "{}/control.sock",
            folder.file_name().unwrap().to_string_lossy()
        ),
        text(&home.join("gateway.sock")).to_owned(),
        String::new(),
    ] {
        write_pointer(&home, names.as_bytes());
        let why = refused(&far, &lab, &home);
        assert_eq!(why, Some(Unchecked::PointerOutside), "{names}");
    }
    write_pointer(&home, &[b'/', 0xff, 0xfe]);
    assert_eq!(refused(&far, &lab, &home), Some(Unchecked::PointerOutside));
    // The folder Hermes would make, open to others.
    private_folder(folder);
    let _own = super::fixture::Gate::serve(lab.listen(&named), Manner::Silent);
    write_pointer(&home, text(&named).as_bytes());
    chmod(folder, 0o750);
    assert_eq!(
        refused(&far, &lab, &home),
        Some(Unchecked::FolderOpenToOthers)
    );
    assert_eq!((gate.connections(), mine.connections()), (0, 0));
}

#[test]
#[ignore = "in the lab: tests::the_lab_tests_pass runs it"]
fn no_python3_is_refused() {
    // A far login that finds `sh` and no `python3`.
    let Some(lab) = FarLab::start_with(&[("PATH", "/bin")]) else {
        return;
    };
    let home = lab.root();
    let gate = lab.gate(&home, Manner::gateway("one", &[&home]));
    let far = Far::default();
    assert_eq!(refused(&far, &lab, &home), Some(Unchecked::NoPython));
    assert_eq!(gate.connections(), 0);
}

#[test]
#[ignore = "in the lab: tests::the_lab_tests_pass runs it"]
fn where_peer_credentials_cannot_be_read_the_relay_refuses() {
    let Some(lab) = FarLab::start() else { return };
    let home = lab.root();
    let gate = lab.gate(&home, Manner::gateway("one", &[&home]));
    // Stubbed: this machine can say who listens, so the relay is told it
    // cannot.
    let far = far_with(SHORT, "def peer_uid(listener):\n    return None\n");
    assert_eq!(
        refused(&far, &lab, &home),
        Some(Unchecked::NoPeerCredentials)
    );
    assert!(gate.connected(1));
    assert_eq!(gate.seen(), Vec::<String>::new(), "nothing was sent");
}

#[test]
#[ignore = "in the lab: tests::the_lab_tests_pass runs it"]
fn a_listener_of_another_user_is_refused_and_no_line_reaches_it() {
    let Some(lab) = FarLab::start() else { return };
    let home = lab.root();
    let gate = lab.gate(&home, Manner::gateway("one", &[&home]));
    // Stubbed: the test machine has no second user to listen, so the user
    // the relay reads for the listener is made to differ from its own.
    let far = far_with(
        SHORT,
        "_real_peer_uid = peer_uid\ndef peer_uid(listener):\n    return _real_peer_uid(listener) + 1\n",
    );
    assert_eq!(
        refused(&far, &lab, &home),
        Some(Unchecked::ListenerNotYours)
    );
    // It was connected to, which is how its user is read, and told nothing.
    assert!(gate.connected(1));
    std::thread::sleep(std::time::Duration::from_millis(300));
    assert_eq!(gate.seen(), Vec::<String>::new(), "nothing was sent");
    // The relay as it is reads the real user, and is let through.
    let far = Far::default();
    assert!(far
        .exchange(&lab.link, text(&home), Request::Identify)
        .said
        .is_ok());
    assert_eq!(gate.seen().len(), 1);
}

/// The boundary: the check on the listener proves it runs as the login
/// user, and cannot tell two processes of that user apart.
#[test]
#[ignore = "in the lab: tests::the_lab_tests_pass runs it"]
fn a_second_listener_of_the_same_user_is_accepted_which_is_the_boundary() {
    let Some(lab) = FarLab::start() else { return };
    let home = lab.root();
    let gate = lab.gate(&home, Manner::gateway("one", &[&home]));
    let far = Far::default();
    let first = far.exchange(&lab.link, text(&home), Request::Identify);
    assert_eq!(first.said.unwrap().value()["result"]["instance_id"], "one");
    // Another process of the same user takes the gateway's place.
    let script = "import os, socket, sys\n\
        os.chdir(sys.argv[1])\n\
        s = socket.socket(socket.AF_UNIX)\n\
        s.bind('second.sock'); os.chmod('second.sock', 0o600); s.listen(4)\n\
        os.rename('second.sock', 'gateway.sock')\n\
        print('up', flush=True)\n\
        c, _ = s.accept(); c.recv(65536)\n\
        c.sendall(b'{\"who\": \"a second process of the same user\"}\\n')\n";
    let mut second = std::process::Command::new("/usr/bin/python3")
        .args(["-c", script, text(&home)])
        .stdout(std::process::Stdio::piped())
        .spawn()
        .expect("python3");
    let mut up = String::new();
    std::io::BufRead::read_line(
        &mut std::io::BufReader::new(second.stdout.take().unwrap()),
        &mut up,
    )
    .unwrap();
    assert_eq!(up.trim(), "up");
    let taken = far.exchange(&lab.link, text(&home), Request::Identify);
    let _ = second.wait();
    assert_eq!(
        taken.said.expect("accepted").value()["who"],
        "a second process of the same user"
    );
    // The gateway itself heard only the first exchange.
    assert_eq!(gate.seen().len(), 1);
}

#[test]
#[ignore = "in the lab: tests::the_lab_tests_pass runs it"]
fn a_path_swapped_after_it_was_opened_does_not_change_what_was_checked() {
    let Some(lab) = FarLab::start() else { return };
    let home = lab.root();
    let gate = lab.gate(
        &home,
        Manner::Says(json!({"who": "the home that was checked"})),
    );
    // Another folder, with a listener of its own, ready to take the path.
    let other = lab.dir.join("far").join("swap");
    private_folder(&other);
    let impostor = lab.gate(
        &other,
        Manner::Says(json!({"who": "the folder swapped in"})),
    );
    let moved = lab.dir.join("far").join("moved");
    // Right after the relay has opened and checked the home, and before it
    // uses it, the path is given to the other folder.
    let swap = format!(
        "_real_open_home = open_home\n\
         def open_home(path, uid):\n\
         \x20   home = _real_open_home(path, uid)\n\
         \x20   os.rename(path, {moved:?})\n\
         \x20   os.rename({other:?}, path)\n\
         \x20   return home\n",
        moved = text(&moved),
        other = text(&other),
    );
    let far = far_with(SHORT, &swap);
    let told = far.exchange(&lab.link, text(&home), Request::Identify);
    assert_eq!(
        told.said.expect("an answer").value()["who"],
        "the home that was checked"
    );
    assert!(moved.join("gateway.sock").exists(), "the swap happened");
    assert_eq!((gate.seen().len(), impostor.connections()), (1, 0));
    // The path now leads to the other folder, and a new exchange says so.
    let told = Far::default().exchange(&lab.link, text(&home), Request::Identify);
    assert_eq!(told.said.unwrap().value()["who"], "the folder swapped in");
}

#[test]
#[ignore = "in the lab: tests::the_lab_tests_pass runs it"]
fn a_verb_that_is_never_sent_is_refused_by_the_relay_too() {
    let Some(lab) = FarLab::start() else { return };
    let home = lab.root();
    let gate = lab.gate(&home, Manner::gateway("one", &[&home]));
    let far = Far::default();
    for line in [
        json!({"protocol": 1, "id": 1, "verb": "pause-for-update"}).to_string(),
        json!({"protocol": 1, "id": 1, "verb": "unserve-profile", "params": {"name": "ops"}})
            .to_string(),
        json!({"protocol": 1, "id": 1}).to_string(),
        json!(["identify"]).to_string(),
        "identify".to_owned(),
        format!("{}\n{}", Request::Identify.line(), Request::Identify.line()),
    ] {
        let said = far.relay(&lab.link, text(&home), &line);
        let why = Silent::Relay(Quiet::Unchecked(Unchecked::VerbNotSent));
        assert_eq!(said.unwrap_err(), why, "{line}");
    }
    assert_eq!(gate.connections(), 0);
}
