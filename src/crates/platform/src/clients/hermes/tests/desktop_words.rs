//! The words for a far session the Hermes desktop app hosts (ticket 39),
//! in the app's language: the machines list's note and the way back's
//! name. The language is one setting for the whole process, so this test
//! runs alone in a process of its own: nothing else can switch it between
//! the setting and the snapshot.
use super::machines::{
    desktop::{desk, started, NOTE, NOTE_ZH},
    mirrors::far,
};
use crate::ssh_link::unix::tests::lab;
use agent_hud_core::ProviderDiscovery;
use serde_json::json;

#[test]
fn the_lab_test_passes() {
    lab::in_a_process_of_their_own("clients::hermes::tests_unix::desktop_words::in_");
}

#[test]
#[ignore = "starts processes: run by the_lab_test_passes"]
fn in_both_languages_the_note_and_the_way_back_say_where_questions_are_answered() {
    let Some(mut desk) = desk(1) else {
        return;
    };
    started(&desk.dbs[0], "d-desk", "desktop");
    desk.holds(0, &["d-desk"]);
    desk.tick(0);
    desk.listed(0, "d-desk");
    // Where the host can open Hermes's link, it is the way back.
    desk.fleet
        .core
        .set_provider_discovery(vec![ProviderDiscovery {
            id: "hermes".into(),
            navigation_target: Some(json!({"kind": "hermes-app"})),
            ..Default::default()
        }]);
    let id = far(&desk.keys[0], "d-desk");
    for (language, note, label) in [
        ("zh-CN", NOTE_ZH, "在 Hermes 桌面版中打开"),
        ("en", NOTE, "Open in Hermes desktop"),
        ("zh-CN", NOTE_ZH, "在 Hermes 桌面版中打开"),
    ] {
        let setting = json!({"key": "language", "value": language});
        desk.fleet.core.request("set_setting", setting).unwrap();
        desk.notes("Mini", Some(note));
        let snapshot = desk.fleet.snapshot();
        let sessions = snapshot["sessions"].as_array().unwrap();
        let row = sessions.iter().find(|session| session["id"] == id).unwrap();
        assert_eq!(row["navigationTarget"], json!({"kind": "hermes-app"}));
        assert_eq!(row["navigationLabel"], label, "{language}");
        let detail = desk.fleet.core.request("session_detail", json!({"id": id}));
        assert_eq!(
            detail.unwrap()["session"]["navigationLabel"],
            label,
            "{language}"
        );
    }
}
