//! The lane the count calls go on (`counting.rs`, ticket 60): a far
//! gateway's `session.stats` calls leave on a socket of their own, so no
//! poll and no card waits for one, however long Hermes takes to say it.
//!
//! The gateway is the pretend one, followed as a far machine's is: its
//! id names a far machine and it has a mirrors' writer, over the route
//! of the tests. Nothing here gives a hint, so only the polls find what
//! happens.
use super::{
    super::{
        control::Silent,
        followers::Followers,
        log::Log,
        mirror::{Routes, Writer},
        route::{Call, Epoch, Home, Inventory, Route, Stop},
        Pace,
    },
    fake::Fake,
    lab,
    machines::{counts::uses, mirrors::works},
    quick,
    route::Way,
    until, Lab,
};
use crate::hooks::Changed;
use agent_hud_agents::hermes::gateway::{machine_key, Failure, Instance, Machine, Profile};
use serde_json::{json, Value};
use std::{
    borrow::Cow,
    collections::HashSet,
    net::TcpStream,
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc,
    },
    time::{Duration, Instant},
};

/// The tests' route with a number and a link that a test changes.
struct Routed {
    way: Arc<Way>,
    number: AtomicU64,
    link: AtomicU64,
}

impl Route for Routed {
    fn homes(&self) -> Result<Vec<Home>, Silent> {
        self.way.homes()
    }
    fn inventory(&self) -> Result<Inventory, Silent> {
        self.way.inventory()
    }
    fn exchange(&self, home: &Home, line: &[u8], timeout: Duration) -> Result<Vec<u8>, Silent> {
        self.way.exchange(home, line, timeout)
    }
    fn named(&self, home: &Home) -> Cow<'_, str> {
        self.way.named(home)
    }
    fn prepare(&self, instance: &Instance, stop: &dyn Stop) -> Result<(), Failure> {
        self.way.prepare(instance, stop)
    }
    fn dial(&self, instance: &Instance) -> Result<TcpStream, Failure> {
        self.way.dial(instance)
    }
    fn number(&self) -> u64 {
        self.number.load(Ordering::Acquire)
    }
    fn epoch(&self) -> Option<Epoch> {
        Some(Epoch::of(self.link.load(Ordering::Acquire)))
    }
    fn admit(&self, _: &Instance, _: Call<'_>) -> Result<(), Failure> {
        Ok(())
    }
}

/// The machine's route number, asked as its mirrors' writer asks the
/// switch of a real machine.
struct Asked(Arc<Routed>);

impl Routes for Asked {
    fn under(&self, number: u64, commit: &mut dyn FnMut()) -> bool {
        let current = self.0.number() == number;
        if current {
            commit();
        }
        current
    }
}

/// How long the gateway holds each `session.stats` back.
const HELD: Duration = Duration::from_millis(700);
/// The polls' pace here: one begins every tenth of a second.
const WATCH: Duration = Duration::from_millis(100);
const BUSY: [&str; 3] = ["s-a", "s-m", "s-z"];

struct Counted {
    lab: Lab,
    gateway: Fake,
    route: Arc<Routed>,
    writer: Arc<Writer>,
    followers: Arc<Followers>,
}

impl Drop for Counted {
    fn drop(&mut self) {
        self.followers.stop();
    }
}

/// A far machine's gateway, followed at `pace`, with its polls running.
fn following(pace: Pace) -> Counted {
    let lab = lab();
    let gateway = Fake::start(&lab.home, "instance-1");
    let route = Arc::new(Routed {
        way: lab.route.clone(),
        number: AtomicU64::new(1),
        link: AtomicU64::new(1),
    });
    let key = machine_key(Some("0123456789abcdef0123456789abcdef"), "/far/.hermes").unwrap();
    let untold: Changed = Arc::new(|_| {});
    let log = Log::new(&lab.data).of(Some("mirror"));
    let asked = Arc::new(Asked(route.clone()));
    let space = Duration::from_millis(50);
    let named = (&key, "Mini");
    let writer = Writer::spaced(&lab.data, named, asked, untold, log, space).unwrap();
    let far = (Machine::Far(key), Some("Mini".to_owned()));
    let (requests, wake) = (lab.requests.clone(), Arc::new(|| {}));
    let mirror = (Some(writer.clone()), None);
    let followers =
        Followers::start(route.clone(), far, &lab.data, requests, wake, pace, mirror).unwrap();
    until("the polls run", || !gateway.world().listed.is_empty());
    Counted {
        lab,
        gateway,
        route,
        writer,
        followers,
    }
}

fn pace() -> Pace {
    Pace {
        watch: WATCH,
        ..quick()
    }
}

impl Counted {
    /// What the mirror's `hud_live` says `session` has used; 0 for none.
    fn kept(&self, session: &str) -> i64 {
        let flags = rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY;
        let path = self.writer.file(&Profile::Default);
        let Ok(db) = rusqlite::Connection::open_with_flags(path, flags) else {
            return 0;
        };
        let sql = "SELECT input_tokens+output_tokens+cache_read_tokens+cache_write_tokens
            FROM hud_live WHERE session_id=?1";
        db.query_row(sql, [session], |row| row.get(0)).unwrap_or(0)
    }

    /// The sockets the gateway got `method` on, call by call; those of
    /// `session` alone where one is named.
    fn sockets(&self, method: &str, session: Option<&str>) -> Vec<usize> {
        let calls = self.gateway.world().calls(method);
        let of = |(_, frame): &&(usize, Value)| {
            session.is_none_or(|session| frame["params"]["session_id"] == session)
        };
        calls.iter().filter(of).map(|(socket, _)| *socket).collect()
    }

    /// No socket that a poll lists on ever carried a count call.
    fn counts_are_on_no_socket_of_the_polls(&self) {
        let lists: HashSet<usize> =
            (self.sockets("session.active_list", None).into_iter()).collect();
        let stats: HashSet<usize> = (self.sockets("session.stats", None).into_iter()).collect();
        assert!(!stats.is_empty(), "no count was asked");
        let both: Vec<&usize> = lists.intersection(&stats).collect();
        assert!(both.is_empty(), "lists and counts on the sockets {both:?}");
    }

    /// How many polls have begun: each lists the gateway's two profiles.
    fn polls(&self) -> usize {
        self.gateway.world().listed.len() / 2
    }

    fn shows(&self, prompt: &str) -> bool {
        let ends = format!(":{prompt}");
        let its = |card: &Value| card["id"].as_str().is_some_and(|id| id.ends_with(&ends));
        (self.lab.requests.views(crate::hooks::now_ms()).iter()).any(its)
    }

    fn log(&self) -> String {
        std::fs::read_to_string(self.lab.data.join("hermes-gateway.log")).unwrap_or_default()
    }
}

/// The verifier's probe of ticket 35 as a test: three sessions work and
/// the gateway holds every `session.stats` back. With the count calls in
/// the poll, a poll took as long as the three of them together and
/// every card waited behind it. On a lane of their own they hold up
/// nothing: the polls keep their pace, a card shows as soon as ever,
/// and the counts still arrive.
#[test]
fn polls_and_cards_do_not_wait_on_counts_that_are_held() {
    let far = following(pace());
    for (at, session) in BUSY.into_iter().enumerate() {
        works(&far.gateway, session, "Busy throughout");
        uses(&far.gateway, session, 100 + at as u64, 20);
    }
    until("the first counts are kept", || {
        BUSY.iter().all(|session| far.kept(session) > 0)
    });
    far.gateway.world().slow_stats = HELD;
    // A round that was out when the calls turned slow is let end.
    std::thread::sleep(WATCH * 3);

    // The polls go on at their pace for as long as three held calls
    // take one after another: a poll that waited for them would be the
    // only one to begin in that time.
    let (from, window) = (far.polls(), HELD * 3);
    std::thread::sleep(window);
    let began = far.polls() - from;
    assert!(began >= 6, "{began} polls began in {window:?}");

    // A card in each busy session, each asked just after a poll began:
    // it shows well before one held call would have ended.
    for (at, session) in BUSY.into_iter().enumerate() {
        let seen = far.polls();
        until("the next poll begins", || far.polls() > seen);
        let (prompt, asked) = (format!("held-{at}"), Instant::now());
        far.gateway.asks_quietly("default", session, &prompt);
        until("its card shows", || far.shows(&prompt));
        let took = asked.elapsed();
        assert!(took < HELD, "the card of {session} took {took:?}");
    }

    // The counts are asked all the while, on sockets no poll lists on,
    // and what they say arrives.
    for session in BUSY {
        uses(&far.gateway, session, 5_000, 500);
    }
    until("the held counts are kept", || {
        BUSY.iter().all(|session| far.kept(session) == 5_500)
    });
    far.counts_are_on_no_socket_of_the_polls();
}

/// A session whose events are slow to read is read apart from the
/// polls (`apart.rs`). It is asked what it has used like any session
/// that works (ticket 35's verifier, v07): the count reads no stream.
#[test]
fn a_session_that_is_read_apart_is_asked_its_count_too() {
    let pace = Pace {
        apart: Duration::from_millis(150),
        ..pace()
    };
    let far = following(pace);
    works(&far.gateway, "s-m-slow", "Slow to read");
    far.gateway
        .tells_quietly("default", "s-m-slow", "message.start", json!({}));
    uses(&far.gateway, "s-m-slow", 100, 20);
    far.gateway.world().slow_replay = Some(("s-m-slow".into(), Duration::from_millis(400)));
    until("the slow session is read apart", || {
        far.log().contains("apart session=")
    });
    let asked = far.sockets("session.stats", Some("s-m-slow")).len();
    uses(&far.gateway, "s-m-slow", 900, 90);
    until("its count is asked again", || {
        far.sockets("session.stats", Some("s-m-slow")).len() > asked + 1
    });
    until("its count is kept", || far.kept("s-m-slow") == 990);
    assert!(
        far.followers.set()[0].apart.count() == 1,
        "still read apart"
    );
}

/// The call for a session's last count is never answered (ticket 60,
/// finding 3): the lane gives its socket up, the session stays owed,
/// and the round after the next poll asks again on another socket and
/// keeps what it says. Once nobody is asked for a while the lane's
/// socket is closed, and a session that works later is asked on a new
/// one.
#[test]
fn a_last_count_that_was_never_answered_is_asked_again_on_another_socket() {
    let pace = Pace {
        answer: Duration::from_millis(800),
        ..pace()
    };
    let far = following(pace);
    works(&far.gateway, "s-turn", "One turn");
    uses(&far.gateway, "s-turn", 100, 20);
    until("its count is kept", || far.kept("s-turn") == 120);
    let first = *far.sockets("session.stats", None).last().unwrap();

    // The turn ends, its last call added to the count, and the gateway
    // does not answer the one call that would say so in time.
    {
        let mut world = far.gateway.world();
        world.slow_stats = pace.answer * 3;
        world.stream_in("default", "s-turn").running = false;
    }
    uses(&far.gateway, "s-turn", 300, 60);
    let asked = far.sockets("session.stats", None).len();
    until("the call at the stop is out", || {
        far.sockets("session.stats", None).len() > asked
    });
    std::thread::sleep(pace.answer + Duration::from_millis(200));
    assert_eq!(far.kept("s-turn"), 120, "nothing answered yet");
    far.gateway.world().slow_stats = Duration::ZERO;
    until("the last count is kept", || far.kept("s-turn") == 360);
    let sockets = far.sockets("session.stats", None);
    let again = *sockets.last().unwrap();
    assert_ne!(again, first, "asked again on the socket given up");
    until("the socket given up is closed", || {
        far.gateway.world().ended.contains(&first)
    });

    // Answered: it is asked no more, though the polls go on.
    let (asked, polls) = (sockets.len(), far.polls());
    until("ten more polls", || far.polls() >= polls + 10);
    assert_eq!(far.sockets("session.stats", None).len(), asked);
    // Nobody is asked for as long as the lane lingers: its socket goes.
    until("the lane's socket is closed", || {
        far.gateway.world().ended.contains(&again)
    });
    works(&far.gateway, "s-later", "A later turn");
    uses(&far.gateway, "s-later", 40, 2);
    until("the later count is kept", || far.kept("s-later") == 42);
    let later = *far
        .sockets("session.stats", Some("s-later"))
        .last()
        .unwrap();
    assert!(later != first && later != again, "a new socket: {later}");
    far.counts_are_on_no_socket_of_the_polls();
}

/// What a round read is written under the route number the lane's
/// socket was opened under. The number goes up and nothing else
/// happens: the count read on that socket is kept out of the mirror.
#[test]
fn a_count_read_on_a_socket_of_an_old_route_number_is_not_written() {
    let far = following(pace());
    works(&far.gateway, "s-turn", "One turn");
    uses(&far.gateway, "s-turn", 100, 20);
    until("its count is kept", || far.kept("s-turn") == 120);
    far.route.number.store(2, Ordering::Release);
    uses(&far.gateway, "s-turn", 700, 70);
    let asked = far.sockets("session.stats", None).len();
    until("three more rounds", || {
        far.sockets("session.stats", None).len() >= asked + 3
    });
    far.writer.settle();
    std::thread::sleep(Duration::from_millis(200));
    assert_eq!(far.kept("s-turn"), 120);
    let log = far.log();
    assert!(
        log.contains("mirror dropped profile=default why=route"),
        "{log}"
    );
}

/// The lane ends with its follower: nothing is asked once it stopped,
/// and a call that is out then is not waited for.
#[test]
fn no_count_is_asked_once_the_follower_ended() {
    let far = following(pace());
    works(&far.gateway, "s-turn", "One turn");
    uses(&far.gateway, "s-turn", 100, 20);
    until("its count is kept", || far.kept("s-turn") == 120);
    far.gateway.world().slow_stats = HELD;
    let asked = far.sockets("session.stats", None).len();
    until("a held call is out", || {
        far.sockets("session.stats", None).len() > asked
    });
    let began = Instant::now();
    far.followers.stop();
    assert!(began.elapsed() < HELD, "stopping waited for a count");
    let asked = far.sockets("session.stats", None).len();
    std::thread::sleep(HELD * 2);
    assert_eq!(far.sockets("session.stats", None).len(), asked);
}

/// A round whose link was lost or replaced while its calls were out is
/// dropped whole, as a poll is (ticket 58): its socket is given up, and
/// the count is asked again over the link that stands.
#[test]
fn a_round_read_over_a_link_that_is_gone_is_dropped_with_its_socket() {
    let far = following(pace());
    works(&far.gateway, "s-turn", "One turn");
    uses(&far.gateway, "s-turn", 100, 20);
    until("its count is kept", || far.kept("s-turn") == 120);
    far.gateway.world().slow_stats = HELD;
    let asked = far.sockets("session.stats", None).len();
    until("a held call is out", || {
        far.sockets("session.stats", None).len() > asked
    });
    let out = *far.sockets("session.stats", None).last().unwrap();
    // The link gives way to another while the call is out.
    far.route.link.store(2, Ordering::Release);
    far.gateway.world().slow_stats = Duration::ZERO;
    uses(&far.gateway, "s-turn", 900, 90);
    until("the socket of the dropped round is closed", || {
        far.gateway.world().ended.contains(&out)
    });
    until("the count is kept over the new link", || {
        far.kept("s-turn") == 990
    });
    let again = *far.sockets("session.stats", None).last().unwrap();
    assert_ne!(again, out);
}

/// After a round the lane rests for as long as the round took: a
/// gateway that is slow to count is asked half of the time at most,
/// however many polls came back meanwhile.
#[test]
fn a_gateway_slow_to_count_is_asked_half_the_time_at_most() {
    let far = following(pace());
    works(&far.gateway, "s-turn", "One turn");
    uses(&far.gateway, "s-turn", 100, 20);
    until("its count is kept", || far.kept("s-turn") == 120);
    let held = Duration::from_millis(400);
    far.gateway.world().slow_stats = held;
    std::thread::sleep(held * 2);
    let (asked, window) = (far.sockets("session.stats", None).len(), held * 10);
    std::thread::sleep(window);
    let rounds = far.sockets("session.stats", None).len() - asked;
    // Asked back to back, it would be ten rounds; resting, five.
    assert!((3..=7).contains(&rounds), "{rounds} rounds in {window:?}");
}

/// A round follows a poll: no more rounds run than polls came back,
/// and a session that works is asked about once a poll.
#[test]
fn a_round_of_counts_follows_a_poll_and_none_runs_without_one() {
    let far = following(pace());
    works(&far.gateway, "s-turn", "One turn");
    uses(&far.gateway, "s-turn", 100, 20);
    until("its count is kept", || far.kept("s-turn") == 120);
    let (asked, polls) = (far.sockets("session.stats", None).len(), far.polls());
    until("twenty more polls", || far.polls() >= polls + 20);
    let (asked, polls) = (
        far.sockets("session.stats", None).len() - asked,
        far.polls() - polls,
    );
    assert!(asked <= polls + 1, "{asked} rounds in {polls} polls");
    assert!(asked * 2 >= polls, "{asked} rounds in {polls} polls");
}
