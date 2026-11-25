//! Agent HUD Fork (ticket 66, round 6): an automatic Allow is told to core
//! as approved once the hook that asked has the whole reply, under the
//! tool call it names, and not where the write to the hook failed.
use super::*;
use crate::hooks::now_ms;
use serde_json::json;

#[test]
fn an_automatic_allow_the_hook_received_is_told_to_core() {
    let root = tempfile::tempdir().unwrap();
    let data = root.path().join("data");
    let requests = Requests::default();
    requests.set_auto_approve(agent_hud_core::requests::AutoApprove::from_settings(
        &json!({"autoApprove": true}),
    ));
    let reminders = super::super::reminders::Reminders::default();
    let _server = Server::start(&data, requests.clone(), Arc::new(|| {}), reminders).unwrap();
    assert!(requests.auto_approved(now_ms()).is_empty());
    let payload = json!({"session_id":"s1","cwd":"/w","hook_event_name":"PermissionRequest",
        "tool_name":"Bash","tool_use_id":"toolu_66","tool_input":{"command":"cargo test"}});
    let reply = send(&data, "claude", "--permission-hook", &payload)
        .unwrap()
        .unwrap();
    assert_eq!(reply["hookSpecificOutput"]["decision"]["behavior"], "allow");
    // The listener tells core right after its write returns.
    let told = || requests.auto_approved(now_ms()).into_values().next();
    let until = std::time::Instant::now() + Duration::from_secs(5);
    while told().is_none() && std::time::Instant::now() < until {
        std::thread::sleep(Duration::from_millis(5));
    }
    assert_eq!(told(), Some(vec!["call:toolu_66".to_owned()]));
}

/// A hook that went away: it takes this many more bytes and then none.
struct Gone(usize);
impl Write for Gone {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        let taken = bytes.len().min(self.0);
        self.0 -= taken;
        match taken {
            0 => Err(std::io::ErrorKind::BrokenPipe.into()),
            taken => Ok(taken),
        }
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

/// The hook hung up at once, or part of the way through the reply: the
/// write says so, and core is told of no approval. Written whole, it is.
#[test]
fn an_automatic_allow_the_hook_did_not_take_whole_is_not_told_to_core() {
    let requests = Requests::default();
    requests.set_auto_approve(agent_hud_core::requests::AutoApprove::from_settings(
        &json!({"autoApprove": true}),
    ));
    let reminders = super::super::reminders::Reminders::default();
    let payload = json!({"session_id":"s1","cwd":"/w","hook_event_name":"PermissionRequest",
        "tool_name":"Bash","tool_use_id":"toolu_66","tool_input":{"command":"cargo test"}});
    let ask = || {
        let flag = "--permission-hook";
        super::super::wait_for_answer(&requests, &reminders, "claude", flag, &payload, || false)
            .expect("an automatic Allow")
    };
    let whole = serde_json::to_vec(&ask().deliver(&requests, |_| false)).unwrap();
    for left in [0, 1, whole.len() - 1, whole.len()] {
        let mut hook = Gone(left);
        ask().deliver(&requests, |reply| written(&mut hook, reply));
        assert!(requests.auto_approved(now_ms()).is_empty(), "{left} bytes");
    }
    let mut heard = Vec::new();
    ask().deliver(&requests, |reply| written(&mut heard, reply));
    assert_eq!(heard, [whole.as_slice(), b"\n"].concat());
    let told = requests.auto_approved(now_ms()).into_values().next();
    assert_eq!(told, Some(vec!["call:toolu_66".to_owned()]));
}

/// The round 6 verifier's hand-over probe. The Windows listener cannot see
/// its pipe write: the handler returns the reply, and the pipe server
/// writes it afterwards, or drops it where the hook has gone or the
/// listener stopped. Replayed here on a socket pair: the reply is taken
/// the way `hooks/windows.rs` takes it, the hook's end closes, and the
/// write fails. Core must know of no approval. Before round 7 the
/// hand-over itself counted as delivery.
#[test]
fn a_reply_handed_over_and_never_written_is_not_told_to_core() {
    // How the Windows handler takes the reply, which this Mac cannot
    // compile: held to the one call that tells core nothing.
    let windows = include_str!("../windows.rs");
    assert!(windows.contains(".map(|answer| answer.into_reply())"));
    assert!(!windows.contains("deliver("), "Windows counts a hand-over");

    let root = tempfile::tempdir().unwrap();
    let mut core = agent_hud_core::AgentHudCore::new(vec![], root.path().join("app"));
    for key in ["sound", "autoApprove"] {
        let on = json!({"key": key, "value": true});
        core.request("set_setting", on).unwrap();
    }
    let requests = core.requests();
    let reminders = super::super::reminders::Reminders::default();
    let payload = json!({"session_id":"s1","cwd":"/w","hook_event_name":"PermissionRequest",
        "tool_name":"Bash","tool_use_id":"toolu_66","tool_input":{"command":"cargo test"}});
    let flag = "--permission-hook";
    let answer =
        super::super::wait_for_answer(&requests, &reminders, "claude", flag, &payload, || true)
            .expect("an automatic Allow");
    let (mut listener_end, hook_end) = UnixStream::pair().unwrap();
    let reply = answer.into_reply();
    drop(hook_end);
    assert!(!written(&mut listener_end, &reply));
    assert!(
        requests.auto_approved(now_ms()).is_empty(),
        "a reply that was never written counted as delivered"
    );
}
