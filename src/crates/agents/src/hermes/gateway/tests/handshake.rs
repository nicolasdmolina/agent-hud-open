//! The fixtures about reaching a gateway: what it says of itself, the
//! ticket, an old build, and the same session as the hooks name it.
use super::super::*;
use super::{fixture, local, spike};
use serde_json::{json, Value};

const HOME: &str = "/Users/USER/.hermes";

/// A recorded `identify` answer as the socket gave it. The restart
/// recordings kept a few of its fields; the profiles it serves are the two
/// the same gateway listed in `gateway-self-description`.
fn whole(recorded: &Value) -> Value {
    let mut answer = recorded.clone();
    if answer.get("served_profiles").is_none() {
        let full = fixture("gateway-self-description");
        let listed = &full["control_socket"]["identify_throwaway_ready_two_profiles"];
        answer["served_profiles"] = listed["result"]["served_profiles"].clone();
    }
    answer
}

/// What the connection learns before a recording's first socket opens.
pub fn before_first_socket(doc: &Value, apply: &mut dyn FnMut(Input<'_>)) {
    if let Some(before) = doc["notes"].get("identify_before") {
        apply(Input::Identified {
            answer: &whole(before),
            home: HOME,
        });
    }
}

/// What it learns between one socket closing and the next opening.
pub fn between_sockets(doc: &Value, apply: &mut dyn FnMut(Input<'_>, Option<String>)) {
    let notes = &doc["notes"];
    if let Some(code) = notes["hud_close_info"]["close_code"].as_u64() {
        apply(
            Input::Lost(Failure::Closed(code as u16)),
            Some(format!("-- socket closed {code}")),
        );
    }
    for step in notes["identify_states_during_start"]
        .as_array()
        .into_iter()
        .flatten()
    {
        let label = format!("-- identify: {}", step[0].as_str().unwrap());
        if step[1].get("_error").is_some() {
            apply(Input::Lost(Failure::Unreachable), Some(label));
        } else {
            let answer = whole(&step[1]);
            apply(
                Input::Identified {
                    answer: &answer,
                    home: HOME,
                },
                Some(label),
            );
        }
    }
}

#[test]
fn a_gateway_describes_itself_and_grants_a_ticket() {
    let doc = fixture("gateway-self-description");
    let control = &doc["control_socket"];
    assert_eq!(identify_line(), control["identify_request"]);

    // Nicky's own gateway, read once: one profile, the default one.
    let Identified::Ready(live) = identify(&control["identify_live_macbook_gateway_ready"], HOME)
    else {
        panic!("the live gateway is ready");
    };
    assert_eq!(
        (live.port, live.authority_epoch, live.draining),
        (57825, 5, false)
    );
    assert_eq!(live.origin, "http://127.0.0.1:57825");
    assert_eq!(live.version.as_deref(), Some("0.22.5"));
    assert_eq!(
        live.served,
        [Served {
            profile: Profile::Default,
            profile_id: HOME.into()
        }]
    );

    // One gateway, two profiles: each is listed, and the ticket is asked
    // for the home whose socket answered.
    let two = &control["identify_throwaway_ready_two_profiles"];
    let Identified::Ready(instance) = identify(two, HOME) else {
        panic!("the throwaway gateway is ready");
    };
    let profiles: Vec<&Profile> = instance.served.iter().map(|s| &s.profile).collect();
    assert_eq!(profiles, [&Profile::Default, &spike()]);
    assert_eq!(instance.profile_id, HOME);
    let Identified::Ready(named) = identify(two, "/Users/USER/.hermes/profiles/hud-spike/") else {
        panic!("a profile's own home is served too");
    };
    assert_eq!(named.profile_id, "/Users/USER/.hermes/profiles/hud-spike");
    assert_eq!(
        identify(two, "/Users/USER/elsewhere"),
        Identified::Failed(Failure::ProfileNotServed)
    );

    // Still starting: no capabilities yet. Wait; never call it unsupported.
    assert_eq!(
        identify(&control["identify_throwaway_starting"], HOME),
        Identified::Wait("starting".into())
    );

    // The ticket request, for one profile and for the whole host.
    let asked = Instance {
        id: "<identify.instance_id>".into(),
        profile_id: "<served_profiles[].profile_id>".into(),
        ..instance.clone()
    };
    let mut expected = control["session_ticket_request"].clone();
    expected["params"]["scope"] = json!("host");
    assert_eq!(ticket_line(&asked, Scope::Host), expected);
    expected["params"].as_object_mut().unwrap().remove("scope");
    assert_eq!(ticket_line(&asked, Scope::Profile), expected);

    // The grant: the ticket rides the second subprotocol and never prints.
    let grant = &control["session_ticket_grant"];
    let granted_by = Instance {
        id: grant["instance_id"].as_str().unwrap().into(),
        ..instance.clone()
    };
    let ticket = granted(grant, &granted_by).unwrap();
    assert_eq!(format!("{ticket:?}"), "Ticket(<masked>)");
    let offered = &doc["websocket"]["subprotocols_offered"];
    assert_eq!(ticket.subprotocols()[0], offered[0].as_str().unwrap());
    assert_eq!(ticket.subprotocols()[0], SUBPROTOCOL);
    assert_eq!(doc["websocket"]["subprotocol_echoed"], SUBPROTOCOL);
    let second = offered[1].as_str().unwrap().replace("<ticket>", "<masked>");
    assert_eq!(ticket.subprotocols()[1], second);
    assert!(doc["websocket"]["url"]
        .as_str()
        .unwrap()
        .ends_with(SOCKET_PATH));
    // A grant from another instance is a restart in between.
    assert_eq!(
        granted(grant, &instance),
        Err(Denied::Failed(Failure::StaleInstance))
    );

    // The refusals.
    let refusal = |name: &str| refused(&control[name]).unwrap();
    assert_eq!(
        refusal("refused_stale_instance"),
        Denied::Failed(Failure::StaleInstance)
    );
    assert_eq!(
        refusal("refused_unserved_profile"),
        Denied::Failed(Failure::ProfileNotServed)
    );
    assert_eq!(
        refusal("unknown_verb"),
        Denied::Unsupported(Unsupported::TicketVerb)
    );
    assert_eq!(refused(two), None);

    // The first frame on every socket opens no card and asks for nothing.
    let ready = &doc["websocket"]["first_frame_gateway_ready"];
    assert!(matches!(
        frame(ready),
        Frame::Event(Event {
            kind: "gateway.ready",
            session: "",
            ..
        })
    ));
    let mut gateway = local();
    assert_eq!(
        gateway.apply(
            Input::Frame {
                profile: &spike(),
                frame: ready
            },
            0
        ),
        []
    );
}

#[test]
fn a_pointer_file_names_only_a_socket_hermes_would_make() {
    assert_eq!(
        (CONTROL_SOCKET, CONTROL_POINTER),
        ("gateway.sock", "gateway.sock.path")
    );
    let made = "/var/folders/x/T/hermes-gw-0a1b2c3d/control.sock";
    assert_eq!(pointed_socket(&format!("{made}\n")), Some(made));
    for other in [
        "",
        "hermes-gw-0a1b2c3d/control.sock",
        "/tmp/hermes-gw-0a1b2c3d/other.sock",
        "/tmp/elsewhere/control.sock",
        "/tmp/hermes-gw-/control.sock",
        "/control.sock",
    ] {
        assert_eq!(pointed_socket(other), None, "{other}");
    }
    // The channel is declared for the OSes whose control socket this is.
    let channel = &crate::hermes::GATEWAY;
    assert_eq!(channel.carries, [crate::Carries::Requests]);
    assert!(channel.runs_on(crate::Os::MacOs) && channel.runs_on(crate::Os::Linux));
    assert!(!channel.runs_on(crate::Os::Windows));
    assert!(std::ptr::eq(crate::hermes::AGENT.channel.unwrap(), channel));
}

#[test]
fn an_old_build_is_unsupported_and_a_starting_one_is_waited_for() {
    let doc = fixture("gateway-self-description-old-build.SYNTHETIC");
    assert!(doc["SYNTHETIC"]
        .as_str()
        .unwrap()
        .starts_with("NOT captured"));
    let cases = doc["cases"].as_array().unwrap();
    assert_eq!(cases.len(), 5);
    for case in cases {
        let verdict = &case["verdict"];
        let expected = match (
            verdict["state"].as_str().unwrap(),
            verdict["reason_code"].as_str(),
        ) {
            ("incompatible", Some("runtime_protocol")) => {
                Identified::Unsupported(Unsupported::RuntimeProtocol)
            }
            ("incompatible", Some("session_authority_unavailable")) => {
                Identified::Unsupported(Unsupported::SessionAuthority)
            }
            ("starting", None) => Identified::Wait("starting".into()),
            ("inaccessible", Some("profile_mismatch")) => {
                Identified::Failed(Failure::ProfileNotServed)
            }
            other => panic!("a verdict the test does not know: {other:?}"),
        };
        let told = identify(&case["identify_result"], HOME);
        assert_eq!(told, expected, "{}", case["why"]);
        if let Identified::Unsupported(why) = told {
            assert_eq!(why.reason_code(), verdict["reason_code"]);
        }
    }
    // An unsupported gateway is a state of its own: the hooks carry on alone.
    let mut gateway = local();
    let effects = gateway.apply(
        Input::Identified {
            answer: &cases[0]["identify_result"],
            home: HOME,
        },
        0,
    );
    let unsupported = Connection::Unsupported(Unsupported::RuntimeProtocol);
    assert_eq!(effects, [Effect::Connection(unsupported.clone())]);
    assert_eq!(*gateway.connection(), unsupported);
}

#[test]
fn a_ticket_is_refused_for_another_instance_or_profile() {
    let doc = fixture("ticket-lifetime");
    let notes = &doc["notes"];
    assert_eq!(
        refused(&notes["control_wrong_instance"]),
        Some(Denied::Failed(Failure::StaleInstance))
    );
    assert_eq!(
        refused(&notes["control_unserved_profile"]),
        Some(Denied::Failed(Failure::ProfileNotServed))
    );
    assert_eq!(
        refused(&notes["control_unknown_verb"]),
        Some(Denied::Unsupported(Unsupported::TicketVerb))
    );
    // A ticket used twice, used late, made up or missing: one refusal at
    // the upgrade for all four, which is `Failure::TicketRefused`.
    for used in [
        "second_use_same_ticket",
        "use_after_31s",
        "garbage_ticket",
        "no_ticket",
    ] {
        assert_eq!(notes[used]["opened"], false);
        assert!(notes[used]["detail"]
            .as_str()
            .unwrap()
            .ends_with("HTTP 403"));
    }
    assert_eq!(notes["grant_fields"]["expires_in_seconds"], 30);
    // An unknown ticket verb makes the gateway unsupported; the other
    // refusals are worth another try from identify.
    let mut old = local();
    let denied = refused(&notes["control_unknown_verb"]).unwrap();
    assert_eq!(
        old.apply(Input::Denied(denied), 0),
        [Effect::Connection(Connection::Unsupported(
            Unsupported::TicketVerb
        ))]
    );
    let mut stale = local();
    let denied = refused(&notes["control_wrong_instance"]).unwrap();
    assert_eq!(
        stale.apply(Input::Denied(denied), 0),
        [Effect::Connection(Connection::Failed(
            Failure::StaleInstance
        ))]
    );
    // A lost connection keeps the reason, to try again from identify.
    let mut gateway = local();
    let effects = gateway.apply(Input::Lost(Failure::TicketRefused), 0);
    assert_eq!(
        effects,
        [Effect::Connection(Connection::Failed(
            Failure::TicketRefused
        ))]
    );
}

#[test]
fn the_live_gateway_is_one_the_model_supports() {
    let doc = fixture("real-gateway-readonly-aggregate");
    let result = &doc["result"];
    // Counts only were kept; the one profile it serves is the default one.
    let mut told = result["identify"].clone();
    assert_eq!(told["served_profiles_count"], 1);
    told["served_profiles"] = json!([{"profile_id": HOME, "home": HOME}]);
    told["instance_id"] = json!("an-instance");
    told["authority_epoch"] = json!(5);
    told["api_origin"] = json!("http://127.0.0.1:57825");
    assert!(matches!(identify(&told, HOME), Identified::Ready(_)));
    // Every status and row field it reported is one the model reads.
    for status in result["session_active_list"]["by_status"]
        .as_object()
        .unwrap()
        .keys()
    {
        assert!(Status::parse(status).is_some(), "{status}");
    }
    let keys = result["session_list_live"]["row_keys"].as_array().unwrap();
    assert!(keys.contains(&json!("id")) && keys.contains(&json!("session_id")));
    // Unsubscribed, nothing about a session arrives on its own.
    assert_eq!(result["unsolicited_events_with_session_id"], 0);
}

#[test]
fn the_hooks_and_the_gateway_name_the_same_session() {
    let hooks = fixture("hook-payloads-same-session");
    let frames = fixture("session-ids-and-hooks");
    let raw = frames["notes"]["gateway_session_id"].as_str().unwrap();
    assert_eq!(frames["notes"]["state_db_row"]["id"], raw);
    let named = local().session(&spike(), raw).hud_id();
    assert_eq!(named, format!("hermes:hud-spike:{raw}"));
    let mut seen = 0;
    for recorded in hooks["payloads"].as_array().unwrap() {
        let mut payload = recorded["payload"].clone();
        let home = recorded["hermes_home"].as_str().map(str::to_owned);
        crate::hooks::add_environment("hermes", &mut payload, &|name| {
            home.clone().filter(|_| name == "HERMES_HOME")
        });
        assert_eq!(Profile::of_home(home.as_deref().unwrap()), spike());
        // Whichever session a hook is about, the gateway's id for it names it the same.
        let of = local().session(&spike(), payload["session_id"].as_str().unwrap());
        let event = crate::parse_event("hermes", "--completion-hook", &payload, 0).unwrap();
        if let Some(event) = event {
            assert_eq!(event.session_id, of.hud_id());
            seen += usize::from(event.session_id == named);
        }
    }
    assert_eq!(seen, 8);
    // The default profile's sessions carry no profile, as its state.db reader names them.
    assert_eq!(
        local().session(&Profile::Default, "abc").hud_id(),
        "hermes:abc"
    );
    assert_eq!(Profile::of_home("/Users/USER/.hermes"), Profile::Default);
    assert_eq!(Profile::named("default"), Profile::Default);
}
