//! The one command that says what Hermes homes a far machine has, and what
//! its answer means.
use super::super::{identity::Profile, machine::canonical};

/// How long the command may run there before it ends itself.
pub const HOMES_SECONDS: u32 = 15;

/// Run under `sh` on the far machine. It prints, each ended by a NUL:
/// `root` and the Hermes root home, `${HERMES_HOME:-$HOME/.hermes}` as the
/// login shell has it, with its links resolved there; `profile`, a name and
/// its resolved home for each folder under `profiles/`; `unresolved` and
/// the name of each entry there whose home could not be resolved and is
/// not known to be gone: a folder that cannot be entered, and a link, or
/// anything else, that leads to no folder for a reason other than that
/// nothing is there (a folder on the way cannot be searched, say);
/// `python` and whether a `python3` can run the relay; `end`. With no root
/// home it prints `none`. It reads and writes nothing else, and it carries
/// its own deadline: it is killed [`HOMES_SECONDS`] after it starts.
///
/// `sh` cannot say why an entry leads nowhere, so `ls` is asked, in the
/// `C` locale, and only its "No such file or directory" is word that the
/// entry is gone, as the local route takes only "not found" for it. A
/// file, and a link to one, is no profile's home and prints nothing. A
/// `profiles` folder that cannot be listed still names no profile.
///
/// A name is printed as the bytes it is. A file name holds no NUL, so the
/// words stay apart whatever a folder is called.
///
/// `/usr/bin/python3` on a Mac without the developer tools is a stub that
/// asks to install them in a window, so it is not run where
/// `xcode-select` names no tools.
pub const HOMES_SCRIPT: &str = r#"( sleep 15; kill -9 $$ ) </dev/null >/dev/null 2>&1 &
watch=$!
real() { ( CDPATH= cd -P -- "$1" 2>/dev/null && pwd -P ); }
if root=$(real "${HERMES_HOME:-$HOME/.hermes}"); then
  printf 'root\0%s\0' "$root"
  for home in "$root"/profiles/*; do
    if [ -d "$home" ] && at=$(real "$home"); then
      printf 'profile\0%s\0%s\0' "${home##*/}" "$at"
    elif [ -d "$home" ]; then
      printf 'unresolved\0%s\0' "${home##*/}"
    elif [ ! -e "$home" ]; then
      case $(LC_ALL=C ls -d "$home/." 2>&1 >/dev/null) in
        *': No such file or directory') ;;
        *) printf 'unresolved\0%s\0' "${home##*/}" ;;
      esac
    fi
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
    /// The profiles whose folder is there and whose home could not be
    /// resolved, by name. None of them is among `homes` and none was
    /// asked: whether a gateway runs there is not known.
    pub unresolved: Vec<Profile>,
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
/// Every home that is left is in the answer, or there is no answer. A
/// profile whose home could not be resolved is in it by name
/// ([`Homes::unresolved`]).
///
/// The words are read as bytes, ended by NULs. A name or a path that is
/// not UTF-8 is no word of the command's, no profile's name and no path
/// that can be written here, and it makes nothing else unreadable. A
/// profile whose home is at such a path is there and cannot be asked: it
/// is among the unresolved.
pub fn homes(output: &[u8]) -> Result<Homes, NoHomes> {
    let mut words = output.split(|byte| *byte == 0);
    let mut next = || words.next().ok_or(NoHomes::Garbled);
    let text = |word| std::str::from_utf8(word).ok();
    let root = match next()? {
        b"none" => return Err(NoHomes::NoHermes),
        b"root" => text(next()?),
        _ => return Err(NoHomes::Garbled),
    };
    let (mut named, mut unresolved) = (Vec::new(), Vec::new());
    let python = loop {
        match next()? {
            b"profile" => {
                let (name, path) = (next()?, next()?);
                let Some(profile) = text(name).and_then(Profile::folder) else {
                    continue;
                };
                match text(path) {
                    Some(path) if canonical(path) => named.push(FarHome {
                        profile,
                        path: path.to_owned(),
                    }),
                    Some(_) => {}
                    None => unresolved.push(profile),
                }
            }
            b"unresolved" => unresolved.extend(text(next()?).and_then(Profile::folder)),
            b"python" => break next()? == b"yes",
            _ => return Err(NoHomes::Garbled),
        }
    };
    // The end is said, and nothing after it.
    let end = (next()?, next()?);
    if end != (&b"end"[..], &b""[..]) || next().is_ok() {
        return Err(NoHomes::Garbled);
    }
    let root = root.filter(|root| canonical(root));
    let root = root.ok_or(NoHomes::RootNotCanonical)?.to_owned();
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
    unresolved.sort();
    unresolved.dedup();
    Ok(Homes {
        root,
        homes,
        unresolved,
    })
}
