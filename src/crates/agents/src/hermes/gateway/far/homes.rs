//! The one command that says what Hermes homes a far machine has, and what
//! its answer means.
use super::super::{identity::Profile, machine::canonical};

/// How long the command may run there before it ends itself.
pub const HOMES_SECONDS: u32 = 15;

/// Run under `sh` on the far machine. It prints, each ended by a NUL:
/// `root` and the Hermes root home, `${HERMES_HOME:-$HOME/.hermes}` as the
/// login shell has it, with its links resolved there; `profile`, a name and
/// its resolved home for each folder under `profiles/`; `python` and
/// whether a `python3` can run the relay; `end`. With no root home it
/// prints `none`. It reads and writes nothing else, and it carries its own
/// deadline: it is killed [`HOMES_SECONDS`] after it starts.
///
/// `/usr/bin/python3` on a Mac without the developer tools is a stub that
/// asks to install them in a window, so it is not run where
/// `xcode-select` names no tools.
pub const HOMES_SCRIPT: &str = r#"( sleep 15; kill -9 $$ ) </dev/null >/dev/null 2>&1 &
watch=$!
real() { ( CDPATH= cd -P -- "$1" 2>/dev/null && pwd -P ); }
if root=$(real "${HERMES_HOME:-$HOME/.hermes}"); then
  printf 'root\0%s\0' "$root"
  for home in "$root"/profiles/*/; do
    name=${home%/}
    at=$(real "$home") && printf 'profile\0%s\0%s\0' "${name##*/}" "$at"
  done
  python=no
  found=$(command -v python3 2>/dev/null) || found=
  if [ "$found" = /usr/bin/python3 ] && [ -x /usr/bin/xcode-select ]; then
    /usr/bin/xcode-select -p >/dev/null 2>&1 || found=
  fi
  if [ -n "$found" ]; then
    python3 -I -S -B -c 'import fcntl, hashlib, json, signal, socket, struct, threading' \
      </dev/null >/dev/null 2>&1 && python=yes
  fi
  printf 'python\0%s\0end\0' "$python"
else
  printf 'none\0'
fi
kill "$watch" 2>/dev/null
exit 0"#;

/// The most profiles one machine may have and be followed. Each is asked
/// in turn, so a machine with more is refused whole ([`NoHomes::TooMany`]):
/// never some of its homes as if they were all of them.
pub const MOST_PROFILES: usize = 200;

/// One Hermes home on a far machine.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct FarHome {
    /// The default profile at the root, or the profile whose folder under
    /// `profiles/` this is.
    pub profile: Profile,
    /// Where it is on the far machine, links resolved there. Never a path
    /// of this machine.
    pub path: String,
}

/// The homes of one far machine: the root first, then its profiles by name.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Homes {
    /// The Hermes root home, canonical as `machine_key` needs it.
    pub root: String,
    pub homes: Vec<FarHome>,
}

/// Why a far machine named no homes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NoHomes {
    /// No Hermes root home there.
    NoHermes,
    /// No `python3` there that can run the relay.
    NoPython,
    /// The root it named is not written as a canonical path is.
    RootNotCanonical,
    /// More than [`MOST_PROFILES`] profiles there.
    TooMany,
    /// The answer is not the command's: cut short, or something else.
    Garbled,
}

/// Reads what [`HOMES_SCRIPT`] printed. A folder under `profiles/` that is
/// no profile's (Hermes allows a profile a name of `a-z`, `0-9`, `_`, `-`)
/// names no home, and neither does one whose home is not a canonical path.
/// Every home that is left is in the answer, or there is no answer.
pub fn homes(output: &[u8]) -> Result<Homes, NoHomes> {
    let text = std::str::from_utf8(output).map_err(|_| NoHomes::Garbled)?;
    let mut words = text.split('\0');
    let mut next = || words.next().ok_or(NoHomes::Garbled);
    let root = match next()? {
        "none" => return Err(NoHomes::NoHermes),
        "root" => next()?.to_owned(),
        _ => return Err(NoHomes::Garbled),
    };
    let mut named = Vec::new();
    let python = loop {
        match next()? {
            "profile" => {
                let (name, path) = (next()?, next()?);
                if let Some(profile) = Profile::folder(name).filter(|_| canonical(path)) {
                    named.push(FarHome {
                        profile,
                        path: path.to_owned(),
                    });
                }
            }
            "python" => break next()? == "yes",
            _ => return Err(NoHomes::Garbled),
        }
    };
    // The end is said, and nothing after it.
    let end = (next()?, next()?);
    if end != ("end", "") || next().is_ok() {
        return Err(NoHomes::Garbled);
    }
    if !canonical(&root) {
        return Err(NoHomes::RootNotCanonical);
    }
    if !python {
        return Err(NoHomes::NoPython);
    }
    named.sort();
    // Two folders that lead to one home are one home, under its first name,
    // and count once.
    let mut seen = std::collections::HashSet::from([root.clone()]);
    named.retain(|home| seen.insert(home.path.clone()));
    if named.len() > MOST_PROFILES {
        return Err(NoHomes::TooMany);
    }
    let mut homes = vec![FarHome {
        profile: Profile::Default,
        path: root.clone(),
    }];
    homes.append(&mut named);
    Ok(Homes { root, homes })
}
