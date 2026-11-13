//! What the link checks before any process exists. These run on every
//! host; the link against a real `sshd` is in `unix/tests/`.
use super::{quote, Lost, Refused, Settings, Target};
use std::time::Duration;

#[test]
fn a_target_is_made_only_from_plain_parts() {
    let key = std::env::temp_dir().join("id_ed25519");
    for (host, user, port) in [
        ("mini", None, None),
        ("mini.local", Some("nicky"), Some(22)),
        ("10.0.0.5", Some("a.b-c_d"), Some(65535)),
        ("my-host_2", None, Some(1)),
        ("::1", None, None),
        ("fe80::1", Some("nicky"), Some(2222)),
    ] {
        let target = Target::new(host, user, port, Some(&key)).expect(host);
        assert_eq!(target.port(), port);
        assert_eq!(target.key(), Some(key.as_path()));
        let destination = user.map_or(host.to_owned(), |user| format!("{user}@{host}"));
        assert_eq!(target.destination(), destination);
    }
    for host in [
        "",
        "-oProxyCommand=touch x",
        "-oProxyCommand=x@host",
        "-v",
        "a b",
        "a;b",
        "a\nb",
        "a\tb",
        "a$b",
        "a`b`",
        "a|b",
        "a&b",
        "a'b",
        "a\"b",
        "a\\b",
        "a/b",
        "a=b",
        "a@b",
        "a,b",
        "a*",
        "ünï",
    ] {
        assert_eq!(
            Target::new(host, None, None, None),
            Err(Refused::Address),
            "host {host:?}"
        );
        assert_eq!(
            Target::new("mini", Some(host), None, None),
            Err(Refused::Address),
            "user {host:?}"
        );
    }
    // A colon is a host's alone, and a name is at most 255 long.
    assert_eq!(
        Target::new("mini", Some("a:b"), None, None),
        Err(Refused::Address)
    );
    assert!(Target::new(&"h".repeat(255), Some(&"u".repeat(255)), None, None).is_ok());
    assert_eq!(
        Target::new(&"h".repeat(256), None, None, None),
        Err(Refused::Address)
    );
    assert_eq!(
        Target::new("mini", Some(&"u".repeat(256)), None, None),
        Err(Refused::Address)
    );
    assert_eq!(
        Target::new("mini", None, Some(0), None),
        Err(Refused::Address)
    );
    for key in ["", "-oProxyCommand=x", "a\nb"] {
        assert_eq!(
            Target::new("mini", None, None, Some(key.as_ref())),
            Err(Refused::KeyPath),
            "key {key:?}"
        );
    }
    assert_eq!(Refused::Address.reason(), "unusable address");
}

#[test]
fn quoting_leaves_nothing_for_a_shell_to_read() {
    assert_eq!(quote("plain"), "'plain'");
    assert_eq!(quote(""), "''");
    assert_eq!(quote("a b $HOME `id` \"q\" *"), "'a b $HOME `id` \"q\" *'");
    assert_eq!(quote("it's"), "'it'\\''s'");
    // Outside the quotes, where fish and POSIX shells read it alike.
    assert_eq!(quote("a\\b"), "'a'\\\\'b'");
}

#[test]
fn the_apps_settings_are_the_systems_ssh_with_long_waits() {
    let settings = Settings::system();
    assert!(settings.program.ends_with("ssh"));
    assert_eq!(settings.config, None);
    assert_eq!(settings.connect_timeout, Duration::from_secs(30));
    assert_eq!(settings.keepalive_bound(), Duration::from_secs(45));
    assert_eq!(Lost::NotTrusted.reason(), "not trusted yet");
    assert_eq!(Lost::SignInRefused.reason(), "sign-in refused");
    assert_eq!(Lost::CannotReach.reason(), "cannot reach");
    assert_eq!(Lost::TimedOut.reason(), "timed out");
}
