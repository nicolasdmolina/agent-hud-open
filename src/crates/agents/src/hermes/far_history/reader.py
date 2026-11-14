"""Agent HUD's reader of a far Hermes home's history. One bounded read.

It is sent on standard input (`python3 -I - MARK DEADLINE HOME REQUEST`)
and is never a file on the far disk. It reads `HOME/state.db` where that
file is, with its write-ahead log, through a connection that cannot change
it, and runs nothing but a SELECT. It imports nothing of Hermes's.

Order of work, which is the contract `far_history.rs` checks:

1. One reader at a time for a home. MARK is one of this process's own
   arguments, so the process list shows every reader. Another one with a
   lower process id is alive: answer `busy` and open nothing. Only ones
   with a higher id: they leave when they see this one, so wait for that,
   and answer `busy` if they stay. The database is opened only after a
   listing that showed no other reader.
2. Write `started`. The deadline counts from here, by this clock.
3. Read, with everything kept in memory, and close the database.
4. Only then write the rows and the last line, which carries their counts.

A watchdog thread ends the process at the deadline whatever it is doing,
a query or a blocked write included, and a timer of the system's stands
behind it. A write that fails means the channel is gone: end at once. The
end of this text on standard input is how the program arrives, not that.
Exit status: 0 said what it had to; 1 crashed (one line on standard
error, with no row text); 3 the deadline; 4 the channel was gone.
"""
import hashlib
import json
import math
import os
import signal
import stat
import subprocess
import sys
import threading
import time
import urllib.parse

MARK = "agent-hud-far-history:"
MOST_USAGE = 500  # the usage rows of one session the HUD's reader accepts
MOST_HELD = 256 * 1024
TIE_WAIT = 3.0  # how long a reader waits for higher process ids to leave
TOKENS = ("input_tokens", "output_tokens", "cache_read_tokens", "cache_write_tokens")
SESSION = ("title", "cwd", "source", "model", "billing_provider", "billing_base_url",
           "billing_mode") + TOKENS + ("ended_at", "last_activity_at", "profile_name")
USAGE = ("model", "billing_provider", "billing_base_url", "billing_mode", "task") \
    + TOKENS + ("first_seen", "last_seen")
END = [0.0]  # when the watchdog ends this process, on the monotonic clock


class TooLarge(Exception):
    """What was read does not fit the output bound."""


def watchdog():
    while True:
        left = END[0] - time.monotonic()
        if left <= 0:
            os._exit(3)
        time.sleep(min(left, 0.2))


def arm(seconds):
    END[0] = time.monotonic() + seconds
    if hasattr(signal, "setitimer"):
        signal.setitimer(signal.ITIMER_REAL, seconds + 1.0)


def line(parts):
    return (json.dumps(parts, separators=(",", ":")) + "\n").encode()


def say(data):
    view = memoryview(data)
    try:
        while view:
            view = view[os.write(1, view):]
    except OSError:
        os._exit(4)


def others(mark):
    """This user's other processes that carry `mark` as one argument."""
    ps = "/bin/ps" if os.path.exists("/bin/ps") else "/usr/bin/ps"
    listed = subprocess.run(
        [ps, "-A", "-ww", "-o", "pid=,uid=,args="], stdin=subprocess.DEVNULL,
        stdout=subprocess.PIPE, stderr=subprocess.DEVNULL, timeout=10,
        env={"LC_ALL": "C", "PATH": "/usr/bin:/bin"})
    if listed.returncode != 0:
        raise OSError("ps")
    me, uid = os.getpid(), str(os.getuid())
    rows = (row.split() for row in listed.stdout.decode("utf-8", "replace").splitlines())
    return [int(part[0]) for part in rows
            if len(part) > 2 and part[1] == uid and mark in part[2:] and int(part[0]) != me]


def alone(mark):
    until = time.monotonic() + TIE_WAIT
    while True:
        found = others(mark)
        if not found:
            return True
        if min(found) < os.getpid() or time.monotonic() >= until:
            return False
        time.sleep(0.05)


def cell(value):
    """A value as JSON carries it: text, a whole number, a finite number, or none."""
    if isinstance(value, bytes) or (isinstance(value, float) and not math.isfinite(value)):
        return None
    return value


def columns(cursor, table):
    return {row[0] for row in cursor.execute("SELECT name FROM pragma_table_info(?)", (table,))}


def col(have, alias, key, fallback="NULL"):
    return "%s.%s" % (alias, key) if key in have else fallback


def tokens(have, alias, wrap):
    return "||'/'||".join(wrap.replace("{}", col(have, alias, key, "0")) for key in TOKENS)


def print_sql(s, u):
    """A session's fingerprint as one text: its two activity stamps, then the
    counts exactly as the HUD's own reader takes them (`session_counts.rs`)."""
    usage = "''"
    if "session_id" in u:
        usage = ("(SELECT COUNT(*)||'/'||%s||'/'||IFNULL(MAX(%s),'') FROM session_model_usage u"
                 " WHERE u.session_id=s.id)") % (tokens(u, "u", "TOTAL({})"), col(u, "u", "last_seen"))
    return "IFNULL(%s,'')||'/'||IFNULL(%s,'')||'/'||%s||'/'||%s" % (
        col(s, "s", "ended_at"), col(s, "s", "last_activity_at"),
        tokens(s, "s", "COALESCE({},0)"), usage)


def prints(cursor, ask, s, u, most):
    """A page of the table by rowid: each session's id and fingerprint. With
    `since`, only the sessions active from then on are returned; the page
    still ends at the `n`th row looked at."""
    after, since = int(ask.get("after", 0)), ask.get("since")
    count = max(1, min(int(ask.get("n", 100)), 500))
    mark, args = print_sql(s, u), [after, count]
    if since is not None:
        seen = "NULL"
        if "session_id" in u and "last_seen" in u:
            seen = "(SELECT MAX(last_seen) FROM session_model_usage WHERE session_id=s.id)"
        mark = "CASE WHEN MAX(IFNULL(%s,0),IFNULL(%s,0),IFNULL(s.started_at,0),IFNULL(%s,0))>=?3 THEN %s END" % (
            col(s, "s", "ended_at"), col(s, "s", "last_activity_at"), seen, mark)
        args.append(float(since))
    body, size, looked, last = [], 0, 0, after
    rows = cursor.execute(
        "SELECT s.rowid,s.id,%s FROM sessions s WHERE s.rowid>?1 ORDER BY s.rowid LIMIT ?2" % mark, args)
    for rowid, name, counts in rows:
        looked, last = looked + 1, rowid
        if isinstance(name, str) and name and isinstance(counts, str):
            body.append(line(["p", rowid, name, counts]))
            size += len(body[-1])
            if size > most:
                raise TooLarge()
    return body, [len(body), 0, looked, last]


def sessions(cursor, ask, s, u, most):
    """Each asked session's row and every one of its usage rows, in one
    statement, so both are of one moment. A session with more usage rows
    than the HUD's reader accepts is named as skipped, with no row of it."""
    ids = [name for name in ask.get("ids", []) if isinstance(name, str)][:200]
    marks = ",".join("?" * len(ids))
    sql = "SELECT 0,s.rowid,s.id,s.started_at,%s,%s FROM sessions s WHERE s.id IN (%s)" % (
        ",".join(col(s, "s", key) for key in SESSION), print_sql(s, u), marks)
    args = list(ids)
    if "session_id" in u and "model" in u:
        sql += " UNION ALL SELECT 1,NULL,u.session_id,%s,NULL,NULL,NULL,NULL,NULL FROM session_model_usage u WHERE u.session_id IN (%s)" % (
            ",".join(col(u, "u", key) for key in USAGE), marks)
        args += ids
    rows, usage, skipped, size = {}, {}, set(), 0
    for row in cursor.execute(sql, args):
        name = row[2]
        if not isinstance(name, str) or name in skipped:
            continue
        if row[0] == 0:
            made = rows[name] = line(["s", row[1], name] + [cell(value) for value in row[3:18]] + [row[18]])
        elif len(usage.setdefault(name, [])) >= MOST_USAGE:
            skipped.add(name)
            size -= sum(len(one) for one in usage.pop(name))
            continue
        else:
            made = line(["u", name] + [cell(value) for value in row[3:14]])
            usage[name].append(made)
        size += len(made)
        if size > most:
            raise TooLarge()
    body, told, counted = [], 0, 0
    for name in ids:
        if name in skipped and name in rows:
            body.append(line(["skip", name, "usage-rows"]))
        elif name in rows:
            body.append(rows[name])
            body += usage.get(name, [])
            told, counted = told + 1, counted + len(usage.get(name, []))
    return body, [told, counted, 0, 0]


def database(ask, home, sqlite3, most):
    """One read of `state.db`. The database is closed when this returns."""
    path, none = os.path.join(home, "state.db"), [0, 0, 0, 0]
    if not os.path.isfile(path):
        return [], "no-database", none
    try:
        db = sqlite3.connect("file:" + urllib.parse.quote(path) + "?mode=ro", uri=True, timeout=2.0)
    except sqlite3.Error:
        return [], "db-open", none
    cursor = None
    try:
        db.text_factory = lambda raw: raw.decode("utf-8", "replace")
        cursor = db.cursor()
        s, u = columns(cursor, "sessions"), columns(cursor, "session_model_usage")
        if "id" not in s or "started_at" not in s:
            return [], "db-schema", none
        body, counts = (prints if ask["k"] == "prints" else sessions)(cursor, ask, s, u, most)
        return body, "ok", counts
    except TooLarge:
        return [], "too-large", none
    except sqlite3.Error as error:
        for word, name in (("locked", "db-locked"), ("unable to open", "db-open"), ("no such", "db-schema"),
                           ("malformed", "db-corrupt"), ("not a database", "db-corrupt")):
            if word in str(error):
                return [], name, none
        return [], "db-error", none
    finally:
        # The statement is let go before the connection: an unfinished one
        # would keep its read transaction past `close`.
        if cursor is not None:
            cursor.close()
        db.close()


def held(home):
    """`runtime/active_sessions.json` as it is, for the caller to read as
    untrusted text. Missing, too long or not JSON: none of it."""
    path = os.path.join(home, "runtime", "active_sessions.json")
    try:
        handle = os.open(path, os.O_RDONLY | os.O_NONBLOCK | getattr(os, "O_NOFOLLOW", 0))
        try:
            if not stat.S_ISREG(os.fstat(handle).st_mode):
                raise ValueError("kind")
            raw, more = b"", b"-"
            while more and len(raw) <= MOST_HELD:
                more = os.read(handle, MOST_HELD + 1 - len(raw))
                raw += more
        finally:
            os.close(handle)
        text = raw.decode("utf-8")
        if len(raw) > MOST_HELD or json.loads(text) is None:
            raise ValueError("size")
    except (OSError, ValueError):
        return [], "none", [0, 0, 0, 0]
    return [line(["held", text])], "ok", [0, 0, 0, 0]


def main():
    mark, deadline, home, request = sys.argv[1:5]
    deadline = max(0.5, min(float(deadline), 120.0))
    arm(deadline)
    threading.Thread(target=watchdog, daemon=True).start()
    ask = json.loads(request)
    digest = hashlib.sha256(home.encode("utf-8", "surrogateescape")).hexdigest()[:16]
    if mark != MARK + digest or os.path.realpath(home) != home or ask.get("k") not in (
            "probe", "prints", "sessions", "held"):
        return say(line(["no", "request"]))
    try:
        lone = alone(mark)
    except (OSError, ValueError, subprocess.SubprocessError):
        return say(line(["no", "processes"]))
    if not lone:
        return say(line(["no", "busy"]))
    try:
        import sqlite3
    except ImportError:
        return say(line(["no", "sqlite"]))
    arm(deadline)
    say(line(["started", os.getpid()]))
    most = max(1024, min(int(ask.get("most", 1 << 20)), 8 << 20))
    body, status, counts = [], "ok", [0, 0, 0, 0]
    if ask["k"] == "held":
        body, status, counts = held(home)
    elif ask["k"] != "probe":
        body, status, counts = database(ask, home, sqlite3, most)
    say(b"".join(body) + line(["end", status] + counts))


try:
    main()
except Exception as error:  # noqa: BLE001 - whatever it is, it is said without its text
    os.write(2, ("far-history: crash %s\n" % type(error).__name__).encode())
    os._exit(1)
