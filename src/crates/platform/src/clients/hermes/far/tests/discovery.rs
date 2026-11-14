//! Finding the gateways of a far machine, on the two layouts Hermes runs
//! in: one gateway for every profile, and one for each.
use super::{
    super::{Far, NoPicture, NotCovered, Silent},
    fixture::{chmod, private_folder, text, FarLab, Gate, Manner},
};
use crate::ssh_link::unix::tests::lab::SHELL;
use agent_hud_agents::hermes::gateway::{
    far::{Asked, FarHome, Picture, Quiet, Unchecked},
    Failure, Identified, Profile,
};
use std::{fs, os::unix::fs::symlink, path::Path};

fn home(profile: Option<&str>, path: &Path) -> FarHome {
    FarHome {
        profile: profile.map_or(Profile::Default, |name| Profile::folder(name).unwrap()),
        path: text(path).to_owned(),
    }
}

/// The picture with the homes asked in their own order, and again last
/// first: the same either way.
fn discovered(lab: &FarLab) -> Picture<Silent> {
    let far = Far::default();
    let found = far.homes(&lab.link).expect("the homes");
    let forward = far.discover(&lab.link).expect("a picture");
    let backward = far
        .identify_all(&lab.link, &found, found.homes.iter().rev())
        .expect("a picture, asked last first");
    assert_eq!(forward, backward, "whichever home is asked first");
    forward
}

/// The MacBook's layout: one gateway serves every profile, and only the
/// root home has its socket.
#[test]
#[ignore = "in the lab: tests::the_lab_tests_pass runs it"]
fn discovery_finds_one_gateway_serving_every_profile() {
    let Some(lab) = FarLab::start() else { return };
    let (root, ops, reports) = (lab.root(), lab.profile("ops"), lab.profile("reports"));
    // A folder that is no profile's, and a file, name no home.
    private_folder(&root.join("profiles").join("Not A Profile"));
    fs::write(root.join("profiles").join("notes"), b"").unwrap();
    let _gate = lab.gate(&root, Manner::gateway("all", &[&root, &ops, &reports]));
    let picture = discovered(&lab);
    assert_eq!(picture.root, text(&root));
    assert_eq!(picture.gateways.len(), 1);
    let gateway = &picture.gateways[0];
    assert_eq!(gateway.instance_id, "all");
    let all = vec![
        home(None, &root),
        home(Some("ops"), &ops),
        home(Some("reports"), &reports),
    ];
    assert_eq!(gateway.served, all);
    assert_eq!(gateway.sockets.len(), 1);
    assert_eq!(gateway.sockets[0].0, all[0]);
    let asked: Vec<&FarHome> = picture.asked.iter().map(|(home, _)| home).collect();
    assert_eq!(asked, all.iter().collect::<Vec<_>>());
    let absent = Asked::Quiet(Silent::Relay(Quiet::Absent));
    assert_eq!(
        (&picture.asked[1].1, &picture.asked[2].1),
        (&absent, &absent)
    );
    assert!(picture.contested.is_empty());
}

/// The Mini's layout: one gateway for each profile, each with a socket of
/// its own, one of them behind a pointer file.
#[test]
#[ignore = "in the lab: tests::the_lab_tests_pass runs it"]
fn discovery_finds_one_gateway_for_each_profile() {
    let Some(lab) = FarLab::start() else { return };
    let (root, reports, idle) = (lab.root(), lab.profile("reports"), lab.profile("idle"));
    let _default = lab.gate(&root, Manner::gateway("for-default", &[&root]));
    let _reports = lab.gate_by_pointer(&reports, Manner::gateway("for-reports", &[&reports]));
    let picture = discovered(&lab);
    assert_eq!(picture.gateways.len(), 2);
    let (first, second) = (&picture.gateways[0], &picture.gateways[1]);
    assert_eq!(
        (first.instance_id.as_str(), &first.served),
        ("for-default", &vec![home(None, &root)])
    );
    assert_eq!(
        (second.instance_id.as_str(), &second.served),
        ("for-reports", &vec![home(Some("reports"), &reports)])
    );
    // A profile with no gateway is asked and found absent.
    let absent = Asked::Quiet(Silent::Relay(Quiet::Absent));
    assert_eq!(picture.asked[1], (home(Some("idle"), &idle), absent));
    assert!(picture.contested.is_empty());
    // Each ticket is asked through its own home's socket.
    assert_eq!(second.sockets[0].1.profile_id, text(&reports));
}

#[test]
#[ignore = "in the lab: tests::the_lab_tests_pass runs it"]
fn a_restart_that_overlaps_is_shown_as_contested() {
    let Some(lab) = FarLab::start() else { return };
    let (root, ops) = (lab.root(), lab.profile("ops"));
    // The old gateway still says it serves `ops`; the new one does too.
    let _old = lab.gate(&root, Manner::gateway("old", &[&root, &ops]));
    let _new = lab.gate(&ops, Manner::gateway("new", &[&ops]));
    // A gateway that names a home no one asked serves nothing by it.
    let stray = lab.profile("stray");
    let outside = lab.dir.join("far").join("elsewhere");
    let _stray = lab.gate(&stray, Manner::gateway("stray", &[&outside]));
    let picture = discovered(&lab);
    assert_eq!(picture.contested, vec![home(Some("ops"), &ops)]);
    assert_eq!(picture.gateways.len(), 2);
    let not_served = Asked::Answered(Identified::Failed(Failure::ProfileNotServed));
    assert_eq!(picture.asked[2], (home(Some("stray"), &stray), not_served));
}

/// A root named by the far login's own shell, and not the default one.
#[test]
#[ignore = "in the lab: tests::the_lab_tests_pass runs it"]
fn a_hermes_home_named_by_the_login_shell_is_found() {
    let Some(lab) = FarLab::start() else { return };
    // The default root is there too, and is not the one.
    let default = lab.root();
    let _wrong = lab.gate(&default, Manner::gateway("default-root", &[&default]));
    let root = lab.dir.join("far").join("other hermes");
    private_folder(&root);
    let ops = root.join("profiles").join("ops");
    private_folder(&ops);
    // Named through a link, as a home on another disk would be.
    let link = lab.dir.join("far").join("hermes-link");
    symlink(&root, &link).unwrap();
    // The login shell's own startup files say where Hermes lives.
    let shell = lab.lab.at(SHELL);
    let zsh = format!("export HERMES_HOME='{}'\n", text(&link));
    let fish = format!("set -gx HERMES_HOME '{}'\n", text(&link));
    let add = |file: &Path, line: &str| {
        let mut text = fs::read_to_string(file).unwrap();
        text.push_str(line);
        fs::write(file, text).unwrap();
    };
    add(&shell.join(".zshenv"), &zsh);
    add(&shell.join("fish").join("config.fish"), &fish);
    let _gate = lab.gate(&root, Manner::gateway("named", &[&root, &ops]));
    let picture = discovered(&lab);
    // The root is the folder the link leads to, as the far side resolves it.
    assert_eq!(picture.root, text(&root));
    assert_eq!(picture.gateways.len(), 1);
    assert_eq!(picture.gateways[0].instance_id, "named");
    assert_eq!(
        picture.gateways[0].served,
        vec![home(None, &root), home(Some("ops"), &ops)]
    );
}

/// The verifier's probe: 201 profiles, and a gateway only in the last.
/// Discovery returns every home or refuses the machine, and never asks
/// some of the homes as if they were all of them.
#[test]
#[ignore = "in the lab: tests::the_lab_tests_pass runs it"]
fn v_discovery_never_silently_drops_a_valid_home() {
    let Some(lab) = FarLab::start() else { return };
    let root = lab.root();
    let paths: Vec<_> = (0..201).map(|n| lab.profile(&format!("p{n:03}"))).collect();
    let late = paths.last().unwrap();
    let gate = lab.gate(late, Manner::gateway("last", &[late]));
    let far = Far::default();
    match far.homes(&lab.link) {
        Ok(found) => {
            let picture = far.discover(&lab.link).expect("a discovery result");
            println!(
                "Actual folders: root + 201 profiles. Returned homes: {}. Final valid profile \
                 included: {}. Discovery asked {} homes and found {} gateways. Final profile \
                 gateway received {} connections.",
                found.homes.len(),
                found.homes.iter().any(|h| h.path == text(late)),
                picture.asked.len(),
                picture.gateways.len(),
                gate.connections()
            );
            assert_eq!(
                found.homes.len(),
                202,
                "successful discovery must return every home, or explicitly refuse an \
                 oversized inventory"
            );
        }
        Err(reason) => {
            println!("Oversized discovery explicitly refused: {reason:?}");
            assert_eq!(format!("{reason:?}"), "NotCovered(TooManyProfiles)");
            let whole = far.discover(&lab.link).unwrap_err();
            assert_eq!(format!("{whole:?}"), "NotCovered(TooManyProfiles)");
            assert_eq!(gate.connections(), 0, "no home of it was asked");
        }
    }
    assert!(root.exists());
    // With one profile fewer every home is named, the last one too.
    fs::remove_dir(&paths[0]).unwrap();
    let found = far.homes(&lab.link).expect("the homes of 200 profiles");
    assert_eq!(found.homes.len(), 201);
    assert_eq!(
        found.homes.last().map(|home| home.path.as_str()),
        Some(text(late))
    );
}

#[test]
#[ignore = "in the lab: tests::the_lab_tests_pass runs it"]
fn a_machine_where_the_relay_cannot_run_or_a_check_fails_is_not_covered() {
    // No Hermes home at all.
    let Some(lab) = FarLab::start() else { return };
    let far = Far::default();
    let none = far.discover(&lab.link).unwrap_err();
    assert_eq!(none, NoPicture::NotCovered(NotCovered::NoHermes));
    // A profile's home that others may write: the machine says which.
    let (root, ops) = (lab.root(), lab.profile("ops"));
    let gate = lab.gate(&root, Manner::gateway("all", &[&root, &ops]));
    chmod(&ops, 0o777);
    let open = far.discover(&lab.link).unwrap_err();
    let why = NotCovered::Unchecked {
        why: Unchecked::HomeOpenToOthers,
        home: Some(text(&ops).to_owned()),
    };
    assert_eq!(open, NoPicture::NotCovered(why));
    chmod(&ops, 0o700);
    assert!(far.discover(&lab.link).is_ok());
    drop(gate);
    drop(lab);
    // No `python3`: said by the homes command, before any relay is sent.
    let Some(lab) = FarLab::start_with(&[("PATH", "/bin")]) else {
        return;
    };
    let root = lab.root();
    let gate = Gate::serve(
        lab.listen(&root.join("gateway.sock")),
        Manner::gateway("all", &[&root]),
    );
    let none = Far::default().discover(&lab.link).unwrap_err();
    let why = NotCovered::Unchecked {
        why: Unchecked::NoPython,
        home: None,
    };
    assert_eq!(none, NoPicture::NotCovered(why));
    assert_eq!(gate.connections(), 0);
}
