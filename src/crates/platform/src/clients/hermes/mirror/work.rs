//! The writer's own thread: it takes the writes off the queue in order,
//! does each, and closes the files when the writer is stopped.
use super::{files, files::Files, Write, Writer};
use agent_hud_agents::hermes::{
    gateway::{Listed, Profile},
    mirror,
};
use std::sync::PoisonError;

impl Writer {
    /// The next write and the label as it is then; None once it is over.
    fn next(&self) -> Option<(Write, String)> {
        let mut state = self.state();
        loop {
            if state.over {
                return None;
            }
            if let Some(write) = state.queue.pop_front() {
                state.busy = true;
                return Some((write, state.label.clone()));
            }
            state = (self.wake.wait(state)).unwrap_or_else(PoisonError::into_inner);
        }
    }

    pub(super) fn work(&self) {
        // Said however the thread ends, so that `stop` never waits on a
        // thread that is gone.
        struct Ended<'a>(&'a Writer);
        impl Drop for Ended<'_> {
            fn drop(&mut self) {
                self.0.state().closed = true;
                self.0.wake.notify_all();
            }
        }
        let _ended = Ended(self);
        let mut files = Files::default();
        while let Some((write, label)) = self.next() {
            match write {
                Write::Label => self.label(&label),
                Write::Listed {
                    profile,
                    sessions,
                    read_ms,
                    route,
                } => self.list(&mut files, &profile, sessions, (read_ms, route), &label),
            }
            self.state().busy = false;
            self.wake.notify_all();
        }
        // Closing a mirror folds its write-ahead log into the file. Core
        // is told, so the file it listed is the file as it stays.
        let closed = files.close();
        if !closed.is_empty() {
            (self.changed)(&closed);
        }
    }

    /// Every mirror of the machine there is says `label`.
    fn label(&self, label: &str) {
        let mut renamed = 0;
        for path in mirror::files(&self.root, &self.key) {
            let said = files::transact(&path, None, &*self.routes, |db| {
                mirror::relabel(db, &self.key, label).map(usize::from)
            });
            match said {
                Ok(Some(1)) => {
                    renamed += 1;
                    (self.changed)(std::slice::from_ref(&path));
                }
                Ok(_) => {}
                Err(why) => self.log.line(&format!("mirror failed label why={why}")),
            }
        }
        if renamed > 0 {
            self.log.line(&format!("mirror label files={renamed}"));
        }
    }

    /// A row for each of `sessions` that the mirror of `profile` lacks.
    fn list(
        &self,
        files: &mut Files,
        profile: &Profile,
        sessions: Vec<(String, Listed)>,
        (read_ms, route): (i64, u64),
        label: &str,
    ) {
        let name = profile.name();
        if !self.routes.under(route, &mut || {}) {
            self.log
                .line(&format!("mirror dropped profile={name} why=route"));
            return;
        }
        if !files.newer(profile, read_ms) {
            return;
        }
        let path = mirror::file(&self.root, &self.key, profile);
        let lacking = files.lacking(profile, &path, sessions);
        if lacking.is_empty() {
            return;
        }
        let mut made = 0;
        let wrote = files.write(profile, &path, Some(route), &*self.routes, |db| {
            let claimed = mirror::claim(db, &self.key, profile, label)?;
            made = mirror::list(db, &lacking, read_ms)?;
            Ok(made + usize::from(claimed))
        });
        match wrote {
            // Every one of them has its row already.
            Ok(Some(0)) => files.hold(profile, lacking),
            Ok(Some(_)) => {
                files.hold(profile, lacking);
                (self.changed)(std::slice::from_ref(&path));
                self.log
                    .line(&format!("mirror wrote profile={name} rows={made}"));
            }
            // The route changed between the look above and the commit.
            Ok(None) => self
                .log
                .line(&format!("mirror dropped profile={name} why=route")),
            Err(why) => {
                files.forget(profile);
                self.log
                    .line(&format!("mirror failed profile={name} why={why}"));
            }
        }
    }
}
