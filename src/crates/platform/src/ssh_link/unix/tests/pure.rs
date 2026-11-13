//! What needs no `sshd`: the argument lists, the far command line, the
//! meaning of `ssh`'s last words, and the Windows file's promise.
use super::super::{
    args::{self, Ask},
    words,
};
use crate::ssh_link::{
    windows_stub, Failure, Forward, Lost, Refused, Settings, Target, LONGEST_SOCKET_PATH,
};
use std::path::Path;

fn strings(args: Vec<std::ffi::OsString>) -> Vec<String> {
    args.into_iter()
        .map(|arg| arg.to_string_lossy().into_owned())
        .collect()
}

/// Whether `-o <option>` is in the list, as two arguments in that order.
fn sets(args: &[String], option: &str) -> bool {
    args.windows(2)
        .any(|pair| pair[0] == "-o" && pair[1] == option)
}

/// Every option spec section 6 lists, as the master is given them.
pub(super) const DESIGN: &[&str] = &[
    "BatchMode=yes",
    "StrictHostKeyChecking=yes",
    "UpdateHostKeys=no",
    "ForwardAgent=no",
    "ForwardX11=no",
    "ClearAllForwardings=yes",
    "PermitLocalCommand=no",
    "RequestTTY=no",
    "ControlMaster=yes",
    "ControlPersist=no",
    "ExitOnForwardFailure=yes",
    "ServerAliveInterval=15",
    "ServerAliveCountMax=3",
    "ConnectTimeout=30",
];

#[test]
fn the_masters_argument_list_holds_every_option_of_the_design() {
    let key = std::env::temp_dir().join("key");
    let target = Target::new("mini", Some("nicky"), Some(2222), Some(&key)).unwrap();
    let args = strings(args::master(&Settings::system(), &target, "\"/x y/s\""));
    for option in DESIGN {
        assert!(sets(&args, option), "{option} is missing from {args:?}");
    }
    assert!(sets(&args, "ControlPath=\"/x y/s\""));
    // The destination is last, after `--`, so it is never read as an option.
    assert_eq!(&args[args.len() - 2..], ["--", "nicky@mini"]);
    let before: Vec<&str> = args[..args.len() - 2].iter().map(String::as_str).collect();
    assert!(before.windows(2).any(|pair| pair == ["-p", "2222"]));
    assert!(before
        .windows(2)
        .any(|pair| pair[0] == "-i" && Path::new(pair[1]) == key));
    let odd = std::env::temp_dir().join("50%off");
    let target = Target::new("mini", None, None, Some(&odd)).unwrap();
    let with_key = strings(args::master(&Settings::system(), &target, "\"/s\""));
    let key_at = with_key.iter().position(|arg| arg == "-i").unwrap();
    assert!(with_key[key_at + 1].ends_with("50%%off"));
    // The user's own configuration is read: no file is named, and no
    // command, forward or background run is asked for.
    for absent in ["-F", "-f", "-L", "-R", "-D", "-A", "-X", "-Y", "-t", "-W"] {
        assert!(!before.contains(&absent), "{absent} in {args:?}");
    }
    assert!(before.contains(&"-N"));
}

#[test]
fn a_client_never_dials_and_reads_no_configuration_file() {
    let forward = Forward {
        local: 4000,
        far: 5000,
    };
    for ask in [
        Ask::Check,
        Ask::Exit,
        Ask::Forward(forward),
        Ask::Cancel(forward),
        Ask::Session("exec sh -c 'true' sh"),
    ] {
        let args = strings(args::client("\"/x/s\"", ask));
        assert_eq!(&args[..2], ["-F", "none"]);
        assert!(sets(&args, "ControlMaster=no"));
        for fixed in args::NO_DIAL.iter().chain(args::FIXED) {
            assert!(sets(&args, fixed), "{fixed} is missing from {args:?}");
        }
        let at = args.iter().position(|arg| arg == "--").unwrap();
        assert_eq!(args[at + 1], args::NOWHERE);
        // `ssh` drops a requested forward when told to clear them.
        let forwards = matches!(ask, Ask::Forward(_) | Ask::Cancel(_));
        assert_eq!(sets(&args, "ClearAllForwardings=yes"), !forwards);
        assert_eq!(
            args.windows(2)
                .any(|pair| pair == ["-L", "127.0.0.1:4000:127.0.0.1:5000"]),
            forwards
        );
    }
}

#[test]
fn a_far_command_runs_under_sh_with_every_word_quoted() {
    assert_eq!(
        words::far_line("cat \"$1\"", &["/a b/it's $HOME"]).unwrap(),
        "exec sh -c 'cat \"$1\"' sh '/a b/it'\\''s $HOME'"
    );
    assert_eq!(
        words::far_line("cat <<EOF\nx\nEOF", &[]),
        Err(Refused::HereDocument)
    );
    let long = "x".repeat(words::LONGEST_LINE);
    assert_eq!(words::far_line(&long, &[]), Err(Refused::CommandTooLong));
    assert_eq!(
        words::far_line("true", &[&long]),
        Err(Refused::CommandTooLong)
    );
    assert_eq!(
        words::far_line("true", &["a\0b"]),
        Err(Refused::Unavailable)
    );
}

#[test]
fn the_masters_last_words_say_why_it_ended() {
    let cases = [
        (
            "No ED25519 host key is known for [127.0.0.1]:22 and you have requested strict checking.\nHost key verification failed.\n",
            Lost::NotTrusted,
        ),
        (
            "@    WARNING: REMOTE HOST IDENTIFICATION HAS CHANGED!     @\nOffending ED25519 key in /k:1\nHost key for [127.0.0.1]:22 has changed and you have requested strict checking.\nHost key verification failed.\n",
            Lost::KeyChanged,
        ),
        ("nicky@mini: Permission denied (publickey).\n", Lost::SignInRefused),
        (
            "ssh: connect to host 127.0.0.1 port 9: Connection refused\n",
            Lost::CannotReach,
        ),
        (
            "ssh: Could not resolve hostname nowhere: nodename nor servname provided, or not known\n",
            Lost::CannotReach,
        ),
        (
            "ssh: connect to host 10.0.0.9 port 22: Operation timed out\n",
            Lost::TimedOut,
        ),
        ("Timeout, server 127.0.0.1 not responding.\n", Lost::TimedOut),
        ("", Lost::Ended),
        // What a healthy master said in passing is not why it ended.
        (
            "channel 2: open failed: connect failed: Connection refused\nbind [127.0.0.1]:4000: Address already in use\n",
            Lost::Ended,
        ),
    ];
    for (log, lost) in cases {
        assert_eq!(words::why_lost(log), lost, "{log}");
    }
}

#[test]
fn the_socket_limit_is_the_address_size_less_what_ssh_adds() {
    assert_eq!(LONGEST_SOCKET_PATH, 86);
}

#[test]
fn the_windows_file_says_not_implemented() {
    let path = std::env::temp_dir().join("agent-hud-no-folder-is-made");
    assert_eq!(
        windows_stub::Folder::open(&path).err(),
        Some(Refused::NotImplemented)
    );
    assert!(!path.exists());
    assert_eq!(Refused::NotImplemented.reason(), "not implemented");
    assert_eq!(
        Failure::Refused(Refused::NotImplemented).to_string(),
        "not implemented"
    );
    // No folder can be had, so no link can be asked for. The entry points
    // are the unix file's all the same, by name and by type.
    type Open =
        fn(&windows_stub::Folder, &Target, &Settings) -> Result<windows_stub::Link, Failure>;
    let _: Open = windows_stub::Link::open;
}
