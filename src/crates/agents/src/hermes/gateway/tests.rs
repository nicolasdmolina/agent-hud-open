//! The gateway model against what Hermes 0.22.5 really sent. The fixtures in
//! `tests/fixtures/hermes/gateway/` are the recordings of ticket 11's spike
//! (its README says how each was made); every one of them is in a table
//! here, and `every_recorded_fixture_is_tested` fails for one that is not.
use super::*;
use serde_json::Value;
use std::path::PathBuf;

mod decisions;
mod handshake;
mod regressions;
mod replay;
mod scenarios;
mod transitions;

fn folder() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join("hermes")
        .join("gateway")
}

fn fixture(name: &str) -> Value {
    let text = std::fs::read_to_string(folder().join(format!("{name}.json")))
        .unwrap_or_else(|error| panic!("{name}: {error}"));
    serde_json::from_str(&text).unwrap_or_else(|error| panic!("{name}: {error}"))
}

fn local() -> Gateway {
    Gateway::new(GatewayId::new("local"))
}

/// The throwaway profile every scenario was recorded in.
fn spike() -> Profile {
    Profile::named("hud-spike")
}

/// The fixtures with no frames to replay, each read by its own test.
const READ_WHOLE: &[&str] = &[
    "gateway-self-description",
    "gateway-self-description-old-build.SYNTHETIC",
    "ticket-lifetime",
    "real-gateway-readonly-aggregate",
    "hook-payloads-same-session",
];

#[test]
fn every_recorded_fixture_is_tested() {
    let mut recorded: Vec<String> = std::fs::read_dir(folder())
        .unwrap()
        .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
        .filter_map(|name| name.strip_suffix(".json").map(str::to_owned))
        .collect();
    recorded.sort();
    let mut tested: Vec<String> = scenarios::SCENARIOS
        .iter()
        .map(|(name, _)| *name)
        .chain(READ_WHOLE.iter().copied())
        .map(str::to_owned)
        .collect();
    tested.sort();
    assert_eq!(recorded, tested);
    assert_eq!(recorded.len(), 37);
}

/// None of the copied recordings names a real person, home or credential.
#[test]
fn the_fixtures_carry_nothing_personal() {
    for entry in std::fs::read_dir(folder()).unwrap() {
        let path = entry.unwrap().path();
        let text = std::fs::read_to_string(&path).unwrap();
        for (at, _) in text.match_indices("/Users/") {
            assert!(text[at..].starts_with("/Users/USER"), "{path:?}");
        }
        for key in ["\"ticket\": \"", "\"install_id\": \""] {
            for (at, _) in text.match_indices(key) {
                assert!(text[at + key.len()..].starts_with("<masked>"), "{path:?}");
            }
        }
        for (at, _) in text.match_indices("uid:") {
            assert!(text[at..].starts_with("uid:1000"), "{path:?}");
        }
        assert!(!text.contains("\"tools\": \""), "{path:?}");
    }
}
