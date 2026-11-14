//! The independent verifier's two probes from the first review of ticket
//! 25 (`evidence/25/verifier25.rs`). The first is as written. The second
//! is as written but for the route it records with: the review asked for
//! a step that hands the route the accepted instance before the ticket,
//! and the route now has one (`Route::prepare`), so the recording route
//! says "accepted-instance/forward" there and "dial" where a stream is
//! asked for. Its assertion stands, and one is added: the stream still
//! comes after the ticket, as it did before the route.
use super::{fake::Fake, lab, quick, until, Observer};
use serde_json::{json, Value};
use std::{path::Path, sync::Arc};

fn start_at(home: &Path, lab: &super::Lab) -> Observer {
    Observer::start_on(
        Arc::new(super::super::route::local::Local::at(home)),
        &lab.data,
        lab.requests.clone(),
        Arc::new(|| {}),
        quick(),
    )
    .unwrap()
}

#[test]
fn verifier25_identify_after_home_alias_changes_keeps_forks_refusal() {
    let mut lab = lab();
    let gateway = Fake::start(&lab.home, "instance-1");
    let alias = lab.data.join("home-alias");
    let replacement = lab.data.join("different-home");
    crate::os::private_directory(&replacement).unwrap();
    std::os::unix::fs::symlink(&lab.home, &alias).unwrap();
    gateway.world().switch_after_identify = Some((alias.clone(), replacement));
    lab.observer = Some(start_at(&alias, &lab));
    until("identify is followed by a failure", || {
        lab.log().contains("connection failed")
    });
    lab.observer.take();
    let log = lab.log();
    eprintln!("ALIAS SWITCH\n{log}");
    assert!(log.contains("reason=ProfileNotServed"), "{log}");
    assert!(!log.contains("connection ready"), "{log}");
}

mod ssh_contract {
    use super::super::super::{
        control::Silent,
        route::{local::Local, Home, Route, Stop},
    };
    use super::*;
    use agent_hud_agents::hermes::gateway::{Failure, Instance};
    use std::{borrow::Cow, net::TcpStream, sync::Mutex, time::Duration};

    struct Ordered {
        local: Local,
        events: Mutex<Vec<String>>,
    }

    impl Route for Ordered {
        fn homes(&self) -> Result<Vec<Home>, Silent> {
            self.events.lock().unwrap().push("homes".into());
            self.local.homes()
        }
        fn exchange(&self, home: &Home, line: &[u8], timeout: Duration) -> Result<Vec<u8>, Silent> {
            let value: Value = serde_json::from_slice(line).unwrap();
            self.events
                .lock()
                .unwrap()
                .push(value["verb"].as_str().unwrap().into());
            self.local.exchange(home, line, timeout)
        }
        fn named(&self, home: &Home) -> Cow<'_, str> {
            self.local.named(home)
        }
        fn prepare(&self, instance: &Instance, stop: &dyn Stop) -> Result<(), Failure> {
            self.events
                .lock()
                .unwrap()
                .push("accepted-instance/forward".into());
            self.local.prepare(instance, stop)
        }
        fn dial(&self, instance: &Instance) -> Result<TcpStream, Failure> {
            self.events.lock().unwrap().push("dial".into());
            self.local.dial(instance)
        }
    }

    #[test]
    fn verifier25_an_ssh_route_can_prepare_its_forward_before_requesting_a_ticket() {
        let lab = lab();
        let gateway = Fake::start(&lab.home, "instance-1");
        let route = Arc::new(Ordered {
            local: Local::at(&lab.home),
            events: Mutex::default(),
        });
        let observer = Observer::start_on(
            route.clone(),
            &lab.data,
            lab.requests.clone(),
            Arc::new(|| {}),
            quick(),
        )
        .unwrap();
        until("the gateway is polled", || {
            !gateway.world().calls("session.active_list").is_empty()
        });
        drop(observer);
        let events = route.events.lock().unwrap().clone();
        eprintln!("ROUTE ORDER {}", json!(events));
        let instance = events
            .iter()
            .position(|e| e == "accepted-instance/forward")
            .unwrap();
        let ticket = events.iter().position(|e| e == "session-ticket").unwrap();
        assert!(
            instance < ticket,
            "ticket 30 requires the forward before the ticket, got {events:?}"
        );
        let dial = events.iter().position(|e| e == "dial").unwrap();
        assert!(
            ticket < dial,
            "the stream is asked for after the ticket, got {events:?}"
        );
    }
}
