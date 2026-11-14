"""Agent HUD's control relay: one exchange with one Hermes gateway's
control socket, made on the machine the gateway runs on.

It is sent on standard input and never written to this machine's disk:

    python3 -I -S -B - <seconds> <home> <request line>

It answers on standard output with one status line, and after `ok` the one
line the gateway answered. Everything else is a named refusal.

Before it sends anything it checks that the socket is the login user's
own, as Hermes's own client does (`hermes_cli/gateway_runtime_discovery.py`,
which is not imported: it can change a lock's permissions):

- the home is canonical, a folder of this user, and no one else may write it;
- `gateway.sock` in it is a socket of this user, closed to everyone else;
- or the pointer file beside it is read without following a link, is this
  user's and closed to others, is short, and names `control.sock` in the
  folder Hermes makes for this very home, which is checked the same way;
- the process that listens runs as this user, as the kernel says.

The checks are made on open descriptors. The home is opened once and what
is in it is looked up from that descriptor, so a path that is swapped for
another after a check leads nowhere new. Where the kernel cannot say who
listens, the relay refuses: it never falls back to the path checks alone.

What that proves: the listener runs as the login user. It cannot tell two
processes of that user apart; same user is the trust boundary.

It opens nothing for writing, imports nothing but the standard library,
and is gone `seconds` after it started, whatever it is doing then.
"""
import errno
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

try:
    import fcntl
except ImportError:  # never on macOS or Linux
    fcntl = None

SOCKET = "gateway.sock"
POINTER = "gateway.sock.path"
POINTED = "control.sock"
VERBS = ("identify", "session-ticket")
# The longest answer passed on; Hermes's own client stops at the same size.
MOST_ANSWER = 512 * 1024
# The longest path a pointer file may name.
MOST_POINTER = 4096
MOST_SECONDS = 60.0
# How this process ends: an answer, a named refusal, its deadline.
DONE, REFUSED, DEADLINE = 0, 2, 3
READ = os.O_RDONLY | os.O_NOFOLLOW | getattr(os, "O_CLOEXEC", 0)
FOLDER = READ | getattr(os, "O_DIRECTORY", 0)


class Refusal(Exception):
    """A named reason nothing was exchanged."""


def watchdog(seconds):
    """Ends this process `seconds` from now, whatever it is doing."""
    def end():
        time.sleep(seconds)
        os._exit(DEADLINE)

    thread = threading.Thread(target=end)
    thread.daemon = True
    thread.start()
    # The kernel's own timer, should the thread never get to run.
    signal.alarm(int(seconds) + 2)


def held_path(descriptor, path):
    """Where an open folder really is, its links resolved."""
    if fcntl is not None and hasattr(fcntl, "F_GETPATH"):
        held = fcntl.fcntl(descriptor, fcntl.F_GETPATH, bytes(1024))
        return os.fsdecode(held.split(b"\0", 1)[0])
    link = "/proc/self/fd/%d" % descriptor
    return os.readlink(link) if os.path.islink(link) else os.path.realpath(path)


def open_folder(path, missing, what):
    """An open descriptor of the folder at `path`, which is no link."""
    try:
        return os.open(path, FOLDER)
    except OSError as error:
        if error.errno == errno.ENOENT:
            raise Refusal(missing)
        raise Refusal(what)


def open_home(path, uid):
    """The home, open, once it is canonical, this user's, and closed to
    writing by anyone else."""
    if not path.startswith("/"):
        raise Refusal("home-not-canonical")
    home = open_folder(path, "no-home", "home-not-canonical")
    node = os.fstat(home)
    if held_path(home, path) != path:
        raise Refusal("home-not-canonical")
    if not stat.S_ISDIR(node.st_mode) or node.st_uid != uid:
        raise Refusal("home-not-yours")
    if stat.S_IMODE(node.st_mode) & 0o022:
        raise Refusal("home-open-to-others")
    return home


def private(node, uid, kind, what):
    """Refuses a node that is not of `kind`, not this user's, or open to
    anyone else."""
    if not kind(node.st_mode) or node.st_uid != uid:
        raise Refusal(what + "-not-yours")
    if stat.S_IMODE(node.st_mode) & 0o077:
        raise Refusal(what + "-open-to-others")


def socket_in(folder, name, uid):
    """Whether `name` in the open `folder` is there, once it is a private
    socket of this user. It is looked up from the descriptor, not a path."""
    try:
        node = os.stat(name, dir_fd=folder, follow_symlinks=False)
    except OSError as error:
        if error.errno == errno.ENOENT:
            return False
        raise Refusal("socket-not-yours")
    private(node, uid, stat.S_ISSOCK, "socket")
    return True


def pointed_folder(home, path, uid):
    """The open folder the home's pointer file names, once the pointer and
    the folder are this user's own and the folder is the one Hermes makes
    for this home (`gateway/control_socket.py`, `_home_hash`)."""
    try:
        # No waiting on a pipe put where the pointer should be, and no links.
        pointer = os.open(POINTER, READ | os.O_NONBLOCK, dir_fd=home)
    except OSError as error:
        if error.errno == errno.ENOENT:
            raise Refusal("no-socket")
        if error.errno in (errno.ELOOP, errno.EMLINK):
            raise Refusal("pointer-is-a-link")
        raise Refusal("pointer-not-yours")
    try:
        private(os.fstat(pointer), uid, stat.S_ISREG, "pointer")
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
    return folder


def peer_uid(listener):
    """The user the process at the other end runs as, as the kernel says.
    None where this system has no way to ask."""
    if hasattr(socket, "SO_PEERCRED"):
        size = struct.calcsize("3i")
        told = listener.getsockopt(socket.SOL_SOCKET, socket.SO_PEERCRED, size)
        return struct.unpack("3i", told)[1]
    try:
        import ctypes

        getpeereid = ctypes.CDLL(None, use_errno=True).getpeereid
    except (ImportError, AttributeError, OSError):
        return None
    unsigned = ctypes.POINTER(ctypes.c_uint)
    getpeereid.argtypes = [ctypes.c_int, unsigned, unsigned]
    getpeereid.restype = ctypes.c_int
    uid, gid = ctypes.c_uint(), ctypes.c_uint()
    if getpeereid(listener.fileno(), ctypes.byref(uid), ctypes.byref(gid)) != 0:
        return None
    return uid.value


def connect(path, uid):
    """A connection to the gateway of the home at `path`, whose listener
    runs as this user."""
    home = open_home(path, uid)
    folder, name = home, SOCKET
    if not socket_in(home, SOCKET, uid):
        folder, name = pointed_folder(home, path, uid), POINTED
        if not socket_in(folder, name, uid):
            raise Refusal("no-socket")
    # The name is found from the folder that was checked, wherever its
    # path leads by now.
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
        raise Refusal("verb-not-sent")
    if not isinstance(asked, dict) or asked.get("verb") not in VERBS:
        raise Refusal("verb-not-sent")
    if "\n" in line or "\r" in line:
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
        seconds = float(sys.argv[1])
        path, line = sys.argv[2], sys.argv[3]
    except (IndexError, ValueError):
        seconds, path, line = 0.0, "", ""
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
