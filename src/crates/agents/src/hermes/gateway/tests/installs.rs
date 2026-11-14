//! What a far gateway is asked for its install's identity, and how its
//! answer is read (ticket 30): the id is taken as reported or not at all.
use super::super::*;
use super::{FAR_HOME, INSTALL};
use crate::hermes::FAR_ROUTE;
use crate::Support;
use serde_json::json;

#[test]
fn the_identity_is_asked_with_profiles_list_and_no_sessions() {
    let named = Profile::named("reports").unwrap();
    let call = profiles_list(&named, Scope::Host);
    assert_eq!(call.method, "profiles.list");
    assert_eq!(
        call.params,
        json!({"include_sessions": false, "profile": "reports"})
    );
    let own = profiles_list(&Profile::Default, Scope::Profile);
    assert_eq!(own.params, json!({"include_sessions": false}));
}

#[test]
fn an_install_id_is_read_as_reported_or_not_at_all() {
    let key = machine_key(Some(INSTALL), FAR_HOME).unwrap();
    let reported = |result: serde_json::Value| {
        let id = reported_install(&result).map(str::to_owned);
        machine_key(id.as_deref(), FAR_HOME)
    };
    assert_eq!(
        reported(json!({"profiles": [], "install_id": INSTALL})),
        Ok(key)
    );
    let cases = [
        (json!({"profiles": []}), NoKey::Missing),
        (json!({"install_id": null}), NoKey::Missing),
        (json!({"install_id": ""}), NoKey::Empty),
        (json!({"install_id": 7}), NoKey::Length),
        (json!({"install_id": [INSTALL]}), NoKey::Length),
        (json!({"install_id": {"id": INSTALL}}), NoKey::Length),
        (json!({"install_id": true}), NoKey::Length),
        (
            json!({"install_id": INSTALL.to_uppercase()}),
            NoKey::UpperCase,
        ),
        (json!({"install_id": format!(" {INSTALL}")}), NoKey::Length),
        (json!({"install_id": &INSTALL[..31]}), NoKey::Length),
        (
            json!({"install_id": INSTALL.replace('a', "g")}),
            NoKey::NotHex,
        ),
    ];
    for (result, why) in cases {
        assert_eq!(reported(result.clone()), Err(why), "{result}");
    }
    // Nothing else in the result stands in for it.
    let elsewhere = json!({"result": {"install_id": INSTALL}, "id": INSTALL});
    assert_eq!(reported(elsewhere), Err(NoKey::Missing));
}

#[test]
fn the_far_route_is_declared_for_each_system() {
    assert!(matches!(FAR_ROUTE.macos, Support::Verified(_)));
    assert!(matches!(FAR_ROUTE.linux, Support::Unverified(_)));
    assert!(matches!(FAR_ROUTE.windows, Support::Unsupported(_)));
}
