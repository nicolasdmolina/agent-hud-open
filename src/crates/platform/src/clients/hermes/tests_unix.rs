//! The connection against a gateway: a pretend one on real sockets that
//! speaks the frames Hermes 0.22.5 was recorded sending (`fake.rs`), and,
//! when asked for by hand, a real one in a Hermes home of its own
//! (`live.rs`).
use super::{
    control::{self, Silent},
    route::{local::Local, Route},
    Observer, Pace,
};
use agent_hud_agents::hermes::AGENT;
use agent_hud_core::requests::Requests;
use fake::Fake;
use serde_json::Value;
use std::{
    path::{Path, PathBuf},
    sync::Arc,
    time::{Duration, Instant},
};

// Unix only, as the control socket is; the files sit in `tests/`.
#[path = "tests/approvals.rs"]
mod approvals;
#[path = "tests/cancelled.rs"]
mod cancelled;
#[path = "tests/cards.rs"]
mod cards;
#[path = "tests/fake.rs"]
mod fake;
#[path = "tests/followers.rs"]
mod followers;
#[path = "tests/gateways.rs"]
mod gateways;
#[path = "tests/handshake.rs"]
mod handshake;
#[path = "tests/live.rs"]
mod live;
#[path = "tests/receipts.rs"]
mod receipts;
#[path = "tests/reconnect.rs"]
mod reconnect;
#[path = "tests/replies.rs"]
mod replies;
#[path = "tests/reverify25.rs"]
mod reverify25;
#[path = "tests/reverify26.rs"]
mod reverify26;
#[path = "tests/reverify3.rs"]
mod reverify3;
#[path = "tests/reviewer27.rs"]
mod reviewer27;
#[path = "tests/reviewer27_extra.rs"]
mod reviewer27_extra;
#[path = "tests/reviewer27_round3.rs"]
mod reviewer27_round3;
#[path = "tests/reviewer27_round4.rs"]
mod reviewer27_round4;
#[path = "tests/reviewer27_round5.rs"]
mod reviewer27_round5;
#[path = "tests/reviewer27_round6.rs"]
mod reviewer27_round6;
#[path = "tests/round3.rs"]
mod round3;
#[path = "tests/round4.rs"]
mod round4;
#[path = "tests/route.rs"]
mod route;
#[path = "tests/script.rs"]
mod script;
#[path = "tests/stalled.rs"]
mod stalled;
#[path = "tests/states.rs"]
mod states;
#[path = "tests/unreachable.rs"]
mod unreachable;
#[path = "tests/unreachable_late.rs"]
mod unreachable_late;
#[path = "tests/verifier.rs"]
mod verifier;
#[path = "tests/verifier2.rs"]
mod verifier2;
#[path = "tests/verifier25.rs"]
mod verifier25;
#[path = "tests/verifier26.rs"]
mod verifier26;
#[path = "tests/verifier3.rs"]
mod verifier3;
#[path = "tests/verifier4.rs"]
mod verifier4;

/// The pace of the tests: everything the app waits seconds for, in
/// hundredths of them.
fn quick() -> Pace {
    Pace {
        retry: Duration::from_millis(20),
        retry_limit: Duration::from_millis(80),
        unsupported: Duration::from_millis(120),
        gap: Duration::from_millis(5),
        watch: Duration::from_millis(30),
        warm: Duration::from_millis(300),
        // Long enough for a machine busy with other builds to answer.
        answer: Duration::from_secs(2),
        // Longer than in the app, so that a test can stop the observer
        // or close a card while a write still waits on the gateway.
        write: Duration::from_secs(3),
        // No look at the machine by the clock: these tests follow one
        // gateway and count what is asked of its route. `followers.rs`
        // turns it on.
        discover: None,
        trace: false,
    }
}

/// A Hermes home and an app data folder of the user's own, the route to
/// that home, and the requests the connection registers its cards with.
struct Lab {
    home: PathBuf,
    route: Arc<route::Way>,
    data: PathBuf,
    requests: Requests,
    observer: Option<Observer>,
    _temp: tempfile::TempDir,
}

fn lab() -> Lab {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().canonicalize().unwrap();
    let (home, data) = (root.join("home"), root.join("data"));
    crate::os::private_directory(&home).unwrap();
    crate::os::private_directory(&data).unwrap();
    Lab {
        route: Arc::new(route::Way::to(&home)),
        home,
        data,
        requests: Requests::default(),
        observer: None,
        _temp: temp,
    }
}

impl Lab {
    /// Starts following the gateway of the home, as turning hooks on does.
    fn follow(&mut self) {
        self.follow_at(quick());
    }
    /// The same, at `pace`.
    fn follow_at(&mut self, pace: Pace) {
        let wake = Arc::new(|| {});
        let observer = Observer::start_on(
            self.route.clone(),
            &self.data,
            self.requests.clone(),
            wake,
            pace,
        );
        self.observer = Some(observer.unwrap());
    }
    fn cards(&self) -> Vec<Value> {
        self.requests.views(crate::hooks::now_ms())
    }
    fn log(&self) -> String {
        std::fs::read_to_string(self.data.join("hermes-gateway.log")).unwrap_or_default()
    }
    /// The sessions core was told wait on a card.
    fn waiting(&self) -> Vec<String> {
        let all = self.requests.native_waiting_sessions();
        let mut sessions: Vec<String> = all.get(AGENT.id).into_iter().flatten().cloned().collect();
        sessions.sort();
        sessions
    }
    /// Plays a recording into the gateway and has it say that its sessions
    /// changed, as Hermes does when a turn starts.
    fn hears(&self, gateway: &Fake, recording: &str, through: &str) -> String {
        let session = gateway.play(recording, Some(through));
        gateway.say("sessions.changed");
        session
    }
    /// Waits until exactly one card shows and returns it.
    fn card(&self) -> Value {
        until("one card shows", || self.cards().len() == 1);
        self.cards().remove(0)
    }
    fn delivery(&self, id: &str) -> Value {
        let found = |row: &Value| row["id"] == id;
        until("the answer has a receipt", || {
            self.requests
                .deliveries(crate::hooks::now_ms())
                .iter()
                .any(found)
        });
        let rows = self.requests.deliveries(crate::hooks::now_ms());
        rows.into_iter().find(found).unwrap()
    }
}

/// A test that fails says what the connection logged.
impl Drop for Lab {
    fn drop(&mut self) {
        if std::thread::panicking() {
            eprintln!("the connection's log:\n{}", self.log());
        }
    }
}

/// Asks the control socket of the Hermes home at `home` one question, as
/// the route to this machine does.
fn ask(home: &Path, line: &Value, wait: Duration) -> Result<Value, Silent> {
    let route = Local::at(home);
    let home = route.homes()?.remove(0);
    control::ask(&route, &home, line, wait)
}

/// The one receipt of an answer that never got out because its request
/// was over first: it failed, and nothing confirmed it (ticket 27, the
/// fourth review). Before that such an answer had no receipt at all.
fn failed_once(lab: &Lab, id: &str) {
    let rows = lab.requests.deliveries(crate::hooks::now_ms());
    let of: Vec<&Value> = rows.iter().filter(|row| row["id"] == id).collect();
    assert_eq!(of.len(), 1, "{rows:?}");
    assert_eq!(of[0]["state"], "failed", "{rows:?}");
    assert!(of[0]["reason"].as_str().is_some_and(|why| !why.is_empty()));
}

/// Whether a card says its gateway cannot be reached (ticket 27).
fn marked(card: &Value) -> bool {
    card["unreachable"]
        .as_str()
        .is_some_and(|why| !why.is_empty())
}

/// The card without its mark: a marked card is the same card but for that.
fn plain(mut card: Value) -> Value {
    card.as_object_mut().unwrap().remove("unreachable");
    card
}

/// Waits for `met`, which takes hundredths of a second; the limit is for a
/// machine busy with other builds.
fn until(what: &str, mut met: impl FnMut() -> bool) {
    let limit = Instant::now() + Duration::from_secs(20);
    while !met() {
        assert!(Instant::now() < limit, "never happened: {what}");
        std::thread::sleep(Duration::from_millis(5));
    }
}
