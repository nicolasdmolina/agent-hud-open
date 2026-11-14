//! Access control lists: on macOS one can open a node to other users while
//! its mode bits say it is private. The relay reads the list of the home,
//! the socket, the pointer file and the folder it names, and refuses what
//! a list opens, or a list it cannot read.
use super::{
    super::{Far, Silent},
    fixture::{far_with, text, FarLab, Gate, Manner, SHORT},
};
use agent_hud_agents::hermes::gateway::far::{Quiet, Request, Unchecked};
use serde_json::json;
use std::{fs, os::unix::fs::PermissionsExt, path::Path, process::Command};

/// Adds `entry` to the access control list of `path`, as `chmod +a` does.
fn allow(path: &Path, entry: &str) {
    let changed = Command::new("/bin/chmod")
        .args(["+a", entry])
        .arg(path)
        .output()
        .expect("chmod");
    let said = String::from_utf8_lossy(&changed.stderr);
    assert!(changed.status.success(), "{entry}: {said}");
}

/// Takes the whole list off `path`.
fn clear(path: &Path) {
    let cleared = Command::new("/bin/chmod").arg("-N").arg(path).status();
    assert!(cleared.expect("chmod").success());
}

fn asked(far: &Far, lab: &FarLab, home: &Path) -> Result<(), Silent> {
    far.exchange(&lab.link, text(home), Request::Identify)
        .said
        .map(|_| ())
}

fn unchecked(why: Unchecked) -> Result<(), Silent> {
    Err(Silent::Relay(Quiet::Unchecked(why)))
}

/// The verifier's probe: mode 0700, and a list that lets everyone write.
#[test]
#[ignore = "in the lab: tests::the_lab_tests_pass runs it"]
fn v_a_home_with_an_everyone_write_acl_is_refused() {
    let Some(lab) = FarLab::start() else { return };
    let home = lab.root();
    let gate = lab.gate(&home, Manner::Says(json!({"ok": true})));
    allow(&home, "everyone allow read,write,execute");
    let listed = Command::new("/bin/ls")
        .args(["-lde"])
        .arg(&home)
        .output()
        .unwrap();
    println!("{}", String::from_utf8_lossy(&listed.stdout));
    assert_eq!(
        fs::metadata(&home).unwrap().permissions().mode() & 0o777,
        0o700,
        "ACL is the only change"
    );
    let told = Far::default().exchange(&lab.link, text(&home), Request::Identify);
    clear(&home);
    println!(
        "With mode 0700 and an explicit everyone-write ACL: {:?}; gateway received {} request(s).",
        told.said,
        gate.seen().len()
    );
    assert_eq!(
        told.said,
        Err(Silent::Relay(Quiet::Unchecked(Unchecked::HomeOpenToOthers))),
        "another user may write this home despite its mode bits"
    );
    assert_eq!(gate.connections(), 0);
    // With the list gone the same home is asked.
    assert_eq!(asked(&Far::default(), &lab, &home), Ok(()));
}

#[test]
#[ignore = "in the lab: tests::the_lab_tests_pass runs it"]
fn a_list_on_a_home_may_let_others_read_it_and_nothing_more() {
    let Some(lab) = FarLab::start() else { return };
    let home = lab.root();
    let gate = lab.gate(&home, Manner::Says(json!({"ok": true})));
    let far = Far::default();
    let user = std::env::var("USER").expect("the user's name");
    // Each of these lets someone else change what is in the home.
    for entry in [
        "everyone allow add_file",
        "everyone allow add_subdirectory",
        "everyone allow delete_child",
        "everyone allow delete",
        "everyone allow writeattr",
        "everyone allow writeextattr",
        "everyone allow writesecurity",
        "everyone allow chown",
        "staff allow list,search,add_file",
        "everyone allow add_file,file_inherit,directory_inherit",
        "daemon allow add_file",
    ] {
        allow(&home, entry);
        let said = asked(&far, &lab, &home);
        clear(&home);
        assert_eq!(said, unchecked(Unchecked::HomeOpenToOthers), "{entry}");
    }
    assert_eq!(gate.connections(), 0);
    // Reading is the user's own business, as with mode 0755; an entry
    // that denies opens nothing; and the user may be allowed anything.
    for entry in [
        "everyone allow list,search,readattr,readextattr,readsecurity",
        "everyone deny add_file,delete_child,delete",
        "daemon deny list,search",
        &format!("{user} allow add_file,delete_child,writesecurity,chown"),
    ] {
        allow(&home, entry);
        let said = asked(&far, &lab, &home);
        clear(&home);
        assert_eq!(said, Ok(()), "{entry}");
    }
    // A write entry is found wherever it is in the list.
    allow(&home, "everyone allow add_file");
    allow(&home, "everyone deny delete");
    allow(&home, &format!("{user} allow add_file"));
    assert_eq!(
        asked(&far, &lab, &home),
        unchecked(Unchecked::HomeOpenToOthers)
    );
    clear(&home);
    assert_eq!(gate.connections(), 4);
}

#[test]
#[ignore = "in the lab: tests::the_lab_tests_pass runs it"]
fn a_list_that_opens_the_socket_the_pointer_or_its_folder_is_refused() {
    let Some(lab) = FarLab::start() else { return };
    let far = Far::default();
    // The socket in the home: closed to everyone else, reading included.
    let home = lab.root();
    let gate = lab.gate(&home, Manner::Says(json!({"ok": true})));
    let socket = home.join("gateway.sock");
    for entry in [
        "everyone allow read",
        "everyone allow write",
        "staff allow read,write",
    ] {
        allow(&socket, entry);
        assert_eq!(
            socket.metadata().unwrap().permissions().mode() & 0o777,
            0o600
        );
        let said = asked(&far, &lab, &home);
        clear(&socket);
        assert_eq!(said, unchecked(Unchecked::SocketOpenToOthers), "{entry}");
    }
    // An entry that denies opens nothing. (One for everyone would deny
    // the user too, and then nobody could connect.)
    allow(&socket, "daemon deny write");
    assert_eq!(asked(&far, &lab, &home), Ok(()));
    assert_eq!(gate.connections(), 1);
    // A home with a pointer file: the pointer, its folder, and the socket
    // in that folder, each in turn.
    let other = lab.profile("ops");
    let pointed = lab.gate_by_pointer(&other, Manner::Says(json!({"ok": true})));
    let named = lab.pointed(&other);
    let folder = named.parent().unwrap();
    let pointer = other.join("gateway.sock.path");
    for (node, entry, why) in [
        (
            pointer.as_path(),
            "everyone allow read",
            Unchecked::PointerOpenToOthers,
        ),
        (
            pointer.as_path(),
            "everyone allow write",
            Unchecked::PointerOpenToOthers,
        ),
        (
            folder,
            "everyone allow list,search",
            Unchecked::FolderOpenToOthers,
        ),
        (
            folder,
            "everyone allow add_file",
            Unchecked::FolderOpenToOthers,
        ),
        (
            named.as_path(),
            "everyone allow read,write",
            Unchecked::SocketOpenToOthers,
        ),
    ] {
        allow(node, entry);
        let said = asked(&far, &lab, &other);
        clear(node);
        assert_eq!(said, unchecked(why), "{entry} on {}", node.display());
    }
    assert_eq!(pointed.connections(), 0);
    assert_eq!(asked(&far, &lab, &other), Ok(()));
}

/// Stubbed: this Mac can read a list, so the relay is given a system
/// library without the calls that read one, and then a list in words it
/// does not know. Either way it cannot say the home is closed, and refuses.
#[test]
#[ignore = "in the lab: tests::the_lab_tests_pass runs it"]
fn where_a_list_cannot_be_read_the_relay_refuses() {
    let Some(lab) = FarLab::start() else { return };
    let home = lab.root();
    let gate: Gate = lab.gate(&home, Manner::Says(json!({"ok": true})));
    let refused = |far: &Far| match asked(far, &lab, &home) {
        Err(Silent::Relay(Quiet::Unchecked(why))) => why.reason_code(),
        other => panic!("not refused by a check: {other:?}"),
    };
    let no_calls = "import ctypes\n\
        _real_library = ctypes.CDLL\n\
        class _WithoutLists:\n\
        \x20   def __init__(self, *given, **named):\n\
        \x20       self._library = _real_library(*given, **named)\n\
        \x20   def __getattr__(self, name):\n\
        \x20       if name.startswith('acl_'):\n\
        \x20           raise AttributeError(name)\n\
        \x20       return getattr(self._library, name)\n\
        ctypes.CDLL = _WithoutLists\n";
    assert_eq!(refused(&far_with(SHORT, no_calls)), "permissions-unread");
    // A list the relay reads and cannot make out.
    allow(&home, "everyone deny delete");
    let odd_words = "import ctypes\n\
        ctypes.string_at = lambda said: b'!#acl 1\\nuser:0:someone:502:audit:read\\n'\n";
    assert_eq!(refused(&far_with(SHORT, odd_words)), "permissions-unread");
    let no_fields = "import ctypes\nctypes.string_at = lambda said: b'!#acl 1\\nallow\\n'\n";
    assert_eq!(refused(&far_with(SHORT, no_fields)), "permissions-unread");
    assert_eq!(gate.connections(), 0);
    // The relay as it is reads the same list and is let through.
    assert_eq!(asked(&Far::default(), &lab, &home), Ok(()));
}
