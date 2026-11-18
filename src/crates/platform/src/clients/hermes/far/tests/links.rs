//! Ticket 62. What the homes command says of an entry under `profiles/`
//! that is no folder it can see.
use super::{
    super::Far,
    fixture::{chmod, private_folder, text, FarLab},
};
use agent_hud_agents::hermes::gateway::{far::FarHome, Profile};
use std::{fs, os::unix::fs::symlink, path::Path};

fn home(profile: Option<&str>, path: &Path) -> FarHome {
    FarHome {
        profile: profile.map_or(Profile::Default, |name| Profile::folder(name).unwrap()),
        path: text(path).to_owned(),
    }
}

/// A profile that is a link into a folder the login cannot search may hold
/// the machine's gateway: what it leads to cannot be told, so it is told
/// by name, as a folder that cannot be entered is. A link whose target is
/// gone, a file and a link to a file are no profile's home, and nothing is
/// said of them.
#[test]
#[ignore = "in the lab: tests::the_lab_tests_pass runs it"]
fn a_link_into_a_folder_that_cannot_be_searched_is_told_and_a_dead_link_is_not() {
    let Some(lab) = FarLab::start() else { return };
    let (root, ops) = (lab.root(), lab.profile("ops"));
    let profiles = root.join("profiles");
    let inner = lab.dir.join("elsewhere").join("inner");
    let work = inner.join("work");
    private_folder(&work);
    symlink(&work, profiles.join("work")).unwrap();
    // Gone where its folder can be searched, not known where it cannot.
    symlink(inner.join("never"), profiles.join("lost")).unwrap();
    symlink(lab.dir.join("nowhere").join("home"), profiles.join("dead")).unwrap();
    fs::write(profiles.join("notes"), b"").unwrap();
    symlink(profiles.join("notes"), profiles.join("filed")).unwrap();
    let far = Far::default();
    let whole = far.homes(&lab.link).expect("the homes");
    let open = vec![
        home(None, &root),
        home(Some("ops"), &ops),
        home(Some("work"), &work),
    ];
    assert_eq!(whole.homes, open);
    assert!(whole.unresolved.is_empty(), "{:?}", whole.unresolved);

    chmod(&inner, 0o000);
    let found = far.homes(&lab.link);
    chmod(&inner, 0o700);
    let found = found.expect("the homes");
    let named = ["lost", "work"].map(|name| Profile::folder(name).unwrap());
    assert_eq!(found.unresolved, named, "told by name, not dropped");
    assert_eq!(found.homes, open[..2]);
    // Searched again, it is a home as before.
    assert_eq!(far.homes(&lab.link), Ok(whole));
}
