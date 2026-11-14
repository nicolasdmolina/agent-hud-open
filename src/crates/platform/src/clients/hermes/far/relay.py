"""Agent HUD's control relay: one exchange with one Hermes gateway's
control socket, made on the machine the gateway runs on. It is sent on
standard input and never written to this machine's disk:

    python3 -I -S -B - <seconds> <home> <request line>

It answers on standard output with one status line, and after `ok` the one
line the gateway answered. Everything else is a named refusal. Before it
sends anything it checks, as Hermes's own client does
(`hermes_cli/gateway_runtime_discovery.py`, which is not imported: it can
change a lock's permissions), that:

- the home is canonical, a folder of this user, and no one else may write it;
- `gateway.sock` in it is a socket of this user, closed to everyone else;
- or the pointer file beside it is no link, is this user's and closed to
  others, is short, and names `control.sock` in the folder Hermes makes
  for this very home, which is checked the same way;
- the process that listens runs as this user, as the kernel says.

"Closed" is the mode bits and the access control list, which on macOS can
open a node whose mode says it is private. The checks are made on open
descriptors, so a path swapped after a check leads nowhere new. Where a
list cannot be read, or the kernel cannot say who listens, it refuses.
It proves the listener runs as the login user, and cannot tell two of that
user's processes apart. It writes no file and imports only the standard library.
"""
import errno
import fcntl
import hashlib
import json
import os
import signal
import socket
import stat
import struct
import sys
import threading
import time

SOCKET = "gateway.sock"
POINTER = "gateway.sock.path"
POINTED = "control.sock"
VERBS = ("identify", "session-ticket")
# The longest answer passed on, as Hermes's own client has it, and the
# longest path a pointer file may name.
MOST_ANSWER = 512 * 1024
MOST_POINTER = 4096
MOST_SECONDS = 60.0
# How this process ends: an answer, a named refusal, its deadline.
DONE, REFUSED, DEADLINE = 0, 2, 3
READ = os.O_RDONLY | os.O_NOFOLLOW | getattr(os, "O_CLOEXEC", 0)
FOLDER = READ | getattr(os, "O_DIRECTORY", 0)
# What a list may let someone else do to a home: read it, as its mode may.
READS = {"read", "list", "execute", "search", "readattr", "readextattr", "readsecurity"}


class Refusal(Exception):
    """A named reason nothing was exchanged."""


def watchdog(seconds):
    """Ends this process `seconds` from now, whatever it is doing."""
    def end():
        time.sleep(seconds)
        os._exit(DEADLINE)

    threading.Thread(target=end, daemon=True).start()
    # The kernel's own timer, should the thread never get to run.
    signal.alarm(int(seconds) + 2)


def held_path(descriptor, path):
    """Where an open folder really is, its links resolved."""
    if hasattr(fcntl, "F_GETPATH"):
        held = fcntl.fcntl(descriptor, fcntl.F_GETPATH, bytes(1024))
        return os.fsdecode(held.split(b"\0", 1)[0])
    link = "/proc/self/fd/%d" % descriptor
    return os.readlink(link) if os.path.islink(link) else os.path.realpath(path)


def allowed(descriptor, name=None):
    """What a node's access control list allows, as (whose, what) for each
    entry. The node is an open descriptor, or `name` in the current folder,
    no link followed. Linux folds a list into the group bits, checked already."""
    if sys.platform.startswith("linux"):
        return []
    try:
        import ctypes

        libc = ctypes.CDLL(None, use_errno=True)
        get = libc.acl_get_fd_np if name is None else libc.acl_get_link_np
        get.restype = libc.acl_to_text.restype = ctypes.c_void_p
        get.argtypes = [ctypes.c_int if name is None else ctypes.c_char_p, ctypes.c_int]
        libc.acl_to_text.argtypes = [ctypes.c_void_p, ctypes.c_void_p]
        libc.acl_free.argtypes = [ctypes.c_void_p]
        held = get(descriptor if name is None else os.fsencode(name), 0x100)
        if not held and ctypes.get_errno() == errno.ENOENT:
            return []  # it has no list
        said = libc.acl_to_text(held, None) if held else None
        if not said:
            raise OSError("no list was read")
        entries = ctypes.string_at(said).decode("utf-8").splitlines()[1:]
        libc.acl_free(said)
        libc.acl_free(held)
        # Each is user:<uuid>:<name>:<id>:allow[,flags]:<what>,<what>
        entries = [entry.rsplit(":", 2) for entry in entries]
        if any(kind.split(",")[0] not in ("allow", "deny") for _, kind, _ in entries):
            raise ValueError("an entry of another kind")
        return [(whose, set(what.split(","))) for whose, kind, what in entries
                if kind.split(",")[0] == "allow"]
    except (ImportError, AttributeError, OSError, ValueError):
        raise Refusal("permissions-unread")  # not read is not known closed


def unshared(allows, uid, what, reads=frozenset()):
    """Refuses a node whose list lets someone else do more than `reads`."""
    mine = ":%d" % uid
    for whose, does in allows:
        if not (whose.startswith("user:") and whose.endswith(mine)) and not does <= reads:
            raise Refusal(what + "-open-to-others")


def open_folder(path, missing, what):
    """An open descriptor of the folder at `path`, which is no link."""
    try:
        return os.open(path, FOLDER)
    except OSError as error:
        raise Refusal(missing if error.errno == errno.ENOENT else what)


def open_home(path, uid):
    """The home, open, once it is canonical, this user's, and no one else's to write."""
    home = open_folder(path, "no-home", "home-not-canonical")
    node = os.fstat(home)
    # Where the descriptor really is: never a relative path, never a link.
    if not path.startswith("/") or held_path(home, path) != path:
        raise Refusal("home-not-canonical")
    if not stat.S_ISDIR(node.st_mode) or node.st_uid != uid:
        raise Refusal("home-not-yours")
    if stat.S_IMODE(node.st_mode) & 0o022:
        raise Refusal("home-open-to-others")
    unshared(allowed(home), uid, "home", READS)
    return home


def private(node, uid, kind, what):
    """Refuses a node not of `kind`, not this user's, or open by its mode."""
    if not kind(node.st_mode) or node.st_uid != uid:
        raise Refusal(what + "-not-yours")
    if stat.S_IMODE(node.st_mode) & 0o077:
        raise Refusal(what + "-open-to-others")


def socket_in(folder, name, uid):
    """Whether `name` is in the open `folder`, once it is this user's private socket."""
    try:
        node = os.stat(name, dir_fd=folder, follow_symlinks=False)
    except OSError as error:
        if error.errno == errno.ENOENT:
            return False
        raise Refusal("socket-not-yours")
    private(node, uid, stat.S_ISSOCK, "socket")
    os.fchdir(folder)  # a socket cannot be opened, so it is named from its folder
    unshared(allowed(None, name), uid, "socket")
    return True


def pointed_folder(home, path, uid):
    """The open folder the home's pointer names, once both are this user's own and
    the folder is the one Hermes makes for this home (`control_socket.py`, `_home_hash`)."""
    try:
        # No waiting on a pipe put where the pointer should be, and no links.
        pointer = os.open(POINTER, READ | os.O_NONBLOCK, dir_fd=home)
    except OSError as error:
        if error.errno == errno.ENOENT:
            raise Refusal("no-socket")
        link = error.errno in (errno.ELOOP, errno.EMLINK)
        raise Refusal("pointer-is-a-link" if link else "pointer-not-yours")
    try:
        private(os.fstat(pointer), uid, stat.S_ISREG, "pointer")
        unshared(allowed(pointer), uid, "pointer")
        named = os.read(pointer, MOST_POINTER + 1)
    finally:
        os.close(pointer)
    if len(named) > MOST_POINTER:
        raise Refusal("pointer-too-long")
    try:
        named = named.decode("utf-8").strip()
    except UnicodeDecodeError:
        raise Refusal("pointer-outside")
    folder, name = os.path.split(named)
    wanted = "hermes-gw-" + hashlib.sha256(path.encode("utf-8")).hexdigest()[:16]
    if not named.startswith("/") or name != POINTED or os.path.basename(folder) != wanted:
        raise Refusal("pointer-outside")
    folder = open_folder(folder, "no-socket", "folder-not-yours")
    private(os.fstat(folder), uid, stat.S_ISDIR, "folder")
    unshared(allowed(folder), uid, "folder")
    return folder


def peer_uid(listener):
    """The user the listener runs as, as the kernel says; None where it cannot."""
    if hasattr(socket, "SO_PEERCRED"):
        told = listener.getsockopt(socket.SOL_SOCKET, socket.SO_PEERCRED, struct.calcsize("3i"))
        return struct.unpack("3i", told)[1]
    try:
        import ctypes

        uid, gid = ctypes.c_uint(), ctypes.c_uint()
        getpeereid = ctypes.CDLL(None, use_errno=True).getpeereid
        failed = getpeereid(listener.fileno(), ctypes.byref(uid), ctypes.byref(gid))
    except (ImportError, AttributeError, OSError):
        return None
    return None if failed else uid.value


def connect(path, uid):
    """A connection to the gateway of the home at `path`, once its listener is this user's."""
    home = open_home(path, uid)
    folder, name = home, SOCKET
    if not socket_in(home, SOCKET, uid):
        folder, name = pointed_folder(home, path, uid), POINTED
        if not socket_in(folder, name, uid):
            raise Refusal("no-socket")
    # Found from the folder that was checked, wherever its path leads now.
    os.fchdir(folder)
    listener = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
    try:
        listener.connect(name)
    except OSError:
        raise Refusal("nobody-listening")
    told = peer_uid(listener)
    if told is None:
        raise Refusal("no-peer-credentials")
    if told != uid:
        raise Refusal("listener-not-yours")
    return listener


def request(line):
    """The one line to send, when it asks for one of the two verbs."""
    try:
        asked = json.loads(line)
    except ValueError:
        asked = None
    known = isinstance(asked, dict) and asked.get("verb") in VERBS
    if not known or "\n" in line or "\r" in line:
        raise Refusal("verb-not-sent")
    return line.encode("utf-8") + b"\n"


def exchange(path, line):
    """Sends the line and gives back the one line answered."""
    line = request(line)
    listener = connect(path, os.getuid())
    # Nothing is sent before the listener is known to be this user's.
    listener.sendall(line)
    answer = b""
    while b"\n" not in answer:
        chunk = listener.recv(min(65536, MOST_ANSWER + 1 - len(answer)))
        if not chunk:
            raise Refusal("answer-cut-short" if answer else "empty-answer")
        answer += chunk
        if len(answer) > MOST_ANSWER:
            raise Refusal("answer-too-long")
    return answer[: answer.index(b"\n") + 1]


def say(data):
    """Writes to standard output. A channel that is gone ends the relay."""
    try:
        while data:
            data = data[os.write(1, data) :]
    except OSError:
        os._exit(REFUSED)


def main():
    try:
        seconds, path, line = float(sys.argv[1]), sys.argv[2], sys.argv[3]
    except (IndexError, ValueError):
        seconds = 0.0
    if len(sys.argv) != 4 or not 0.0 < seconds <= MOST_SECONDS:
        say(b"usage\n")
        os._exit(REFUSED)
    watchdog(seconds)
    try:
        answer = exchange(path, line)
    except Refusal as refusal:
        say(str(refusal).encode("ascii") + b"\n")
        os._exit(REFUSED)
    except Exception:  # what failed is not said: it could hold an answer
        say(b"failed\n")
        os._exit(REFUSED)
    say(b"ok\n" + answer)
    os._exit(DONE)


# The last line, so a program cut short on its way here runs nothing.
main()
