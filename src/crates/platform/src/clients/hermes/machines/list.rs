//! Reading the connection list. The file is looked at on a slow timer:
//! opened, read, and its text compared with the last reading that
//! parsed. Only a text that changed is parsed. It is compared by a mark
//! (`Watch::mark`), not kept: a file's size and time of change say
//! nothing of a file put back with both as they were. The text is read
//! into the typed reader (`agents::hermes::connections`), which builds
//! only the allowlisted fields, and is dropped here: it is never logged,
//! kept or copied, and nothing that says why a reading failed holds any
//! of it.
use agent_hud_agents::hermes::connections::{self, Entry, ListError};
use std::{
    fs::File,
    hash::{BuildHasher, RandomState},
    io::{ErrorKind, Read},
    path::PathBuf,
};

/// More than any list holds. A longer file is not read.
const MOST: u64 = 4 * 1024 * 1024;

/// Why the list could not be read. None of them holds text of the file.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Unread {
    /// There is no such file.
    Missing,
    /// It is there and cannot be opened or read, or is too long.
    Unreadable,
    /// Not the list's JSON: half written, or broken.
    Broken,
    /// A version this reader does not understand.
    NotUnderstood,
}

/// What one look at the file found.
pub(crate) enum Looked {
    /// It is as it was when it last parsed.
    Same,
    /// It changed, and parsed.
    List(Vec<Entry>),
    Unread(Unread),
}

pub(crate) struct Watch {
    path: PathBuf,
    /// The mark of the last text that parsed.
    parsed: Option<u64>,
    /// What the marks are made with: this watch's own, made anew at
    /// every launch, so a mark says nothing of the text to anyone else.
    marks: RandomState,
}

impl Watch {
    pub(crate) fn new(path: PathBuf) -> Self {
        let (parsed, marks) = (None, RandomState::new());
        Self {
            path,
            parsed,
            marks,
        }
    }

    /// A number that stands for `text` in this watch alone: two texts
    /// with one mark are the same text. It is held in memory and written
    /// nowhere.
    fn mark(&self, text: &str) -> u64 {
        self.marks.hash_one(text)
    }

    /// Looks at the file. A reading that fails is tried again at every
    /// look: only a text that parsed is remembered.
    pub(crate) fn look(&mut self) -> Looked {
        let file = match File::open(&self.path) {
            Ok(file) => file,
            Err(error) if error.kind() == ErrorKind::NotFound => {
                return Looked::Unread(Unread::Missing)
            }
            Err(_) => return Looked::Unread(Unread::Unreadable),
        };
        let Ok(stat) = file.metadata() else {
            return Looked::Unread(Unread::Unreadable);
        };
        if !stat.is_file() || stat.len() > MOST {
            return Looked::Unread(Unread::Unreadable);
        }
        // Read, compared, parsed and dropped, all here.
        let mut text = String::new();
        if file.take(MOST + 1).read_to_string(&mut text).is_err() {
            return Looked::Unread(Unread::Unreadable);
        }
        let mark = self.mark(&text);
        if self.parsed == Some(mark) {
            return Looked::Same;
        }
        let read = connections::parse(&text);
        drop(text);
        match read {
            Ok(list) => {
                self.parsed = Some(mark);
                Looked::List(list)
            }
            Err(ListError::NotUnderstood) => Looked::Unread(Unread::NotUnderstood),
            Err(ListError::Broken { .. }) => Looked::Unread(Unread::Broken),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const LIST: &str = r#"{"version": 2, "connections": [
        {"id": "local", "kind": "local", "label": "Here"},
        {"id": "a", "kind": "ssh", "label": "Far", "host": "far",
         "token": {"encoding": "plain", "value": "MARKER-list"}}]}"#;

    fn written(text: &str) -> (tempfile::TempDir, Watch) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("connections.json");
        std::fs::write(&path, text).unwrap();
        (dir, Watch::new(path))
    }

    /// Writes `text` so that the file's stamp is another, whatever the
    /// clock's grain.
    fn rewrite(watch: &Watch, text: &str) {
        std::fs::write(&watch.path, text).unwrap();
        let now = std::time::SystemTime::now();
        let later = now + std::time::Duration::from_secs(text.len() as u64);
        let file = File::options().write(true).open(&watch.path).unwrap();
        file.set_modified(later).unwrap();
    }

    #[test]
    fn a_list_is_parsed_when_it_changed_and_not_otherwise() {
        let (_dir, mut watch) = written(LIST);
        let Looked::List(list) = watch.look() else {
            panic!("the list parses");
        };
        assert_eq!(list.len(), 2);
        assert!(matches!(watch.look(), Looked::Same));
        assert!(matches!(watch.look(), Looked::Same));
        // Rewritten with another token: read again, and the same list.
        rewrite(&watch, &LIST.replace("MARKER-list", "another token"));
        let Looked::List(again) = watch.look() else {
            panic!("the list parses again");
        };
        assert_eq!(again, list, "only the allowlisted fields are compared");
        assert!(matches!(watch.look(), Looked::Same));
    }

    #[test]
    fn a_rewrite_of_the_same_size_and_time_is_seen() {
        let (_dir, mut watch) = written(LIST);
        assert!(matches!(watch.look(), Looked::List(_)));
        let was = std::fs::metadata(&watch.path).unwrap();
        let stamp = (was.len(), was.modified().unwrap());
        // Another file of the same length moved into its place with the
        // time of change put back, as a restore that keeps times does;
        // then the file written over where it is, the same way.
        for (label, in_place) in [("Fir", false), ("Fur", true)] {
            let text = LIST.replace("\"Far\"", &format!("\"{label}\""));
            let new = match in_place {
                true => watch.path.clone(),
                false => watch.path.with_extension("new"),
            };
            std::fs::write(&new, &text).unwrap();
            let file = File::options().write(true).open(&new).unwrap();
            file.set_modified(stamp.1).unwrap();
            drop(file);
            std::fs::rename(&new, &watch.path).unwrap();
            let now = std::fs::metadata(&watch.path).unwrap();
            assert_eq!((now.len(), now.modified().unwrap()), stamp);
            let Looked::List(list) = watch.look() else {
                panic!("the list that says {label} was not seen");
            };
            assert_eq!(list[1].label, label);
            assert!(matches!(watch.look(), Looked::Same));
        }
    }

    #[test]
    fn a_list_that_cannot_be_read_says_why_and_is_read_again_at_every_look() {
        let missing = Watch::new("/nonexistent/agent-hud/connections.json".into()).look();
        assert!(matches!(missing, Looked::Unread(Unread::Missing)));

        let (_dir, mut watch) = written(LIST);
        assert!(matches!(watch.look(), Looked::List(_)));
        let faults = [
            (
                "{\"version\": 2, \"connections\": [{\"id\":",
                Unread::Broken,
            ),
            ("", Unread::Broken),
            (
                &LIST.replace("\"version\": 2", "\"version\": 3"),
                Unread::NotUnderstood,
            ),
            ("{\"version\": 2}", Unread::NotUnderstood),
        ];
        for (text, why) in faults {
            rewrite(&watch, text);
            for _ in 0..3 {
                let Looked::Unread(found) = watch.look() else {
                    panic!("{text} was read");
                };
                assert_eq!(found, why, "{text}");
            }
        }
        // A folder in its place cannot be read as a list.
        std::fs::remove_file(&watch.path).unwrap();
        assert!(matches!(watch.look(), Looked::Unread(Unread::Missing)));
        std::fs::create_dir(&watch.path).unwrap();
        assert!(matches!(watch.look(), Looked::Unread(Unread::Unreadable)));
        // The list as it was is the text that last parsed: it is read,
        // found the same and not parsed again. Another list parses.
        std::fs::remove_dir(&watch.path).unwrap();
        rewrite(&watch, LIST);
        assert!(matches!(watch.look(), Looked::Same));
        rewrite(&watch, &LIST.replace("Far", "Near"));
        assert!(matches!(watch.look(), Looked::List(_)));
    }

    #[test]
    fn nothing_that_says_why_holds_text_of_the_file() {
        let (_dir, mut watch) = written(&LIST[..LIST.find("MARKER-list").unwrap() + 11]);
        let Looked::Unread(why) = watch.look() else {
            panic!("a list cut off after its token was read");
        };
        assert_eq!(format!("{why:?}"), "Broken");
        assert_eq!(std::mem::size_of::<Unread>(), 1, "a word, and no text");
    }
}
