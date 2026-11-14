//! A ticket is in an answer and nowhere else: no log, no error, and
//! nothing a test prints holds one.
use super::{
    super::{Far, Silent},
    fixture::{far_with, text, FarLab, Manner, SHORT},
};
use agent_hud_agents::hermes::gateway::{
    far::{Quiet, Request},
    granted, identify, Identified, Scope,
};
use std::{fs, process::Command};

/// Names the ticket the inner run is granted, so the outer one knows what
/// to look for.
const TICKET: &str = "AGENT_HUD_TEST_FAR_TICKET";
const INNER: &str =
    "clients::hermes::far::tests::secret::a_granted_ticket_and_everything_around_it";

/// Everything that could carry a ticket is printed here on purpose: the
/// answers, the errors, and every log the lab and the link keep. Run by
/// itself it only shows that none of it fails; `a_ticket_is_in_no_output`
/// runs it and reads what it printed.
#[test]
#[ignore = "in the lab: tests::the_lab_tests_pass runs it"]
fn a_granted_ticket_and_everything_around_it() {
    let ticket = std::env::var(TICKET).unwrap_or_else(|_| "a-ticket-of-the-lab".into());
    let Some(lab) = FarLab::start() else { return };
    let home = lab.root();
    let manner = Manner::Gateway {
        instance: "one".into(),
        served: vec![home.clone()],
        ticket: ticket.clone(),
    };
    let gate = lab.gate(&home, manner);
    let far = Far::default();
    let told = far.exchange(&lab.link, text(&home), Request::Identify);
    println!("identify: {told:?}");
    let Identified::Ready(instance) =
        identify(told.said.unwrap().value(), text(&home), text(&home))
    else {
        panic!("no instance");
    };
    let asked = Request::Ticket {
        profile_id: &instance.profile_id,
        instance_id: &instance.id,
        scope: Scope::Host,
    };
    // Granted: the answer and the ticket print as masked.
    let grant = far.exchange(&lab.link, text(&home), asked);
    println!("granted: {grant:?}");
    eprintln!("granted: {:?}", grant.said);
    let held = granted(grant.said.as_ref().unwrap().value(), &instance).expect("a ticket");
    println!("ticket: {held:?}");
    // It is the ticket, though: the test is not looking for nothing.
    assert!(held.subprotocols()[1].ends_with(&ticket));
    // Granted and then lost on the way: a relay that fails after it has
    // the answer, and one that is refused, say nothing of what they held.
    let failing = far_with(
        SHORT,
        "_real_exchange = exchange\ndef exchange(path, line):\n    \
         raise RuntimeError(_real_exchange(path, line).decode())\n",
    );
    let failed = failing.exchange(&lab.link, text(&home), asked);
    println!("failed after the answer: {failed:?}");
    assert_eq!(failed.said.unwrap_err(), Silent::Relay(Quiet::Failed));
    let stale = Request::Ticket {
        profile_id: &instance.profile_id,
        instance_id: "another",
        scope: Scope::Host,
    };
    println!("stale: {:?}", far.exchange(&lab.link, text(&home), stale));
    assert_eq!(gate.seen().len(), 4);
    // Every log there is: the lab's `sshd`, and what the link's master said.
    let mut logs = vec![lab.lab.at("sshd.log")];
    for entry in fs::read_dir(lab.lab.at("c s")).unwrap().flatten() {
        if entry.file_type().is_ok_and(|kind| kind.is_file()) {
            logs.push(entry.path());
        }
    }
    for log in logs {
        let said = String::from_utf8_lossy(&fs::read(&log).unwrap_or_default()).into_owned();
        println!("--- {}\n{said}", log.display());
    }
}

#[test]
#[ignore = "in the lab: tests::the_lab_tests_pass runs it"]
fn a_ticket_is_in_no_output() {
    let ticket = format!("grant-{:032x}", rand_number());
    let ran = Command::new(std::env::current_exe().expect("this test program"))
        .args([INNER, "--exact", "--ignored", "--nocapture"])
        .env(TICKET, &ticket)
        .output()
        .expect("the test program starts");
    let printed = format!(
        "{}\n{}",
        String::from_utf8_lossy(&ran.stdout),
        String::from_utf8_lossy(&ran.stderr)
    );
    assert!(ran.status.success(), "{printed}");
    // It ran, was granted the ticket, and printed around it.
    for said in [
        "1 passed",
        "granted: Exchanged",
        "Answered(<masked>)",
        "Ticket(<masked>)",
    ] {
        assert!(printed.contains(said), "{said} is missing from:\n{printed}");
    }
    assert!(printed.contains("sshd.log"), "the logs were printed");
    assert!(!printed.contains(&ticket), "a ticket was printed");
    // Not in pieces either: the part after its fixed start.
    assert!(!printed.contains(&ticket["grant-".len()..]));
    println!("{} bytes of output, none of them the ticket", printed.len());
}

/// A number no earlier run had: the time, and where this process is.
fn rand_number() -> u128 {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |since| since.as_nanos());
    now ^ (u128::from(std::process::id()) << 64)
}
