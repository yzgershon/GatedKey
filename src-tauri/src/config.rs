//! Bindings, actions, and the on-disk config.
//!
//! The config is plain JSON so it can be read, diffed and hand-edited. It holds
//! key bindings and nothing else: no telemetry, no identifiers, no history.

use serde::{Deserialize, Serialize};
use std::path::Path;

use crate::keys::Chord;

/// What a bound key does when pressed.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum Action {
    /// Run an executable directly.
    LaunchApp {
        path: String,
        #[serde(default)]
        args: String,
    },
    /// Hand a URL to the default browser.
    OpenUrl { url: String },
    /// Open a folder in Explorer.
    OpenFolder { path: String },
    /// Run a shell command.
    RunCommand { command: String },
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Binding {
    pub id: String,
    pub name: String,
    pub chord: Chord,
    pub action: Action,
    #[serde(default = "default_true")]
    pub enabled: bool,
}

fn default_true() -> bool {
    true
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Config {
    pub bindings: Vec<Binding>,
    /// Master switch. When false the hook stays installed but matches nothing,
    /// so toggling is instant and never leaves a key half-captured.
    pub enabled: bool,
    pub start_with_windows: bool,
    pub start_minimised: bool,
}

impl Default for Config {
    fn default() -> Self {
        Config {
            bindings: Vec::new(),
            enabled: true,
            // See the first-run note in lib.rs: bound keys stop working entirely
            // when the app is not running, so both of these default on.
            start_with_windows: true,
            start_minimised: true,
        }
    }
}

impl Config {
    /// Read the config, falling back to defaults. A corrupt file is moved aside
    /// rather than deleted, and rather than blocking startup.
    pub fn load(path: &Path) -> Self {
        let Ok(text) = std::fs::read_to_string(path) else {
            return Config::default();
        };
        // Notepad, and most Windows editors, save UTF-8 with a byte-order mark,
        // and serde_json refuses to parse one. The config is documented as
        // hand-editable, so without this, opening it in Notepad and pressing
        // save would silently throw away every binding the user had.
        let text = text.strip_prefix('\u{feff}').unwrap_or(text.as_str());
        match serde_json::from_str(text) {
            Ok(cfg) => cfg,
            Err(err) => {
                eprintln!("gatedkey: config unreadable ({err}), starting fresh");
                let _ = std::fs::rename(path, path.with_extension("json.corrupt"));
                Config::default()
            }
        }
    }

    /// Write via a temp file and rename, so an interrupted save leaves the old
    /// config intact instead of a truncated one that reads as "no bindings".
    pub fn save(&self, path: &Path) -> std::io::Result<()> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let text = serde_json::to_string_pretty(self)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
        let tmp = path.with_extension("json.tmp");
        std::fs::write(&tmp, text)?;
        std::fs::rename(&tmp, path)
    }
}

// GatedKey ships with no bindings on purpose. An app that swallows one of your
// keys before you have asked it to is indistinguishable from a broken keyboard.
// The Copilot quick-start lives in the UI, where the user picks the target.

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_through_json() {
        let cfg = Config {
            bindings: vec![Binding {
                id: "a".into(),
                name: "Test".into(),
                chord: Chord::copilot_key(),
                action: Action::OpenUrl {
                    url: "https://example.com".into(),
                },
                enabled: true,
            }],
            ..Config::default()
        };
        let text = serde_json::to_string(&cfg).unwrap();
        let back: Config = serde_json::from_str(&text).unwrap();
        assert_eq!(back.bindings.len(), 1);
        assert_eq!(back.bindings[0].chord, Chord::copilot_key());
        assert_eq!(back.bindings[0].action, cfg.bindings[0].action);
    }

    #[test]
    fn a_utf8_bom_does_not_destroy_the_config() {
        // Notepad writes one of these. Before this was handled, a hand-edited
        // config was treated as corrupt and every binding was lost.
        let dir = std::env::temp_dir().join("gatedkey-test-bom");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("config.json");

        let cfg = Config {
            bindings: vec![Binding {
                id: "a".into(),
                name: "Survives".into(),
                chord: Chord::copilot_key(),
                action: Action::OpenUrl {
                    url: "https://example.com".into(),
                },
                enabled: true,
            }],
            ..Config::default()
        };
        let json = serde_json::to_string(&cfg).unwrap();
        std::fs::write(&path, format!("\u{feff}{json}")).unwrap();

        let back = Config::load(&path);
        assert_eq!(
            back.bindings.len(),
            1,
            "BOM caused the config to be dropped"
        );
        assert_eq!(back.bindings[0].name, "Survives");
        // and it must not have been quarantined
        assert!(!path.with_extension("json.corrupt").exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn missing_file_yields_defaults_not_an_error() {
        let cfg = Config::load(Path::new("does-not-exist-anywhere.json"));
        assert!(cfg.bindings.is_empty());
        assert!(cfg.enabled);
    }

    #[test]
    fn save_then_load_survives_a_round_trip() {
        let dir = std::env::temp_dir().join("gatedkey-test-save");
        let _ = std::fs::remove_dir_all(&dir);
        let path = dir.join("config.json");
        let cfg = Config {
            start_with_windows: true,
            ..Config::default()
        };
        cfg.save(&path).unwrap();
        let back = Config::load(&path);
        assert!(back.start_with_windows);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
