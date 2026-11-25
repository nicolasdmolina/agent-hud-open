//! What the machines list says of each entry, in the app's language. A
//! reason is told by a word everywhere else (the log, the route's own
//! report); this is the one place those words become text for the user.
//! A note says the reason and does not repeat the state beside it.
use super::list::Unread;
use crate::clients::hermes::{
    history::{Ages, Without},
    machine::Standing,
};
use agent_hud_agents::hermes::connections::NotCovered;
use agent_hud_core::l10n::tr;

/// What this machine is called where the list has no entry for it.
pub(in crate::clients::hermes) fn this_machine() -> String {
    tr!("This Mac")
}

/// Why this machine is not connected, from what its own followers know
/// (ticket 69). The far machines' words, said of this one.
pub(in crate::clients::hermes) fn here_no_gateway() -> String {
    tr!("No Hermes gateway runs on this Mac.")
}

pub(in crate::clients::hermes) fn here_unreachable() -> String {
    tr!("This Mac's Hermes gateway cannot be reached right now.")
}

pub(in crate::clients::hermes) fn here_no_live_connection() -> String {
    tr!("Hermes on this Mac has no live connection. It needs an update.")
}

/// No Hermes home at all: nothing failed, there is nothing to follow.
pub(in crate::clients::hermes) fn here_no_hermes() -> String {
    tr!("There is no Hermes on this Mac.")
}

/// The source-wide note: the list itself could not be read.
pub(in crate::clients::hermes) fn unread(why: Unread) -> String {
    match why {
        Unread::NotUnderstood => tr!("The Hermes desktop app's connection list could not be read: its version is not one Agent HUD understands. The machines stay as they were."),
        _ => tr!("The Hermes desktop app's connection list could not be read. It is tried again, and the machines stay as they were."),
    }
}

/// A machine verified before, shown while no list has been read: nothing
/// is dialled from what was saved.
pub(in crate::clients::hermes) fn awaiting_list() -> String {
    tr!("Not dialled: the connection list has not been read yet.")
}

/// Why an entry of this kind is not followed.
pub(in crate::clients::hermes) fn kind(why: NotCovered) -> String {
    match why {
        NotCovered::Remote => tr!(
            "A gateway reached over the network. Agent HUD follows this Mac and SSH hosts only."
        ),
        NotCovered::Cloud => {
            tr!("A Hermes Cloud gateway. Agent HUD follows this Mac and SSH hosts only.")
        }
        NotCovered::UnknownKind => tr!("A kind of connection Agent HUD does not know."),
        NotCovered::UnusableAddress => {
            tr!("Unusable address: its host, user or port is not one Agent HUD dials.")
        }
    }
}

/// Why an `ssh` entry has no route at all.
pub(in crate::clients::hermes) fn no_route(why: &str) -> String {
    tr!("No SSH connection can be made: {{why}}.", why = why)
}

pub(in crate::clients::hermes) fn not_trusted() -> String {
    tr!("The host's key is not in known_hosts, or is not the one there. Connect once with ssh to trust it.")
}

/// Why there is no link just now, from the link's own word.
pub(in crate::clients::hermes) fn cannot_reach(why: &str) -> String {
    match why {
        "sign-in refused" => {
            tr!("Sign-in refused: it needs a key or the SSH agent, with no prompt.")
        }
        "timed out" => tr!("Timed out. It is tried again."),
        "connection ended" => tr!("The connection ended. Reconnecting."),
        "cannot reach" => tr!("The host cannot be reached. It is tried again."),
        other => tr!("No connection: {{why}}.", why = other),
    }
}

/// Why a machine that was reached is not followed, from the route's word.
pub(in crate::clients::hermes) fn reason(code: &str) -> String {
    match code {
        "install_id_missing" | "install_id_empty" => {
            tr!("Hermes there reports no install identity.")
        }
        "install_id_length" | "install_id_upper_case" | "install_id_not_hex" => {
            tr!("Hermes there reports an install identity that is not valid.")
        }
        "install_ids_disagree" => tr!("The gateways there report different install identities."),
        "install_id_changed" | "root_home_changed" => {
            tr!("This address leads to another machine now. It is verified again.")
        }
        "no_live_connection" => tr!("Hermes there has no live connection. It needs an update."),
        "no_gateway_running" => tr!("No Hermes gateway runs there."),
        "no_hermes_home" => tr!("There is no Hermes there."),
        "no-python3" => tr!("The machine has no python3, so Hermes there cannot be asked safely."),
        other => tr!("Hermes there cannot be asked safely: {{why}}.", why = other),
    }
}

/// Why a machine that is followed shows no stored history.
pub(in crate::clients::hermes) fn no_history(why: Without) -> String {
    match why {
        Without::Python => tr!("No history: the machine has no python3 with sqlite3 to read it."),
        Without::SqliteOld => tr!("No history: the SQLite there is too old to read it safely."),
    }
}

/// Sessions of a followed machine that its mirror would not take: their
/// usage is not counted until their far rows change (ticket 63).
pub(in crate::clients::hermes) fn set_aside(count: usize) -> String {
    tr!(
        count = count,
        "{{count}} sessions' usage is not counted: their records could not be stored."
    )
}

/// A machine where the Hermes desktop app hosts a session no gateway
/// lists (ticket 39): it is listed, with no card.
pub(in crate::clients::hermes) fn desktop_hosted() -> String {
    tr!("Questions asked there are answered in Hermes desktop.")
}

/// The notes of a followed machine that has a history reader: that the
/// Hermes desktop app answers questions there, then sessions its mirror
/// would not take. Each is said once; None where neither holds.
pub(in crate::clients::hermes) fn far_notes(ages: &Ages) -> Option<String> {
    let notes: Vec<String> = [
        ages.desktop.then(desktop_hosted),
        (ages.refused > 0).then(|| set_aside(ages.refused)),
    ]
    .into_iter()
    .flatten()
    .collect();
    (!notes.is_empty()).then(|| notes.join(" "))
}

/// Whether a route that verified as no machine was reached and refused:
/// it is then not the machine that waits on it. A route that cannot be
/// reached, whose gateways are stopped, or whose Hermes cannot say who
/// it is just now, says nothing of which machine it leads to.
pub(in crate::clients::hermes) fn refused(standing: &Standing) -> bool {
    let unsaid = ["no_gateway_running", "no_live_connection"];
    matches!(standing, Standing::NotCovered(why) if !unsaid.contains(&why.as_str()))
}
