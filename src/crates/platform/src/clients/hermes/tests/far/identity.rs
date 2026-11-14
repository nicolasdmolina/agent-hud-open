//! Identity before anything is told: which machine a route leads to is
//! read from its own Hermes, on a socket opened with a ticket, and a
//! machine that gives no usable answer, or two, is not covered. Nothing
//! of it reaches core.
use super::{super::fake::Fake, far_lab, until, FarLab, Standing, INSTALL, OTHER_INSTALL};
use serde_json::{json, Value};
use std::time::Duration;

/// Nothing of the far machine is in core: no card, no session that
/// waits, no reader that told a state.
fn nothing_told(far: &FarLab) {
    assert_eq!(far.cards(), Vec::<Value>::new());
    let waiting = far.requests.native_waiting_sessions();
    assert!(
        waiting.values().all(|sessions| sessions.is_empty()),
        "{waiting:?}"
    );
    assert!(!far.log().contains("followed machine="), "{}", far.log());
    assert!(!far.log().contains("open id="), "{}", far.log());
}

fn not_covered(far: &FarLab, why: &str) {
    let wanted = Standing::NotCovered(why.to_owned());
    until(&format!("not covered: {why}"), || far.standing() == wanted);
    until("the log says so", || {
        far.log().contains(&format!("not covered why={why}"))
    });
}

#[test]
#[ignore = "starts processes: run by the_lab_tests_pass"]
fn a_gateway_with_no_usable_install_id_makes_no_machine_and_says_why() {
    let Some(mut far) = far_lab() else {
        return;
    };
    let gateway = far.gateway("instance-1");
    gateway.world().install = None;
    // A question waits there all along: it must not show.
    gateway.play("question-answered-here", Some("clarify.request"));
    far.follow();
    let cases = [
        (None, "install_id_missing"),
        (Some(json!("")), "install_id_empty"),
        (Some(json!("not-an-install-id")), "install_id_length"),
        (Some(json!(INSTALL.to_uppercase())), "install_id_upper_case"),
        (Some(json!(INSTALL.replace('a', "z"))), "install_id_not_hex"),
        (Some(json!(42)), "install_id_length"),
        (Some(Value::Null), "install_id_missing"),
    ];
    for (install, why) in cases {
        gateway.world().install = install.clone();
        // Asked again, and refused for this reason.
        let asked = gateway.world().calls("profiles.list").len();
        until("it is asked again", || {
            gateway.world().calls("profiles.list").len() > asked
        });
        not_covered(&far, why);
        nothing_told(&far);
    }
    // Each time it was the first socket of a ticket that was asked.
    assert!(gateway.world().calls("session.active_list").is_empty());
    assert!(gateway.world().calls("session.resume").is_empty());
    // The very same machine is followed once its Hermes says who it is.
    gateway.world().install = Some(INSTALL.into());
    far.followed();
    far.card();
}

#[test]
#[ignore = "starts processes: run by the_lab_tests_pass"]
fn two_gateways_that_report_different_installs_are_not_covered() {
    let Some(mut far) = far_lab() else {
        return;
    };
    let one = Fake::serving(&far.root, "instance-a", &["default"]);
    let other = Fake::serving(&far.root, "instance-b", &["reports"]);
    one.world().install = Some(INSTALL.into());
    other.world().install = Some(OTHER_INSTALL.into());
    one.asks_quietly("default", "s-one", "p-one");
    other.asks_quietly("reports", "s-other", "p-other");
    far.follow();
    not_covered(&far, "install_ids_disagree");
    std::thread::sleep(Duration::from_millis(600));
    nothing_told(&far);
    for gateway in [&one, &other] {
        assert!(gateway.world().calls("session.active_list").is_empty());
    }
    // Once they agree, the machine is followed and both questions show.
    other.world().install = Some(INSTALL.into());
    far.followed();
    until("both cards show", || far.cards().len() == 2);
}

#[test]
#[ignore = "starts processes: run by the_lab_tests_pass"]
fn a_gateway_whose_origin_is_not_loopback_is_refused_and_no_forward_is_made() {
    let Some(mut far) = far_lab() else {
        return;
    };
    let gateway = far.gateway("instance-1");
    let port = {
        let mut world = gateway.world();
        let identify = world.identify.as_mut().unwrap();
        let origin = identify["result"]["api_origin"]
            .as_str()
            .unwrap()
            .to_owned();
        identify["result"]["api_origin"] = json!(origin.replace("127.0.0.1", "10.1.2.3"));
        origin.rsplit(':').next().unwrap().to_owned()
    };
    gateway.play("question-answered-here", Some("clarify.request"));
    far.follow();
    // The model's own check refuses it: it is left to nothing, and said.
    not_covered(&far, "no_live_connection");
    let lacking = vec![("default".to_owned(), "api_origin_not_local_http")];
    assert_eq!(far.observer().not_followed(), lacking);
    nothing_told(&far);
    let untouched = |far: &FarLab, gateway: &Fake| {
        assert_eq!(far.inner().route.forwards_made(), 0);
        assert_eq!(far.inner().route.steps("prepare"), 0);
        assert!(gateway.world().granted.is_empty(), "no ticket was asked");
        assert_eq!(gateway.world().sockets, 0);
    };
    untouched(&far, &gateway);

    // An origin the model takes for this machine and a forward cannot
    // lead to: the route itself refuses it, before any forward or ticket.
    for origin in [
        format!("http://127.0.0.2:{port}"),
        format!("http://[::1]:{port}"),
    ] {
        gateway.world().identify.as_mut().unwrap()["result"]["api_origin"] = json!(origin);
        until("it is taken up and refused by the route", || {
            far.log()
                .matches("why=Invalid(\"invalid_api_origin\")")
                .count()
                >= 1
                && far.standing() == Standing::Connecting
        });
        std::thread::sleep(Duration::from_millis(300));
        untouched(&far, &gateway);
        nothing_told(&far);
        gateway.world().identify.as_mut().unwrap()["result"]["api_origin"] =
            json!(format!("http://10.1.2.3:{port}"));
        not_covered(&far, "no_live_connection");
        let data = far.data.join("hermes-gateway.log");
        std::fs::write(data, "").unwrap();
    }
}

/// A gateway found after the machine was verified says who it is on its
/// own first socket. One that reports another install means the route
/// leads to no one machine: nothing of that gateway is ever told, and
/// what was told of the machine is withdrawn.
#[test]
#[ignore = "starts processes: run by the_lab_tests_pass"]
fn a_gateway_that_turns_up_as_another_install_is_never_told_and_uncovers_the_machine() {
    let Some(mut far) = far_lab() else {
        return;
    };
    let one = Fake::serving(&far.root, "instance-a", &["default"]);
    one.world().install = Some(INSTALL.into());
    one.asks_quietly("default", "s-one", "p-one");
    far.follow();
    let key = far.followed();
    far.card();

    let other = Fake::serving(&far.root, "instance-b", &["reports"]);
    other.world().install = Some(OTHER_INSTALL.into());
    other.asks_quietly("reports", "s-other", "p-other");
    let its = format!("hermes:{}/reports:s-other", key.as_str());
    let mut shown = false;
    until("the machine is uncovered", || {
        shown |= far
            .cards()
            .iter()
            .any(|card| card["sessionId"] == its.as_str());
        matches!(far.standing(), Standing::NotCovered(_))
    });
    assert!(!shown, "a card of the other install showed");
    assert!(other.world().calls("session.active_list").is_empty());
    until("its cards are withdrawn", || far.cards().is_empty());
    let waiting = far.requests.native_waiting_sessions();
    assert!(
        waiting.values().all(|sessions| sessions.is_empty()),
        "{waiting:?}"
    );
    let log = far.log();
    assert!(log.contains("not covered why=install_id_changed"), "{log}");
    not_covered(&far, "install_ids_disagree");
}
