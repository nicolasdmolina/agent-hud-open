//! A pretend gateway on real sockets: a control socket in a Hermes home
//! that answers `identify` and grants tickets, and a WebSocket server that
//! takes a ticket once and answers the calls the connection makes, by
//! Hermes's own rules (`gateway/session_events.py`, `since`;
//! `gateway/session_active_list.py`, `_status`). What it says is what
//! Hermes 0.22.5 was recorded saying: the fixtures in
//! `agents/tests/fixtures/hermes/gateway/`. A test plays a recording into
//! it up to some frame, and looks at the cards that follow.
use serde_json::{json, Value};
use std::{
    collections::BTreeMap,
    io::{BufRead, BufReader, Write},
    net::{TcpListener, TcpStream},
    os::unix::{fs::PermissionsExt, net::UnixListener},
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, AtomicUsize, Ordering},
        Arc, Mutex, MutexGuard,
    },
    thread,
    time::{Duration, Instant},
};
use tungstenite::{
    handshake::server::{ErrorResponse, Request, Response},
    http::{header::SEC_WEBSOCKET_PROTOCOL, HeaderValue, StatusCode},
    Message,
};

/// The profile every scenario was recorded in.
pub const PROFILE: &str = "hud-spike";

pub fn fixture(name: &str) -> Value {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../agents/tests/fixtures/hermes/gateway")
        .join(format!("{name}.json"));
    let text = std::fs::read_to_string(&path).unwrap_or_else(|error| panic!("{path:?}: {error}"));
    serde_json::from_str(&text).unwrap()
}

/// One session's stream, as the model's own pretend gateway keeps it.
#[derive(Default)]
pub struct Stream {
    pub epoch: String,
    pub events: Vec<Value>,
    pub latest: u64,
    pub prompts: Vec<Value>,
    pub open: Vec<Value>,
    pub running: bool,
}

impl Stream {
    fn status(&self) -> &'static str {
        match (self.prompts.is_empty(), self.running) {
            (false, _) => "waiting",
            (true, true) => "working",
            (true, false) => "idle",
        }
    }
    fn since(&self, epoch: Option<&str>, after: u64) -> Value {
        let missing = epoch != Some(&self.epoch) || after > self.latest;
        let events: Vec<&Value> = self
            .events
            .iter()
            .filter(|event| !missing && event["seq"].as_u64() > Some(after))
            .collect();
        json!({
            "events": events, "count": events.len(),
            "latest_seq": self.latest, "last_sequence": self.latest,
            "epoch": self.epoch, "replay_epoch": self.epoch,
            "truncated": missing, "snapshot_required": missing,
            "open_requests": self.open,
        })
    }
    /// Takes one event as Hermes stamped it.
    fn publish(&mut self, event: &Value) {
        let payload = &event["payload"];
        match event["type"].as_str().unwrap_or("") {
            "clarify.request" | "approval.request" => self.prompts.push(payload.clone()),
            "clarify.settled" | "approval.settled" => self
                .prompts
                .retain(|prompt| prompt["prompt_id"] != payload["prompt_id"]),
            "request.cancel" => self.open.retain(|open| open["id"] != payload["id"]),
            "message.start" => self.running = true,
            "message.complete" => {
                self.running = false;
                self.prompts.clear();
            }
            _ => {}
        }
        if let Some(epoch) = event["replay_epoch"].as_str() {
            self.epoch = epoch.to_owned();
        }
        self.latest = event["seq"].as_u64().unwrap_or(self.latest + 1);
        self.events.push(event.clone());
    }
}

#[derive(Default)]
pub struct Hold {
    pub entered: AtomicBool,
    pub released: AtomicBool,
}

#[derive(Default)]
pub struct World {
    /// Once `identify` is next asked, and before it is answered, the link
    /// at the first path is made to lead to the second: a home that
    /// changes while its gateway is asked (the first review of ticket 25).
    pub switch_after_identify: Option<(PathBuf, PathBuf)>,
    /// The socket that next subscribes stops reading after its resume is
    /// answered, until released: a peer that takes nothing more in.
    pub stall_after_resume: Option<Arc<Hold>>,
    /// The sockets the gateway is done with, by number: closed, cut or
    /// reset.
    pub ended: Vec<usize>,
    pub hint_on_resume: bool,
    pub hold_next: Option<(&'static str, Arc<Hold>)>,
    /// A second reply held at the same time, by its method.
    pub hold_too: Option<(&'static str, Arc<Hold>)>,
    /// What `identify` answers; None closes the socket without a word.
    pub identify: Option<Value>,
    pub instance: String,
    /// Whether the control socket knows `session-ticket`.
    pub grants: bool,
    /// Tickets granted and not yet used.
    unspent: Vec<String>,
    /// Every ticket ever granted, for looking for one where none belongs.
    pub granted: Vec<String>,
    pub streams: BTreeMap<(String, String), Stream>,
    /// Every frame a socket sent, with the number of the socket.
    pub received: Vec<(usize, Value)>,
    /// How many sockets were opened, and how many were turned away.
    pub sockets: usize,
    pub turned_away: usize,
    /// Events for every socket, such as `sessions.changed`.
    said: Vec<Value>,
    /// How long each reply is held back.
    pub slow: Duration,
    /// The most `session.active_list` calls of one profile ever unanswered
    /// at once: more than one means two polls ran together.
    pub lists_at_once: usize,
    listing: BTreeMap<String, usize>,
    /// What to answer `clarify.respond` and `approval.respond` with, in
    /// place of what Hermes would: an error reason, `silence`, or
    /// `already_resolved` as when someone else answered a moment before.
    pub answers_with: Option<&'static str>,
    /// Counts the times every open socket was cut with the gateway still up.
    cuts: usize,
    /// The profiles the gateway serves, by name; None for one that takes
    /// a call for any profile, as the first pretend gateway did.
    pub serves: Option<Vec<String>>,
    /// Every profile a call named that the gateway does not serve, which
    /// it refused as `profile_mismatch`.
    pub mismatched: Vec<String>,
    /// The control sockets that were closed while the gateway runs.
    closed: Vec<PathBuf>,
    /// What `profiles.list` reports as the install's id; None for an
    /// answer that names none.
    pub install: Option<Value>,
    /// One session whose events are slow to read: each read of them is
    /// held back this long.
    pub slow_replay: Option<(String, Duration)>,
    /// When each `session.active_list` arrived: when each poll began.
    pub listed: Vec<Instant>,
    /// Sessions the gateway holds and leaves out of its live list.
    pub unlisted: Vec<String>,
}

impl World {
    pub fn stream(&mut self, session: &str) -> &mut Stream {
        self.stream_in(PROFILE, session)
    }
    /// The stream of a session of `profile`, by the profile's name.
    pub fn stream_in(&mut self, profile: &str, session: &str) -> &mut Stream {
        self.streams
            .entry((profile.to_owned(), session.to_owned()))
            .or_default()
    }
    /// Whether the gateway is done with the socket that subscribed last:
    /// the one an answer or a snapshot went on.
    pub fn visit_over(&self) -> bool {
        let subscribed = self.calls("session.resume");
        subscribed
            .last()
            .is_some_and(|(socket, _)| self.ended.contains(socket))
    }
    /// Every call of `method` received, with the socket it came on.
    pub fn calls(&self, method: &str) -> Vec<(usize, Value)> {
        let named = |(_, frame): &&(usize, Value)| frame["method"] == method;
        self.received.iter().filter(named).cloned().collect()
    }
}

pub struct Fake {
    pub home: PathBuf,
    /// Every control socket it listens on.
    sockets: Vec<PathBuf>,
    world: Arc<Mutex<World>>,
    running: Arc<AtomicBool>,
    workers: Vec<thread::JoinHandle<()>>,
}

impl Fake {
    /// Starts a gateway whose home is `home`, a private folder, as instance
    /// `instance`. It serves the default profile and `hud-spike`.
    pub fn start(home: &Path, instance: &str) -> Self {
        let root = home.to_string_lossy().into_owned();
        let named = format!("{root}/profiles/{PROFILE}");
        Self::listening(home, instance, &[home.to_path_buf()], &[root, named], None)
    }

    /// Starts a gateway in the Hermes home `root` that serves the homes at
    /// `profiles` and nothing else, each by its place there (`default` is
    /// `root` itself, any other name `root/profiles/<name>`), with a
    /// control socket in each. A machine with one gateway for each profile
    /// runs several of these in one root. What it calls each profile on
    /// the wire is what Hermes calls it (`hermes_name`).
    pub fn serving(root: &Path, instance: &str, profiles: &[&str]) -> Self {
        let homes: Vec<PathBuf> = profiles.iter().map(|name| home_of(root, name)).collect();
        for home in &homes {
            crate::os::private_directory(home).unwrap();
        }
        let served: Vec<String> = homes
            .iter()
            .map(|home| home.to_string_lossy().into_owned())
            .collect();
        let names = homes.iter().map(|home| hermes_name(home)).collect();
        Self::listening(root, instance, &homes, &served, Some(names))
    }

    fn listening(
        home: &Path,
        instance: &str,
        sockets: &[PathBuf],
        served: &[String],
        serves: Option<Vec<String>>,
    ) -> Self {
        let web = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let port = web.local_addr().unwrap().port();
        let said = fixture("gateway-self-description");
        // The recorded answer of a ready gateway with two profiles, with
        // this home, this instance and this port in it.
        let mut identify = said["control_socket"]["identify_throwaway_ready_two_profiles"].clone();
        identify["result"]["hermes_home"] = json!(home.to_string_lossy());
        identify["result"]["instance_id"] = json!(instance);
        identify["result"]["api_origin"] = json!(format!("http://127.0.0.1:{port}"));
        let served: Vec<Value> = served
            .iter()
            .map(|home| json!({"profile_id": home, "home": home}))
            .collect();
        identify["result"]["served_profiles"] = json!(served);
        let world = Arc::new(Mutex::new(World {
            identify: Some(identify),
            instance: instance.to_owned(),
            grants: true,
            serves,
            ..World::default()
        }));
        let running = Arc::new(AtomicBool::new(true));
        web.set_nonblocking(true).unwrap();
        let sockets: Vec<PathBuf> = sockets
            .iter()
            .map(|home| home.join("gateway.sock"))
            .collect();
        let mut workers = Vec::new();
        for path in &sockets {
            let _ = std::fs::remove_file(path);
            // Bound under a short name and moved, so a home whose path is
            // long (a far lab's) can still hold its socket.
            static BOUND: AtomicUsize = AtomicUsize::new(0);
            let short = home.join(format!(".s{}", BOUND.fetch_add(1, Ordering::Relaxed)));
            let _ = std::fs::remove_file(&short);
            let control = UnixListener::bind(&short).unwrap();
            std::fs::set_permissions(&short, std::fs::Permissions::from_mode(0o600)).unwrap();
            std::fs::rename(&short, path).unwrap();
            control.set_nonblocking(true).unwrap();
            let (world, running, path) = (world.clone(), running.clone(), path.clone());
            workers.push(thread::spawn(move || {
                serve_control(control, world, running, path)
            }));
        }
        let (served, alive) = (world.clone(), running.clone());
        workers.push(thread::spawn(move || serve_web(web, served, alive)));
        Self {
            home: home.to_path_buf(),
            sockets,
            world,
            running,
            workers,
        }
    }

    /// Serves `profiles` from now on, as its answer to `identify` and to
    /// every call says. The control socket of a profile it no longer
    /// serves is closed and gone.
    pub fn serve(&self, profiles: &[&str]) {
        let homes: Vec<PathBuf> = profiles
            .iter()
            .map(|name| home_of(&self.home, name))
            .collect();
        let served: Vec<Value> = (homes.iter().map(|home| home.to_string_lossy()))
            .map(|home| json!({"profile_id": home, "home": home}))
            .collect();
        let mut world = self.world();
        world.identify.as_mut().unwrap()["result"]["served_profiles"] = json!(served);
        world.serves = Some(homes.iter().map(|home| hermes_name(home)).collect());
        let kept = |socket: &&PathBuf| {
            !homes
                .iter()
                .any(|home| home.join("gateway.sock") == **socket)
        };
        let closed: Vec<PathBuf> = self.sockets.iter().filter(kept).cloned().collect();
        world.closed.extend(closed);
    }

    /// What the gateway calls the profile whose home is at `place`.
    pub fn named(&self, place: &str) -> String {
        hermes_name(&home_of(&self.home, place))
    }

    /// A question waits in a session of the profile at `profile`, in a
    /// turn under way.
    pub fn asks(&self, profile: &str, session: &str, prompt: &str) {
        self.asks_quietly(profile, session, prompt);
        self.say("sessions.changed");
    }

    /// The same, with no hint from the gateway that anything changed.
    pub fn asks_quietly(&self, profile: &str, session: &str, prompt: &str) {
        let profile = &self.named(profile);
        let event = |kind: &str, seq: u64, payload: Value| {
            json!({"type": kind, "session_id": session, "replay_epoch": "epoch-1", "seq": seq,
                "execution_generation": 1, "payload": payload})
        };
        let asked = json!({"kind": "clarify", "prompt_id": prompt, "execution_generation": 1,
            "question": "Which one?", "choices": ["one", "two"], "multi_select": false,
            "timeout_s": 600});
        let mut world = self.world();
        let stream = world.stream_in(profile, session);
        let next = stream.latest + 1;
        if !stream.running {
            stream.publish(&event("message.start", next, json!({})));
        }
        let next = stream.latest + 1;
        stream.publish(&event("clarify.request", next, asked));
    }

    /// One event in a session of the profile at `profile`, stamped as
    /// Hermes stamps it, with no hint that anything changed: a request
    /// settled in another window, a turn that ended.
    pub fn tells_quietly(&self, profile: &str, session: &str, kind: &str, payload: Value) {
        let profile = &self.named(profile);
        let mut world = self.world();
        let stream = world.stream_in(profile, session);
        let event = json!({"type": kind, "session_id": session, "replay_epoch": "epoch-1",
            "seq": stream.latest + 1, "execution_generation": 1, "payload": payload});
        stream.publish(&event);
    }

    /// A session of the profile at `profile` is in a turn, or not.
    pub fn runs(&self, profile: &str, session: &str, running: bool) {
        let profile = &self.named(profile);
        {
            let mut world = self.world();
            let stream = world.stream_in(profile, session);
            stream.epoch = "epoch-1".into();
            stream.running = running;
        }
        self.say("sessions.changed");
    }

    pub fn world(&self) -> MutexGuard<'_, World> {
        self.world.lock().unwrap()
    }

    /// Plays the recording `name` into the gateway, frame by frame as the
    /// recorded socket got them, up to and including the first event of
    /// kind `through` (or all of it). Returns the session it is about.
    pub fn play(&self, name: &str, through: Option<&str>) -> String {
        let recorded = fixture(name);
        let mut world = self.world();
        let mut session = String::new();
        for entry in recorded["clients"]["hud"].as_array().unwrap() {
            let frame = &entry["frame"];
            if entry["dir"] != "in" {
                continue;
            }
            let params = &frame["params"];
            if frame["method"] == "event" {
                let Some(id) = params["session_id"].as_str().filter(|id| !id.is_empty()) else {
                    continue;
                };
                session = id.to_owned();
                // Played once: a later call to go further skips what is in.
                let seq = params["seq"].as_u64().unwrap_or(0);
                if seq > world.stream(id).latest {
                    world.stream(id).publish(params);
                }
                if through == params["type"].as_str() {
                    break;
                }
            } else if frame["id"]
                .as_str()
                .is_some_and(|id| id.starts_with("srq-"))
            {
                // A set of questions, which Hermes lists as open.
                let id = params["session_id"].as_str().unwrap().to_owned();
                let listed = world
                    .stream(&id)
                    .open
                    .iter()
                    .any(|open| open["id"] == frame["id"]);
                if !listed {
                    let open =
                        json!({"id": frame["id"], "method": frame["method"], "params": params});
                    world.stream(&id).open.push(open);
                }
                if through == frame["method"].as_str() {
                    break;
                }
            }
        }
        session
    }

    /// Says an event on every socket, as Hermes does its hints.
    pub fn say(&self, kind: &str) {
        let event =
            json!({"jsonrpc": "2.0", "method": "event", "params": {"type": kind, "payload": {}}});
        self.world().said.push(event);
    }

    /// Cuts every open socket and stays up: a dropped connection.
    pub fn cut(&self) {
        self.world().cuts += 1;
    }

    /// Stops as a killed gateway does: every socket cut, the control socket
    /// gone. The home stays.
    pub fn kill(mut self) {
        self.stop();
    }

    /// The same, with what the gateway received still there to look at.
    pub fn stop(&mut self) {
        self.running.store(false, Ordering::Release);
        for worker in self.workers.drain(..) {
            let _ = worker.join();
        }
        for socket in &self.sockets {
            let _ = std::fs::remove_file(socket);
        }
    }
}

/// What Hermes calls the profile whose home is `home`, and so the only
/// name it takes for it in a call: `<name>` for a home at
/// `…/profiles/<name>`, and `default` for any other. Hermes takes the
/// folder above `profiles` for the root of such a home wherever it lies,
/// so a home configured at `/x/profiles/work` is the profile `work` to
/// it, and never `default` (`hermes_constants.py`, `_hermes_root_of`;
/// `hermes_cli/profiles.py`, `get_profile_dir`, `profile_matches_home`;
/// traced and run against Hermes 0.22.5 in the second review of ticket
/// 26, `evidence/26/verifier-files-2/hermes-routing-source.txt`).
pub fn hermes_name(home: &Path) -> String {
    let inside = home.parent().and_then(Path::file_name);
    let name = home.file_name().and_then(|name| name.to_str());
    let plain = |name: &&str| {
        let letters = |c: char| c.is_ascii_lowercase() || c.is_ascii_digit() || "_-".contains(c);
        !name.is_empty() && name.chars().all(letters) && *name != "default"
    };
    match name.filter(plain) {
        Some(name) if inside.is_some_and(|folder| folder == "profiles") => name.to_owned(),
        _ => "default".to_owned(),
    }
}

/// The home at the place `name` in the Hermes home `root`.
pub fn home_of(root: &Path, name: &str) -> PathBuf {
    match name {
        "default" => root.to_path_buf(),
        name => root.join("profiles").join(name),
    }
}

impl Drop for Fake {
    fn drop(&mut self) {
        self.running.store(false, Ordering::Release);
    }
}

fn serve_control(
    listener: UnixListener,
    world: Arc<Mutex<World>>,
    running: Arc<AtomicBool>,
    path: PathBuf,
) {
    let said = fixture("gateway-self-description");
    while running.load(Ordering::Acquire) {
        if world.lock().unwrap().closed.contains(&path) {
            let _ = std::fs::remove_file(&path);
            return;
        }
        let Ok((stream, _)) = listener.accept() else {
            thread::sleep(Duration::from_millis(5));
            continue;
        };
        stream.set_nonblocking(false).unwrap();
        let mut line = String::new();
        if BufReader::new(&stream).read_line(&mut line).is_err() {
            continue;
        }
        let asked: Value = serde_json::from_str(&line).unwrap_or_default();
        let mut world = world.lock().unwrap();
        let answer = match asked["verb"].as_str() {
            Some("identify") => world.identify.clone(),
            Some("session-ticket") if !world.grants => {
                Some(said["control_socket"]["unknown_verb"].clone())
            }
            Some("session-ticket") if asked["params"]["instance_id"] != world.instance.as_str() => {
                Some(said["control_socket"]["refused_stale_instance"].clone())
            }
            Some("session-ticket") => {
                let ticket = format!("ticket-{}-{}", world.instance, world.granted.len());
                world.unspent.push(ticket.clone());
                world.granted.push(ticket.clone());
                let mut grant = said["control_socket"]["session_ticket_grant"].clone();
                grant["ticket"] = json!(ticket);
                grant["instance_id"] = json!(world.instance);
                grant["profile_id"] = asked["params"]["profile_id"].clone();
                Some(json!({"ok": true, "protocol": 1, "id": 1, "result": grant}))
            }
            _ => Some(said["control_socket"]["unknown_verb"].clone()),
        };
        let switched = if asked["verb"] == "identify" {
            world.switch_after_identify.take()
        } else {
            None
        };
        drop(world);
        if let Some((alias, replacement)) = switched {
            std::fs::remove_file(&alias).unwrap();
            std::os::unix::fs::symlink(replacement, alias).unwrap();
        }
        if let Some(answer) = answer {
            let _ = (&stream).write_all(format!("{answer}\n").as_bytes());
        }
    }
}

fn serve_web(listener: TcpListener, world: Arc<Mutex<World>>, running: Arc<AtomicBool>) {
    let mut sockets = Vec::new();
    while running.load(Ordering::Acquire) {
        let Ok((stream, _)) = listener.accept() else {
            thread::sleep(Duration::from_millis(5));
            continue;
        };
        let (world, running) = (world.clone(), running.clone());
        sockets.push(thread::spawn(move || serve_socket(stream, world, running)));
    }
    for socket in sockets {
        let _ = socket.join();
    }
}

/// One WebSocket: a ticket used once opens it, anything else gets 403.
// The refusal's type is the one tungstenite's handshake takes.
#[allow(clippy::result_large_err)]
fn serve_socket(stream: TcpStream, world: Arc<Mutex<World>>, running: Arc<AtomicBool>) {
    stream.set_nonblocking(false).unwrap();
    let admitted = world.clone();
    let admit = move |request: &Request,
                      mut response: Response|
          -> Result<Response, ErrorResponse> {
        let offered = request
            .headers()
            .get(SEC_WEBSOCKET_PROTOCOL)
            .and_then(|value| value.to_str().ok())
            .unwrap_or("");
        let mut offered = offered.split(',').map(str::trim);
        let speaks = offered.next() == Some("hermes-gateway-v1");
        let ticket = offered
            .next()
            .and_then(|second| second.strip_prefix("hermes-gateway-ticket."));
        let mut world = admitted.lock().unwrap();
        let spent = ticket.and_then(|ticket| world.unspent.iter().position(|held| held == ticket));
        let no_origin = !request.headers().contains_key("origin");
        match spent {
            Some(index) if speaks && no_origin => {
                world.unspent.remove(index);
                world.sockets += 1;
                let spoken = HeaderValue::from_static("hermes-gateway-v1");
                response
                    .headers_mut()
                    .insert(SEC_WEBSOCKET_PROTOCOL, spoken);
                Ok(response)
            }
            _ => {
                world.turned_away += 1;
                let mut refused = ErrorResponse::new(None);
                *refused.status_mut() = StatusCode::FORBIDDEN;
                Err(refused)
            }
        }
    };
    let Ok(mut ws) = tungstenite::accept_hdr(stream, admit) else {
        return;
    };
    let number = world.lock().unwrap().sockets;
    // Counted when this socket is over, however it ends.
    struct Over(Arc<Mutex<World>>, usize);
    impl Drop for Over {
        fn drop(&mut self) {
            self.0.lock().unwrap().ended.push(self.1);
        }
    }
    let _over = Over(world.clone(), number);
    let ready =
        fixture("gateway-self-description")["websocket"]["first_frame_gateway_ready"].clone();
    let _ = ws.send(Message::Text(ready.to_string().into()));
    let _ = ws
        .get_ref()
        .set_read_timeout(Some(Duration::from_millis(10)));
    let mut subscribed: Vec<String> = Vec::new();
    let (mut heard, cuts) = {
        let world = world.lock().unwrap();
        (world.said.len(), world.cuts)
    };
    while running.load(Ordering::Acquire) && world.lock().unwrap().cuts == cuts {
        let said: Vec<Value> = world.lock().unwrap().said[heard..].to_vec();
        heard += said.len();
        for event in said {
            let _ = ws.send(Message::Text(event.to_string().into()));
        }
        let frame: Value = match ws.read() {
            Ok(Message::Text(text)) => serde_json::from_str(text.as_ref()).unwrap(),
            Ok(Message::Close(_)) => return,
            Ok(_) => continue,
            Err(tungstenite::Error::Io(error))
                if matches!(
                    error.kind(),
                    std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                ) =>
            {
                continue
            }
            Err(_) => return,
        };
        let slow = {
            let mut world = world.lock().unwrap();
            world.received.push((number, frame.clone()));
            if frame["method"] == "session.active_list" {
                world.listed.push(Instant::now());
                let profile = frame["params"]["profile"].as_str().unwrap_or("").to_owned();
                let unanswered = world.listing.entry(profile).or_default();
                *unanswered += 1;
                let unanswered = *unanswered;
                world.lists_at_once = world.lists_at_once.max(unanswered);
            }
            // The events of one session, slow to read where a test says so.
            let replay = frame["method"] == "session.events.since"
                && frame["params"].get("replay_epoch").is_some();
            let held = |(session, _): &&(String, Duration)| {
                replay && frame["params"]["session_id"] == session.as_str()
            };
            let slow = world.slow_replay.as_ref().filter(held);
            world.slow + slow.map_or(Duration::ZERO, |(_, slow)| *slow)
        };
        thread::sleep(slow);
        let reply = {
            let mut world = world.lock().unwrap();
            if frame["method"] == "session.active_list" {
                let profile = frame["params"]["profile"].as_str().unwrap_or("").to_owned();
                *world.listing.entry(profile).or_default() -= 1;
            }
            answer(&mut world, &frame, &mut subscribed)
        };
        let hold = {
            let mut world = world.lock().unwrap();
            if world.hold_next.as_ref().is_some_and(|(method, _)| {
                frame["method"] == *method
                    // The first read of a session, which asks where its
                    // stream is and names no epoch.
                    || (*method == "stream-probe"
                        && frame["method"] == "session.events.since"
                        && frame["params"].get("replay_epoch").is_none())
                    || (*method == "nonempty-replay"
                        && frame["method"] == "session.events.since"
                        && reply.as_ref().is_some_and(|r| {
                            r["result"]["events"]
                                .as_array()
                                .is_some_and(|events| !events.is_empty())
                        }))
            }) {
                world.hold_next.take().map(|(_, hold)| hold)
            } else if (world.hold_too.as_ref())
                .is_some_and(|(method, _)| frame["method"] == *method)
            {
                world.hold_too.take().map(|(_, hold)| hold)
            } else {
                None
            }
        };
        if let Some(hold) = hold {
            hold.entered.store(true, Ordering::Release);
            let limit = std::time::Instant::now() + Duration::from_secs(15);
            while !hold.released.load(Ordering::Acquire) && std::time::Instant::now() < limit {
                thread::sleep(Duration::from_millis(2));
            }
        }
        if frame["method"] == "session.resume" && world.lock().unwrap().hint_on_resume {
            let hint = json!({"jsonrpc":"2.0", "method":"event", "params":{"type":"sessions.changed", "payload":{}}});
            let _ = ws.send(Message::Text(hint.to_string().into()));
        }
        if let Some(reply) = reply {
            let _ = ws.send(Message::Text(reply.to_string().into()));
        }
        if frame["method"] == "session.resume" {
            let hold = world.lock().unwrap().stall_after_resume.take();
            if let Some(hold) = hold {
                hold.entered.store(true, Ordering::Release);
                let limit = std::time::Instant::now() + Duration::from_secs(70);
                while !hold.released.load(Ordering::Acquire) && std::time::Instant::now() < limit {
                    thread::sleep(Duration::from_millis(2));
                }
            }
        }
    }
    // Killed: the socket is cut with no closing handshake.
    let _ = ws.get_ref().shutdown(std::net::Shutdown::Both);
}

fn refused(id: &Value, reason: &str) -> Value {
    json!({"jsonrpc": "2.0", "id": id, "error": {"code": 4001, "message": reason, "data": {"reason": reason}}})
}

/// What Hermes replies to one frame; None where it says nothing.
fn answer(world: &mut World, frame: &Value, subscribed: &mut Vec<String>) -> Option<Value> {
    let (id, params) = (&frame["id"], &frame["params"]);
    let Some(method) = frame["method"].as_str() else {
        // A result frame: the answer to a set of questions. Hermes takes it
        // from a subscribed socket and never replies.
        let set = id.as_str()?.to_owned();
        for stream in world.streams.values_mut() {
            stream.open.retain(|open| open["id"] != set.as_str());
        }
        return None;
    };
    // A socket for the whole host names the profile in every call.
    let Some(profile) = params["profile"].as_str() else {
        return Some(refused(id, "profile_mismatch"));
    };
    // A gateway that serves some profiles refuses a call for any other.
    let serves = |served: &Vec<String>| served.iter().any(|name| name == profile);
    if !world.serves.as_ref().is_none_or(serves) {
        world.mismatched.push(profile.to_owned());
        return Some(refused(id, "profile_mismatch"));
    }
    let session = params["session_id"].as_str().unwrap_or("").to_owned();
    let key = (profile.to_owned(), session.clone());
    let result = match method {
        "session.active_list" => {
            let rows: Vec<Value> = world
                .streams
                .iter()
                .filter(|((of, _), _)| of == profile)
                .filter(|((_, id), _)| !world.unlisted.contains(id))
                .map(|((_, id), stream)| json!({"id": id, "session_key": id, "status": stream.status()}))
                .collect();
            json!({ "sessions": rows })
        }
        "session.events.since" => {
            let Some(stream) = world.streams.get(&key) else {
                return Some(refused(id, "not_found"));
            };
            stream.since(
                params["replay_epoch"].as_str(),
                params["last_sequence"].as_u64().unwrap_or(0),
            )
        }
        "session.resume" => {
            let Some(stream) = world.streams.get(&key) else {
                return Some(refused(id, "not_found"));
            };
            subscribed.push(session.clone());
            json!({
                "session_id": session, "replay_epoch": stream.epoch,
                "last_sequence": stream.latest, "subscription_id": format!("sub-{session}"),
                "prompts": stream.prompts, "open_requests": stream.open, "messages": [],
            })
        }
        // The install's own id, as `profiles.list` reports it.
        "profiles.list" => match &world.install {
            Some(install) => json!({"profiles": [], "install_id": install}),
            None => json!({"profiles": []}),
        },
        "session.detach" => {
            subscribed.retain(|held| *held != session);
            json!({"detached": true, "session_id": session, "subscription_id": params["subscription_id"]})
        }
        "clarify.respond" | "approval.respond" => {
            match world.answers_with {
                Some("silence") => return None,
                // Someone else answered a moment before: Hermes has
                // published that it settled, and says so to this answer.
                Some("already_resolved") | None => {}
                Some(reason) => return Some(refused(id, reason)),
            }
            let status = world.answers_with.unwrap_or("resolved");
            if !subscribed.contains(&session) {
                return Some(refused(id, "permission_denied"));
            }
            let stream = world.streams.get_mut(&key)?;
            let waits = |prompt: &Value| prompt["prompt_id"] == params["prompt_id"];
            let Some(prompt) = stream.prompts.iter().find(|prompt| waits(prompt)).cloned() else {
                let over = !stream.running;
                return Some(if over {
                    refused(id, "stale_generation")
                } else {
                    json!({"jsonrpc": "2.0", "id": id, "result": {"status": "already_resolved", "prompt_id": params["prompt_id"]}})
                });
            };
            if prompt["execution_generation"] != params["execution_generation"] {
                return Some(refused(id, "stale_generation"));
            }
            // Settled, as Hermes publishes it before it replies.
            let kind = if method == "clarify.respond" {
                "clarify.settled"
            } else {
                "approval.settled"
            };
            let settled = json!({
                "type": kind, "session_id": session, "replay_epoch": stream.epoch,
                "seq": stream.latest + 1, "execution_generation": prompt["execution_generation"],
                "payload": {"prompt_id": prompt["prompt_id"], "execution_generation": prompt["execution_generation"]},
            });
            stream.publish(&settled);
            json!({"status": status, "prompt_id": params["prompt_id"]})
        }
        _ => {
            return Some(
                json!({"jsonrpc": "2.0", "id": id, "error": {"code": -32601, "message": "method not found"}}),
            )
        }
    };
    Some(json!({"jsonrpc": "2.0", "id": id, "result": result}))
}
