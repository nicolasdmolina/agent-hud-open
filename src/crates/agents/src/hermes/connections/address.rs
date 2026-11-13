//! An `ssh` entry's address: `host` split the way the desktop app splits
//! it (`normalizeSshConfig` in its `connection-config.ts`), then every part
//! checked, so nothing that reaches `ssh` can read as an option or a shell
//! word.
use super::{Shape, SshAddress};

/// The longest host or user name taken.
const LONGEST: usize = 255;

/// The address an entry names, or `None` when a part fails the checks.
/// An explicit `user` or `port` wins over one written into `host`.
pub(super) fn address(
    host: &str,
    user: Option<&str>,
    port: Option<Shape>,
    key_path: Option<&str>,
) -> Option<SshAddress> {
    let split = split(host);
    let user = match user.map(str::trim) {
        Some(user) if !user.is_empty() => Some(user),
        _ => split.user,
    };
    // The port as written, if one is: `None` inside when it is no number.
    let written = match port {
        None | Some(Shape::Empty) => split.port,
        Some(Shape::Whole(number) | Shape::Digits(number)) => Some(Some(number)),
        Some(Shape::Array | Shape::Object | Shape::Other) => Some(None),
    };
    // A port that is written must be usable.
    let port = match written {
        None => None,
        Some(number) => Some(number.and_then(self::port)?),
    };
    if !name(split.host, split.bracketed) || !user.is_none_or(|user| name(user, false)) {
        return None;
    }
    Some(SshAddress {
        host: split.host.to_owned(),
        user: user.map(str::to_owned),
        port,
        key_path: key_path
            .map(str::trim)
            .filter(|path| !path.is_empty())
            .map(str::to_owned),
    })
}

/// A port from a whole number: 1 to 65535.
pub(super) fn port(number: u64) -> Option<u16> {
    u16::try_from(number).ok().filter(|port| *port > 0)
}

/// The number a string of ASCII digits writes, if it fits.
pub(super) fn digits(text: &str) -> Option<u64> {
    if text.is_empty() || !text.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    text.parse().ok()
}

/// `host` in parts, before any check.
struct Split<'a> {
    host: &'a str,
    /// Whether the host was written in brackets, `[::1]:2222`.
    bracketed: bool,
    user: Option<&'a str>,
    /// A port written into `host`: `Some(None)` when it is out of reach of
    /// a number.
    port: Option<Option<u64>>,
}

/// `host` split as the desktop app splits it: a pasted `ssh ` dropped,
/// `user@` before the first `@` that is not the first character, then
/// `[host]:port`, or `host:port` where the host has one colon and the port
/// is all digits.
fn split(host: &str) -> Split<'_> {
    let mut host = without_ssh(host.trim()).trim();
    let mut user = None;
    if let Some(at) = host.find('@').filter(|at| *at > 0) {
        user = Some(&host[..at]);
        host = &host[at + 1..];
    }
    if let Some((inner, port)) = bracketed(host) {
        return Split {
            host: inner,
            bracketed: true,
            user,
            port: port.map(digits),
        };
    }
    let mut port = None;
    if let Some((name, written)) = host.split_once(':') {
        if !written.contains(':')
            && written.bytes().all(|b| b.is_ascii_digit())
            && !written.is_empty()
        {
            host = name;
            port = Some(digits(written));
        }
    }
    Split {
        host,
        bracketed: false,
        user,
        port,
    }
}

/// `host` without a leading `ssh` and the whitespace after it, any case.
fn without_ssh(host: &str) -> &str {
    let rest = host.get(3..).unwrap_or("");
    let ssh = host
        .get(..3)
        .is_some_and(|word| word.eq_ignore_ascii_case("ssh"));
    if ssh && rest.starts_with(char::is_whitespace) {
        rest.trim_start()
    } else {
        host
    }
}

/// `[inner]` or `[inner]:digits`: the inner part and the digits, if any.
fn bracketed(host: &str) -> Option<(&str, Option<&str>)> {
    let rest = host.strip_prefix('[')?;
    let close = rest.find(']')?;
    let (inner, after) = (&rest[..close], &rest[close + 1..]);
    if inner.is_empty() {
        return None;
    }
    match after.strip_prefix(':') {
        None if after.is_empty() => Some((inner, None)),
        Some(port) if !port.is_empty() && port.bytes().all(|b| b.is_ascii_digit()) => {
            Some((inner, Some(port)))
        }
        _ => None,
    }
}

/// A host or user name: letters, digits, `.`, `_` and `-`, not starting
/// with `-`, at most 255 long. A bracketed host may hold `:` as well, as an
/// IPv6 address does.
fn name(text: &str, bracketed: bool) -> bool {
    !text.is_empty()
        && text.len() <= LONGEST
        && !text.starts_with('-')
        && text.bytes().all(|b| {
            b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-') || (bracketed && b == b':')
        })
}
