//! A far gateway's control socket: what can be asked, what the relay's
//! words mean, and how one round of answers becomes the machine's gateways.
use crate::hermes::gateway::{
    far::{
        homes, picture, relay_said, Asked, FarHome, Homes, NoHomes, Quiet, Request, Unchecked,
        HOMES_SCRIPT, MOST_ANSWER,
    },
    identify, identify_line, ticket_line, Failure, Identified, Profile, Scope,
};
use serde_json::{json, Value};

fn fixture() -> Value {
    let text =
        include_str!("../../../../tests/fixtures/hermes/gateway/gateway-self-description.json");
    serde_json::from_str(text).unwrap()
}

/// What a gateway `instance` serving `served` answers to `identify`.
fn identified(instance: &str, served: &[&str]) -> Value {
    let mut answer = fixture()["control_socket"]["identify_throwaway_ready_two_profiles"].clone();
    let homes: Vec<Value> = served
        .iter()
        .map(|home| json!({"profile_id": home, "home": home}))
        .collect();
    answer["result"]["instance_id"] = json!(instance);
    answer["result"]["served_profiles"] = json!(homes);
    answer
}

/// The relay's words for an answer.
fn ok(answer: &Value) -> Vec<u8> {
    format!("ok\n{answer}\n").into_bytes()
}

fn far_home(profile: Option<&str>, path: &str) -> FarHome {
    FarHome {
        profile: profile.map_or(Profile::Default, |name| Profile::folder(name).unwrap()),
        path: path.to_owned(),
    }
}

const ROOT: &str = "/Users/far/.hermes";
const OPS: &str = "/Users/far/.hermes/profiles/ops";
const REPORTS: &str = "/Users/far/.hermes/profiles/reports";

fn three() -> Homes {
    Homes {
        root: ROOT.to_owned(),
        homes: vec![
            far_home(None, ROOT),
            far_home(Some("ops"), OPS),
            far_home(Some("reports"), REPORTS),
        ],
    }
}

#[test]
fn only_identify_and_session_ticket_can_be_asked() {
    // The two requests are all the type has, and each is the line Hermes's
    // own client sends.
    let said = identify(&identified("one", &[ROOT, OPS]), OPS);
    let Identified::Ready(instance) = said else {
        panic!("{said:?}");
    };
    let line = |request: Request| serde_json::from_str::<Value>(&request.line()).unwrap();
    assert_eq!(line(Request::Identify), identify_line());
    for scope in [Scope::Profile, Scope::Host] {
        let request = Request::Ticket {
            profile_id: &instance.profile_id,
            instance_id: &instance.id,
            scope,
        };
        assert_eq!(line(request), ticket_line(&instance, scope));
        assert_eq!(line(request)["verb"], "session-ticket");
    }
    // A line is a request only where it is one of the two, exactly.
    assert_eq!(Request::of(&identify_line()), Some(Request::Identify));
    for scope in [Scope::Profile, Scope::Host] {
        let line = ticket_line(&instance, scope);
        let request = Request::of(&line).expect("a ticket request");
        assert_eq!(
            serde_json::from_str::<Value>(&request.line()).unwrap(),
            line
        );
    }
    // Every other verb the socket takes, as the gateway lists them.
    let listed = fixture()["control_socket"]["unknown_verb"]["supported_verbs"].clone();
    let listed: Vec<String> = serde_json::from_value(listed).unwrap();
    assert!(listed.len() >= 9 && listed.iter().any(|verb| verb == "pause-for-update"));
    for verb in listed.iter().filter(|verb| *verb != "identify") {
        let line = json!({"protocol": 1, "id": 1, "verb": verb});
        assert_eq!(Request::of(&line), None, "{verb}");
        let mut ticket = ticket_line(&instance, Scope::Host);
        ticket["verb"] = json!(verb);
        assert_eq!(Request::of(&ticket), None, "{verb} with a ticket's fields");
    }
    // One of the two with anything more in it, or less, is no request.
    let mut more = identify_line();
    more["params"] = json!({"name": "ops"});
    assert_eq!(Request::of(&more), None);
    let mut subject = ticket_line(&instance, Scope::Host);
    subject["params"]["subject"] = json!("auth:v1:[\"a\",\"b\",\"c\"]");
    assert_eq!(Request::of(&subject), None);
    let mut scope = ticket_line(&instance, Scope::Host);
    scope["params"]["scope"] = json!("everything");
    assert_eq!(Request::of(&scope), None);
    let mut purpose = ticket_line(&instance, Scope::Profile);
    purpose["params"]["purpose"] = json!("admin");
    assert_eq!(Request::of(&purpose), None);
    for line in [json!({}), json!([]), json!("identify"), json!({"verb": 1})] {
        assert_eq!(Request::of(&line), None, "{line}");
    }
    // Whatever is put in a ticket request's fields, the verb is its own.
    let odd = Request::Ticket {
        profile_id: "\",\"verb\":\"pause-for-update",
        instance_id: "x\n{\"verb\":\"unserve-profile\"}",
        scope: Scope::Host,
    };
    assert_eq!(line(odd)["verb"], "session-ticket");
    assert!(!odd.line().contains('\n'), "one line");
}

#[test]
fn the_relays_words_are_an_answer_or_a_named_reason() {
    let answer = json!({"ok": true, "result": {"ticket": "kept-out-of-print"}});
    let said = relay_said(0, &ok(&answer)).unwrap();
    assert_eq!(said.value(), &answer);
    // An answer never prints: it can hold a ticket.
    assert_eq!(format!("{said:?}"), "Answered(<masked>)");
    let quiet = |exit: i32, output: &str| relay_said(exit, output.as_bytes()).unwrap_err();
    for (word, why) in [
        ("no-home", Quiet::Absent),
        ("no-socket", Quiet::Absent),
        ("nobody-listening", Quiet::Absent),
        ("empty-answer", Quiet::Empty),
        ("answer-too-long", Quiet::TooLong),
        ("answer-cut-short", Quiet::Garbled),
        ("failed", Quiet::Failed),
        ("usage", Quiet::Failed),
        ("a word the relay never says", Quiet::Failed),
        ("", Quiet::Failed),
    ] {
        assert_eq!(quiet(2, &format!("{word}\n")), why, "{word}");
    }
    // Each check has its own name, which is the word the relay says.
    let checks = [
        Unchecked::HomeNotCanonical,
        Unchecked::HomeNotYours,
        Unchecked::HomeOpenToOthers,
        Unchecked::SocketNotYours,
        Unchecked::SocketOpenToOthers,
        Unchecked::PointerIsALink,
        Unchecked::PointerNotYours,
        Unchecked::PointerOpenToOthers,
        Unchecked::PointerTooLong,
        Unchecked::PointerOutside,
        Unchecked::FolderNotYours,
        Unchecked::FolderOpenToOthers,
        Unchecked::NoPeerCredentials,
        Unchecked::ListenerNotYours,
        Unchecked::VerbNotSent,
        Unchecked::PermissionsUnread,
    ];
    let mut names: Vec<&str> = checks.iter().map(|why| why.reason_code()).collect();
    for why in checks {
        let said = quiet(2, &format!("{}\n", why.reason_code()));
        assert_eq!(said, Quiet::Unchecked(why));
    }
    names.push(Unchecked::NoPython.reason_code());
    names.sort_unstable();
    names.dedup();
    assert_eq!(names.len(), 17, "seventeen reasons, seventeen names");
    // No `python3`, as `sh` says it; the deadline, which says nothing.
    assert_eq!(quiet(127, ""), Quiet::Unchecked(Unchecked::NoPython));
    assert_eq!(quiet(126, ""), Quiet::Unchecked(Unchecked::NoPython));
    assert_eq!(quiet(3, ""), Quiet::TimedOut);
    assert_eq!(quiet(3, "ok\n{}\n"), Quiet::TimedOut);
    // `ok` and then something that is not one JSON object on one line.
    for rest in [
        "",
        "{}",
        "\n",
        "[1]\n",
        "\"text\"\n",
        "{}\n{}\n",
        "{}\n\n",
        "{\n",
    ] {
        assert_eq!(quiet(0, &format!("ok\n{rest}")), Quiet::Garbled, "{rest:?}");
    }
    assert_eq!(quiet(1, "ok\n{}\n"), Quiet::Failed);
    let long = format!("ok\n{{\"a\": \"{}\"}}\n", "x".repeat(MOST_ANSWER));
    assert_eq!(quiet(0, &long), Quiet::TooLong);
}

#[test]
fn the_homes_command_names_the_root_and_each_profile() {
    let said = |words: &[&str]| {
        let mut text = words.join("\0");
        text.push('\0');
        homes(text.as_bytes())
    };
    let found = said(&[
        "root", ROOT, "profile", "reports", REPORTS, "profile", "ops", OPS, "python", "yes", "end",
    ]);
    assert_eq!(found, Ok(three()), "the root first, then profiles by name");
    // A folder that is no profile's names no home, nor does a home that is
    // not a canonical path. A profile behind a link is where it leads.
    let found = said(&[
        "root",
        ROOT,
        "profile",
        "Not A Profile",
        "/Users/far/.hermes/profiles/Not A Profile",
        "profile",
        "default",
        "/Users/far/.hermes/profiles/default",
        "profile",
        "ops",
        "relative/ops",
        "profile",
        "linked",
        "/Volumes/big/linked",
        "profile",
        "again",
        "/Volumes/big/linked",
        "python",
        "yes",
        "end",
    ])
    .unwrap();
    let linked = far_home(Some("again"), "/Volumes/big/linked");
    assert_eq!(found.homes, vec![far_home(None, ROOT), linked]);
    assert_eq!(said(&["none"]), Err(NoHomes::NoHermes));
    assert_eq!(
        said(&["root", ROOT, "python", "no", "end"]),
        Err(NoHomes::NoPython)
    );
    for root in [
        "relative/.hermes",
        "/Users/far/.hermes/",
        "/Users/far/../far/.hermes",
        "",
    ] {
        let found = said(&["root", root, "python", "yes", "end"]);
        assert_eq!(found, Err(NoHomes::RootNotCanonical), "{root}");
    }
    // Cut short, or not the command's words at all.
    for words in [
        &["root", ROOT][..],
        &["root", ROOT, "python", "yes"],
        &["root", ROOT, "profile", "ops"],
        &["root", ROOT, "python", "yes", "end", "more"],
        &["zsh: command not found"],
        &[],
    ] {
        assert_eq!(said(words), Err(NoHomes::Garbled), "{words:?}");
    }
    assert_eq!(
        homes(b"root\0/Users/far/.hermes\0python\0yes\0end"),
        Err(NoHomes::Garbled)
    );
    assert_eq!(homes(&[0xff, 0x00]), Err(NoHomes::Garbled));
    // The command is short enough to send, has no here-document, and names
    // nothing but what it reads.
    assert!(HOMES_SCRIPT.len() < 2048 && !HOMES_SCRIPT.contains("<<"));
    assert!(HOMES_SCRIPT.contains("${HERMES_HOME:-$HOME/.hermes}"));
    for word in [" rm ", "mkdir", "touch", ">>", "hermes "] {
        assert!(!HOMES_SCRIPT.contains(word), "{word}");
    }
}

/// What the homes command prints for a root and `profiles`, each a folder
/// name and where it leads.
fn printed(profiles: &[(String, String)]) -> Vec<u8> {
    let mut text = format!("root\0{ROOT}\0");
    for (name, path) in profiles {
        text.push_str(&format!("profile\0{name}\0{path}\0"));
    }
    text.push_str("python\0yes\0end\0");
    text.into_bytes()
}

/// `count` profiles, each in its own folder under the root.
fn profiles(count: usize) -> Vec<(String, String)> {
    (0..count)
        .map(|at| (format!("p{at:03}"), format!("{ROOT}/profiles/p{at:03}")))
        .collect()
}

/// The verifier's probe of the parser: an inventory is answered whole or
/// refused, never cut to a part of it and called an answer.
#[test]
fn v_a_large_inventory_is_whole_or_refused_never_part_of_it() {
    // As many as are asked: every one is there.
    let most = homes(&printed(&profiles(200))).expect("200 profiles");
    assert_eq!(most.homes.len(), 201);
    assert_eq!(most.homes[200].path, format!("{ROOT}/profiles/p199"));
    // One more: no homes at all, and the reason says why.
    let over = homes(&printed(&profiles(201)));
    assert!(
        over.as_ref().map_or(true, |found| found.homes.len() == 202),
        "a successful answer holds every home: {} of 202",
        over.as_ref().map_or(0, |found| found.homes.len())
    );
    assert_eq!(format!("{over:?}"), "Err(TooMany)");
    assert_eq!(
        format!("{:?}", homes(&printed(&profiles(5000)))),
        "Err(TooMany)"
    );
    // Folders that lead to one home count once, before anything is
    // counted: 250 names for one home do not crowd out five others.
    let mut aliased: Vec<(String, String)> = (0..250)
        .map(|at| (format!("a{at:03}"), "/Volumes/big/shared".to_owned()))
        .collect();
    aliased.extend(profiles(5));
    let found = homes(&printed(&aliased)).expect("six homes under 255 names");
    let paths: Vec<&str> = found.homes.iter().map(|home| home.path.as_str()).collect();
    assert_eq!(paths.len(), 7, "{paths:?}");
    assert_eq!(paths[1], "/Volumes/big/shared");
    assert_eq!(paths[6], format!("{ROOT}/profiles/p004"));
    // A profile folder that leads back to the root is the root.
    let back = homes(&printed(&[("loop".to_owned(), ROOT.to_owned())])).unwrap();
    assert_eq!(back.homes.len(), 1);
}

/// Every order the three homes can be asked in.
fn orders() -> Vec<[usize; 3]> {
    vec![
        [0, 1, 2],
        [0, 2, 1],
        [1, 0, 2],
        [1, 2, 0],
        [2, 0, 1],
        [2, 1, 0],
    ]
}

fn pictured(answers: &[Result<Value, &'static str>; 3], order: [usize; 3]) -> Pictured {
    let homes = three();
    let asked = order
        .iter()
        .map(|at| {
            let said = answers[*at]
                .clone()
                .map(|answer| relay_said(0, &ok(&answer)).unwrap());
            (homes.homes[*at].clone(), said)
        })
        .collect();
    picture(&homes, asked)
}

type Pictured = crate::hermes::gateway::far::Picture<&'static str>;

#[test]
fn one_gateway_for_every_profile_and_one_for_each_are_both_found_in_any_order() {
    // The MacBook's layout: the root's socket answers for every profile.
    let all = [
        Ok(identified("all", &[ROOT, OPS, REPORTS])),
        Err("absent"),
        Err("absent"),
    ];
    // The Mini's: each home's socket answers for itself.
    let each = [
        Ok(identified("for-default", &[ROOT])),
        Ok(identified("for-ops", &[OPS])),
        Ok(identified("for-reports", &[REPORTS])),
    ];
    for answers in [&all, &each] {
        let first = pictured(answers, [0, 1, 2]);
        for order in orders() {
            assert_eq!(pictured(answers, order), first, "{order:?}");
        }
    }
    let one = pictured(&all, [2, 1, 0]);
    assert_eq!(one.root, ROOT);
    assert_eq!(one.gateways.len(), 1);
    assert_eq!(one.gateways[0].served, three().homes);
    assert_eq!(one.gateways[0].sockets.len(), 1);
    assert_eq!(
        one.asked[1],
        (three().homes[1].clone(), Asked::Quiet("absent"))
    );
    let many = pictured(&each, [1, 2, 0]);
    let ids: Vec<&str> = many
        .gateways
        .iter()
        .map(|g| g.instance_id.as_str())
        .collect();
    assert_eq!(ids, ["for-default", "for-ops", "for-reports"]);
    for (gateway, home) in many.gateways.iter().zip(three().homes) {
        assert_eq!(gateway.served, vec![home.clone()]);
        // The ticket for a gateway is asked with its own home's profile id.
        assert_eq!(gateway.sockets[0].1.profile_id, home.path);
    }
    assert!(one.contested.is_empty() && many.contested.is_empty());
}

#[test]
fn served_homes_are_far_paths_among_the_homes_asked() {
    // A gateway that also names a home of another machine's shape, and one
    // written another way, serves neither by it.
    let answers = [
        Ok(identified(
            "one",
            &[
                ROOT,
                "/Users/nickymolina/.hermes/profiles/ops",
                "/Users/far/.hermes/profiles/ops/",
            ],
        )),
        // It answers, and does not serve the home that was asked.
        Ok(identified("two", &[REPORTS])),
        // Two instances both say they serve one profile: a restart overlapping.
        Ok(identified("three", &[REPORTS])),
    ];
    for order in orders() {
        let found = pictured(&answers, order);
        let ids: Vec<&str> = found
            .gateways
            .iter()
            .map(|g| g.instance_id.as_str())
            .collect();
        assert_eq!(ids, ["one", "three"], "{order:?}");
        assert_eq!(found.gateways[0].served, vec![far_home(None, ROOT)]);
        assert_eq!(
            found.gateways[1].served,
            vec![far_home(Some("reports"), REPORTS)]
        );
        let not_served = Asked::Answered(Identified::Failed(Failure::ProfileNotServed));
        assert_eq!(found.asked[1].1, not_served);
        assert!(found.contested.is_empty());
    }
    // A home no one asked is not counted as any home that was.
    let stray = [
        Err("absent"),
        Ok(identified(
            "two",
            &[
                OPS,
                "/Users/elsewhere/.hermes",
                "/Users/far/.hermes/profiles",
            ],
        )),
        Err("absent"),
    ];
    for order in orders() {
        let found = pictured(&stray, order);
        assert_eq!(found.gateways.len(), 1, "{order:?}");
        assert_eq!(found.gateways[0].served, vec![far_home(Some("ops"), OPS)]);
    }
    let overlap = [
        Ok(identified("old", &[ROOT, REPORTS])),
        Err("absent"),
        Ok(identified("new", &[REPORTS])),
    ];
    for order in orders() {
        let found = pictured(&overlap, order);
        assert_eq!(
            found.contested,
            vec![far_home(Some("reports"), REPORTS)],
            "{order:?}"
        );
    }
}
