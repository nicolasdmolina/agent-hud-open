//! The control socket on macOS and Linux: `<home>/gateway.sock`, or, where
//! the home cannot hold a socket, the one a pointer file beside it names.
//! The checks are the ones Hermes's own client makes before it trusts a
//! socket (`hermes_cli/gateway_runtime_discovery.py`, `_socket_path` and
//! `connect_private`): the home is the user's and no one else may write
//! it, the pointer, its folder and the socket are the user's and closed to
//! everyone else, and the process listening is the same user. Nothing here
//! writes.
use crate::clients::hermes::control::Silent;
use agent_hud_agents::hermes::gateway::{pointed_socket, CONTROL_POINTER, CONTROL_SOCKET};
use sha2::{Digest, Sha256};
use std::{
    fs,
    io::{ErrorKind, Read, Write},
    os::unix::{
        ffi::OsStrExt,
        fs::{FileTypeExt, MetadataExt, OpenOptionsExt},
        io::AsRawFd,
        net::UnixStream,
    },
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

/// The longest answer read; Hermes's client stops at the same size.
const ANSWER_LIMIT: usize = 512 * 1024;
/// The longest path a pointer file may name.
const POINTER_LIMIT: usize = 4096;

pub(super) fn exchange(home: &Path, line: &[u8], timeout: Duration) -> Result<Vec<u8>, Silent> {
    let until = Instant::now() + timeout;
    let path = socket(home)?;
    let mut stream = UnixStream::connect(&path).map_err(|_| Silent::Absent)?;
    // The path was checked, but a link put there since would be followed:
    // the listener itself has to be the same user.
    if peer_uid(&stream) != Some(unsafe { libc::getuid() }) {
        return Err(Silent::Unsafe("listener"));
    }
    let left = |until: Instant| {
        let left = until.saturating_duration_since(Instant::now());
        (!left.is_zero()).then_some(left).ok_or(Silent::TimedOut)
    };
    let failed = |error: std::io::Error| match error.kind() {
        ErrorKind::WouldBlock | ErrorKind::TimedOut => Silent::TimedOut,
        _ => Silent::Absent,
    };
    // Both waits get their end before anything is sent. The gateway closes
    // the socket as soon as it has answered, and macOS refuses to change
    // a timeout on a socket whose other end is closed: taken as a failure,
    // that would lose an answer that is already waiting to be read.
    let wait = Some(left(until)?);
    stream.set_write_timeout(wait).map_err(failed)?;
    stream.set_read_timeout(wait).map_err(failed)?;
    stream.write_all(line).map_err(failed)?;
    let mut answer = Vec::new();
    let mut chunk = [0u8; 16 * 1024];
    while !answer.contains(&b'\n') {
        // Shortened to what is left where it can be; else the first holds.
        let _ = stream.set_read_timeout(Some(left(until)?));
        match stream.read(&mut chunk) {
            Ok(0) => break,
            Ok(count) => answer.extend_from_slice(&chunk[..count]),
            Err(error) if error.kind() == ErrorKind::Interrupted => {}
            Err(error) => return Err(failed(error)),
        }
        if answer.len() > ANSWER_LIMIT {
            return Err(Silent::Garbled);
        }
    }
    if answer.is_empty() {
        return Err(Silent::Garbled);
    }
    Ok(answer)
}

/// The socket to ask for `home`, once everything on the way to it is the
/// user's own.
fn socket(home: &Path) -> Result<PathBuf, Silent> {
    let uid = unsafe { libc::getuid() };
    let node = fs::symlink_metadata(home).map_err(|_| Silent::Absent)?;
    // Hermes lets a home be read by others, never written: whoever can
    // write it can put a socket of their own there. A home its group may
    // write is refused here outright, where Hermes looks up whether the
    // group is the user's private one.
    if !node.is_dir() || node.uid() != uid || node.mode() & 0o022 != 0 {
        return Err(Silent::Unsafe("home"));
    }
    let private = |node: &fs::Metadata| node.uid() == uid && node.mode() & 0o077 == 0;
    let direct = home.join(CONTROL_SOCKET);
    if let Ok(node) = fs::symlink_metadata(&direct) {
        return (node.file_type().is_socket() && private(&node))
            .then_some(direct)
            .ok_or(Silent::Unsafe("socket"));
    }
    // No waiting on a pipe put where the pointer should be, and no links.
    let mut pointer = fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
        .open(home.join(CONTROL_POINTER))
        .map_err(|_| Silent::Absent)?;
    let node = pointer.metadata().map_err(|_| Silent::Absent)?;
    if !node.is_file() || !private(&node) {
        return Err(Silent::Unsafe("pointer"));
    }
    let mut named = Vec::new();
    (&mut pointer)
        .take(POINTER_LIMIT as u64 + 1)
        .read_to_end(&mut named)
        .map_err(|_| Silent::Absent)?;
    let named = std::str::from_utf8(&named)
        .ok()
        .filter(|named| named.len() <= POINTER_LIMIT)
        .and_then(pointed_socket)
        .ok_or(Silent::Unsafe("pointer"))?;
    // Only the folder Hermes makes for this very home, whatever temporary
    // root its gateway runs with.
    let target = Path::new(named);
    let folder = target.parent().ok_or(Silent::Unsafe("pointer"))?;
    if folder.file_name().map(|name| name.as_bytes()) != Some(folder_name(home).as_bytes()) {
        return Err(Silent::Unsafe("pointer"));
    }
    let inside = fs::symlink_metadata(folder).map_err(|_| Silent::Absent)?;
    let node = fs::symlink_metadata(target).map_err(|_| Silent::Absent)?;
    if !inside.is_dir() || !private(&inside) || !node.file_type().is_socket() || !private(&node) {
        return Err(Silent::Unsafe("socket"));
    }
    Ok(target.to_path_buf())
}

/// `hermes-gw-<hash>`, as Hermes names the folder of a home's socket
/// (`gateway/control_socket.py`, `_home_hash`): the first 16 hex digits of
/// the SHA-256 of the home's resolved path.
fn folder_name(home: &Path) -> String {
    let digest = Sha256::digest(home.as_os_str().as_bytes());
    let hex: String = digest.iter().map(|byte| format!("{byte:02x}")).collect();
    format!("hermes-gw-{}", &hex[..16])
}

/// The user the process at the other end runs as, as the kernel says.
#[cfg(target_os = "linux")]
fn peer_uid(stream: &UnixStream) -> Option<u32> {
    let mut peer: libc::ucred = unsafe { std::mem::zeroed() };
    let mut size = std::mem::size_of::<libc::ucred>() as libc::socklen_t;
    let status = unsafe {
        libc::getsockopt(
            stream.as_raw_fd(),
            libc::SOL_SOCKET,
            libc::SO_PEERCRED,
            (&mut peer as *mut libc::ucred).cast(),
            &mut size,
        )
    };
    (status == 0).then_some(peer.uid)
}

/// The user the process at the other end runs as, as the kernel says.
#[cfg(not(target_os = "linux"))]
fn peer_uid(stream: &UnixStream) -> Option<u32> {
    let (mut uid, mut gid) = (0, 0);
    let status = unsafe { libc::getpeereid(stream.as_raw_fd(), &mut uid, &mut gid) };
    (status == 0).then_some(uid)
}

#[cfg(test)]
pub(in crate::clients::hermes) fn folder_of(home: &Path) -> String {
    folder_name(home)
}
