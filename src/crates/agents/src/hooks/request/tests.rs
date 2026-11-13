use super::*;
use serde_json::json;

#[test]
fn a_request_keeps_the_tool_call_its_hook_names() {
    let payload = json!({"session_id":"s1","hook_event_name":"PermissionRequest","tool_name":"Bash","tool_input":{"command":"ls"},"tool_use_id":"toolu_1"});
    let request = parse_request("claude", &payload).unwrap().unwrap();
    assert_eq!(request.call_id.as_deref(), Some("toolu_1"));
    // Claude Code's PermissionRequest names none (2.1.322): nothing is made up.
    let mut payload = payload;
    payload.as_object_mut().unwrap().remove("tool_use_id");
    let request = parse_request("claude", &payload).unwrap().unwrap();
    assert_eq!(request.call_id, None);
}
