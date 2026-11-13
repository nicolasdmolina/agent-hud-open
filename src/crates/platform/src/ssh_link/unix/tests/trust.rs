//! Whose host it is and who may sign in.
use super::lab::{entries, far, free_port, Lab};
use crate::ssh_link::{Failure, Link, Lost};
use std::fs;

#[test]
fn an_unknown_host_is_not_trusted_yet_and_known_hosts_stays_as_it_was() {
    let Some(lab) = Lab::start() else { return };
    let folder = lab.folder();
    // A file that knows some other host, and not this one.
    let known = lab
        .host_line("otherhost")
        .replace(&lab.port.to_string(), "1");
    fs::write(lab.at("elsewhere"), &known).unwrap();
    let config = lab.config_text().replace("known_hosts", "elsewhere");
    let refused = Link::open(&folder, &lab.target(), &lab.settings_with("c", &config));
    let Err(Failure::Lost(lost)) = refused else {
        panic!("an unknown host was let in");
    };
    assert_eq!(lost, Lost::NotTrusted);
    assert_eq!(lost.reason(), "not trusted yet");
    assert_eq!(fs::read(lab.at("elsewhere")).unwrap(), known.as_bytes());
    assert_eq!(lab.link_processes(&folder), Vec::<String>::new());
    assert_eq!(entries(folder.path()), Vec::<String>::new());
}

#[test]
fn a_changed_host_key_is_refused() {
    let Some(lab) = Lab::start() else { return };
    let folder = lab.folder();
    // The right host and port, with a key that is not the one it shows.
    let known = lab.host_line("otherhost");
    fs::write(lab.at("stale"), &known).unwrap();
    let config = lab.config_text().replace("known_hosts", "stale");
    let refused = Link::open(&folder, &lab.target(), &lab.settings_with("c", &config));
    assert_eq!(refused.err(), Some(Failure::Lost(Lost::KeyChanged)));
    assert_eq!(fs::read(lab.at("stale")).unwrap(), known.as_bytes());
    assert_eq!(lab.link_processes(&folder), Vec::<String>::new());
    assert_eq!(entries(folder.path()), Vec::<String>::new());
}

#[test]
fn a_refused_sign_in_and_a_closed_port_each_say_which() {
    let Some(lab) = Lab::start() else { return };
    let folder = lab.folder();
    let stranger = lab.config_text().replace("/id\"", "/strangerkey\"");
    let refused = Link::open(&folder, &lab.target(), &lab.settings_with("s", &stranger));
    assert_eq!(refused.err(), Some(Failure::Lost(Lost::SignInRefused)));

    let closed = lab.config_text().replace(
        &format!("Port {}", lab.port),
        &format!("Port {}", free_port()),
    );
    let refused = Link::open(&folder, &lab.target(), &lab.settings_with("p", &closed));
    assert_eq!(refused.err(), Some(Failure::Lost(Lost::CannotReach)));
    assert_eq!(lab.link_processes(&folder), Vec::<String>::new());
    assert_eq!(entries(folder.path()), Vec::<String>::new());
}

#[test]
fn the_agent_may_sign_in_and_is_never_forwarded() {
    let Some(lab) = Lab::start() else { return };
    let folder = lab.folder();
    let agent = lab.agent();
    // Only the agent holds this key: its private file is gone, and the
    // file named is the public half.
    let by_agent = lab
        .config_text()
        .replace("/id\"", "/agentkey.pub\"")
        .replace(
            "IdentityAgent none",
            &format!(
                "IdentityAgent \"{}\"\n  ForwardAgent yes",
                agent.1.display()
            ),
        );
    assert!(!lab.at("agentkey").exists());
    let link = Link::open(&folder, &lab.target(), &lab.settings_with("a", &by_agent))
        .expect("the agent signs in");
    let seen = link
        .run(&far("printf '%s' \"${SSH_AUTH_SOCK-none}\"", &[]))
        .unwrap();
    assert_eq!(seen.stdout, b"none");
    link.close();

    // Without the agent the same file signs nobody in: it was the agent.
    let no_agent = lab.config_text().replace("/id\"", "/agentkey.pub\"");
    let refused = Link::open(&folder, &lab.target(), &lab.settings_with("n", &no_agent));
    assert_eq!(refused.err(), Some(Failure::Lost(Lost::SignInRefused)));
}
