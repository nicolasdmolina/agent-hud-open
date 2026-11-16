//! What the machines list says of each entry, in both languages. A
//! reason is told by a word everywhere else (the log, the route's own
//! report); this is the one place those words become text for the user.
//! A note says the reason and does not repeat the state beside it.
use super::list::Unread;
use crate::clients::hermes::machine::Standing;
use agent_hud_agents::hermes::connections::NotCovered;
use agent_hud_core::l10n::text;

/// What this machine is called where the list has no entry for it.
pub(in crate::clients::hermes) fn this_machine() -> String {
    text("本机", "This Mac")
}

/// The source-wide note: the list itself could not be read.
pub(in crate::clients::hermes) fn unread(why: Unread) -> String {
    match why {
        Unread::NotUnderstood => text(
            "无法读取 Hermes 桌面应用的连接列表：版本无法识别。当前的机器保持不变。",
            "The Hermes desktop app's connection list could not be read: its version is not one Agent HUD understands. The machines stay as they were.",
        ),
        _ => text(
            "无法读取 Hermes 桌面应用的连接列表，稍后重试。当前的机器保持不变。",
            "The Hermes desktop app's connection list could not be read. It is tried again, and the machines stay as they were.",
        ),
    }
}

/// A machine verified before, shown while no list has been read: nothing
/// is dialled from what was saved.
pub(in crate::clients::hermes) fn awaiting_list() -> String {
    text(
        "尚未读到连接列表，暂不连接。",
        "Not dialled: the connection list has not been read yet.",
    )
}

/// Why an entry of this kind is not followed.
pub(in crate::clients::hermes) fn kind(why: NotCovered) -> String {
    match why {
        NotCovered::Remote => text(
            "这是通过网络访问的网关，Agent HUD 只跟随本机和 SSH 主机。",
            "A gateway reached over the network. Agent HUD follows this Mac and SSH hosts only.",
        ),
        NotCovered::Cloud => text(
            "这是 Hermes Cloud 网关，Agent HUD 只跟随本机和 SSH 主机。",
            "A Hermes Cloud gateway. Agent HUD follows this Mac and SSH hosts only.",
        ),
        NotCovered::UnknownKind => text(
            "Agent HUD 不认识这种连接。",
            "A kind of connection Agent HUD does not know.",
        ),
        NotCovered::UnusableAddress => text(
            "地址无法使用：主机、用户或端口不符合要求。",
            "Unusable address: its host, user or port is not one Agent HUD dials.",
        ),
    }
}

/// Why an `ssh` entry has no route at all.
pub(in crate::clients::hermes) fn no_route(why: &str) -> String {
    text(
        &format!("无法建立 SSH 连接：{why}"),
        &format!("No SSH connection can be made: {why}."),
    )
}

pub(in crate::clients::hermes) fn not_trusted() -> String {
    text(
        "这台主机的密钥不在 known_hosts 中，或与其中的不同。请先用 ssh 连接一次并确认。",
        "The host's key is not in known_hosts, or is not the one there. Connect once with ssh to trust it.",
    )
}

/// Why there is no link just now, from the link's own word.
pub(in crate::clients::hermes) fn cannot_reach(why: &str) -> String {
    match why {
        "sign-in refused" => text(
            "登录被拒绝：需要无需输入的密钥或 SSH agent。",
            "Sign-in refused: it needs a key or the SSH agent, with no prompt.",
        ),
        "timed out" => text("连接超时，稍后重试。", "Timed out. It is tried again."),
        "connection ended" => text(
            "连接已断开，正在重连。",
            "The connection ended. Reconnecting.",
        ),
        "cannot reach" => text(
            "无法连接主机，稍后重试。",
            "The host cannot be reached. It is tried again.",
        ),
        other => text(
            &format!("无法连接：{other}"),
            &format!("No connection: {other}."),
        ),
    }
}

/// Why a machine that was reached is not followed, from the route's word.
pub(in crate::clients::hermes) fn reason(code: &str) -> String {
    match code {
        "install_id_missing" | "install_id_empty" => text(
            "那里的 Hermes 没有报告安装标识。",
            "Hermes there reports no install identity.",
        ),
        "install_id_length" | "install_id_upper_case" | "install_id_not_hex" => text(
            "那里的 Hermes 报告的安装标识无效。",
            "Hermes there reports an install identity that is not valid.",
        ),
        "install_ids_disagree" => text(
            "那里的网关报告了不同的安装标识。",
            "The gateways there report different install identities.",
        ),
        "install_id_changed" | "root_home_changed" => text(
            "这个地址现在通向另一台机器，正在重新确认。",
            "This address leads to another machine now. It is verified again.",
        ),
        "no_live_connection" => text(
            "那里的 Hermes 没有实时连接，需要更新。",
            "Hermes there has no live connection. It needs an update.",
        ),
        "no_gateway_running" => text(
            "那里没有运行中的 Hermes 网关。",
            "No Hermes gateway runs there.",
        ),
        "no_hermes_home" => text("那里没有 Hermes。", "There is no Hermes there."),
        "no-python3" => text(
            "那台机器没有 python3，无法安全地询问 Hermes。",
            "The machine has no python3, so Hermes there cannot be asked safely.",
        ),
        other => text(
            &format!("无法安全地询问那里的 Hermes：{other}"),
            &format!("Hermes there cannot be asked safely: {other}."),
        ),
    }
}

/// Whether a route that verified as no machine was reached and refused:
/// it is then not the machine that waits on it. A route that cannot be
/// reached, whose gateways are stopped, or whose Hermes cannot say who
/// it is just now, says nothing of which machine it leads to.
pub(in crate::clients::hermes) fn refused(standing: &Standing) -> bool {
    let unsaid = ["no_gateway_running", "no_live_connection"];
    matches!(standing, Standing::NotCovered(why) if !unsaid.contains(&why.as_str()))
}
