//! The route a far machine's followers hold. A machine is a verified
//! identity, and it outlives the way to it: the entry that led to it can
//! be edited or removed while another entry reaches the same machine. So
//! the followers are given this, and the way behind it is put in and
//! taken out as the machine's owner changes (`machines/book.rs`).
//!
//! It has a number, the route number, which goes up every time the way is
//! taken out. That is the first step of every route change, before
//! anything waits or is torn down: from that moment there is no way, so
//! nothing more is asked of the old one, and what the old one still hands
//! back is refused here. A call that began under one number and comes
//! back under another failed, whatever the old way said: a stream it
//! dialled is dropped unspoken on. The next way is put in under the
//! number as it then is.
use super::{Call, Home, Inventory, Route, Stop};
use crate::clients::hermes::control::Silent;
use agent_hud_agents::hermes::gateway::{Failure, Instance};
use std::{
    borrow::Cow,
    net::TcpStream,
    sync::{Arc, Mutex, MutexGuard, PoisonError},
    time::Duration,
};

pub(crate) struct Switch {
    state: Mutex<State>,
}

struct State {
    number: u64,
    way: Option<Arc<dyn Route>>,
}

impl Switch {
    /// A machine reached by `way`, under the first route number.
    pub(crate) fn new(way: Arc<dyn Route>) -> Self {
        let (number, way) = (1, Some(way));
        Self {
            state: Mutex::new(State { number, way }),
        }
    }

    fn state(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// The route number as it is now.
    pub(crate) fn number(&self) -> u64 {
        self.state().number
    }

    /// The route changes: its number goes up, and there is no way to the
    /// machine until the next one is put in. Nothing waits here.
    pub(crate) fn raise(&self) -> u64 {
        let mut state = self.state();
        state.number += 1;
        state.way = None;
        state.number
    }

    /// The way to the machine from now on, under the number as it is.
    pub(crate) fn install(&self, way: Arc<dyn Route>) {
        self.state().way = Some(way);
    }

    /// The way there is now, with the number it is used under.
    fn way(&self) -> Option<(u64, Arc<dyn Route>)> {
        let state = self.state();
        Some((state.number, state.way.clone()?))
    }

    /// What `asked` brought, where the route is still the one it was
    /// asked of; `changed` where the route changed meanwhile.
    fn kept<T, E>(&self, number: u64, asked: Result<T, E>, changed: E) -> Result<T, E> {
        match self.state().number == number {
            true => asked,
            false => Err(changed),
        }
    }
}

impl Route for Switch {
    fn homes(&self) -> Result<Vec<Home>, Silent> {
        self.inventory().map(|told| told.homes)
    }

    fn inventory(&self) -> Result<Inventory, Silent> {
        let (number, way) = self.way().ok_or(Silent::Unclear)?;
        self.kept(number, way.inventory(), Silent::Unclear)
    }

    /// With no way, a control socket was not heard: that is never word
    /// that nobody listens there.
    fn exchange(&self, home: &Home, line: &[u8], timeout: Duration) -> Result<Vec<u8>, Silent> {
        let (number, way) = self.way().ok_or(Silent::TimedOut)?;
        self.kept(number, way.exchange(home, line, timeout), Silent::TimedOut)
    }

    fn named(&self, home: &Home) -> Cow<'_, str> {
        let named = match self.way() {
            Some((_, way)) => way.named(home).into_owned(),
            None => home.path().to_string_lossy().into_owned(),
        };
        Cow::Owned(named)
    }

    fn prepare(&self, instance: &Instance, stop: &dyn Stop) -> Result<(), Failure> {
        let (number, way) = self.way().ok_or(Failure::Unreachable)?;
        self.kept(number, way.prepare(instance, stop), Failure::Unreachable)
    }

    /// A stream the old way dialled while the route changed is dropped
    /// here, with nothing said on it.
    fn dial(&self, instance: &Instance) -> Result<TcpStream, Failure> {
        let (number, way) = self.way().ok_or(Failure::Unreachable)?;
        self.kept(number, way.dial(instance), Failure::Unreachable)
    }

    fn release(&self, instance: &str) {
        if let Some((_, way)) = self.way() {
            way.release(instance);
        }
    }

    fn admit(&self, instance: &Instance, call: Call<'_>) -> Result<(), Failure> {
        let (number, way) = self.way().ok_or(Failure::Unreachable)?;
        self.kept(number, way.admit(instance, call), Failure::Unreachable)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use agent_hud_agents::hermes::gateway::Profile;
    use std::{
        net::TcpListener,
        path::PathBuf,
        sync::atomic::{AtomicBool, AtomicUsize, Ordering},
    };

    /// A way that counts what is asked of it, and can hold an answer back.
    #[derive(Default)]
    struct Way {
        asked: AtomicUsize,
        hold: AtomicBool,
        listener: Mutex<Option<TcpListener>>,
    }

    impl Way {
        fn held(&self) {
            self.asked.fetch_add(1, Ordering::AcqRel);
            while self.hold.load(Ordering::Acquire) {
                std::thread::sleep(Duration::from_millis(2));
            }
        }
    }

    impl Route for Way {
        fn homes(&self) -> Result<Vec<Home>, Silent> {
            self.held();
            Ok(vec![home()])
        }
        fn exchange(&self, _: &Home, line: &[u8], _: Duration) -> Result<Vec<u8>, Silent> {
            self.held();
            Ok(line.to_vec())
        }
        fn named(&self, _: &Home) -> Cow<'_, str> {
            Cow::Borrowed("named by the way")
        }
        fn prepare(&self, _: &Instance, _: &dyn Stop) -> Result<(), Failure> {
            self.held();
            Ok(())
        }
        fn dial(&self, _: &Instance) -> Result<TcpStream, Failure> {
            self.held();
            let listener = self.listener.lock().unwrap();
            let at = listener.as_ref().unwrap().local_addr().unwrap();
            TcpStream::connect(at).map_err(|_| Failure::Unreachable)
        }
        fn admit(&self, _: &Instance, _: Call<'_>) -> Result<(), Failure> {
            self.held();
            Ok(())
        }
    }

    fn home() -> Home {
        let path: PathBuf = std::env::temp_dir().join("far").join(".hermes");
        Home::at(path.clone(), path, Profile::Default)
    }

    fn instance() -> Instance {
        Instance {
            id: "instance".into(),
            authority_epoch: 1,
            origin: "http://127.0.0.1:1".into(),
            port: 1,
            profile_id: "p".into(),
            served: Vec::new(),
            draining: false,
            version: None,
        }
    }

    struct Never;
    impl Stop for Never {
        fn wait(&self, _: Duration) -> bool {
            false
        }
    }

    /// Everything a follower asks of its route, each as it ended.
    fn asked(switch: &Switch) -> [bool; 5] {
        let (home, instance) = (home(), instance());
        let mut call = |_: &_| Ok(serde_json::Value::Null);
        [
            switch.inventory().is_ok(),
            switch.exchange(&home, b"x", Duration::ZERO).is_ok(),
            switch.prepare(&instance, &Never).is_ok(),
            switch.dial(&instance).is_ok(),
            switch.admit(&instance, &mut call).is_ok(),
        ]
    }

    fn way() -> Arc<Way> {
        let way = Arc::new(Way::default());
        *way.listener.lock().unwrap() = Some(TcpListener::bind("127.0.0.1:0").unwrap());
        way
    }

    #[test]
    fn a_route_whose_number_went_up_is_asked_nothing_until_a_way_is_put_in() {
        let old = way();
        let switch = Switch::new(old.clone());
        assert_eq!(switch.number(), 1);
        assert_eq!(asked(&switch), [true; 5]);
        let before = old.asked.load(Ordering::Acquire);

        assert_eq!(switch.raise(), 2);
        // No way: nothing is asked of the old one, and each call says
        // what a lost link says, never that nobody listens.
        assert_eq!(asked(&switch), [false; 5]);
        assert_eq!(old.asked.load(Ordering::Acquire), before);
        assert!(matches!(switch.inventory(), Err(Silent::Unclear)));
        let unheard = switch.exchange(&home(), b"x", Duration::ZERO);
        assert!(matches!(unheard, Err(Silent::TimedOut)));
        assert_eq!(switch.dial(&instance()).err(), Some(Failure::Unreachable));

        // The next way is used under the number as it is, and the old
        // one is asked nothing more.
        let new = way();
        switch.install(new.clone());
        assert_eq!(switch.number(), 2);
        assert_eq!(asked(&switch), [true; 5]);
        assert_eq!(old.asked.load(Ordering::Acquire), before);
        assert_eq!(new.asked.load(Ordering::Acquire), 5);
    }

    #[test]
    fn what_the_old_way_hands_back_after_the_number_went_up_is_refused() {
        // Each call is under way on the old route when the number goes
        // up, and comes back with what the old route got.
        for call in 0..5 {
            let old = way();
            let switch = Arc::new(Switch::new(old.clone()));
            old.hold.store(true, Ordering::Release);
            let asking = switch.clone();
            let under_way = std::thread::spawn(move || asked_one(&asking, call));
            while old.asked.load(Ordering::Acquire) == 0 {
                std::thread::sleep(Duration::from_millis(1));
            }
            switch.raise();
            // Even with the next way already in.
            switch.install(way());
            old.hold.store(false, Ordering::Release);
            assert!(!under_way.join().unwrap(), "call {call} was taken");
            // A stream the old way dialled is dropped, unspoken on.
            if call == 3 {
                use std::io::Read;
                let listener = old.listener.lock().unwrap().take().unwrap();
                let (mut stream, _) = listener.accept().unwrap();
                let mut said = Vec::new();
                stream.read_to_end(&mut said).unwrap();
                assert!(said.is_empty());
            }
        }
    }

    fn asked_one(switch: &Switch, call: usize) -> bool {
        let (home, instance) = (home(), instance());
        let mut say = |_: &_| Ok(serde_json::Value::Null);
        match call {
            0 => switch.inventory().is_ok(),
            1 => switch.exchange(&home, b"x", Duration::ZERO).is_ok(),
            2 => switch.prepare(&instance, &Never).is_ok(),
            3 => switch.dial(&instance).is_ok(),
            _ => switch.admit(&instance, &mut say).is_ok(),
        }
    }
}
