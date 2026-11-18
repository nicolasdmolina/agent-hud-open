//! The reads of the two sweeps, and the read of sessions by id that all
//! three kinds of work end in (`super`).
use super::{Pages, Run, Stop};
use crate::clients::hermes::{
    far_history::Failure,
    mirror::{Batch, Wrote},
};
use crate::ssh_link::Cut;
use std::collections::VecDeque;

/// The status of a far reader that its own deadline ended.
const DEADLINE: i32 = 3;
/// The fewest rows a page is cut down to.
const LEAST_PAGE: u32 = 10;

impl Run<'_> {
    /// One read of a sweep, the whole one or the recent one: the
    /// sessions its last page found changed, else its next page.
    pub(super) fn page(&mut self, whole: bool) -> Result<(), Stop> {
        let (history, plan) = (self.history, self.history.plan);
        let now = crate::hooks::now_ms();
        self.swept |= whole;
        let pages = match whole {
            true => &mut self.at.whole,
            false => &mut self.at.recent,
        };
        if !pages.pending.is_empty() {
            let count = plan.batch.max(1).min(pages.pending.len());
            let ids: Vec<(String, String)> = pages.pending.drain(..count).collect();
            let (unread, did) = self.fetch(&ids);
            let pages = match whole {
                true => &mut self.at.whole,
                false => &mut self.at.recent,
            };
            for one in unread.into_iter().rev() {
                pages.pending.push_front(one);
            }
            did?;
        } else {
            let after = pages.after;
            let since = (!whole).then(|| (now - plan.recent.as_millis() as i64) as f64 / 1000.0);
            let size = match pages.size {
                0 => plan.page,
                size => size,
            };
            let read =
                (history.reader).prints(self.link, &history.slot, self.home, after, size, since);
            let page = match read {
                Ok(page) => page,
                Err(failure) => {
                    // A page the far side could not read in its time is
                    // asked for at half the size from the next tick on,
                    // until this sweep ends: a slow database slows the
                    // sweep and does not stop it.
                    if matches!(
                        failure,
                        Failure::Exit(DEADLINE) | Failure::CutOff(Cut::TimeLimit)
                    ) {
                        pages.size = (size / 2).max(LEAST_PAGE);
                    }
                    return Err(Stop::Read(failure));
                }
            };
            self.contacted();
            if whole && after == 0 {
                // A whole sweep begins.
                self.at.reads.sweep_started_ms = Some(now);
            }
            let mut found = VecDeque::new();
            for print in page.prints {
                let known = self.stored.get(&print.id) == Some(&print.print);
                let skipped = match self.at.skipped.get_mut(&print.id) {
                    // Skipped with nothing known of it: as it is now.
                    Some(at) if at.is_empty() => {
                        at.clone_from(&print.print);
                        true
                    }
                    Some(at) => *at == print.print,
                    None => false,
                };
                if !known && !skipped {
                    found.push_back((print.id, print.print));
                }
            }
            let pages = match whole {
                true => &mut self.at.whole,
                false => &mut self.at.recent,
            };
            (pages.from, pages.after, pages.ended) = (after, page.last, page.ended);
            pages.pending = found;
        }
        let pages = match whole {
            true => &mut self.at.whole,
            false => &mut self.at.recent,
        };
        if pages.ended && pages.pending.is_empty() {
            // The table's end, with every session it found written: the
            // sweep is done, and the next one begins at the start.
            *pages = Pages::default();
            match whole {
                true => {
                    // Stamped with the answer that ended it, so a machine's
                    // whole check is never later than its last contact.
                    let ended = self.contact.unwrap_or_else(crate::hooks::now_ms);
                    self.at.reads.sweep_done_ms = Some(ended);
                    self.at.reads.sweep_started_ms = None;
                }
                false => self.recent_done = true,
            }
        }
        Ok(())
    }

    /// Reads the sessions `ids`, each with the fingerprint a page showed
    /// for it or none, and writes the ones that are not what the mirror
    /// holds. What is still to be read, and why where the reads stopped.
    pub(super) fn fetch(
        &mut self,
        ids: &[(String, String)],
    ) -> (Vec<(String, String)>, Result<(), Stop>) {
        let history = self.history;
        let asked: Vec<String> = ids.iter().map(|(id, _)| id.clone()).collect();
        let read_ms = crate::hooks::now_ms();
        let got = (history.reader).sessions(self.link, &history.slot, self.home, &asked);
        let answered = got.stopped.is_none() || !(got.read.is_empty() && got.gone.is_empty());
        if answered {
            self.contacted();
        }
        let print_of = |id: &str| ids.iter().find(|(of, _)| of == id).map(|(_, print)| print);
        for (id, _) in &got.skipped {
            let print = print_of(id).cloned().unwrap_or_default();
            self.at.skipped.insert(id.clone(), print);
        }
        let waits = |id: &String| (id.clone(), print_of(id).cloned().unwrap_or_default());
        let mut unread: Vec<(String, String)> = got.unread.iter().map(waits).collect();
        let mut did = got
            .stopped
            .map_or(Ok(()), |failure| Err(Stop::Read(failure)));
        let changed: Vec<_> = (got.read.into_iter())
            .filter(|session| self.stored.get(&session.id) != Some(&session.print))
            .collect();
        if changed.is_empty() {
            return (unread, did);
        }
        let prints: Vec<(String, String)> = (changed.iter())
            .map(|session| (session.id.clone(), session.print.clone()))
            .collect();
        let batch = Batch {
            profile: self.profile.clone(),
            sessions: changed,
            held: None,
            reads: None,
            read_ms,
            route: self.route,
        };
        match history.writer.history(batch) {
            wrote @ (Wrote::Written(_) | Wrote::Refused(..)) => {
                let refused = match wrote {
                    Wrote::Refused(_, refused) => refused,
                    _ => Vec::new(),
                };
                for (id, print) in prints {
                    // One the mirror would not take is set aside as it
                    // is now, like one the far side cannot give: the
                    // writer logged it, and nothing waits on it.
                    match refused.contains(&id) {
                        true => self.at.skipped.insert(id, print),
                        false => {
                            self.at.skipped.remove(&id);
                            self.stored.insert(id, print)
                        }
                    };
                }
            }
            Wrote::Dropped(_) | Wrote::Failed(_) => {
                // Not written: they wait, ahead of what was not read.
                let mut again: Vec<(String, String)> =
                    prints.iter().map(|(id, _)| waits(id)).collect();
                again.append(&mut unread);
                unread = again;
                did = Err(Stop::Write);
            }
        }
        (unread, did)
    }
}
