//! A lab login is a login to the real account of whoever runs the tests.
//! These hold what such a login would otherwise read of the user's.
use super::lab::{far, Lab, MARKER, SHELL};
use crate::ssh_link::{End, Link};
use std::process::Command;

/// The values `sshd -T` prints for one keyword, lowercased.
fn values<'a>(settings: &'a str, key: &str) -> Vec<&'a str> {
    settings
        .lines()
        .filter_map(|line| line.strip_prefix(key)?.strip_prefix(' '))
        .collect()
}

/// The effective settings of the lab's own `sshd`, read with `sshd -T`.
/// Nobody signs in for this.
#[test]
#[ignore = "in the lab: tests::the_lab_tests_pass runs it"]
fn v_lab_disables_user_ssh_startup_files() {
    let Some(lab) = Lab::start() else { return };
    let printed = Command::new("/usr/sbin/sshd")
        .args(["-T", "-f"])
        .arg(lab.at("sshd_config"))
        .output()
        .unwrap();
    assert!(
        printed.status.success(),
        "{}",
        String::from_utf8_lossy(&printed.stderr)
    );
    let settings = String::from_utf8_lossy(&printed.stdout).to_lowercase();
    for key in [
        "permituserrc",
        "permituserenvironment",
        "authorizedkeysfile",
        "hostbasedauthentication",
        "ignorerhosts",
        "x11forwarding",
        "setenv",
    ] {
        println!("{key} {:?}", values(&settings, key));
    }
    assert_eq!(
        values(&settings, "permituserrc"),
        ["no"],
        "lab logins must not run the real login user's ~/.ssh/rc"
    );
    assert_eq!(values(&settings, "permituserenvironment"), ["no"]);
    let keys = lab
        .at("authorized_keys")
        .display()
        .to_string()
        .to_lowercase();
    assert_eq!(values(&settings, "authorizedkeysfile"), [keys.as_str()]);
    assert_eq!(values(&settings, "hostbasedauthentication"), ["no"]);
    assert_eq!(values(&settings, "ignorerhosts"), ["yes"]);
    assert_eq!(values(&settings, "x11forwarding"), ["no"]);
    let shell = lab.at(SHELL).display().to_string().to_lowercase();
    let set = values(&settings, "setenv").join(" ");
    for variable in [
        format!("zdotdir={shell}"),
        format!("xdg_config_home={shell}"),
        "bash_env=/dev/null".to_owned(),
        "env=/dev/null".to_owned(),
    ] {
        assert!(set.contains(&variable), "{variable} is not in {set}");
    }
}

/// The login shell reads the lab's startup file and so not the user's: a
/// shell reads one or the other, from where `ZDOTDIR` or `XDG_CONFIG_HOME`
/// points.
#[test]
#[ignore = "in the lab: tests::the_lab_tests_pass runs it"]
fn a_lab_login_reads_the_labs_shell_startup_file_and_not_the_users() {
    let Some(lab) = Lab::start() else { return };
    let folder = lab.folder();
    let link = Link::open(&folder, &lab.target(), &lab.settings()).unwrap();
    let script = format!(
        "printf '%s\\n' \"${{ZDOTDIR-unset}}\" \"${{XDG_CONFIG_HOME-unset}}\" \
         \"${{BASH_ENV-unset}}\" \"${{ENV-unset}}\" \"${{{MARKER}-unset}}\" \"$SHELL\""
    );
    let ran = link.run(&far(&script, &[])).unwrap();
    assert_eq!(ran.end, End::Ended(0), "{}", ran.stderr);
    let said = String::from_utf8(ran.stdout).unwrap();
    println!("{said}");
    let said: Vec<&str> = said.lines().collect();
    let shell = lab.at(SHELL).display().to_string();
    assert_eq!(
        said[..4],
        [shell.as_str(), shell.as_str(), "/dev/null", "/dev/null"]
    );
    // The marker is set by the startup file itself, so the login shell
    // read the lab's.
    let login = said[5].rsplit('/').next().unwrap();
    match login {
        "zsh" | "fish" => assert_eq!(said[4], login),
        // bash and sh read no startup file for a command.
        _ => assert_eq!(said[4], "unset"),
    }
    link.close();
}
