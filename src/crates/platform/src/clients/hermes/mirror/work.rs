//! The writer's own thread: it takes the writes off the queue in order,
//! does each, writes the counts that wait when their time comes, and
//! closes the files when the writer is stopped.
use super::{files, files::Files, Write, Writer};
use agent_hud_agents::hermes::{
    gateway::{Listed, Profile, Sample},
    mirror,
};
use std::{path::PathBuf, sync::PoisonError, time::Instant};

/// What the thread does next.
enum Turn {
    /// A write taken off the queue.
    Queued(Write),
    /// The counts that wait may be written now.
    Counts,
}

/// The rows one poll's list gives a mirror: the sessions it lacks, when
/// they were read and the route number they were read under.
type Rows = (Vec<(String, Listed)>, i64, u64);

impl Writer {
    /// What to do next and the label as it is then; None once it is
    /// over. Counts whose time has come (`due`) go before the queue, so
    /// a queue that is never empty cannot keep them waiting.
    fn next(&self, due: Option<Instant>) -> Option<(Turn, String)> {
        let mut state = self.state();
        loop {
            if state.over {
                return None;
            }
            let left = due.map(|at| at.saturating_duration_since(Instant::now()));
            let turn = match left {
                Some(left) if left.is_zero() => Some(Turn::Counts),
                _ => state.queue.pop_front().map(Turn::Queued),
            };
            if let Some(turn) = turn {
                state.busy = true;
                return Some((turn, state.label.clone()));
            }
            state = match left {
                Some(left) => {
                    let waited = self.wake.wait_timeout(state, left);
                    waited.unwrap_or_else(PoisonError::into_inner).0
                }
                None => (self.wake.wait(state)).unwrap_or_else(PoisonError::into_inner),
            };
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
        while let Some((turn, label)) = self.next(files.counts.due(self.space)) {
            match turn {
                Turn::Queued(Write::Label) => self.label(&label),
                Turn::Queued(Write::Listed {
                    profile,
                    sessions,
                    read_ms,
                    route,
                }) => self.list(&mut files, &profile, sessions, (read_ms, route), &label),
                Turn::Queued(Write::Counted {
                    profile,
                    samples,
                    read_ms,
                    route,
                }) => self.count(&mut files, &profile, samples, (read_ms, route), &label),
                // Whoever asked may have stopped waiting: then nobody hears.
                Turn::Queued(Write::History(batch, done)) => {
                    let _ = done.send(self.keep(&mut files, batch, &label));
                }
                Turn::Counts => self.commit(&mut files, None, &label),
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

    /// What a round of count calls said of `profile`: its samples wait
    /// for their time, and are written now where it has come
    /// (`commit`).
    fn count(
        &self,
        files: &mut Files,
        profile: &Profile,
        samples: Vec<(String, Sample)>,
        (read_ms, route): (i64, u64),
        label: &str,
    ) {
        if !self.routes.under(route, &mut || {}) {
            let name = profile.name();
            self.log
                .line(&format!("mirror dropped profile={name} why=route"));
            return;
        }
        files.counts.sample(profile, samples, read_ms, route);
        self.commit(files, None, label);
    }

    /// What one poll said of `profile`: a row for each of its sessions
    /// that the mirror lacks, written at once.
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
        let ids: Vec<String> = sessions.iter().map(|(id, _)| id.clone()).collect();
        let lacking = files.lacking(profile, &path, sessions);
        // Counts that waited under another route number wait no more,
        // and those taken before their row wait now that it is listed.
        files.counts.sample(profile, Vec::new(), read_ms, route);
        let listed: Vec<&str> = ids.iter().map(String::as_str).collect();
        files.counts.replay(profile, &listed, read_ms, route);
        if !lacking.is_empty() {
            self.commit(files, Some((profile, (lacking, read_ms, route))), label);
        }
    }

    /// Writes the rows a poll's list gave, and the counts that wait
    /// where their time has come: every profile's, each mirror in one
    /// transaction. Core is told once, of every file that changed.
    fn commit(&self, files: &mut Files, listed: Option<(&Profile, Rows)>, label: &str) {
        let counts = files
            .counts
            .due(self.space)
            .is_some_and(|at| at <= Instant::now());
        let mut profiles = match counts {
            true => files.counts.waiting(),
            false => Vec::new(),
        };
        let mut listed = listed.map(|(profile, rows)| (profile.clone(), rows));
        if let Some((profile, _)) = &listed {
            if !profiles.contains(profile) {
                profiles.push(profile.clone());
            }
        }
        let mut told = Vec::new();
        for profile in profiles {
            let rows = listed
                .take_if(|(of, _)| *of == profile)
                .map(|(_, rows)| rows);
            // Samples whose time has not come go on waiting.
            let waited = match counts {
                true => files.counts.take(&profile),
                false => None,
            };
            told.extend(self.write(files, &profile, rows, waited, label));
        }
        if counts {
            files.counts.counted();
        }
        if !told.is_empty() {
            (self.changed)(&told);
        }
    }

    /// One transaction on the mirror of `profile`: its `rows`, and the
    /// samples that `waited` with the route number they were read
    /// under. The file, where the write changed it.
    fn write(
        &self,
        files: &mut Files,
        profile: &Profile,
        rows: Option<Rows>,
        waited: Option<(u64, Vec<(String, Sample, i64)>)>,
        label: &str,
    ) -> Option<PathBuf> {
        let name = profile.name();
        let path = mirror::file(&self.root, &self.key, profile);
        let (samples_route, samples) = waited.unzip();
        let samples = samples.unwrap_or_default();
        let (lacking, read_ms, rows_route) = rows.unwrap_or_default();
        // A count alone is for a mirror that is there: it makes none,
        // and is held until a list gives its session a row.
        if lacking.is_empty() && !path.is_file() {
            files.forget(profile);
            if let Some(route) = samples_route {
                files.counts.rowless(profile, route, samples);
            }
            return None;
        }
        // Samples that wait were read under the route number of the
        // newest list of their profile (`Files::sample`).
        let route = samples_route.unwrap_or(rows_route);
        let (mut made, mut kept) = (0, 0);
        let wrote = files.write(profile, &path, Some(route), &*self.routes, |db| {
            let claimed = mirror::claim(db, &self.key, profile, label)?;
            made = mirror::list(db, &lacking, read_ms)?;
            kept = mirror::live(db, &samples)?;
            Ok(made + kept + usize::from(claimed))
        });
        match wrote {
            Ok(Some(changed)) => {
                // Each has its row, and the mirror says at least this
                // much of each that has one, whether or not this write
                // was the one. One without a row is held for when it
                // has.
                files.hold(profile, lacking);
                let (rowed, rowless) = (samples.into_iter())
                    .partition::<Vec<_>, _>(|(id, _, _)| files.has(profile, id));
                files.counts.said(profile, &rowed);
                files.counts.rowless(profile, route, rowless);
                (changed > 0).then(|| {
                    let line = format!("mirror wrote profile={name} rows={made} counts={kept}");
                    self.log.line(&line);
                    path
                })
            }
            // The route changed between the look above and the commit.
            Ok(None) => {
                self.log
                    .line(&format!("mirror dropped profile={name} why=route"));
                None
            }
            Err(why) => {
                files.forget(profile);
                // Every poll lists its sessions anew, and would say this
                // anew: it is said once for as long as it stays so.
                if files.fails(profile, &why) {
                    self.log
                        .line(&format!("mirror failed profile={name} why={why}"));
                }
                None
            }
        }
    }
}
