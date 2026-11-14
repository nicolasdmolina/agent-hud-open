//! The rows of an answer, and the ways an answer says it has none.
use serde_json::Value;

/// One value of a far row, as SQLite held it. A blob comes as none.
#[derive(Clone, Debug, PartialEq)]
pub enum Cell {
    Null,
    Int(i64),
    Real(f64),
    Text(String),
}

impl Cell {
    pub(super) fn of(value: &Value) -> Option<Self> {
        match value {
            Value::Null => Some(Self::Null),
            Value::String(text) => Some(Self::Text(text.clone())),
            Value::Number(number) => match number.as_i64() {
                Some(whole) => Some(Self::Int(whole)),
                None if number.is_f64() => number.as_f64().map(Self::Real),
                None => None,
            },
            _ => None,
        }
    }
}

impl From<&Cell> for rusqlite::types::Value {
    fn from(cell: &Cell) -> Self {
        match cell {
            Cell::Null => Self::Null,
            Cell::Int(whole) => Self::Integer(*whole),
            Cell::Real(number) => Self::Real(*number),
            Cell::Text(text) => Self::Text(text.clone()),
        }
    }
}

/// One session with every one of its usage rows.
#[derive(Clone, Debug, PartialEq)]
pub struct Session {
    pub rowid: i64,
    pub id: String,
    /// Its fingerprint at this read.
    pub print: String,
    /// In the order of `SESSION_COLUMNS`.
    pub cells: Vec<Cell>,
    /// Each in the order of `USAGE_COLUMNS`.
    pub usage: Vec<Vec<Cell>>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Batch {
    pub sessions: Vec<Session>,
    /// Sessions with more usage rows than `MOST_USAGE_ROWS`: no row of
    /// them came.
    pub skipped: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Print {
    pub rowid: i64,
    pub id: String,
    pub print: String,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Prints {
    pub prints: Vec<Print>,
    /// How many rows the page looked at, and the last one's rowid: the
    /// next page starts after it.
    pub seen: u32,
    pub last: i64,
    /// The page reached the end of the table.
    pub ended: bool,
}

/// Why a reader did not start. It opened nothing.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum No {
    /// Another reader of the same home is alive there.
    Busy,
    /// That Python has no `sqlite3` module.
    Sqlite,
    /// The far process list could not be read, so nothing showed that no
    /// other reader runs.
    Processes,
    /// The home is not its own canonical path, or the request is not one
    /// the program knows.
    Request,
}

/// What a reader that started could not read.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Far {
    NoDatabase,
    Open,
    Locked,
    Schema,
    Corrupt,
    Error,
}

impl Far {
    pub fn reason(self) -> &'static str {
        match self {
            Self::NoDatabase => "no state.db in that home",
            Self::Open => "state.db could not be opened",
            Self::Locked => "state.db is locked",
            Self::Schema => "state.db has no sessions table this reader knows",
            Self::Corrupt => "state.db is not a readable database",
            Self::Error => "the read failed",
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum Reply {
    No(No),
    Probed,
    Prints(Prints),
    Sessions(Batch),
    /// The far `active_sessions.json` as it is, untrusted, or none where
    /// it is missing, too long or not JSON.
    Held(Option<String>),
    /// The rows did not fit the output bound. None came.
    TooLarge,
    Failed(Far),
}

/// Why an answer was not used.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Rejected {
    /// Its first line is neither `started` nor why not.
    NotStarted,
    /// Its last line is not there, or is cut.
    NoCompletion,
    /// This line, counted from one, has the wrong shape or does not
    /// belong to what was asked.
    Line(usize),
    /// The last line's counts are not the rows that came.
    Counts,
    /// A status this reader does not know, or one the request cannot have.
    Status,
}

impl std::fmt::Display for Rejected {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotStarted => f.write_str("no started line"),
            Self::NoCompletion => f.write_str("no completion line"),
            Self::Line(line) => write!(f, "line {line} has the wrong shape"),
            Self::Counts => f.write_str("the counts do not match the rows"),
            Self::Status => f.write_str("an unknown status"),
        }
    }
}
