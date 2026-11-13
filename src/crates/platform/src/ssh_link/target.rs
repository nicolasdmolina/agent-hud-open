//! Where a link leads. A `Target` exists only once its parts passed the
//! link's own check, so nothing unchecked can reach `ssh`'s command line:
//! a host or user that starts with a dash, or holds a space or a shell
//! character, makes no target. The rules are the connection list reader's,
//! held here as well so the link does not depend on who calls it.
use super::Refused;
use std::path::{Path, PathBuf};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Target {
    host: String,
    user: Option<String>,
    port: Option<u16>,
    key: Option<PathBuf>,
}

impl Target {
    /// `host` is a name, an address or an alias from the user's `ssh`
    /// configuration. `port` and `key` are passed only where given.
    pub fn new(
        host: &str,
        user: Option<&str>,
        port: Option<u16>,
        key: Option<&Path>,
    ) -> Result<Self, Refused> {
        if !name(host, 253) || user.is_some_and(|user| !name(user, 64)) || port == Some(0) {
            return Err(Refused::Address);
        }
        if key.is_some_and(|key| !key_path(key)) {
            return Err(Refused::KeyPath);
        }
        Ok(Self {
            host: host.to_owned(),
            user: user.map(str::to_owned),
            port,
            key: key.map(Path::to_owned),
        })
    }
    /// `user@host`, or the host alone.
    pub fn destination(&self) -> String {
        match &self.user {
            Some(user) => format!("{user}@{}", self.host),
            None => self.host.clone(),
        }
    }
    pub fn port(&self) -> Option<u16> {
        self.port
    }
    pub fn key(&self) -> Option<&Path> {
        self.key.as_deref()
    }
}

/// Letters, digits, dots, dashes and underscores, and no dash in front.
fn name(text: &str, longest: usize) -> bool {
    !text.is_empty()
        && text.len() <= longest
        && !text.starts_with('-')
        && text
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'-' | b'_'))
}

fn key_path(path: &Path) -> bool {
    path.to_str().is_some_and(|text| {
        !text.is_empty() && !text.starts_with('-') && !text.chars().any(char::is_control)
    })
}
