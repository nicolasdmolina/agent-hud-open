//! Reading the connection list. The file is looked at on a slow timer:
//! opened, and its size and time of change compared with the last
//! reading. Only a file that changed is read and parsed. Its text is
//! read into the typed reader (`agents::hermes::connections`), which
//! builds only the allowlisted fields, and is dropped here: it is never
//! logged, kept or copied, and nothing that says why a reading failed
//! holds any of it.
use agent_hud_agents::hermes::connections::{self, Entry, ListError};
use std::{
    fs::File,
    io::{ErrorKind, Read},
    path::PathBuf,
    time::SystemTime,
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

/// The file as it was when it was last read: its size and when it
/// changed.
type Stamp = (u64, Option<SystemTime>);

pub(crate) struct Watch {
    path: PathBuf,
    /// The stamp of the last reading that parsed.
    parsed: Option<Stamp>,
}

impl Watch {
    pub(crate) fn new(path: PathBuf) -> Self {
        let parsed = None;
        Self { path, parsed }
    }

    /// Looks at the file. A reading that fails is tried again at every
    /// look, since the file's stamp says nothing of whether it can be
    /// read now.
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
        let stamp = (stat.len(), stat.modified().ok());
        if self.parsed == Some(stamp) {
            return Looked::Same;
        }
        if !stat.is_file() || stat.len() > MOST {
            return Looked::Unread(Unread::Unreadable);
        }
        // Read, parsed and dropped, all here.
        let mut text = String::new();
        if file.take(MOST + 1).read_to_string(&mut text).is_err() {
            return Looked::Unread(Unread::Unreadable);
        }
        let read = connections::parse(&text);
        drop(text);
        match read {
            Ok(list) => {
                self.parsed = Some(stamp);
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
        let later = SystemTime::now() + std::time::Duration::from_secs(text.len() as u64);
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
        // And the list as it was parses again.
        std::fs::remove_dir(&watch.path).unwrap();
        rewrite(&watch, LIST);
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
