#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

/// The installed executable that owns client hooks, named at build time by
/// `AGENT_HUD_HOOK_OWNER` (`src/tooling/fork-install.sh` sets it). Unset or
/// empty, the build owns no hooks, as upstream's does.
fn hook_owner(setting: Option<&str>) -> Option<std::path::PathBuf> {
    setting.filter(|path| !path.is_empty()).map(Into::into)
}

fn main() {
    let options = agent_hud_host::HostOptions {
        canonical_executable: hook_owner(option_env!("AGENT_HUD_HOOK_OWNER")),
    };
    agent_hud_host::run_with_options(tauri::generate_context!(), None, options);
}

#[cfg(test)]
mod tests {
    use super::hook_owner;

    #[test]
    fn only_a_named_path_owns_hooks() {
        assert_eq!(hook_owner(None), None);
        assert_eq!(hook_owner(Some("")), None);
        assert_eq!(
            hook_owner(Some(
                "/Applications/Agent HUD Fork.app/Contents/MacOS/Agent HUD Fork"
            )),
            Some("/Applications/Agent HUD Fork.app/Contents/MacOS/Agent HUD Fork".into())
        );
    }

    /// The fork never asks the author's feeds for an update (FORK.md,
    /// "Self-update"): no updater plugin in the build, no updater section in
    /// any Tauri config, and no product extension, which is where the
    /// official app's update check lives. Upstream publishes its feeds in
    /// `updates/`; a merge that wires them in fails here.
    #[test]
    fn the_fork_build_has_no_way_to_ask_for_an_update() {
        let lock = include_str!("../../../../../Cargo.lock");
        assert!(!lock.contains("tauri-plugin-updater"));
        for config in [
            include_str!("../tauri.conf.json"),
            include_str!("../tauri.fork.conf.json"),
            include_str!("../tauri.macos.conf.json"),
            include_str!("../tauri.windows.conf.json"),
            include_str!("../tauri.linux.conf.json"),
        ] {
            assert!(!config.contains("\"updater\""));
            assert!(!config.contains("createUpdaterArtifacts"));
            assert!(!config.contains("updates/"));
        }
        let app = include_str!("main.rs")
            .split("#[cfg(test)]")
            .next()
            .unwrap();
        assert!(app.contains("generate_context!(), None, options)"));
    }
}
