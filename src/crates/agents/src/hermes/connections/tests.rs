use super::*;

const REAL_SHAPE: &str =
    include_str!("../../../tests/fixtures/hermes/connections/real-shape.SYNTHETIC.json");
const EVERY_KIND: &str =
    include_str!("../../../tests/fixtures/hermes/connections/every-kind.SYNTHETIC.json");

/// A value no parsed entry, `Debug` output or error may hold.
const MARKER: &str = "MARKER-7f3c-not-for-any-output";

fn entry(id: &str, label: &str, kind: Kind) -> Entry {
    Entry {
        id: id.into(),
        label: label.into(),
        kind,
    }
}

fn ssh(host: &str, user: Option<&str>, port: Option<u16>, key_path: Option<&str>) -> Kind {
    Kind::Ssh(SshAddress {
        host: host.into(),
        user: user.map(Into::into),
        port,
        key_path: key_path.map(Into::into),
    })
}

/// The kind of a list's one `ssh` entry with these fields.
fn ssh_entry(fields: &str) -> Kind {
    let text = format!(
        r#"{{"version":2,"connections":[{{"id":"x","kind":"ssh","label":"X",{fields}}}]}}"#
    );
    let mut entries = parse(&text).expect("the list parses");
    assert_eq!(entries.len(), 1);
    entries.remove(0).kind
}

const UNUSABLE: Kind = Kind::NotCovered(NotCovered::UnusableAddress);

#[test]
fn the_real_shape_reads_as_this_machine_and_an_ssh_alias() {
    assert_eq!(
        parse(REAL_SHAPE),
        Ok(vec![
            entry("local", "This device", Kind::This),
            entry("lab-box", "Lab box", ssh("lab-box", None, None, None)),
        ])
    );
}

#[test]
fn every_kind_reads_as_an_entry_or_a_reason() {
    assert_eq!(
        parse(EVERY_KIND),
        Ok(vec![
            entry("local", "This device", Kind::This),
            entry(
                "build-host",
                "Build host",
                ssh(
                    "build.example.test",
                    Some("builder"),
                    Some(2222),
                    Some("~/.ssh/id_lab")
                ),
            ),
            entry(
                "office-gateway",
                "Office gateway",
                Kind::NotCovered(NotCovered::Remote)
            ),
            entry("cloud-oauth", "Cloud", Kind::NotCovered(NotCovered::Cloud)),
            entry(
                "cloud-legacy",
                "Cloud (old sign-in)",
                Kind::NotCovered(NotCovered::Cloud)
            ),
            entry(
                "future",
                "Future kind",
                Kind::NotCovered(NotCovered::UnknownKind)
            ),
        ])
    );
}

#[test]
fn a_cloud_entry_stored_with_token_auth_is_not_covered() {
    let text = r#"{"version":2,"connections":[{"id":"c","kind":"cloud","label":"C",
        "url":"https://c.example.test","authMode":"token",
        "token":{"encoding":"plain","value":"SYNTHETIC"}}]}"#;
    assert_eq!(
        parse(text),
        Ok(vec![entry("c", "C", Kind::NotCovered(NotCovered::Cloud))])
    );
}

#[test]
fn only_version_2_is_understood() {
    for version in [
        r#""version":1,"connections":[]"#,
        r#""version":3,"connections":[]"#,
        r#""version":"2","connections":[]"#,
        r#""version":2.5,"connections":[]"#,
        r#""version":-2,"connections":[]"#,
        r#""version":null,"connections":[]"#,
        r#""version":{"major":2},"connections":[]"#,
        r#""connections":[]"#,
        // Another version's shape does not break the read.
        r#""version":1,"connections":{"local":{}}"#,
    ] {
        assert_eq!(
            parse(&format!("{{{version}}}")),
            Err(ListError::NotUnderstood),
            "{version}"
        );
    }
    assert_eq!(parse(r#"{"version":2,"connections":[]}"#), Ok(vec![]));
}

/// A list a later reader would act on removes every machine it no longer
/// holds, so only an explicit array may read as a list, even an empty one.
#[test]
fn a_list_without_its_connection_array_is_not_understood() {
    for text in [
        r#"{"version":2}"#,
        r#"{"version":2,"primary":"local","lastUsed":"local"}"#,
        r#"{"version":2,"connections":null}"#,
        r#"{"version":2,"connections":{}}"#,
        r#"{"version":2,"connections":{"0":{"id":"a","kind":"ssh","host":"box"}}}"#,
        r#"{"version":2,"connections":"[]"}"#,
        r#"{"version":2,"connections":0}"#,
        r#"{"version":2,"connections":false}"#,
        // A document that is not an object holds no list.
        "null",
        "[]",
        "[2,[]]",
        r#""connections""#,
        "2",
        "true",
    ] {
        assert_eq!(parse(text), Err(ListError::NotUnderstood), "{text}");
    }
    // An explicit empty array is a real, empty list.
    assert_eq!(parse(r#"{"connections":[],"version":2}"#), Ok(vec![]));
}

#[test]
fn broken_json_says_where_and_what_kind() {
    assert_eq!(
        parse("{\n  \"version\": 2,\n  \"connections\": [ }"),
        Err(ListError::Broken {
            line: 3,
            column: 20,
            fault: Fault::Syntax
        })
    );
    assert!(matches!(
        parse(r#"{"version":2,"connections":["#),
        Err(ListError::Broken {
            fault: Fault::Eof,
            ..
        })
    ));
    assert!(matches!(
        parse(r#"{"version":2,"connections":[{"id":5}]}"#),
        Err(ListError::Broken {
            fault: Fault::Data,
            ..
        })
    ));
    assert!(matches!(
        parse(""),
        Err(ListError::Broken {
            fault: Fault::Eof,
            ..
        })
    ));
}

/// A list whose every secret is the marker: the `ssh` and `remote` tokens,
/// a header, and an entry the desktop app set aside.
fn marked_list() -> String {
    format!(
        r#"{{"version":2,"primary":"local",
  "connections":[
    {{"id":"local","kind":"local","label":"This device"}},
    {{"id":"s","kind":"ssh","label":"S","host":"lab-box","token":{{"encoding":"plain","value":"{MARKER}"}}}},
    {{"id":"r","kind":"remote","label":"R","url":"https://r.example.test","authMode":"token",
      "token":{{"encoding":"plain","value":"{MARKER}"}},
      "headers":{{"X-Secret":{{"encoding":"plain","value":"{MARKER}"}}}}}}
  ],
  "quarantined":[{{"reason":"entry-malformed","entry":{{"token":"{MARKER}"}}}}]}}"#
    )
}

#[test]
fn no_token_or_header_value_reaches_an_entry_or_an_error() {
    let text = marked_list();
    let entries = parse(&text).expect("the marked list parses");
    assert_eq!(entries.len(), 3);
    assert!(!format!("{entries:?}").contains(MARKER));

    let at = text.find(MARKER).expect("the marker is in the list") + MARKER.len();
    let broken = [
        // Cut off right after the marker, inside its string.
        text[..at].to_owned(),
        // Its string closed, then a stray character.
        format!("{}\"x{}", &text[..at], &text[at + 1..]),
        // A field of the wrong type whose own value is the marker.
        format!(r#"{{"version":2,"connections":["{MARKER}"]}}"#),
        format!(r#"{{"version":2,"connections":"{MARKER}"}}"#),
        format!(r#"{{"version":2,"connections":[{{"id":"{MARKER}","kind":["{MARKER}"]}}]}}"#),
        format!(r#"{{"version":"{MARKER}","connections":[]}}"#),
        format!(r#"{{"version":2,"connections":[{{"kind":"ssh","port":"{MARKER}"}}]}}"#),
    ];
    for text in &broken {
        match parse(text) {
            Err(error) => assert!(!format!("{error:?}").contains(MARKER), "{error:?}"),
            Ok(entries) => assert!(!format!("{entries:?}").contains(MARKER)),
        }
    }
    assert!(matches!(
        parse(&broken[0]),
        Err(ListError::Broken {
            fault: Fault::Eof,
            ..
        })
    ));
    assert!(matches!(
        parse(&broken[1]),
        Err(ListError::Broken {
            fault: Fault::Syntax,
            ..
        })
    ));
    assert!(matches!(
        parse(&broken[2]),
        Err(ListError::Broken {
            fault: Fault::Data,
            ..
        })
    ));
    assert_eq!(parse(&broken[3]), Err(ListError::NotUnderstood));
    // Why the error keeps only position and class: the JSON library's own
    // message quotes the file.
    let quoted = serde_json::from_str::<List>(&broken[2])
        .err()
        .expect("a type error");
    assert!(quoted.to_string().contains(MARKER));
}

#[test]
fn lists_that_differ_only_outside_the_allowlist_are_equal() {
    let other = REAL_SHAPE
        .replace("SYNTHETIC-TOKEN-ONE", "SYNTHETIC-TOKEN-ROTATED")
        .replace(r#""lastUsed": "lab-box""#, r#""lastUsed": "local""#);
    assert_ne!(other, REAL_SHAPE);
    assert_eq!(parse(&other), parse(REAL_SHAPE));
    let marked = marked_list();
    assert_eq!(
        parse(&marked.replace(MARKER, "another-value")),
        parse(&marked)
    );
    // An allowlisted field does count.
    assert_ne!(
        parse(&REAL_SHAPE.replace("Lab box", "Lab")),
        parse(REAL_SHAPE)
    );
}

#[test]
fn host_splits_the_way_the_desktop_app_splits_it() {
    let cases = [
        (
            r#""host":"admin@box""#,
            ssh("box", Some("admin"), None, None),
        ),
        (
            r#""host":"admin@box:2200""#,
            ssh("box", Some("admin"), Some(2200), None),
        ),
        (
            r#""host":"ssh admin@box""#,
            ssh("box", Some("admin"), None, None),
        ),
        (r#""host":"SSH  box""#, ssh("box", None, None, None)),
        (r#""host":"[::1]:2222""#, ssh("::1", None, Some(2222), None)),
        (
            r#""host":"root@[fe80::1]""#,
            ssh("fe80::1", Some("root"), None, None),
        ),
        (
            r#""host":" box ","keyPath":"  ""#,
            ssh("box", None, None, None),
        ),
        // An explicit user or port wins over one written into the host.
        (
            r#""host":"admin@box:2200","user":"deploy","port":"2201""#,
            ssh("box", Some("deploy"), Some(2201), None),
        ),
        (
            r#""host":"box:2200","port":"""#,
            ssh("box", None, Some(2200), None),
        ),
        (
            r#""host":"box","port":22.0"#,
            ssh("box", None, Some(22), None),
        ),
        (r#""host":"box","port":null"#, ssh("box", None, None, None)),
    ];
    for (fields, kind) in cases {
        assert_eq!(ssh_entry(fields), kind, "{fields}");
    }
}

#[test]
fn an_unusable_address_is_refused_after_splitting() {
    for fields in [
        r#""host":"-oProxyCommand=x""#,
        r#""host":"-oProxyCommand=x@host""#,
        r#""host":"box","user":"-l""#,
        r#""host":"my box""#,
        r#""host":"box;reboot""#,
        r#""host":"box$(id)""#,
        r#""host":"box`id`""#,
        r#""host":"box|x""#,
        r#""host":"box","user":"a b""#,
        r#""host":"box","user":"a;b""#,
        r#""host":"a'b@box""#,
        r#""host":"box","port":0"#,
        r#""host":"box","port":65536"#,
        r#""host":"box","port":"ssh""#,
        r#""host":"box","port":"-1""#,
        r#""host":"box","port":-1"#,
        r#""host":"box","port":true"#,
        r#""host":"box:0""#,
        r#""host":"box:65536""#,
        r#""host":"[box]:99999999999999999999999""#,
        // A colon outside brackets is not an address.
        r#""host":"box:ssh""#,
        r#""host":"a:b:c""#,
        r#""host":"@box""#,
        r#""host":"admin@""#,
        r#""host":"[]""#,
        r#""host":"""#,
        r#""user":"admin""#,
    ] {
        assert_eq!(ssh_entry(fields), UNUSABLE, "{fields}");
    }
}

#[test]
fn ids_kinds_and_labels() {
    let text = r#"{"version":2,"connections":[
        {"id":"mine","kind":"local","label":" Here "},
        {"id":"second","kind":"local","label":"Again"},
        {"id":"k","label":"No kind"},
        {"id":"n","kind":null,"label":"Null kind"}]}"#;
    assert_eq!(
        parse(text),
        Ok(vec![
            entry("local", "Here", Kind::This),
            entry("k", "No kind", Kind::NotCovered(NotCovered::UnknownKind)),
            entry("n", "Null kind", Kind::NotCovered(NotCovered::UnknownKind)),
        ])
    );
}

#[test]
fn the_list_is_declared_for_each_system() {
    assert_eq!(CONNECTIONS_FILE.relative(), "Hermes/connections.json");
    assert!(matches!(CONNECTIONS.macos, Support::Verified(_)));
    assert!(matches!(CONNECTIONS.linux, Support::Unverified(_)));
    assert!(matches!(CONNECTIONS.windows, Support::Unsupported(_)));
}
