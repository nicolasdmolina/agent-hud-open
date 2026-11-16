//! The bindings: for each entry of the connection list, the address it
//! last had and the machine its route last verified as, with the label
//! it went by. One file in the app's data folder, which holds entry ids,
//! address fields, keys and labels and nothing else.
//!
//! It is a cache and never proof. It says which stored sessions are a
//! machine's while that machine cannot be reached, under which label.
//! Nothing is dialled from it, and no route, poll, card or answer comes
//! of a key read from it: a key here is a `StoredKey`, which leads to no
//! machine (`agents/src/hermes/gateway/machine.rs`).
use agent_hud_agents::hermes::{
    connections::SshAddress,
    gateway::{MachineKey, StoredKey},
};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// The file's name in the app's data folder.
pub(crate) const FILE: &str = "hermes-machines.json";
const VERSION: u32 = 1;
/// More than any list holds: a longer file is not read.
const MOST: u64 = 256 * 1024;

#[derive(Default)]
pub(crate) struct Bindings {
    /// None for bindings that are kept in memory alone.
    path: Option<PathBuf>,
    bound: Vec<Binding>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Binding {
    entry: String,
    host: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    user: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    port: Option<u16>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    key_path: Option<String>,
    /// The machine's key, as it is written.
    key: String,
    label: String,
}

#[derive(Serialize, Deserialize)]
struct Saved {
    version: u32,
    bindings: Vec<Binding>,
}

impl Binding {
    fn leads(&self, address: &SshAddress) -> bool {
        self.host == address.host
            && self.user == address.user
            && self.port == address.port
            && self.key_path == address.key_path
    }
}

impl Bindings {
    /// The bindings saved in `data`. A file that is missing, too long,
    /// of another version or not the bindings' own reads as none; so
    /// does each binding whose key is not written as a key is.
    pub(crate) fn load(data: &Path) -> Self {
        let path = data.join(FILE);
        let short = std::fs::metadata(&path).is_ok_and(|file| file.len() <= MOST);
        let text = short.then(|| std::fs::read_to_string(&path).ok()).flatten();
        let saved = text.and_then(|text| serde_json::from_str::<Saved>(&text).ok());
        let mut bound = saved
            .filter(|saved| saved.version == VERSION)
            .map(|saved| saved.bindings)
            .unwrap_or_default();
        bound.retain(|binding| StoredKey::parse(&binding.key).is_some());
        Self {
            path: Some(path),
            bound,
        }
    }

    /// The entry `entry`, at `address`, verified as the machine `key`
    /// and goes by `label`.
    pub(super) fn record(
        &mut self,
        entry: &str,
        address: &SshAddress,
        key: &MachineKey,
        label: &str,
    ) {
        let binding = Binding {
            entry: entry.to_owned(),
            host: address.host.clone(),
            user: address.user.clone(),
            port: address.port,
            key_path: address.key_path.clone(),
            key: key.as_str().to_owned(),
            label: label.to_owned(),
        };
        match self.bound.iter_mut().find(|bound| bound.entry == entry) {
            Some(bound) if *bound == binding => return,
            Some(bound) => *bound = binding,
            None => self.bound.push(binding),
        }
        self.save();
    }

    pub(super) fn relabel(&mut self, entry: &str, label: &str) {
        let renamed = |bound: &&mut Binding| bound.entry == entry && bound.label != label;
        if let Some(bound) = self.bound.iter_mut().find(renamed) {
            bound.label = label.to_owned();
            self.save();
        }
    }

    /// Keeps the bindings of the entries `listed`, and drops the rest: an
    /// entry that left the list leads nowhere.
    pub(super) fn keep(&mut self, listed: &dyn Fn(&str) -> bool) {
        let before = self.bound.len();
        self.bound.retain(|bound| listed(&bound.entry));
        if self.bound.len() != before {
            self.save();
        }
    }

    /// The machine the entry `entry` verified as when it last had
    /// `address`. None for an entry whose address is another now.
    pub(super) fn of(&self, entry: &str, address: &SshAddress) -> Option<&str> {
        let its = |bound: &&Binding| bound.entry == entry && bound.leads(address);
        self.bound.iter().find(its).map(|bound| bound.key.as_str())
    }

    /// Every machine verified before, once each: its key and its label.
    pub(super) fn machines(&self) -> Vec<(&str, &str)> {
        let mut known: Vec<(&str, &str)> = Vec::new();
        for bound in &self.bound {
            if !known.iter().any(|(key, _)| *key == bound.key) {
                known.push((&bound.key, &bound.label));
            }
        }
        known
    }

    pub(super) fn is_empty(&self) -> bool {
        self.bound.is_empty()
    }

    /// Written whole, for this user alone. A write that fails loses a
    /// cache: the next verification writes it again.
    fn save(&self) {
        let Some(path) = &self.path else {
            return;
        };
        let saved = Saved {
            version: VERSION,
            bindings: self.bound.clone(),
        };
        if let Ok(text) = serde_json::to_vec_pretty(&saved) {
            let _ = crate::claude_plugin::write_private(path, &text);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use agent_hud_agents::hermes::gateway::machine_key;
    use serde_json::{json, Value};

    fn address(host: &str) -> SshAddress {
        SshAddress {
            host: host.to_owned(),
            user: Some("nicky".into()),
            port: Some(2222),
            key_path: Some("~/.ssh/far".into()),
        }
    }

    /// The app's data folder, made in a temporary one as the app makes it.
    fn data() -> (tempfile::TempDir, std::path::PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let data = dir.path().join("data");
        crate::os::private_directory(&data).unwrap();
        (dir, data)
    }

    fn key(home: &str) -> MachineKey {
        machine_key(Some("0123456789abcdef0123456789abcdef"), home).unwrap()
    }

    #[test]
    fn a_binding_is_saved_and_read_back_and_holds_only_its_four_kinds_of_field() {
        let (_dir, data) = data();
        let mut bindings = Bindings::load(&data);
        assert!(bindings.is_empty());
        bindings.record("a", &address("far"), &key("/far/.hermes"), "Mini");
        bindings.record(
            "b",
            &address("far.tail"),
            &key("/far/.hermes"),
            "Mini over VPN",
        );
        let saved: Value =
            serde_json::from_str(&std::fs::read_to_string(data.join(FILE)).unwrap()).unwrap();
        let key = key("/far/.hermes");
        assert_eq!(
            saved,
            json!({"version": 1, "bindings": [
                {"entry": "a", "host": "far", "user": "nicky", "port": 2222,
                 "keyPath": "~/.ssh/far", "key": key.as_str(), "label": "Mini"},
                {"entry": "b", "host": "far.tail", "user": "nicky", "port": 2222,
                 "keyPath": "~/.ssh/far", "key": key.as_str(), "label": "Mini over VPN"}]})
        );
        // Read back by a later launch: one machine, under its first label.
        let mut later = Bindings::load(&data);
        assert_eq!(later.machines(), [(key.as_str(), "Mini")]);
        assert_eq!(later.of("a", &address("far")), Some(key.as_str()));
        // An entry whose address is another now leads nowhere known.
        assert_eq!(later.of("a", &address("elsewhere")), None);
        assert_eq!(later.of("c", &address("far")), None);
        later.relabel("a", "Studio");
        later.keep(&|entry| entry == "a");
        let again = Bindings::load(&data);
        assert_eq!(again.machines(), [(key.as_str(), "Studio")]);
        assert_eq!(again.of("b", &address("far.tail")), None);
    }

    #[test]
    fn a_file_that_is_not_the_bindings_own_reads_as_none() {
        let (_dir, data) = data();
        let file = data.join(FILE);
        let good = json!({"entry": "a", "host": "far", "key": "0123456789abcdef", "label": "M"});
        let refused = [
            "not json".to_owned(),
            json!({"version": 2, "bindings": [good]}).to_string(),
            json!({"bindings": [good]}).to_string(),
            json!({"version": 1, "bindings": [{"entry": "a", "host": "far",
                "key": "0123456789ABCDEF", "label": "M"}]})
            .to_string(),
            json!({"version": 1, "bindings": [{"entry": "a", "host": "far",
                "key": "0123456789abcdef0123456789abcdef", "label": "M"}]})
            .to_string(),
        ];
        for text in refused {
            std::fs::write(&file, &text).unwrap();
            assert!(Bindings::load(&data).is_empty(), "{text}");
        }
        let kept = json!({"version": 1, "bindings": [good]}).to_string();
        std::fs::write(&file, kept).unwrap();
        assert_eq!(
            Bindings::load(&data).machines(),
            [("0123456789abcdef", "M")]
        );
    }
}
