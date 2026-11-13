//! The control folder: what start-up cleanup closes and removes, what it
//! never touches, and which folders are refused.
use super::{
    super::{args, process},
    lab::{entries, far, runs, until, Lab},
};
use crate::ssh_link::{End, Folder, Link, Refused, Settings, Swept, Target, LONGEST_SOCKET_PATH};
use std::{
    fs,
    net::TcpListener,
    os::unix::{
        fs::{symlink, PermissionsExt},
        net::{UnixListener, UnixStream},
    },
    process::{Command, Stdio},
    time::Duration,
};

/// A master with nobody to close it, as a crash leaves one: started by a
/// shell that has already exited, with its record in the folder.
fn orphan(folder: &Folder, settings: &Settings, host: &str) -> (String, u32) {
    let name = folder.fresh().unwrap();
    let target = Target::new(host, None, None, None).unwrap();
    let started = Command::new("/bin/sh")
        .arg("-c")
        .arg("\"$@\" </dev/null >/dev/null 2>\"$LOG\" & echo $!")
        .arg("sh")
        .arg(&settings.program)
        .args(args::master(settings, &target, &folder.control(&name)))
        .env("LOG", folder.file(&format!("{name}.log")))
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
        .unwrap();
    let pid: u32 = String::from_utf8_lossy(&started.stdout)
        .trim()
        .parse()
        .unwrap();
    folder.record(&name, pid).unwrap();
    (name, pid)
}

fn up(folder: &Folder, name: &str) -> bool {
    until(Duration::from_secs(15), || folder.is_socket(name))
}

#[test]
fn start_up_closes_every_master_left_behind_wanted_or_not() {
    let Some(lab) = Lab::start() else { return };
    let folder = lab.folder();
    let settings = lab.settings();
    // Two masters outlive their owner. `other` is a machine that is no
    // longer wanted: nothing will ever ask for it again.
    let (wanted, wanted_pid) = orphan(&folder, &settings, "lab");
    let (unwanted, unwanted_pid) = orphan(&folder, &settings, "other");
    assert!(up(&folder, &wanted) && up(&folder, &unwanted));
    assert!(process::master_runs(wanted_pid, &folder.control(&wanted)));
    assert!(process::master_runs(
        unwanted_pid,
        &folder.control(&unwanted)
    ));
    assert_eq!(lab.link_processes(&folder).len(), 2);
    println!("before: {:?}", entries(folder.path()));

    let swept = folder.sweep(&settings);
    println!("swept: {swept:?}");
    assert_eq!(
        swept,
        Swept {
            closed: 2,
            removed: 2,
            left: 0
        }
    );
    assert!(until(Duration::from_secs(5), || !runs(wanted_pid)
        && !runs(unwanted_pid)));
    assert_eq!(lab.link_processes(&folder), Vec::<String>::new());
    assert_eq!(entries(folder.path()), Vec::<String>::new());

    // The folder is as good as new.
    let link = Link::open(&folder, &lab.target(), &settings).unwrap();
    assert_eq!(link.run(&far("exit 2", &[])).unwrap().end, End::Ended(2));
    link.close();
}

#[test]
fn a_dead_masters_socket_goes_and_a_master_still_coming_up_is_ended() {
    let Some(lab) = Lab::start() else { return };
    let folder = lab.folder();
    let settings = lab.settings();
    // Killed outright: its socket stays behind with nobody listening.
    let (dead, dead_pid) = orphan(&folder, &settings, "lab");
    assert!(up(&folder, &dead));
    // SAFETY: a signal to the master this test just started.
    unsafe { libc::kill(dead_pid as libc::pid_t, libc::SIGKILL) };
    assert!(until(Duration::from_secs(5), || !runs(dead_pid)));
    assert!(folder.is_socket(&dead), "a killed master leaves its socket");

    // Still connecting when its owner died: a record and no socket. The
    // host accepts and says nothing, so the master waits.
    let silent = TcpListener::bind("127.0.0.1:0").unwrap();
    let config = format!(
        "{}Host silent\n  HostName 127.0.0.1\n  Port {}\n",
        lab.config_text(),
        silent.local_addr().unwrap().port()
    );
    let slow = Settings {
        connect_timeout: Duration::from_secs(60),
        ..lab.settings_with("silent", &config)
    };
    let (pending, pending_pid) = orphan(&folder, &slow, "silent");
    assert!(until(Duration::from_secs(5), || process::master_runs(
        pending_pid,
        &folder.control(&pending)
    )));
    assert!(!folder.is_socket(&pending));
    // And one whose owner died before it wrote the record.
    let (unrecorded, unrecorded_pid) = orphan(&folder, &slow, "silent");
    fs::remove_file(folder.file(&format!("{unrecorded}.pid"))).unwrap();
    assert!(until(Duration::from_secs(5), || process::master_runs(
        unrecorded_pid,
        &folder.control(&unrecorded)
    )));

    let swept = folder.sweep(&settings);
    println!("swept: {swept:?}");
    assert_eq!(
        swept,
        Swept {
            closed: 2,
            removed: 1,
            left: 0
        }
    );
    assert!(until(Duration::from_secs(5), || !runs(pending_pid)
        && !runs(unrecorded_pid)));
    assert_eq!(entries(folder.path()), Vec::<String>::new());
    assert_eq!(lab.link_processes(&folder), Vec::<String>::new());
}

#[test]
fn a_link_in_the_folder_is_not_followed_and_what_it_points_at_stays() {
    let Some(lab) = Lab::start() else { return };
    let folder = lab.folder();
    // A socket and a file outside the folder, and links to them inside it
    // under the names cleanup looks for.
    let outside = lab.at("outside.sock");
    let listener = UnixListener::bind(&outside).unwrap();
    fs::write(lab.at("keep.txt"), "4242\n").unwrap();
    symlink(&outside, folder.path().join("trap")).unwrap();
    symlink(lab.at("keep.txt"), folder.path().join("trap.pid")).unwrap();
    symlink(lab.at("keep.txt"), folder.path().join("other.log")).unwrap();
    fs::create_dir(folder.path().join("sub")).unwrap();

    let swept = folder.sweep(&lab.settings());
    println!("swept: {swept:?}");
    assert_eq!(
        swept,
        Swept {
            closed: 0,
            removed: 0,
            left: 4
        }
    );
    assert!(
        UnixStream::connect(&outside).is_ok(),
        "the socket outside still answers"
    );
    drop(listener);
    assert_eq!(fs::read_to_string(lab.at("keep.txt")).unwrap(), "4242\n");
    assert_eq!(
        entries(folder.path()),
        ["other.log", "sub", "trap", "trap.pid"]
    );
    for name in ["trap", "trap.pid", "other.log"] {
        let node = fs::symlink_metadata(folder.path().join(name)).unwrap();
        assert!(node.file_type().is_symlink());
    }
    // Nor is one read as a socket or a record.
    assert!(!folder.is_socket("trap"));
    assert_eq!(folder.recorded("trap"), None);
}

#[test]
fn a_folder_with_the_wrong_owner_or_mode_is_refused() {
    let temp = tempfile::tempdir().unwrap();
    let open = temp.path().join("open");
    fs::create_dir(&open).unwrap();
    fs::set_permissions(&open, fs::Permissions::from_mode(0o755)).unwrap();
    assert_eq!(Folder::open(&open).err(), Some(Refused::FolderMode));
    // It is not closed up and trusted: it stays as it was found.
    assert_eq!(
        fs::metadata(&open).unwrap().permissions().mode() & 0o777,
        0o755
    );
    fs::set_permissions(&open, fs::Permissions::from_mode(0o750)).unwrap();
    assert_eq!(Folder::open(&open).err(), Some(Refused::FolderMode));
    fs::set_permissions(&open, fs::Permissions::from_mode(0o700)).unwrap();
    assert!(Folder::open(&open).is_ok());

    // A link to a private folder is not that folder.
    let link = temp.path().join("link");
    symlink(&open, &link).unwrap();
    assert_eq!(Folder::open(&link).err(), Some(Refused::FolderKind));
    let file = temp.path().join("file");
    fs::write(&file, "").unwrap();
    assert_eq!(Folder::open(&file).err(), Some(Refused::Unavailable));

    // A folder absent so far is made private.
    let made = temp.path().join("a").join("made");
    assert!(Folder::open(&made).is_ok());
    assert_eq!(
        fs::metadata(&made).unwrap().permissions().mode() & 0o777,
        0o700
    );

    // Root's own home is another user's folder, whatever its mode.
    let roots = ["/var/root", "/root"];
    let root = roots
        .iter()
        .find(|path| fs::symlink_metadata(path).is_ok_and(|node| node.is_dir()))
        .expect("root has a home folder here");
    assert_eq!(
        Folder::open(root.as_ref()).err(),
        Some(Refused::FolderOwner)
    );
    assert_eq!(
        Folder::open("relative".as_ref()).err(),
        Some(Refused::FolderPath)
    );
    assert_eq!(
        Folder::open("/tmp/a$b".as_ref()).err(),
        Some(Refused::FolderPath)
    );
}

#[test]
fn the_control_path_is_held_to_the_socket_limit() {
    let Some(lab) = Lab::start() else { return };
    // A socket's name is eight characters after the folder and a slash.
    let longest = LONGEST_SOCKET_PATH - 9;
    let folder_of = |length: usize| {
        let base = lab.at("");
        let base = base.to_str().unwrap().trim_end_matches('/').to_owned();
        format!("{base}/{}", "p".repeat(length - base.len() - 1))
    };
    let fits = folder_of(longest);
    assert_eq!(fits.len(), 77);
    let folder = Folder::open(fits.as_ref()).expect("the longest folder that fits");
    // A real master binds its socket there, so the limit is `ssh`'s own.
    let link = Link::open(&folder, &lab.target(), &lab.settings()).expect("it opens at the limit");
    let socket = entries(folder.path())
        .into_iter()
        .find(|name| folder.is_socket(name))
        .unwrap();
    assert_eq!(
        folder.file(&socket).to_str().unwrap().len(),
        LONGEST_SOCKET_PATH
    );
    assert_eq!(link.run(&far("exit 5", &[])).unwrap().end, End::Ended(5));
    link.close();

    // One byte more is refused before anything is made.
    let over = folder_of(longest + 1);
    assert_eq!(
        Folder::open(over.as_ref()).err(),
        Some(Refused::PathTooLong)
    );
    assert!(!std::path::Path::new(&over).exists());
    // A long home folder fails with that reason and no other.
    let home = format!(
        "/Users/{}/Library/Application Support/Agent HUD/ssh",
        "n".repeat(40)
    );
    let refused = Folder::open(home.as_ref()).err().unwrap();
    assert_eq!(refused, Refused::PathTooLong);
    assert_eq!(
        refused.reason(),
        "control folder path too long for a socket"
    );
}

#[test]
fn a_percent_in_the_folders_path_is_plain_text_to_ssh() {
    let Some(lab) = Lab::start() else { return };
    // `ssh` would read `%h` in a control path as the host's name.
    let folder = Folder::open(&lab.at("a %h b")).unwrap();
    let link = Link::open(&folder, &lab.target(), &lab.settings()).expect("it opens");
    assert_eq!(
        entries(folder.path())
            .iter()
            .filter(|name| folder.is_socket(name))
            .count(),
        1
    );
    assert_eq!(lab.link_processes(&folder).len(), 1);
    assert!(link.close().gone);
    assert_eq!(entries(folder.path()), Vec::<String>::new());
}
