//! The relay and discovery against real Hermes gateways, each in a Hermes
//! home of its own, through the lab's `sshd`. Run by hand:
//!
//! ```sh
//! AGENT_HUD_FAR_LAB_ROOTS=/path/to/a/.hermes:/another/.hermes \
//!   cargo test -p agent-hud-platform --lib clients::hermes::far::tests::real -- --ignored --nocapture
//! ```
//!
//! Each root is a throwaway Hermes home whose gateway is running, never
//! the user's own. One whose path is too long to hold a socket shows the
//! pointer file Hermes itself writes.
use super::{super::Far, fixture::FarLab};
use agent_hud_agents::hermes::gateway::{
    far::{Asked, Request},
    granted, Identified, Scope,
};
use std::{path::Path, time::Duration};

const ROOTS: &str = "AGENT_HUD_FAR_LAB_ROOTS";
/// How many times each exchange is timed.
const TIMES: usize = 10;

fn spread(mut took: Vec<Duration>) -> String {
    took.sort();
    let middle = took[took.len() / 2];
    format!(
        "least {:?}, middle {middle:?}, most {:?} of {}",
        took[0],
        took[took.len() - 1],
        took.len()
    )
}

#[test]
#[ignore = "by hand: needs real Hermes gateways, named in AGENT_HUD_FAR_LAB_ROOTS"]
fn real_gateways_answer_through_the_relay() {
    let Ok(roots) = std::env::var(ROOTS) else {
        println!("skipped: {ROOTS} names no Hermes home");
        return;
    };
    let own = std::env::var("HOME").unwrap_or_default();
    for root in roots.split(':') {
        assert!(
            !Path::new(root).starts_with(Path::new(&own).join(".hermes")),
            "never the user's own Hermes home"
        );
        // The far login is told where Hermes lives, as a far machine with
        // a root of its own would be.
        let Some(lab) = FarLab::start_with(&[("HERMES_HOME", root)]) else {
            return;
        };
        let far = Far::default();
        let picture = far.discover(&lab.link).expect("a picture");
        // The same picture with the homes asked last first.
        let found = far.homes(&lab.link).expect("the homes");
        let backward = far.identify_all(&lab.link, &found, found.homes.iter().rev());
        assert_eq!(
            backward.as_ref(),
            Ok(&picture),
            "whichever home is asked first"
        );
        println!("root {}", picture.root);
        for (home, asked) in &picture.asked {
            let pointer = Path::new(&home.path).join("gateway.sock.path").exists();
            let socket = Path::new(&home.path).join("gateway.sock").exists();
            // What came of it, without the whole instance.
            let came = match asked {
                Asked::Answered(Identified::Ready(instance)) => {
                    format!("answered as instance {}", instance.id)
                }
                other => format!("{other:?}"),
            };
            println!(
                "  asked {} ({}): socket in the home {socket}, pointer file {pointer}: {came}",
                home.path,
                home.profile.name()
            );
        }
        assert!(!picture.gateways.is_empty(), "no gateway answered");
        for gateway in &picture.gateways {
            let served: Vec<&str> = gateway.served.iter().map(|h| h.path.as_str()).collect();
            println!("  gateway {} serves {served:?}", gateway.instance_id);
            let (home, instance) = &gateway.sockets[0];
            let asked = Request::Ticket {
                profile_id: &instance.profile_id,
                instance_id: &instance.id,
                scope: Scope::Host,
            };
            let (mut identifies, mut tickets) = (Vec::new(), Vec::new());
            for _ in 0..TIMES {
                let told = far.exchange(&lab.link, &home.path, Request::Identify);
                assert!(told.said.is_ok(), "{told:?}");
                identifies.push(told.took);
                let grant = far.exchange(&lab.link, &home.path, asked);
                let ticket = granted(grant.said.as_ref().expect("an answer").value(), instance);
                assert!(ticket.is_ok(), "no ticket was granted");
                tickets.push(grant.took);
            }
            println!("  identify through the relay: {}", spread(identifies));
            println!("  session-ticket through the relay: {}", spread(tickets));
        }
    }
}

/// How long each exchange takes over the open link, with a pretend
/// gateway that answers at once: the relay's own cost.
#[test]
#[ignore = "in the lab: tests::the_lab_tests_pass runs it"]
fn each_exchange_is_timed() {
    use super::fixture::{text, Manner};
    let Some(lab) = FarLab::start() else { return };
    let (root, ops) = (lab.root(), lab.profile("ops"));
    let _gate = lab.gate(&root, Manner::gateway("one", &[&root, &ops]));
    let far = Far::default();
    let mut took = Vec::new();
    for _ in 0..TIMES {
        let told = far.exchange(&lab.link, text(&root), Request::Identify);
        assert!(told.said.is_ok());
        assert_eq!(told.tries, 1);
        took.push(told.took);
    }
    println!(
        "identify through the relay, pretend gateway: {}",
        spread(took)
    );
    let started = std::time::Instant::now();
    far.homes(&lab.link).expect("the homes");
    println!("the homes command: {:?}", started.elapsed());
    let started = std::time::Instant::now();
    far.discover(&lab.link).expect("a picture");
    println!("a whole discovery of two homes: {:?}", started.elapsed());
}
