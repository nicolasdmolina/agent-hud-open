//! Hermes's gateway is asked through a named pipe on Windows
//! (`\\.\pipe\hermes-gateway-<hash>`). Not implemented: the channel's
//! descriptor says it does not run here, so nothing calls this.
use crate::clients::hermes::control::Silent;
use std::{path::Path, time::Duration};

pub(super) fn exchange(_home: &Path, _line: &[u8], _timeout: Duration) -> Result<Vec<u8>, Silent> {
    Err(Silent::Absent)
}
